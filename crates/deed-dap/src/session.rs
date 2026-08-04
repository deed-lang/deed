//! One debugging session, as a stream of messages in and out.
//!
//! [`Session::handle`] is synchronous, including the requests that resume the
//! program: it comes back with the events up to the next stop. A debug adapter
//! is usually written the other way, with a reader thread and events arriving
//! whenever, and that shape is untestable without timing. This one is a
//! function from a message to messages, which is the same thing
//! `deed_lsp::Server` is and for the same reason.
//!
//! What it costs is `pause`. While the program is running nothing is reading
//! the client's stream, so a request to interrupt a program that is not going
//! to stop on its own cannot arrive. A program with no breakpoints in it and
//! no end runs until the process is killed, which is written down in
//! `design/decisions/2026-08-04-a-place-to-stand.md` rather than discovered.

use std::path::PathBuf;

use deed_lsp::Json;

use crate::running::Running;
use crate::stepper::{Breakpoints, Event, Mode, Stopped};

/// Whether the adapter should keep reading.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Next {
    Continue,
    Stop,
}

/// One client, one program.
pub struct Session {
    seq: i64,
    /// What the client counts lines and columns from. The protocol's own
    /// default for both is one, and a client that says nothing gets it.
    lines_start_at_1: bool,
    columns_start_at_1: bool,
    program: Option<PathBuf>,
    stop_on_entry: bool,
    breakpoints: Breakpoints,
    running: Option<Running>,
    stopped: Option<Stopped>,
    /// How many output lines have already been sent as events.
    reported: usize,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

impl Session {
    pub fn new() -> Session {
        Session {
            seq: 0,
            lines_start_at_1: true,
            columns_start_at_1: true,
            program: None,
            stop_on_entry: false,
            breakpoints: Vec::new(),
            running: None,
            stopped: None,
            reported: 0,
        }
    }

    /// Answers one message, with whatever events it caused.
    pub fn handle(&mut self, message: &Json) -> (Vec<Json>, Next) {
        let command = message.get("command").and_then(Json::as_str).unwrap_or("");
        let seq = message.get("seq").and_then(Json::as_i64).unwrap_or(0);
        let arguments = message.get("arguments");

        match command {
            "initialize" => {
                let body = Json::object(vec![
                    ("supportsConfigurationDoneRequest", Json::Bool(true)),
                    ("supportsTerminateRequest", Json::Bool(true)),
                ]);
                self.lines_start_at_1 = flag(arguments, "linesStartAt1");
                self.columns_start_at_1 = flag(arguments, "columnsStartAt1");
                let mut replies = vec![self.responded(seq, command, body)];
                replies.push(self.event("initialized", Json::Null));
                (replies, Next::Continue)
            }

            "launch" => {
                let program = arguments
                    .and_then(|arguments| arguments.get("program"))
                    .and_then(Json::as_str);
                let Some(program) = program else {
                    return (
                        vec![self.refused(seq, command, "this launch named no program")],
                        Next::Continue,
                    );
                };
                let path = PathBuf::from(program);
                if !path.is_file() {
                    let why = format!("`{program}` is not a file");
                    return (vec![self.refused(seq, command, &why)], Next::Continue);
                }
                self.program = Some(path);
                self.stop_on_entry = arguments
                    .and_then(|arguments| arguments.get("stopOnEntry"))
                    .map(|value| value == &Json::Bool(true))
                    .unwrap_or(false);
                (
                    vec![self.responded(seq, command, Json::Null)],
                    Next::Continue,
                )
            }

            "setBreakpoints" => {
                let (body, taken) = self.take_breakpoints(arguments);
                self.remember(taken);
                (vec![self.responded(seq, command, body)], Next::Continue)
            }

            "configurationDone" => {
                let mut replies = vec![self.responded(seq, command, Json::Null)];
                replies.extend(self.start());
                (replies, Next::Continue)
            }

            "threads" => {
                let body = Json::object(vec![(
                    "threads",
                    Json::Array(vec![Json::object(vec![
                        ("id", Json::number(1)),
                        ("name", Json::string("main")),
                    ])]),
                )]);
                (vec![self.responded(seq, command, body)], Next::Continue)
            }

            "stackTrace" => {
                let body = self.stack_trace();
                (vec![self.responded(seq, command, body)], Next::Continue)
            }

            "scopes" => {
                let frame = arguments
                    .and_then(|arguments| arguments.get("frameId"))
                    .and_then(Json::as_i64)
                    .unwrap_or(0);
                let body = Json::object(vec![(
                    "scopes",
                    Json::Array(vec![Json::object(vec![
                        ("name", Json::string("Locals")),
                        // Zero means "nothing to expand", so a reference is
                        // one more than the frame it stands for.
                        ("variablesReference", Json::number(frame + 1)),
                        ("expensive", Json::Bool(false)),
                    ])]),
                )]);
                (vec![self.responded(seq, command, body)], Next::Continue)
            }

            "variables" => {
                let reference = arguments
                    .and_then(|arguments| arguments.get("variablesReference"))
                    .and_then(Json::as_i64)
                    .unwrap_or(0);
                let body = self.variables(reference);
                (vec![self.responded(seq, command, body)], Next::Continue)
            }

            "continue" => {
                let body = Json::object(vec![("allThreadsContinued", Json::Bool(true))]);
                let mut replies = vec![self.responded(seq, command, body)];
                replies.extend(self.resume(Mode::Go));
                (replies, Next::Continue)
            }

            "next" | "stepIn" | "stepOut" => {
                let depth = self.depth();
                let mode = match command {
                    "next" => Mode::Over(depth),
                    "stepOut" => Mode::Out(depth),
                    _ => Mode::In,
                };
                let mut replies = vec![self.responded(seq, command, Json::Null)];
                replies.extend(self.resume(mode));
                (replies, Next::Continue)
            }

            "disconnect" | "terminate" => {
                if let Some(running) = &mut self.running {
                    running.finish();
                }
                self.running = None;
                (vec![self.responded(seq, command, Json::Null)], Next::Stop)
            }

            other => {
                let why = format!("this adapter does not answer `{other}`");
                (vec![self.refused(seq, command, &why)], Next::Continue)
            }
        }
    }

