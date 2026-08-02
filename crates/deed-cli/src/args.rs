//! Argument parsing.
//!
//! Hand written, because the workspace has no dependencies and five flags do
//! not justify starting. If this grows past a page, that is the signal to
//! reach for a real parser rather than to make this cleverer.

use std::path::PathBuf;

pub const USAGE: &str = "\
deed, a contract-first language

Usage:
  deed check [options] <path>...
  deed test  [options] <path>...
  deed run   [options] <path>... [-- <argument>...]
  deed build [options] <path>...
  deed doc   <path>...
  deed fmt   [--check] <path>...
  deed fix   [--check] <path>...
  deed explain <code>
  deed lsp
  deed mcp

Options:
  --format <human|json>   How to print diagnostics. Default: human.
  --obligations           Report which tier each refinement obligation landed in.
  --timings               Report how long each pass took.
  --profile-runtime       With `run`, report where runtime went.
    --compiled              With `run` or `test`, use the compiled backend.
  --dir <path>            What `sys.files` reaches when running. Default: the
                          current directory. A program cannot get outside it.
  --check                 With `fmt` or `fix`, change nothing and report what
                          would have changed.
  --compiled              With `test`, run test blocks through the compiled
                          WebAssembly backend instead of the interpreter.
  --component             With `build`, produce a component instead of a
                          standalone module. Writes a `.wit` world alongside
                          the `.wasm`. Refuses programs that declare `main` or
                          use a capability in an exported function's signature.
  --lock <path>           Write a lock file listing every input with its
                          SHA-256 hash. Existing file is overwritten.
  --locked <path>         Refuse to proceed if any input differs from the lock
                          file. Use after `--lock` to reproduce a build exactly.
  -h, --help              Print this.
  -V, --version           Print the version.

Paths may be files or directories. A directory is searched for `.deed` files.

`deed test` refuses to run anything that does not check.
`deed test --compiled` runs the same test blocks through the compiled backend.
  Blocks the backend cannot compile are skipped, and the count of the ones that
  ran has to match what the interpreter ran.
`deed run` calls `main`, handing it the one `System` capability there is.
Everything after `--` goes to the program, which reads it with `Io.args`.
`deed build` compiles to a WebAssembly module beside the file it was given. It
says what it could not compile rather than guessing, which as of #877 is a
function value compared with another one and nothing else in the corpus.
`deed build --component` produces a WebAssembly module and a `.wit` world file
beside the source. The component's exported interface is every function the
module declares. A function is its own export; there is no `main`. Functions
whose signatures contain a capability have no world-level type in WIT and are
refused with an explanation. Tests are not part of the interface.
`deed doc` writes the API reference a module carries to standard output, as
Markdown. There are no visibility modifiers here, so every declaration is API
and there is nothing to decide about what belongs on the page: the signature,
the row, the contract, the comment above it, and the lines of its own tests
that name it.
`deed fmt` has no options for the output. There is one canonical form.
`deed fix` applies the fixes that are certain and leaves the guesses alone.
`deed explain` prints the page for one diagnostic code. The argument may be the
code identifier (`DEED4025`) or the constant name (`BROKEN_PRECONDITION`).
`deed lsp` speaks the language server protocol on stdin and stdout. It is for an
editor to start, not for a person to type.
`deed mcp` speaks the Model Context Protocol on stdin and stdout, so an agent can
ask the compiler the same questions an editor does. It holds no capability: a
program arrives as text and the answer leaves as text, and nothing it runs can
reach a file.
There is no REPL. A Deed input is a module of declarations, and `deed run`
enters through `main` rather than evaluating a top-level expression. For quick
experiments, use a scratch `.deed` file with `deed check`, `deed test` or
`deed run`, or use a playground that edits a whole program rather than one
expression.

Exit codes:
  0   no errors, though there may be warnings
  1   errors were found, or a test failed
  2   the invocation itself was wrong
