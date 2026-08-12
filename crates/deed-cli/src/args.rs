//! Argument parsing.
//!
//! Hand written, because the workspace has no dependencies and five flags do
//! not justify starting. If this grows past a page, that is the signal to
//! reach for a real parser rather than to make this cleverer.

use std::path::PathBuf;

pub const USAGE: &str = "\
deed, a contract-first language

Usage:
  deed new   <name>
  deed check [options] <path>...
    deed review --before <path>... --after <path>... [options]
  deed test  [options] <path>...
  deed run   [options] <path>... [-- <argument>...]
  deed build [options] <path>...
  deed doc   <path>...
  deed fmt   [--check] <path>...
  deed fix   [--check] <path>...
  deed explain <code>
  deed lsp
  deed debug
  deed mcp

Options:
  --format <human|json>   How to print diagnostics. Default: human.
    --before <path>         A file or directory before the change. Repeatable.
    --after <path>          A file or directory after the change. Repeatable.
    --deny-new-authority    With `review`, fail if authority was added.
    --deny-weaker-promises With `review`, fail if an obligation's tier regressed.
    --deny-new-guarded      With `review`, fail if a new Guarded obligation appeared.
  --obligations           Report which tier each refinement obligation landed in.
  --timings               Report how long each pass took.
  --profile-runtime       With `run`, report where runtime went.
    --compiled              With `run` or `test`, use the compiled backend.
  --dir <path>            What `sys.files` reaches when running. Default: the
                          current directory. A program cannot get outside it.
  --allow <host>          A host `sys.net` may reach when running. Repeatable.
                          Default: none, so a program reaches nothing. A host
                          may carry a port (`example.com:8080`), and one
                          without a port grants every port on that host.
  --env <name>            An environment variable `Io.env` may read when
                          running. Repeatable. Default: none, so every name
                          reads as absent. An environment routinely carries
                          credentials, so what a program sees is a list rather
                          than all of it.
  --check                 With `fmt` or `fix`, change nothing and report what
                          would have changed.
    --compiled              With `test`, run test blocks and generated properties
                                                    through the compiled WebAssembly backend.
  --component             With `build`, write a core module, the `.wit` world
                          its exports describe, and a component binary, instead
                          of a standalone program. The component is written
                          when every export carries numbers, booleans, text or
                          a list of numbers; anything wider needs adapters that
                          are not written, and says so by name. Refuses programs
                          that declare `main` or use a capability in an exported
                          function's signature.
  --reuse                 With `build`, print what each function does with each
                          parameter: `releases`, `returns`, `retains` or
                          `keeps`. Answers the question a caller has to ask
                          before handing over its only reference. Prints only;
                          nothing acts on it yet.
  --lock <path>           Write a lock file listing every input with its
                          SHA-256 hash. Existing file is overwritten.
  --locked <path>         Refuse to proceed if any input differs from the lock
                          file. Use after `--lock` to reproduce a build exactly.
  -h, --help              Print this.
  -V, --version           Print the version.

Paths may be files or directories. A directory is searched for `.deed` files.

`deed new` writes a directory of that name holding a library module with a
contract and its tests, and a program that imports it. The name becomes both
the directory and the module path, so it is lowercase letters, digits and `_`.
It writes no manifest: a manifest here says where code outside your tree lives,
and a new project has none.
`deed test` refuses to run anything that does not check.
`deed test --compiled` runs test blocks and the properties contracts generate
through the compiled backend. Blocks the backend cannot compile are skipped and
named, so the summary says both how much ran and what did not.
`deed review` compares two checked module sets. Its receipt names authority
added to effect rows and refinement obligations that moved to a weaker tier.
Findings are informational unless a `--deny-*` policy is enabled. Code that
does not check is refused before the two sets are compared.
`deed run` calls `main`, handing it the one `System` capability there is.
Everything after `--` goes to the program, which reads it with `Io.args`.
Standard input is read when, and only when, `main`'s row says `Io.line`. A
program that never mentions it is never left waiting for input, and there is
no flag to remember because the signature already says which programs read.
`deed build` compiles to a WebAssembly module beside the file it was given. It
says what it could not compile rather than guessing, which as of #877 is a
function value compared with another one and nothing else in the corpus.
`deed build --component` produces a WebAssembly core module, a `.wit` world file
and a component binary beside the source. The world's exported interface is
every function the module declares. A function is its own export; there is no
`main`. Functions whose signatures contain a capability have no world-level type
in WIT and are refused with an explanation. Tests are not part of the interface.

