//! A debugging session, driven the way a client drives one.
//!
//! Requests in, messages out, against a real program that really runs. A test
//! that faked the interpreter would be a test of the fake, and the thing worth
//! knowing about a debugger is whether the place it says the program is, is
//! the place the program is.

use std::path::{Path, PathBuf};

use deed_dap::{Next, Session};
use deed_lsp::Json;

/// A program whose lines are known, so a test can say which one it means.
///
/// ```text
///  1 module app
///  2
///  3 fn total(a: Int, b: Int) -> Int {
///  4     let sum = a + b
///  5     sum
///  6 }
///  7
///  8 fn main(sys: System) -> Int
///  9   uses
/// 10     Io.write,
/// 11 {
/// 12     let x = 2
/// 13     let y = 3
/// 14     let answer = total(x, y)
/// 15     Io.write(sys.console, "done")
/// 16     answer
/// 17 }
/// ```
const PROGRAM: &str = "module app\n\
     \n\
     fn total(a: Int, b: Int) -> Int {\n\
     \x20   let sum = a + b\n\
     \x20   sum\n\
     }\n\
     \n\
     fn main(sys: System) -> Int\n\
     \x20 uses\n\
     \x20   Io.write,\n\
     {\n\
     \x20   let x = 2\n\
     \x20   let y = 3\n\
     \x20   let answer = total(x, y)\n\
     \x20   Io.write(sys.console, \"done\")\n\
     \x20   answer\n\
     }\n";

struct Debugging {
    session: Session,
    program: PathBuf,
    dir: PathBuf,
    seq: i64,
    /// Everything the session has sent, in order, for the tests that are about
    /// the stream rather than about one answer.
    all: Vec<Json>,
}