";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Human,
    Json,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Check,
    Test,
    Run,
    Fmt,
    Fix,
    /// Compile to a WebAssembly module rather than running it.
    Build,
    /// Write the API reference a module carries to standard output.
    Doc,
}

#[derive(Debug)]
pub struct CheckArgs {
    pub mode: Mode,
    pub paths: Vec<PathBuf>,
    pub format: Format,
    pub obligations: bool,
    /// Report how long each pass took.
    pub timings: bool,
    /// With `run`, report where runtime went.
    pub runtime_profile: bool,
    /// What `sys.files` is rooted at. `None` means the current directory.
    ///
    /// Granting the whole working directory by default is what every other
    /// tool does and it is not obviously right, but making people spell it out
    /// every time would just teach them to type it without reading it.
    pub dir: Option<PathBuf>,
    /// With `fmt`, report rather than rewrite.
    pub check_only: bool,
    /// With `build`, produce a component instead of a standalone module.
    pub component: bool,
    /// Everything after `--`, handed to the program rather than read here.
    ///
    /// A separator rather than "whatever is left over", because a program's
    /// arguments can look exactly like this tool's and guessing which is which
    /// is how a flag ends up being eaten by the wrong reader.
    pub arguments: Vec<String>,
    /// With `run` or `test`, use the compiled backend.
    pub compiled: bool,
    /// Path to write a lock file recording every input with its hash.
    pub lock: Option<PathBuf>,
    /// Path to a lock file; refuse to proceed if any input has changed.
    pub locked: Option<PathBuf>,
}

#[derive(Debug)]
pub enum Command {
    Check(CheckArgs),
    /// Print the page for one diagnostic code.
    Explain(String),
    /// Speak the language server protocol on stdin and stdout.
    Lsp,
    /// Speak the Model Context Protocol on stdin and stdout.
    Mcp,
    Help,
    Version,
}

fn repl_refusal(command: Option<&str>) -> String {
    let lead = match command {
        Some(name) => format!("`deed {name}` is not a command here because there is no REPL"),
        None => "there is no REPL".to_string(),
    };
    format!(
        "{lead}: a Deed input is a module of declarations, `deed run` enters through `main`, \
         and a prompt would need its own capability row and contract context. For experiments, \
         use a scratch `.deed` file with `deed check`, `deed test` or `deed run`, or use a \
         playground that edits a whole program."
    )
}