The component binary is written when every export carries values the canonical
ABI has a crossing for here: a number and a boolean cross unchanged, and text
and a list of numbers cross through `cabi_realloc` and a wrapper per export,
which the component carries. A list of numbers needs no element loop because
both sides keep an `s64` in eight bytes. Anything wider -- a list of anything
else, a record, a choice -- has adapters that are not written; those modules get
the core module and the world, and a line naming the export and what it needs. A
component that answered wrongly would be worse than one that is not written. Every part of this is measured on every commit
against the Bytecode Alliance's own tooling, and
`design/decisions/2026-08-09-a-component-for-what-crosses-unchanged.md` says
what is still missing.
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
`deed debug` speaks the debug adapter protocol on stdin and stdout, so an editor
can set breakpoints, step, and read the stack and the bindings of every active
call. Which program to debug arrives in the client's `launch` request, so this
command takes no path. There is no `pause`: a program stops where it was told to
and runs otherwise.
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
    /// What `sys.net` reaches. Empty means nothing.
    ///
    /// Unlike `--dir`, this does not default to something. Running a command
    /// in a directory is already a choice about that directory, so granting
    /// the working one inherits a decision somebody made; there is no
    /// equivalent ambient choice about the network, and "the network" is not
    /// a place anyone is standing. So a program that was not told which hosts
    /// it may reach reaches none, and says so rather than failing to connect.
    pub allow: Vec<String>,
    /// The environment variables `Io.env` may read. Empty means none.
    pub env: Vec<String>,
    /// With `fmt`, report rather than rewrite.
    pub check_only: bool,
    /// With `build`, produce a component instead of a standalone module.
    pub component: bool,
    /// With `build`, print what each function does with its parameters.
    pub reuse: bool,
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
pub struct ReviewArgs {
    pub before: Vec<PathBuf>,
    pub after: Vec<PathBuf>,
    pub format: Format,
    pub deny_new_authority: bool,
    pub deny_weaker_promises: bool,
    pub deny_new_guarded: bool,
}

