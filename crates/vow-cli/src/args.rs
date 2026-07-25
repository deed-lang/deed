//! Argument parsing.
//!
//! Hand written, because the workspace has no dependencies and four flags do
//! not justify starting. If this grows past a page, that is the signal to
//! reach for a real parser rather than to make this cleverer.

use std::path::PathBuf;

pub const USAGE: &str = "\
vow, a contract-first language

Usage:
  vow check [options] <path>...
  vow test  [options] <path>...
  vow run   [options] <path>...

Options:
  --format <human|json>   How to print diagnostics. Default: human.
  --obligations           Report which tier each refinement obligation landed in.
  --dir <path>            What `sys.files` reaches when running. Default: the
                          current directory. A program cannot get outside it.
  -h, --help              Print this.
  -V, --version           Print the version.

Paths may be files or directories. A directory is searched for `.vow` files.

`vow test` refuses to run anything that does not check.
`vow run` calls `main`, handing it the one `System` capability there is.

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
}

#[derive(Debug)]
pub struct CheckArgs {
    pub mode: Mode,
    pub paths: Vec<PathBuf>,
    pub format: Format,
    pub obligations: bool,
    /// What `sys.files` is rooted at. `None` means the current directory.
    ///
    /// Granting the whole working directory by default is what every other
    /// tool does and it is not obviously right, but making people spell it out
    /// every time would just teach them to type it without reading it.
    pub dir: Option<PathBuf>,
}

#[derive(Debug)]
pub enum Command {
    Check(CheckArgs),
    Help,
    Version,
}

pub fn parse<I: Iterator<Item = String>>(mut args: I) -> Result<Command, String> {
    let Some(first) = args.next() else {
        return Err("expected a command, try `vow check <path>`".to_string());
    };

    let mode = match first.as_str() {
        "-h" | "--help" | "help" => return Ok(Command::Help),
        "-V" | "--version" | "version" => return Ok(Command::Version),
        "check" => Mode::Check,
        "test" => Mode::Test,
        "run" => Mode::Run,
        other => {
            return Err(format!(
                "unknown command `{other}`, the choices are `check`, `test` and `run`"
            ));
        }
    };

    let mut paths = Vec::new();
    let mut format = Format::Human;
    let mut obligations = false;
    let mut dir = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--obligations" => obligations = true,
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
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option `{other}`"));
            }
            path => paths.push(PathBuf::from(path)),
        }
    }

    if paths.is_empty() {
        return Err(format!(
            "`vow {}` needs at least one path",
            match mode {
                Mode::Check => "check",
                Mode::Test => "test",
                Mode::Run => "run",
            }
        ));
    }

    Ok(Command::Check(CheckArgs {
        mode,
        paths,
        format,
        obligations,
        dir,
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
        let Ok(Command::Check(check)) = parse(args(&["test", "a.vow"])) else {
            panic!("should parse");
        };
        assert_eq!(check.mode, Mode::Test);
    }

    #[test]
    fn paths_and_flags_may_be_interleaved() {
        let Ok(Command::Check(check)) = parse(args(&["check", "a.vow", "--obligations", "b.vow"]))
        else {
            panic!("should parse");
        };
        assert_eq!(check.paths.len(), 2);
        assert!(check.obligations);
        assert_eq!(check.format, Format::Human);
    }

    #[test]
    fn format_accepts_both_spellings() {
        for spelling in [
            vec!["check", "--format", "json", "a.vow"],
            vec!["check", "--format=json", "a.vow"],
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
        assert!(parse(args(&["check", "--oblgations", "a.vow"])).is_err());
    }

    #[test]
    fn a_bad_format_says_what_the_choices_are() {
        let error = parse(args(&["check", "--format", "yaml", "a.vow"])).unwrap_err();
        assert!(error.contains("`human` and `json`"), "{error}");
    }

    #[test]
    fn help_wins_wherever_it_appears() {
        assert!(matches!(parse(args(&["--help"])), Ok(Command::Help)));
        assert!(matches!(parse(args(&["check", "-h"])), Ok(Command::Help)));
    }

    #[test]
    fn an_unknown_command_names_the_one_that_exists() {
        let error = parse(args(&["buidl"])).unwrap_err();
        assert!(error.contains("check"), "{error}");
        assert!(error.contains("test"), "{error}");
    }
}