pub fn parse<I: Iterator<Item = String>>(mut args: I) -> Result<Command, String> {
    let Some(first) = args.next() else {
        return Err(repl_refusal(None));
    };

    let mode = match first.as_str() {
        "-h" | "--help" | "help" => return Ok(Command::Help),
        "-V" | "--version" | "version" => return Ok(Command::Version),
        "repl" | "interactive" => return Err(repl_refusal(Some(first.as_str()))),
        // No options and no paths. An editor starts it and talks to it down
        // the pipe, so anything else on the line is a mistake worth saying so
        // about rather than ignoring.
        "lsp" => {
            return match args.next() {
                None => Ok(Command::Lsp),
                Some(extra) => Err(format!("`deed lsp` takes no arguments, found `{extra}`")),
            };
        }
        "mcp" => {
            return match args.next() {
                None => Ok(Command::Mcp),
                Some(extra) => Err(format!("`deed mcp` takes no arguments, found `{extra}`")),
            };
        }
        "explain" => {
            return match args.next() {
                None => {
                    Err("`deed explain` needs a code, e.g. `deed explain DEED4025`".to_string())
                }
                Some(code) => Ok(Command::Explain(code)),
            };
        }
        "check" => Mode::Check,
        "test" => Mode::Test,
        "run" => Mode::Run,
        "fmt" => Mode::Fmt,
        "fix" => Mode::Fix,
        "build" => Mode::Build,
        "doc" => Mode::Doc,
        other => {
            return Err(format!(
                "unknown command `{other}`, the choices are `check`, `test`, `run`, `build`, `fmt`, `fix`, `explain` and `lsp`"
            ));
        }
    };

    let mut paths = Vec::new();
    let mut format = Format::Human;
    let mut obligations = false;
    let mut timings = false;
    let mut runtime_profile = false;
    let mut dir = None;
    let mut check_only = false;
    let mut component = false;
    let mut arguments = Vec::new();
    let mut compiled = false;
    let mut lock = None;
    let mut locked = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            // Everything after this belongs to the program being run, and
            // nothing here looks at it again.
            "--" => {
                arguments.extend(args.by_ref());
                break;
            }
            "-h" | "--help" => return Ok(Command::Help),
            "--obligations" => obligations = true,
            "--timings" => timings = true,
            "--profile-runtime" => runtime_profile = true,
            "--check" => check_only = true,
            "--compiled" => compiled = true,
            "--component" => component = true,
            "--format" => {
                let value = args
                    .next()
                    .ok_or_else(|| "`--format` needs a value, `human` or `json`".to_string())?;
                format = parse_format(&value)?;
            }
            other if other.starts_with("--format=") => {
                format = parse_format(&other["--format=".len()..])?;
            }
            "--dir" => {
                let value = args
                    .next()
                    .ok_or_else(|| "`--dir` needs a directory".to_string())?;
                dir = Some(PathBuf::from(value));
            }
            other if other.starts_with("--dir=") => {
                dir = Some(PathBuf::from(&other["--dir=".len()..]));
            }
            "--lock" => {
                let value = args
                    .next()
                    .ok_or_else(|| "`--lock` needs a path".to_string())?;
                lock = Some(PathBuf::from(value));
            }
            other if other.starts_with("--lock=") => {
                lock = Some(PathBuf::from(&other["--lock=".len()..]));
            }
            "--locked" => {
                let value = args
                    .next()
                    .ok_or_else(|| "`--locked` needs a path".to_string())?;
                locked = Some(PathBuf::from(value));
            }
            other if other.starts_with("--locked=") => {
                locked = Some(PathBuf::from(&other["--locked=".len()..]));
            }
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option `{other}`"));
            }
            path => paths.push(PathBuf::from(path)),
        }
    }

    if paths.is_empty() {
        return Err(format!(
            "`deed {}` needs at least one path",
            match mode {
                Mode::Check => "check",
                Mode::Test => "test",
                Mode::Run => "run",
                Mode::Fmt => "fmt",
                Mode::Fix => "fix",
                Mode::Build => "build",
                Mode::Doc => "doc",
            }
        ));
    }

    if compiled && !matches!(mode, Mode::Run | Mode::Test) {
        return Err("`--compiled` is only valid with `deed run` or `deed test`".to_string());
    }

    Ok(Command::Check(CheckArgs {
        mode,
        paths,
        format,
        obligations,
        timings,
        runtime_profile,
        dir,
        check_only,
        component,
        arguments,
        compiled,
        lock,
        locked,
    }))
}