#[derive(Debug)]
pub enum Command {
    Check(CheckArgs),
    /// Compare the authority and contract evidence in two module sets.
    Review(ReviewArgs),
    /// Write a new project into a directory of this name.
    New(String),
    /// Print the page for one diagnostic code.
    Explain(String),
    /// Speak the language server protocol on stdin and stdout.
    Lsp,
    /// Speak the debug adapter protocol on stdin and stdout.
    Debug,
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
        // The program is named by the client's `launch` request rather than
        // here. A path on this line would be a second place to say which
        // program is being debugged, and the two would disagree.
        "debug" => {
            return match args.next() {
                None => Ok(Command::Debug),
                Some(extra) => Err(format!(
                    "`deed debug` takes no arguments, found `{extra}`: the program comes from the \
                     client's launch request"
                )),
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
        // One name and nothing else. Everything a project needs beyond the
        // name is a decision the files themselves make, and a flag here would
        // be a second place to make it.
        "new" => {
            return match (args.next(), args.next()) {
                (None, _) => Err("`deed new` needs a name, e.g. `deed new greeter`".to_string()),
                (Some(_), Some(extra)) => Err(format!(
                    "`deed new` takes one name, found `{extra}` as well"
                )),
                (Some(name), None) => Ok(Command::New(name)),
            };
        }
        "review" => return parse_review(args),
        "check" => Mode::Check,
        "test" => Mode::Test,
        "run" => Mode::Run,
        "fmt" => Mode::Fmt,
        "fix" => Mode::Fix,
        "build" => Mode::Build,
        "doc" => Mode::Doc,
        other => {
            return Err(format!(
                "unknown command `{other}`, the choices are `new`, `check`, `review`, `test`, `run`, `build`, `doc`, `fmt`, `fix`, `explain`, `lsp`, `debug` and `mcp`"
            ));
        }
    };

    let mut paths = Vec::new();
    let mut format = Format::Human;
    let mut obligations = false;
    let mut timings = false;
    let mut runtime_profile = false;
    let mut dir = None;
    let mut allow: Vec<String> = Vec::new();
    let mut env: Vec<String> = Vec::new();
    let mut check_only = false;
    let mut component = false;
    let mut reuse = false;
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
            "--reuse" => reuse = true,
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
            // Repeatable rather than comma separated. A host cannot contain a
            // space and a shell already splits on one, so the list is the
            // argument list, and there is no second escaping rule to know.
            "--allow" => {
                let value = args.next().ok_or_else(|| {
                    "`--allow` needs a host, e.g. `--allow example.com`".to_string()
                })?;
                allow.push(value);
            }
            other if other.starts_with("--allow=") => {
                allow.push(other["--allow=".len()..].to_string());
            }
            // Repeatable, and a name rather than a name and a value. What is
            // being granted is a look at what the machine is already carrying,
            // the way `--allow` grants a look at a host that already exists.
            "--env" => {
                let value = args.next().ok_or_else(|| {
                    "`--env` needs a variable name, e.g. `--env HOME`".to_string()
                })?;
                env.push(value);
            }
            other if other.starts_with("--env=") => {
                env.push(other["--env=".len()..].to_string());
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

    // A grant nothing will read is a grant somebody believes they made. `deed
    // check --allow` looks like it narrows what the check considers and it
    // does not, and `deed test --allow` looks like it lets a test reach a
    // host, which is the thing a test must not do.
    if !allow.is_empty() && !matches!(mode, Mode::Run) {
        return Err(
            "`--allow` is only valid with `deed run`; a test that reaches a host is a \
                    test of that host"
                .to_string(),
        );
    }

    // The same sentence about the same mistake: a test whose answer depended
    // on what the machine running it was carrying would be a test of that
    // machine.
    if !env.is_empty() && !matches!(mode, Mode::Run) {
        return Err(
            "`--env` is only valid with `deed run`; a test that reads the environment is a \
                    test of that environment"
                .to_string(),
        );
    }

    Ok(Command::Check(CheckArgs {
        mode,
        paths,
        format,
        obligations,
        timings,
        runtime_profile,
        dir,
        allow,
        env,
        check_only,
        component,
        reuse,
        arguments,
        compiled,
        lock,
        locked,
    }))
}

fn parse_review<I: Iterator<Item = String>>(mut args: I) -> Result<Command, String> {
    let mut before = Vec::new();
    let mut after = Vec::new();
    let mut format = Format::Human;
    let mut deny_new_authority = false;
    let mut deny_weaker_promises = false;
    let mut deny_new_guarded = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--deny-new-authority" => deny_new_authority = true,
            "--deny-weaker-promises" => deny_weaker_promises = true,
            "--deny-new-guarded" => deny_new_guarded = true,
            "--before" => before.push(PathBuf::from(
                args.next()
                    .ok_or_else(|| "`--before` needs a path".to_string())?,
            )),
            other if other.starts_with("--before=") => {
                before.push(PathBuf::from(&other["--before=".len()..]));
            }
            "--after" => after.push(PathBuf::from(
                args.next()
                    .ok_or_else(|| "`--after` needs a path".to_string())?,
            )),
            other if other.starts_with("--after=") => {
                after.push(PathBuf::from(&other["--after=".len()..]));
            }
            "--format" => {
                let value = args
                    .next()
                    .ok_or_else(|| "`--format` needs a value, `human` or `json`".to_string())?;
                format = parse_format(&value)?;
            }
            other if other.starts_with("--format=") => {
                format = parse_format(&other["--format=".len()..])?;
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option `{other}`"));
            }
            path => {
                return Err(format!(
                    "review path `{path}` must follow `--before` or `--after`"
                ));
            }
        }
    }

    if before.is_empty() || after.is_empty() {
        return Err(
            "`deed review` needs at least one `--before` and one `--after` path".to_string(),
        );
    }

