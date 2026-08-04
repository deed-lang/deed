//! What a step is, and when to stop.
//!
//! All of it is here rather than in the interpreter. The interpreter offers a
//! point before each statement and knows nothing about lines, breakpoints or
//! what a client meant by "over", and adding that knowledge to it would put
//! the same decision in two crates.
//!
//! The stepper runs on the program's thread. When it decides to stop it sends
//! what it can see and waits for a command, which is what holds the program
//! still: the call it is inside has not returned, so nothing has unwound and
//! nothing has to be rebuilt.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

use deed_diagnostics::{FileId, SourceMap};
use deed_interp::{Paused, Watcher};
use deed_lsp::Lines;

/// Where a stop happened, in the terms a client uses.
///
/// Lines and columns rather than byte offsets, and rendered values rather than
/// values. Everything here crosses a thread boundary, and a `Value` holds
/// reference counts that belong to the thread that made them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameInfo {
    pub function: String,
    pub module: String,
    pub path: String,
    /// Zero based. The protocol's own base is applied when it is written out,
    /// because a client may ask for either and the conversion belongs at the
    /// edge rather than in every producer.
    pub line: u32,
    pub column: u32,
    pub variables: Vec<(String, String)>,
}

/// A program held still.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stopped {
    pub reason: &'static str,
    /// Innermost first, as `stackTrace` wants it.
    pub frames: Vec<FrameInfo>,
    /// Everything written through a `Console` so far, not only what is new.
    /// The session tracks what it has already reported, because it is the one
    /// that knows what it sent.
    pub output: Vec<String>,
}

/// A program that is no longer running, for whatever reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ended {
    pub output: Vec<String>,
    /// What went wrong, rendered. `None` when the program finished.
    ///
    /// A program that does not compile ends this way too. That is the same
    /// answer `deed run` gives, and a debugger that had its own way of
    /// refusing a broken program would be a second thing to keep true.
    pub failure: Option<String>,
    pub exit: i32,
}

/// What the program's thread sends back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Stopped(Box<Stopped>),
    Ended(Box<Ended>),
}

/// What a client asked for next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Run until a breakpoint.
    Go,
    /// Stop at the next statement anywhere, which is stepping in.
    In,
    /// Stop at the next statement no deeper than this, which is stepping over.
    Over(usize),
    /// Stop at the next statement shallower than this, which is stepping out.
    Out(usize),
}

/// Where a client wants the program to stop, by file and line.
///
/// Carried with every resume rather than set once. A client may move a
/// breakpoint while the program is held still, and sending the whole set each
/// time means the stepper cannot hold a stale one.
pub type Breakpoints = Vec<(String, Vec<u32>)>;

/// A resume, and what to stop for after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub mode: Mode,
    pub breakpoints: Breakpoints,
}

/// The [`Watcher`] the adapter installs.
pub struct Stepper {
    files: HashMap<FileId, (String, Lines, String)>,
    breakpoints: HashMap<FileId, Vec<u32>>,
    mode: Mode,
    events: Sender<Event>,
    commands: Receiver<Command>,
}

impl Stepper {
    pub fn new(
        sources: &SourceMap,
        ids: &[FileId],
        mode: Mode,
        breakpoints: &Breakpoints,
        events: Sender<Event>,
        commands: Receiver<Command>,
    ) -> Stepper {
        let mut files = HashMap::new();
        for id in ids {
            let file = sources.file(*id);
            let text = file.text().to_string();
            files.insert(*id, (file.name().to_string(), Lines::of(&text), text));
        }

        let mut stepper = Stepper {
            files,
            breakpoints: HashMap::new(),
            mode,
            events,
            commands,
        };
        stepper.set_breakpoints(breakpoints);
        stepper
    }