fn parse_format(value: &str) -> Result<Format, String> {
    match value {
        "human" => Ok(Format::Human),
        "json" => Ok(Format::Json),
        other => Err(format!(
            "`{other}` is not a format, the choices are `human` and `json`"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, Format, Mode, parse};

    fn args(items: &[&str]) -> impl Iterator<Item = String> {
        items
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn a_bare_check_needs_a_path() {
        assert!(parse(args(&["check"])).is_err());
        assert!(parse(args(&["test"])).is_err());
    }

    #[test]
    fn test_is_a_command() {
        let Ok(Command::Check(check)) = parse(args(&["test", "a.deed"])) else {
            panic!("should parse");
        };
        assert_eq!(check.mode, Mode::Test);
    }

    #[test]
    fn lsp_is_a_command_and_takes_nothing_else() {
        assert!(matches!(parse(args(&["lsp"])), Ok(Command::Lsp)));
        assert!(parse(args(&["lsp", "a.deed"])).is_err());
    }

    #[test]
    fn paths_and_flags_may_be_interleaved() {
        let Ok(Command::Check(check)) =
            parse(args(&["check", "a.deed", "--obligations", "b.deed"]))
        else {
            panic!("should parse");
        };
        assert_eq!(check.paths.len(), 2);
        assert!(check.obligations);
        assert_eq!(check.format, Format::Human);
    }

    #[test]
    fn run_accepts_runtime_profiling() {
        let Ok(Command::Check(check)) = parse(args(&["run", "--profile-runtime", "a.deed"])) else {
            panic!("should parse");
        };
        assert_eq!(check.mode, Mode::Run);
        assert!(check.runtime_profile);
    }

    #[test]
    fn format_accepts_both_spellings() {
        for spelling in [
            vec!["check", "--format", "json", "a.deed"],
            vec!["check", "--format=json", "a.deed"],
        ] {
            let Ok(Command::Check(check)) = parse(args(&spelling)) else {
                panic!("should parse {spelling:?}");
            };
            assert_eq!(check.format, Format::Json);
        }
    }

    #[test]
    fn an_unknown_option_is_not_treated_as_a_path() {
        // Otherwise a typo silently becomes a file that does not exist, and the
        // error talks about the wrong thing.
        assert!(parse(args(&["check", "--oblgations", "a.deed"])).is_err());
    }

    #[test]
    fn a_bad_format_says_what_the_choices_are() {
        let error = parse(args(&["check", "--format", "yaml", "a.deed"])).unwrap_err();
        assert!(error.contains("`human` and `json`"), "{error}");
    }

    #[test]
    fn help_wins_wherever_it_appears() {
        assert!(matches!(parse(args(&["--help"])), Ok(Command::Help)));
        assert!(matches!(parse(args(&["check", "-h"])), Ok(Command::Help)));
    }

    #[test]
    fn explain_is_a_command() {
        let Ok(Command::Explain(code)) = parse(args(&["explain", "DEED4025"])) else {
            panic!("should parse");
        };
        assert_eq!(code, "DEED4025");
    }

    #[test]
    fn explain_without_a_code_is_an_error() {
        assert!(parse(args(&["explain"])).is_err());
    }

    #[test]
    fn an_unknown_command_names_the_one_that_exists() {
        let error = parse(args(&["buidl"])).unwrap_err();
        assert!(error.contains("check"), "{error}");
        assert!(error.contains("test"), "{error}");
    }

    #[test]
    fn repl_is_refused_with_a_program_shaped_alternative() {
        let error = parse(args(&["repl"])).unwrap_err();
        assert!(error.contains("there is no REPL"), "{error}");
        assert!(error.contains("scratch `.deed` file"), "{error}");
        assert!(error.contains("playground"), "{error}");
    }
}

#[cfg(test)]
mod lock_tests {
    use super::{Command, parse};

    fn args(items: &[&str]) -> impl Iterator<Item = String> {
        items
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn lock_accepts_both_spellings() {
        for spelling in [
            vec!["check", "--lock", "deed.lock", "a.deed"],
            vec!["check", "--lock=deed.lock", "a.deed"],
        ] {
            let Ok(Command::Check(check)) = parse(args(&spelling)) else {
                panic!("should parse {spelling:?}");
            };
            assert_eq!(
                check.lock.as_deref(),
                Some(std::path::Path::new("deed.lock"))
            );
        }
    }

    #[test]
    fn locked_accepts_both_spellings() {
        for spelling in [
            vec!["check", "--locked", "deed.lock", "a.deed"],
            vec!["check", "--locked=deed.lock", "a.deed"],
        ] {
            let Ok(Command::Check(check)) = parse(args(&spelling)) else {
                panic!("should parse {spelling:?}");
            };
            assert_eq!(
                check.locked.as_deref(),
                Some(std::path::Path::new("deed.lock"))
            );
        }
    }

    #[test]
    fn lock_without_a_path_is_an_error() {
        assert!(parse(args(&["check", "--lock"])).is_err());
    }

    #[test]
    fn locked_without_a_path_is_an_error() {
        assert!(parse(args(&["check", "--locked"])).is_err());
    }
}