    /// Starts the program, and runs it to its first stop.
    fn start(&mut self) -> Vec<Json> {
        let Some(program) = self.program.clone() else {
            return vec![self.event(
                "output",
                Json::object(vec![
                    ("category", Json::string("stderr")),
                    (
                        "output",
                        Json::string("nothing was launched, so there is nothing to debug\n"),
                    ),
                ]),
            )];
        };

        let mode = if self.stop_on_entry {
            Mode::In
        } else {
            Mode::Go
        };
        self.running = Some(Running::start(program, mode, self.breakpoints.clone()));
        self.pump()
    }

    /// Lets the program go, and waits for whatever happens next.
    fn resume(&mut self, mode: Mode) -> Vec<Json> {
        let breakpoints = self.breakpoints.clone();
        let Some(running) = &mut self.running else {
            return Vec::new();
        };
        running.resume(mode, breakpoints);
        self.pump()
    }

    /// Turns the next thing the program says into events.
    fn pump(&mut self) -> Vec<Json> {
        let Some(running) = &mut self.running else {
            return Vec::new();
        };

        match running.next_event() {
            Event::Stopped(stopped) => {
                let mut events = self.written(&stopped.output);
                self.stopped = Some(*stopped.clone());
                events.push(self.event(
                    "stopped",
                    Json::object(vec![
                        ("reason", Json::string(stopped.reason)),
                        ("threadId", Json::number(1)),
                        ("allThreadsStopped", Json::Bool(true)),
                    ]),
                ));
                events
            }
            Event::Ended(ended) => {
                let mut events = self.written(&ended.output);
                if let Some(failure) = &ended.failure {
                    events.push(self.event(
                        "output",
                        Json::object(vec![
                            ("category", Json::string("stderr")),
                            ("output", Json::string(format!("{failure}\n"))),
                        ]),
                    ));
                }
                events.push(self.event(
                    "exited",
                    Json::object(vec![("exitCode", Json::number(i64::from(ended.exit)))]),
                ));
                events.push(self.event("terminated", Json::Null));
                self.stopped = None;
                self.running = None;
                events
            }
        }
    }

    /// Output events for the lines that have not been reported yet.
    fn written(&mut self, output: &[String]) -> Vec<Json> {
        let fresh: Vec<String> = output
            .iter()
            .skip(self.reported)
            .map(String::clone)
            .collect();
        self.reported = output.len();
        fresh
            .into_iter()
            .map(|line| {
                self.event(
                    "output",
                    Json::object(vec![
                        ("category", Json::string("stdout")),
                        ("output", Json::string(format!("{line}\n"))),
                    ]),
                )
            })
            .collect()
    }

    /// How many calls are active where the program is held.
    fn depth(&self) -> usize {
        self.stopped
            .as_ref()
            .map(|stopped| stopped.frames.len())
            .unwrap_or(0)
    }

    fn stack_trace(&self) -> Json {
        let Some(stopped) = &self.stopped else {
            return Json::object(vec![
                ("stackFrames", Json::Array(Vec::new())),
                ("totalFrames", Json::number(0)),
            ]);
        };

        let frames: Vec<Json> = stopped
            .frames
            .iter()
            .enumerate()
            .map(|(index, frame)| {
                Json::object(vec![
                    ("id", Json::number(index as i64)),
                    ("name", Json::string(&frame.function)),
                    (
                        "source",
                        Json::object(vec![
                            ("name", Json::string(&frame.module)),
                            ("path", Json::string(&frame.path)),
                        ]),
                    ),
                    ("line", Json::number(self.line(frame.line))),
                    ("column", Json::number(self.column(frame.column))),
                ])
            })
            .collect();

        Json::object(vec![
            ("totalFrames", Json::number(frames.len() as i64)),
            ("stackFrames", Json::Array(frames)),
        ])
    }