    Ok(Command::Review(ReviewArgs {
        before,
        after,
        format,
        deny_new_authority,
        deny_weaker_promises,
        deny_new_guarded,
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
    fn review_has_explicit_repeatable_sides_a_format_and_policies() {
        let Ok(Command::Review(review)) = parse(args(&[
            "review",
            "--before",
            "old/a.deed",
            "--before=old/b.deed",
            "--after=new",
            "--format=json",
            "--deny-new-authority",
            "--deny-weaker-promises",
            "--deny-new-guarded",
        ])) else {
            panic!("should parse");
        };
        assert_eq!(review.before.len(), 2);
        assert_eq!(review.after, vec![std::path::PathBuf::from("new")]);
        assert_eq!(review.format, Format::Json);
        assert!(review.deny_new_authority);
        assert!(review.deny_weaker_promises);
        assert!(review.deny_new_guarded);
    }

    #[test]
    fn review_needs_both_sides_and_labels_every_path() {
        let missing = parse(args(&["review", "--before", "old"])).unwrap_err();
        assert!(missing.contains("one `--after`"), "{missing}");

        let unlabelled = parse(args(&["review", "old", "--after", "new"])).unwrap_err();
        assert!(unlabelled.contains("must follow"), "{unlabelled}");
    }

    #[test]
    fn review_distinguishes_an_unknown_option_from_an_unlabelled_path() {
        let unknown = parse(args(&[
            "review", "--before", "old", "--after", "new", "--strict",
        ]))
        .unwrap_err();
        assert_eq!(unknown, "unknown option `--strict`");

        let unlabelled = parse(args(&[
            "review", "--before", "old", "--after", "new", "extra",
        ]))
        .unwrap_err();
        assert!(unlabelled.contains("must follow"), "{unlabelled}");
    }

    #[test]
    fn lsp_is_a_command_and_takes_nothing_else() {
        assert!(matches!(parse(args(&["lsp"])), Ok(Command::Lsp)));
        assert!(parse(args(&["lsp", "a.deed"])).is_err());
    }

    /// A path here is somebody typing what belongs after `deed run`. The
    /// refusal says where the program actually comes from, because "takes no
    /// arguments" alone leaves a reader with no idea how to name one.
    #[test]
    fn debug_is_a_command_and_says_where_its_program_comes_from() {
        assert!(matches!(parse(args(&["debug"])), Ok(Command::Debug)));
        let refused = parse(args(&["debug", "a.deed"])).unwrap_err();
        assert!(refused.contains("launch request"), "{refused}");
    }

    /// The default, and the one worth a test of its own: a run nobody told
    /// about a host reaches none.
    #[test]
    fn a_run_that_names_no_host_grants_none() {
        let Ok(Command::Check(check)) = parse(args(&["run", "a.deed"])) else {
            panic!("should parse");
        };
        assert!(check.allow.is_empty());
    }

    #[test]
    fn allow_is_repeatable_and_takes_either_spelling() {
        let Ok(Command::Check(check)) = parse(args(&[
            "run",
            "a.deed",
            "--allow",
            "one.example",
            "--allow=two.example:8080",
        ])) else {
            panic!("should parse");
        };
        assert_eq!(check.allow, vec!["one.example", "two.example:8080"]);
    }

    #[test]
    fn allow_needs_a_host() {
        assert!(parse(args(&["run", "a.deed", "--allow"])).is_err());
    }

    #[test]
    fn a_run_that_names_no_variable_grants_none() {
        let Ok(Command::Check(check)) = parse(args(&["run", "a.deed"])) else {
            panic!("should parse");
        };
        assert!(check.env.is_empty());
    }

    #[test]
    fn env_is_repeatable_and_takes_either_spelling() {
        let Ok(Command::Check(check)) =
            parse(args(&["run", "a.deed", "--env", "HOME", "--env=PATH"]))
        else {
            panic!("should parse");
        };
        assert_eq!(check.env, vec!["HOME", "PATH"]);
    }

    #[test]
    fn env_needs_a_name() {
        assert!(parse(args(&["run", "a.deed", "--env"])).is_err());
    }

    /// The same sentence `--allow` gets, about the same mistake.
    #[test]
    fn env_is_refused_where_nothing_would_read_it() {
        for mode in ["check", "test", "build", "fmt"] {
            assert!(
                parse(args(&[mode, "a.deed", "--env", "HOME"])).is_err(),
                "`deed {mode} --env` should be refused"
            );
        }
    }

    /// A grant nothing will read is a grant somebody believes they made.
    #[test]
    fn allow_is_refused_where_nothing_would_read_it() {
        for mode in ["check", "test", "build", "fmt"] {
            let refused = parse(args(&[mode, "a.deed", "--allow", "one.example"]))
                .expect_err("`--allow` should only be valid with `run`");
            assert!(refused.contains("only valid with `deed run`"), "{refused}");
        }
    }

    /// Everything after `--` belongs to the program, including something that
    /// looks like this tool's own flag.
    #[test]
    fn a_program_may_be_handed_something_spelled_like_allow() {
        let Ok(Command::Check(check)) = parse(args(&["run", "a.deed", "--", "--allow", "x"]))
        else {
            panic!("should parse");
        };
        assert!(check.allow.is_empty());
        assert_eq!(check.arguments, vec!["--allow", "x"]);
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
    fn build_may_be_asked_what_a_callee_does_with_its_argument() {
        let Ok(Command::Check(check)) = parse(args(&["build", "a.deed", "--reuse"])) else {
            panic!("should parse");
        };
        assert!(check.reuse);
        assert!(!check.component, "one flag does not turn on the other");

        let Ok(Command::Check(plain)) = parse(args(&["build", "a.deed"])) else {
            panic!("should parse");
        };
        assert!(!plain.reuse);
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