impl Debugging {
    fn new(tag: &str, source: &str) -> Debugging {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("deed-dap-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("app.deed");
        std::fs::write(&program, source).unwrap();

        Debugging {
            session: Session::new(),
            program,
            dir,
            seq: 0,
            all: Vec::new(),
        }
    }

    fn ask(&mut self, command: &str, arguments: Json) -> Vec<Json> {
        self.seq += 1;
        let mut fields = vec![
            ("seq", Json::number(self.seq)),
            ("type", Json::string("request")),
            ("command", Json::string(command)),
        ];
        if arguments != Json::Null {
            fields.push(("arguments", arguments));
        }
        let (replies, _) = self.session.handle(&Json::object(fields));
        self.all.extend(replies.iter().cloned());
        replies
    }

    /// Everything up to and including `configurationDone`, with breakpoints on
    /// the given one-based lines of the program.
    fn launched(&mut self, lines: &[i64], stop_on_entry: bool) -> Vec<Json> {
        self.ask("initialize", Json::Null);
        let program = self.program.to_string_lossy().to_string();
        self.ask(
            "launch",
            Json::object(vec![
                ("program", Json::string(program.clone())),
                ("stopOnEntry", Json::Bool(stop_on_entry)),
            ]),
        );
        self.breakpoints(&program, lines);
        self.ask("configurationDone", Json::Null)
    }

    fn breakpoints(&mut self, path: &str, lines: &[i64]) -> Vec<Json> {
        let wanted = lines
            .iter()
            .map(|line| Json::object(vec![("line", Json::number(*line))]))
            .collect();
        self.ask(
            "setBreakpoints",
            Json::object(vec![
                ("source", Json::object(vec![("path", Json::string(path))])),
                ("breakpoints", Json::Array(wanted)),
            ]),
        )
    }

    fn stack(&mut self) -> Vec<Json> {
        let replies = self.stack_messages();
        replies[0]
            .at(&["body", "stackFrames"])
            .and_then(Json::as_array)
            .map(<[Json]>::to_vec)
            .unwrap_or_default()
    }

    fn stack_messages(&mut self) -> Vec<Json> {
        self.ask(
            "stackTrace",
            Json::object(vec![("threadId", Json::number(1))]),
        )
    }

    /// The names and values a frame can see, as `name = value`.
    fn variables(&mut self, frame: i64) -> Vec<String> {
        let scopes = self.ask(
            "scopes",
            Json::object(vec![("frameId", Json::number(frame))]),
        );
        let reference = scopes[0]
            .at(&["body", "scopes"])
            .and_then(Json::as_array)
            .and_then(|scopes| scopes.first())
            .and_then(|scope| scope.get("variablesReference"))
            .and_then(Json::as_i64)
            .expect("a scope should say what to ask for");

        let replies = self.ask(
            "variables",
            Json::object(vec![("variablesReference", Json::number(reference))]),
        );
        replies[0]
            .at(&["body", "variables"])
            .and_then(Json::as_array)
            .map(|variables| {
                variables
                    .iter()
                    .map(|variable| {
                        format!("{} = {}", text(variable, "name"), text(variable, "value"))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Drop for Debugging {
    fn drop(&mut self) {
        // The program is either finished or waiting to be told what to do
        // next, and dropping the session is what lets a waiting one go.
        self.ask("disconnect", Json::Null);
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn text(message: &Json, field: &str) -> String {
    message
        .get(field)
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The events among a batch of messages, by name.
fn events(messages: &[Json]) -> Vec<String> {
    messages
        .iter()
        .filter(|message| text(message, "type") == "event")
        .map(|message| text(message, "event"))
        .collect()
}

fn stopped_for(messages: &[Json]) -> Option<String> {
    messages
        .iter()
        .find(|message| text(message, "event") == "stopped")
        .map(|message| {
            message
                .at(&["body", "reason"])
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string()
        })
}

/// Everything an `output` event carried, in order.
fn written(messages: &[Json]) -> Vec<String> {
    messages
        .iter()
        .filter(|message| text(message, "event") == "output")
        .map(|message| {
            message
                .at(&["body", "output"])
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

// -- the handshake ---------------------------------------------------------

#[test]
fn a_session_says_what_it_can_do_and_then_says_it_is_ready() {
    let mut session = Session::new();
    let (replies, next) = session.handle(&Json::object(vec![
        ("seq", Json::number(1)),
        ("type", Json::string("request")),
        ("command", Json::string("initialize")),
    ]));

    assert_eq!(next, Next::Continue);
    assert_eq!(text(&replies[0], "command"), "initialize");
    assert_eq!(
        replies[0].at(&["body", "supportsConfigurationDoneRequest"]),
        Some(&Json::Bool(true))
    );
    assert_eq!(events(&replies), vec!["initialized"]);
}

/// A launch that names nothing is refused before anything is started, because
/// the alternative is a session that looks alive and never stops anywhere.
#[test]
fn a_launch_that_names_no_program_is_refused() {
    let mut session = Session::new();
    let (replies, _) = session.handle(&Json::object(vec![
        ("seq", Json::number(1)),
        ("type", Json::string("request")),
        ("command", Json::string("launch")),
        ("arguments", Json::object(vec![])),
    ]));

    assert_eq!(replies[0].get("success"), Some(&Json::Bool(false)));
    assert!(text(&replies[0], "message").contains("named no program"));
}

#[test]
fn a_launch_that_names_a_file_that_is_not_there_is_refused() {
    let mut session = Session::new();
    let (replies, _) = session.handle(&Json::object(vec![
        ("seq", Json::number(1)),
        ("type", Json::string("request")),
        ("command", Json::string("launch")),
        (
            "arguments",
            Json::object(vec![("program", Json::string("no/such/file.deed"))]),
        ),
    ]));

    assert_eq!(replies[0].get("success"), Some(&Json::Bool(false)));
    assert!(text(&replies[0], "message").contains("is not a file"));
}

#[test]
fn a_request_this_adapter_does_not_answer_is_refused_by_name() {
    let mut session = Session::new();
    let (replies, next) = session.handle(&Json::object(vec![
        ("seq", Json::number(1)),
        ("type", Json::string("request")),
        ("command", Json::string("pause")),
    ]));

    assert_eq!(next, Next::Continue);
    assert_eq!(replies[0].get("success"), Some(&Json::Bool(false)));
    assert!(text(&replies[0], "message").contains("`pause`"));
}

// -- stopping --------------------------------------------------------------

#[test]
fn a_breakpoint_stops_the_program_where_it_was_set() {
    let mut run = Debugging::new("breakpoint", PROGRAM);
    let started = run.launched(&[14], false);

    assert_eq!(stopped_for(&started).as_deref(), Some("breakpoint"));

    let stack = run.stack();
    assert_eq!(text(&stack[0], "name"), "main");
    assert_eq!(stack[0].get("line"), Some(&Json::number(14)));
    assert_eq!(stack.len(), 1, "only `main` is running");
}

#[test]
fn stopping_on_entry_stops_before_the_first_statement() {
    let mut run = Debugging::new("entry", PROGRAM);
    let started = run.launched(&[], true);

    assert_eq!(stopped_for(&started).as_deref(), Some("step"));
    let stack = run.stack();
    assert_eq!(stack[0].get("line"), Some(&Json::number(12)));
}

/// A program with nothing to stop for runs to the end, which is the case that
/// separates a debugger from a thing that hangs.
#[test]
fn a_program_with_no_breakpoints_runs_to_the_end() {
    let mut run = Debugging::new("no-breakpoints", PROGRAM);
    let started = run.launched(&[], false);

    assert_eq!(stopped_for(&started), None);
    assert_eq!(written(&started), vec!["done\n"]);
    assert_eq!(events(&started), vec!["output", "exited", "terminated"]);
    let exited = started
        .iter()
        .find(|message| text(message, "event") == "exited")
        .unwrap();
    assert_eq!(exited.at(&["body", "exitCode"]), Some(&Json::number(0)));
}

// -- stepping --------------------------------------------------------------

#[test]
fn stepping_in_enters_the_call_and_the_stack_says_so() {
    let mut run = Debugging::new("step-in", PROGRAM);
    run.launched(&[14], false);

    let stepped = run.ask("stepIn", Json::object(vec![("threadId", Json::number(1))]));
    assert_eq!(stopped_for(&stepped).as_deref(), Some("step"));

    let stack = run.stack();
    assert_eq!(text(&stack[0], "name"), "total", "innermost frame first");
    assert_eq!(stack[0].get("line"), Some(&Json::number(4)));
    assert_eq!(text(&stack[1], "name"), "main");
    assert_eq!(
        stack[1].get("line"),
        Some(&Json::number(14)),
        "the caller waits at the line it called from"
    );
}

/// The one that separates `next` from `stepIn`: the call happens, and nothing
/// stops inside it.
#[test]
fn stepping_over_a_call_does_not_enter_it() {
    let mut run = Debugging::new("step-over", PROGRAM);
    run.launched(&[14], false);

    let stepped = run.ask("next", Json::object(vec![("threadId", Json::number(1))]));
    assert_eq!(stopped_for(&stepped).as_deref(), Some("step"));

    let stack = run.stack();
    assert_eq!(stack.len(), 1, "still only `main`");
    assert_eq!(stack[0].get("line"), Some(&Json::number(15)));
}

#[test]
fn stepping_out_comes_back_to_the_caller() {
    let mut run = Debugging::new("step-out", PROGRAM);
    run.launched(&[4], false);
    assert_eq!(run.stack().len(), 2, "stopped inside `total`");

    let stepped = run.ask("stepOut", Json::object(vec![("threadId", Json::number(1))]));
    assert_eq!(stopped_for(&stepped).as_deref(), Some("step"));

    let stack = run.stack();
    assert_eq!(stack.len(), 1);
    assert_eq!(text(&stack[0], "name"), "main");
}

#[test]
fn continuing_from_a_breakpoint_runs_to_the_end() {
    let mut run = Debugging::new("continue", PROGRAM);
    run.launched(&[14], false);

    let carried_on = run.ask(
        "continue",
        Json::object(vec![("threadId", Json::number(1))]),
    );
    assert_eq!(written(&carried_on), vec!["done\n"]);
    assert_eq!(events(&carried_on), vec!["output", "exited", "terminated"]);
}

// -- what a stop can be asked ---------------------------------------------

#[test]
fn a_frame_shows_the_bindings_it_can_see() {
    let mut run = Debugging::new("variables", PROGRAM);
    run.launched(&[14], false);

    let seen = run.variables(0);
    assert!(seen.contains(&"x = 2".to_string()), "{seen:?}");
    assert!(seen.contains(&"y = 3".to_string()), "{seen:?}");
    assert!(
        !seen.iter().any(|line| line.starts_with("answer")),
        "a binding the statement has not made yet is not there: {seen:?}"
    );
}

/// The frame a client asks about is the one it gets. Reading the innermost
/// frame for every request is a debugger that shows the wrong values with no
/// sign that it is doing so.
#[test]
fn each_frame_shows_its_own_bindings() {
    let mut run = Debugging::new("two-frames", PROGRAM);
    run.launched(&[5], false);

    let inner = run.variables(0);
    assert!(inner.contains(&"sum = 5".to_string()), "{inner:?}");

    let outer = run.variables(1);
    assert!(outer.contains(&"x = 2".to_string()), "{outer:?}");
    assert!(
        !outer.iter().any(|line| line.starts_with("sum")),
        "`sum` belongs to `total`: {outer:?}"
    );
}

// -- the two bases ---------------------------------------------------------

/// The protocol lets a client count lines from zero, and one that does is
/// answered in its own terms rather than in ours.
///
/// The line a stop reports is not enough to hold this on its own: a breakpoint
/// is converted one way and a stop is converted back the other, so an adapter
/// that had both bases backwards would answer with the number it was given
/// while stopping a line away from it. What says where it really stopped is
/// what the frame can see, so this asks for that instead.
#[test]
fn a_client_that_counts_from_zero_is_answered_from_zero() {
    let mut run = Debugging::new("zero-based", PROGRAM);
    run.ask(
        "initialize",
        Json::object(vec![("linesStartAt1", Json::Bool(false))]),
    );
    let program = run.program.to_string_lossy().to_string();
    run.ask(
        "launch",
        Json::object(vec![("program", Json::string(program.clone()))]),
    );
    run.breakpoints(&program, &[13]);
    let started = run.ask("configurationDone", Json::Null);

    assert_eq!(stopped_for(&started).as_deref(), Some("breakpoint"));
    let stack = run.stack();
    assert_eq!(stack[0].get("line"), Some(&Json::number(13)));

    let seen = run.variables(0);
    assert!(
        seen.contains(&"y = 3".to_string()),
        "zero-based line 13 is `let answer`, so `let y` has already run: {seen:?}"
    );
    assert!(
        !seen.iter().any(|line| line.starts_with("answer")),
        "and `let answer` has not: {seen:?}"
    );
}

/// A column is what an editor puts the arrow on. Nothing else here reads one,
/// so without this it could be off by one, or zero for every stop, and every
/// other assertion in this file would still hold.
#[test]
fn a_stop_says_which_column_the_statement_starts_at() {
    let mut run = Debugging::new("column", PROGRAM);
    run.launched(&[14], false);

    let stack = run.stack();
    assert_eq!(
        stack[0].get("column"),
        Some(&Json::number(5)),
        "`let answer` is indented by four, and columns start at one"
    );
}

// -- output ----------------------------------------------------------------

/// What a program has printed by the time it stops, rather than only what it
/// printed by the time it ended.
///
/// A run collects its output and hands it over at the end, which is fine for a
/// test and useless for somebody watching one line at a time. This is the
/// difference between a debug console that fills in as you step and one that
/// stays empty until the program is over.
#[test]
fn what_a_program_printed_before_it_stopped_arrives_with_the_stop() {
    let mut run = Debugging::new("printing", PRINTING);
    let started = run.launched(&[8], false);

    assert_eq!(stopped_for(&started).as_deref(), Some("breakpoint"));
    assert_eq!(
        written(&started),
        vec!["before\n"],
        "the line was written before the breakpoint, so it should be here"
    );
    assert_eq!(events(&started), vec!["output", "stopped"]);

    // And it is not sent twice when the program goes on to end.
    let carried_on = run.ask(
        "continue",
        Json::object(vec![("threadId", Json::number(1))]),
    );
    assert_eq!(written(&carried_on), Vec::<String>::new());
}

const PRINTING: &str = "module app\n\
     \n\
     fn main(sys: System) -> Int\n\
     \x20 uses\n\
     \x20   Io.write,\n\
     {\n\
     \x20   Io.write(sys.console, \"before\")\n\
     \x20   let x = 1\n\
     \x20   x\n\
     }\n";

// -- the envelope ----------------------------------------------------------

/// Every message the protocol carries has a sequence number, and it goes up.
///
/// Nothing else in this file reads one, because nothing else needs to: the
/// answers are correlated by `request_seq`. A client is entitled to the
/// numbering anyway, and one that is not counting is one nobody would notice
/// was not counting.
#[test]
fn every_message_is_numbered_in_order() {
    let mut run = Debugging::new("numbered", PROGRAM);
    run.launched(&[14], false);
    run.stack_messages();
    // A refusal is a message like any other, and it is the one that would go
    // uncounted if only the answers a client wanted were.
    run.ask("pause", Json::object(vec![("threadId", Json::number(1))]));
    run.ask(
        "continue",
        Json::object(vec![("threadId", Json::number(1))]),
    );

    let numbers: Vec<i64> = run
        .all
        .iter()
        .map(|message| {
            message
                .get("seq")
                .and_then(Json::as_i64)
                .expect("every message carries a sequence number")
        })
        .collect();

    assert!(numbers.len() > 5, "{numbers:?}");
    assert_eq!(numbers[0], 1, "numbering starts at one: {numbers:?}");
    assert!(
        numbers.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "each message is one past the last: {numbers:?}"
    );
}

// -- breakpoints -----------------------------------------------------------

#[test]
fn a_breakpoint_in_a_file_that_is_there_is_verified() {
    let mut run = Debugging::new("verified", PROGRAM);
    run.ask("initialize", Json::Null);
    let program = run.program.to_string_lossy().to_string();
    let answered = run.breakpoints(&program, &[14]);

    let breakpoints = answered[0]
        .at(&["body", "breakpoints"])
        .and_then(Json::as_array)
        .unwrap()
        .to_vec();
    assert_eq!(breakpoints[0].get("verified"), Some(&Json::Bool(true)));
    assert_eq!(breakpoints[0].get("line"), Some(&Json::number(14)));
}

#[test]
fn a_breakpoint_in_a_file_that_is_not_there_is_not_verified() {
    let mut session = Session::new();
    let (replies, _) = session.handle(&Json::object(vec![
        ("seq", Json::number(1)),
        ("type", Json::string("request")),
        ("command", Json::string("setBreakpoints")),
        (
            "arguments",
            Json::object(vec![
                (
                    "source",
                    Json::object(vec![("path", Json::string("no/such/file.deed"))]),
                ),
                (
                    "breakpoints",
                    Json::Array(vec![Json::object(vec![("line", Json::number(1))])]),
                ),
            ]),
        ),
    ]));

    let breakpoints = replies[0]
        .at(&["body", "breakpoints"])
        .and_then(Json::as_array)
        .unwrap();
    assert_eq!(breakpoints[0].get("verified"), Some(&Json::Bool(false)));
}

/// Clearing is sending none, and a set that was not cleared has to survive it.
#[test]
fn clearing_one_file_leaves_another_alone() {
    let mut run = Debugging::new("cleared", PROGRAM);
    run.ask("initialize", Json::Null);
    let program = run.program.to_string_lossy().to_string();
    let other = run.dir.join("other.deed").to_string_lossy().to_string();

    run.breakpoints(&program, &[14]);
    run.breakpoints(&other, &[3]);
    run.breakpoints(&other, &[]);

    run.ask(
        "launch",
        Json::object(vec![("program", Json::string(program))]),
    );
    let started = run.ask("configurationDone", Json::Null);
    assert_eq!(
        stopped_for(&started).as_deref(),
        Some("breakpoint"),
        "clearing another file's breakpoints removed this one"
    );
}

// -- programs that do not work --------------------------------------------

/// A program that does not compile ends the same way `deed run` refuses it,
/// rather than through some second refusal only a debugger can produce.
#[test]
fn a_program_that_does_not_check_is_refused_with_its_diagnostics() {
    let broken = "module app\n\nfn main(sys: System) -> Int {\n    missing()\n}\n";
    let mut run = Debugging::new("broken", broken);
    let started = run.launched(&[], false);

    assert_eq!(events(&started), vec!["output", "exited", "terminated"]);
    let output = written(&started).join("");
    assert!(output.contains("DEED"), "{output}");
    let exited = started
        .iter()
        .find(|message| text(message, "event") == "exited")
        .unwrap();
    assert_eq!(exited.at(&["body", "exitCode"]), Some(&Json::number(1)));
}

#[test]
fn a_program_that_fails_at_runtime_says_so_and_exits_nonzero() {
    let failing = "module app\n\n\
         fn main(sys: System) -> Int {\n\
         \x20   assert 1 == 2\n\
         \x20   0\n\
         }\n";
    let mut run = Debugging::new("failing", failing);
    let started = run.launched(&[], false);

    let output = written(&started).join("");
    assert!(output.contains("DEED"), "{output}");
    let exited = started
        .iter()
        .find(|message| text(message, "event") == "exited")
        .unwrap();
    assert_eq!(exited.at(&["body", "exitCode"]), Some(&Json::number(1)));
}

// -- imports ---------------------------------------------------------------

/// A call into another module is a frame like any other, and the module a
/// frame is in is what the source of that frame says.
#[test]
fn a_call_into_another_module_can_be_stepped_into() {
    let mut run = Debugging::new("imports", "");
    let root = run.dir.clone();
    write(&root.join("app.deed"), IMPORTING);
    write(&root.join("far.deed"), IMPORTED);

    run.launched(&[7], false);
    let stepped = run.ask("stepIn", Json::object(vec![("threadId", Json::number(1))]));
    assert_eq!(stopped_for(&stepped).as_deref(), Some("step"));

    let stack = run.stack();
    assert_eq!(text(&stack[0], "name"), "answer");
    assert_eq!(
        stack[0].at(&["source", "name"]).and_then(Json::as_str),
        Some("far"),
        "the frame should name the module it is written in"
    );
}

const IMPORTING: &str = "module app\n\
     \n\
     use far.{answer}\n\
     \n\
     fn main(sys: System) -> Int\n\
     {\n\
     \x20   let got = answer()\n\
     \x20   got\n\
     }\n";

const IMPORTED: &str = "module far\n\
     \n\
     fn answer() -> Int {\n\
     \x20   let n = 42\n\
     \x20   n\n\
     }\n";

fn write(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
}

// -- the stream ------------------------------------------------------------

/// The framing is the language server's, so this checks that the adapter is
/// reachable through it rather than only through `handle`.
#[test]
fn a_framed_stream_is_read_and_answered() {
    let answered = served(&[r#"{"seq":1,"type":"request","command":"initialize"}"#]);
    assert!(answered.contains("Content-Length: "), "{answered}");
    assert!(answered.contains("\"initialized\""), "{answered}");
}

/// The loop reads until it is told to stop, and then stops.
///
/// Both halves matter and neither is visible from `handle`. A loop that
/// carried on past a disconnect would answer a client that has gone; one that
/// stopped after every message would answer the first request an editor sends
/// and then appear to hang.
#[test]
fn the_stream_ends_at_a_disconnect_and_not_before() {
    let carried_on = served(&[
        r#"{"seq":1,"type":"request","command":"initialize"}"#,
        r#"{"seq":2,"type":"request","command":"threads"}"#,
    ]);
    assert!(
        carried_on.contains("\"threads\""),
        "the second request should have been answered: {carried_on}"
    );

    let stopped = served(&[
        r#"{"seq":1,"type":"request","command":"disconnect"}"#,
        r#"{"seq":2,"type":"request","command":"threads"}"#,
    ]);
    assert!(
        !stopped.contains("\"threads\""),
        "nothing after a disconnect should be answered: {stopped}"
    );
}

fn served(messages: &[&str]) -> String {
    let mut framed = String::new();
    for message in messages {
        framed.push_str(&format!(
            "Content-Length: {}\r\n\r\n{message}",
            message.len()
        ));
    }
    let mut input = std::io::Cursor::new(framed.into_bytes());
    let mut output = Vec::new();
    deed_dap::serve(&mut input, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[test]
fn disconnecting_ends_the_session() {
    let mut session = Session::new();
    let (replies, next) = session.handle(&Json::object(vec![
        ("seq", Json::number(1)),
        ("type", Json::string("request")),
        ("command", Json::string("disconnect")),
    ]));

    assert_eq!(next, Next::Stop);
    assert_eq!(replies[0].get("success"), Some(&Json::Bool(true)));
}