    fn variables(&self, reference: i64) -> Json {
        let empty = Json::object(vec![("variables", Json::Array(Vec::new()))]);
        let Some(stopped) = &self.stopped else {
            return empty;
        };
        let Ok(index) = usize::try_from(reference - 1) else {
            return empty;
        };
        let Some(frame) = stopped.frames.get(index) else {
            return empty;
        };

        let variables: Vec<Json> = frame
            .variables
            .iter()
            .map(|(name, value)| {
                Json::object(vec![
                    ("name", Json::string(name)),
                    ("value", Json::string(value)),
                    // Nothing here expands. A value is rendered whole, so
                    // there is no second request that could disagree with the
                    // first about what it says.
                    ("variablesReference", Json::number(0)),
                ])
            })
            .collect();
        Json::object(vec![("variables", Json::Array(variables))])
    }

    /// Reads a `setBreakpoints` request, and says which ones were taken.
    ///
    /// `verified` is whether the file is one that could hold a breakpoint at
    /// all, which is all that can be known before the program is compiled. A
    /// breakpoint in a file this program does not use is verified and never
    /// reached, and that is the honest answer rather than a guess.
    fn take_breakpoints(&self, arguments: Option<&Json>) -> (Json, Option<(String, Vec<u32>)>) {
        let path = arguments
            .and_then(|arguments| arguments.at(&["source", "path"]))
            .and_then(Json::as_str);
        let Some(path) = path else {
            return (
                Json::object(vec![("breakpoints", Json::Array(Vec::new()))]),
                None,
            );
        };

        let verified = std::path::Path::new(path).is_file();
        let wanted = arguments
            .and_then(|arguments| arguments.get("breakpoints"))
            .and_then(Json::as_array)
            .map(<[Json]>::to_vec)
            .unwrap_or_default();

        let mut lines = Vec::new();
        let mut answered = Vec::new();
        for breakpoint in &wanted {
            let Some(line) = breakpoint.get("line").and_then(Json::as_i64) else {
                continue;
            };
            lines.push(self.zero_based(line));
            answered.push(Json::object(vec![
                ("verified", Json::Bool(verified)),
                ("line", Json::number(line)),
            ]));
        }

        (
            Json::object(vec![("breakpoints", Json::Array(answered))]),
            Some((path.to_string(), lines)),
        )
    }

    /// Replaces the breakpoints for one file, leaving the others alone.
    ///
    /// A client sends the whole set for one source at a time, and an empty set
    /// is how it clears them, so a file that arrives with nothing must lose
    /// what it had rather than keep it.
    fn remember(&mut self, taken: Option<(String, Vec<u32>)>) {
        let Some((path, lines)) = taken else {
            return;
        };
        self.breakpoints.retain(|(known, _)| known != &path);
        if !lines.is_empty() {
            self.breakpoints.push((path, lines));
        }
    }

    // -- the two bases -----------------------------------------------------

    fn line(&self, line: u32) -> i64 {
        i64::from(line) + i64::from(self.lines_start_at_1)
    }

    fn column(&self, column: u32) -> i64 {
        i64::from(column) + i64::from(self.columns_start_at_1)
    }

    fn zero_based(&self, line: i64) -> u32 {
        let zero = line - i64::from(self.lines_start_at_1);
        u32::try_from(zero.max(0)).unwrap_or(0)
    }

    // -- messages ----------------------------------------------------------

    fn responded(&mut self, request: i64, command: &str, body: Json) -> Json {
        self.seq += 1;
        let mut fields = vec![
            ("seq", Json::number(self.seq)),
            ("type", Json::string("response")),
            ("request_seq", Json::number(request)),
            ("success", Json::Bool(true)),
            ("command", Json::string(command)),
        ];
        if body != Json::Null {
            fields.push(("body", body));
        }
        Json::object(fields)
    }

    fn refused(&mut self, request: i64, command: &str, why: &str) -> Json {
        self.seq += 1;
        Json::object(vec![
            ("seq", Json::number(self.seq)),
            ("type", Json::string("response")),
            ("request_seq", Json::number(request)),
            ("success", Json::Bool(false)),
            ("command", Json::string(command)),
            ("message", Json::string(why)),
        ])
    }

    fn event(&mut self, event: &str, body: Json) -> Json {
        self.seq += 1;
        let mut fields = vec![
            ("seq", Json::number(self.seq)),
            ("type", Json::string("event")),
            ("event", Json::string(event)),
        ];
        if body != Json::Null {
            fields.push(("body", body));
        }
        Json::object(fields)
    }
}

/// A boolean argument that defaults to true, which is what the protocol says
/// both of the ones read here do.
fn flag(arguments: Option<&Json>, name: &str) -> bool {
    arguments
        .and_then(|arguments| arguments.get(name))
        .map(|value| value != &Json::Bool(false))
        .unwrap_or(true)
}