    /// Turns paths into files, dropping any that name nothing being run.
    ///
    /// A breakpoint in a file that is not part of this program is not an
    /// error. An editor sets them in whatever is open, and a client is told
    /// which ones were taken by the `setBreakpoints` response.
    fn set_breakpoints(&mut self, wanted: &Breakpoints) {
        self.breakpoints.clear();
        for (path, lines) in wanted {
            for (id, (name, _, _)) in &self.files {
                if same_file(name, path) {
                    self.breakpoints.insert(*id, lines.clone());
                    break;
                }
            }
        }
    }

    /// The zero based line and column an offset lands on.
    fn place(&self, file: FileId, offset: u32) -> Option<(String, u32, u32)> {
        let (name, lines, text) = self.files.get(&file)?;
        let position = lines.position(text, offset);
        Some((name.clone(), position.line, position.character))
    }
}

impl Watcher for Stepper {
    fn at(&mut self, paused: Paused<'_, '_>) {
        let Some((_, line, _)) = self.place(paused.file(), paused.span().start) else {
            return;
        };
        let depth = paused.depth();

        let on_a_breakpoint = self
            .breakpoints
            .get(&paused.file())
            .is_some_and(|lines| lines.contains(&line));

        let stepped = match self.mode {
            Mode::Go => false,
            Mode::In => true,
            Mode::Over(from) => depth <= from,
            Mode::Out(from) => depth < from,
        };

        if !on_a_breakpoint && !stepped {
            return;
        }

        // A breakpoint is the reason even when a step landed on one, because
        // that is the one a reader set on purpose.
        let reason = if on_a_breakpoint {
            "breakpoint"
        } else {
            "step"
        };
        let frames = paused
            .stack()
            .into_iter()
            .filter_map(|frame| {
                let (path, line, column) = self.place(frame.file, frame.span.start)?;
                Some(FrameInfo {
                    function: frame.function,
                    module: frame.module,
                    path,
                    line,
                    column,
                    variables: frame.variables,
                })
            })
            .collect();

        let stopped = Stopped {
            reason,
            frames,
            output: paused.output().to_vec(),
        };
        if self.events.send(Event::Stopped(Box::new(stopped))).is_err() {
            return;
        }

        // Blocking here is the whole of suspension. A client that goes away
        // closes the channel, and then the only useful thing left is to let
        // the program finish rather than to hold a thread for nobody.
        match self.commands.recv() {
            Ok(command) => {
                self.mode = command.mode;
                self.set_breakpoints(&command.breakpoints);
            }
            Err(_) => self.mode = Mode::Go,
        }
    }
}

/// Whether two spellings name the same file.
///
/// A client sends the path it has, which is absolute and in the platform's own
/// spelling, and the compiler holds the one it was given. Comparing the text
/// alone would mean a breakpoint set in an editor never binds, which is a
/// debugger that appears to work and stops at nothing.
fn same_file(one: &str, other: &str) -> bool {
    if one == other {
        return true;
    }
    match (
        std::fs::canonicalize(one).ok(),
        std::fs::canonicalize(other).ok(),
    ) {
        (Some(left), Some(right)) => left == right,
        // A file that is not on disk is a shipped module or something the
        // caller made up, and neither can be the file an editor is showing.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::same_file;

    #[test]
    fn a_path_is_the_same_file_as_itself() {
        assert!(same_file("a/b.deed", "a/b.deed"));
    }

    #[test]
    fn two_spellings_of_one_file_are_the_same_file() {
        let dir = std::env::temp_dir().join(format!("deed-dap-same-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.deed");
        std::fs::write(&path, "module a\n").unwrap();

        let indirect = dir.join(".").join("a.deed");
        assert!(same_file(
            path.to_str().unwrap(),
            indirect.to_str().unwrap()
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two files that do not exist are not each other. Falling back to string
    /// equality alone would make every shipped module a candidate for every
    /// breakpoint whose path happened not to resolve.
    #[test]
    fn different_names_that_are_not_on_disk_are_not_the_same_file() {
        assert!(!same_file(
            "<shipped>/std/list.deed",
            "/somewhere/list.deed"
        ));
    }
}
