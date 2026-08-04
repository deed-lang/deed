//! The program's own thread.
//!
//! A debugged program runs somewhere the adapter is not, so that the adapter
//! can keep answering while the program is held still. Nothing else about it
//! is different: it is compiled the way `deed run` compiles it and run by the
//! same interpreter, with a watcher installed.
//!
//! Everything that crosses between the two is plain data. The syntax tree, the
//! resolutions and the values all stay on the thread that made them, which is
//! what lets the interpreter go on using `Rc` for the things it copies most.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use deed_diagnostics::SourceMap;

use crate::stepper::{Breakpoints, Command, Ended, Event, Mode, Stepper};

/// A program being debugged.
pub struct Running {
    events: Receiver<Event>,
    commands: Sender<Command>,
    handle: Option<JoinHandle<()>>,
}

impl Running {
    /// Starts `program` and lets it run until its watcher stops it.
    pub fn start(program: PathBuf, mode: Mode, breakpoints: Breakpoints) -> Running {
        let (events, from_program) = channel();
        let (commands, to_program) = channel();
        let ending = events.clone();

        let handle = std::thread::spawn(move || {
            let ended = run(&program, mode, &breakpoints, events, to_program);
            // A send that fails means the session has gone. There is nobody
            // left to tell and nothing left to do.
            let _ = ending.send(Event::Ended(Box::new(ended)));
        });

        Running {
            events: from_program,
            commands,
            handle: Some(handle),
        }
    }

    /// Waits for the program to stop or to end.
    ///
    /// A channel that closes without an `Ended` means the thread went away
    /// without saying so, which nothing here can do, so it is reported as a
    /// program that ended rather than as a session that hangs.
    pub fn next_event(&mut self) -> Event {
        self.events.recv().unwrap_or_else(|_| {
            Event::Ended(Box::new(Ended {
                output: Vec::new(),
                failure: Some("the program stopped without saying why".to_string()),
                exit: 1,
            }))
        })
    }

    pub fn resume(&mut self, mode: Mode, breakpoints: Breakpoints) {
        let _ = self.commands.send(Command { mode, breakpoints });
    }

    /// Lets a held program go and waits for its thread.
    ///
    /// Dropping the command channel is what releases a stepper that is waiting
    /// for an answer: its `recv` fails, it stops asking, and the program runs
    /// to the end. Waiting for that is what makes a disconnect leave nothing
    /// behind.
    pub fn finish(&mut self) {
        let (dead, _) = channel();
        self.commands = dead;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Compiles and runs one program with a watcher installed.
fn run(
    program: &Path,
    mode: Mode,
    breakpoints: &Breakpoints,
    events: Sender<Event>,
    commands: Receiver<Command>,
) -> Ended {
    let Ok(text) = std::fs::read_to_string(program) else {
        return refused(format!("`{}` could not be read", display(program)));
    };

    let mut sources = SourceMap::new();
    let mut ids = vec![sources.add(display(program), text.clone())];
    for (name, found) in imports(program, &text) {
        ids.push(sources.add(name, found));
    }

    let checks = deed_driver::check_all(&sources, &ids);
    let refusals: Vec<String> = checks
        .iter()
        .flat_map(|checked| &checked.diagnostics)
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| deed_diagnostics::render_human(&sources, diagnostic))
        .collect();
    if !refusals.is_empty() {
        return refused(refusals.join("\n"));
    }

    let stepper = Stepper::new(&sources, &ids, mode, breakpoints, events, commands);
    let compiled = deed_driver::program_of(&checks);
    let root = program.parent().unwrap_or(Path::new(".")).to_path_buf();

    let run = deed_interp::run_main_watched(
        &compiled,
        ids[0],
        &root,
        &[],
        &deed_rt::Reach::none(),
        Box::new(stepper),
    );

    let Some(run) = run else {
        return refused(deed_driver::NOTHING_TO_RUN.to_string());
    };

    let failure = run
        .result
        .as_ref()
        .err()
        .map(|diagnostic| deed_diagnostics::render_human(&sources, diagnostic));
    Ended {
        output: run.output,
        exit: i32::from(failure.is_some()),
        failure,
    }
}

fn refused(why: String) -> Ended {
    Ended {
        output: Vec::new(),
        failure: Some(why),
        exit: 1,
    }
}

/// The modules `program` imports, found beside it.
///
/// The rule is the one `deed check` uses: a file that is where its own module
/// name says it should be settles a root, and every import is looked for under
/// that root. A program whose modules come from a manifest is not found this
/// way, which is written down in the decision record rather than guessed at.
fn imports(program: &Path, text: &str) -> Vec<(String, String)> {
    let mut roots = Vec::new();
    if let Some((module, _)) = deed_driver::imports_of(text)
        && let Some(root) = root_of(program, &module)
    {
        roots.push(root);
    }
    if let Some(parent) = program.parent() {
        roots.push(parent.to_path_buf());
    }

    let (extras, shipped) = deed_driver::resolve_inputs([text], |module| {
        for root in &roots {
            let candidate = root.join(format!("{module}.deed"));
            if let Ok(found) = std::fs::read_to_string(&candidate) {
                return Some((display(&candidate), found));
            }
        }
        None
    });

    let mut found = extras;
    for module in shipped {
        if let Some(text) = deed_driver::shipped_source(module) {
            found.push((format!("{module}.deed"), text.to_string()));
        }
    }
    found
}

/// The directory a module path is relative to, when the file is where its name
/// says it should be.
fn root_of(path: &Path, module: &str) -> Option<PathBuf> {
    let mut root = path.to_path_buf();
    root.set_extension("");

    for segment in module.split('/').rev() {
        if root.file_name()?.to_str()? != segment {
            return None;
        }
        root.pop();
    }
    Some(root)
}

/// Forward slashes everywhere, so a name does not depend on the platform.
fn display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::root_of;

    #[test]
    fn a_file_where_its_name_says_it_should_be_settles_a_root() {
        let root = root_of(Path::new("src/a/b.deed"), "a/b");
        assert_eq!(root, Some(Path::new("src").to_path_buf()));
    }

    #[test]
    fn a_file_that_is_not_where_its_name_says_settles_nothing() {
        assert_eq!(root_of(Path::new("src/other.deed"), "a/b"), None);
    }
}
