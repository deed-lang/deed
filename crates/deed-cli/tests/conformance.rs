//! A small external conformance suite, run through the CLI.
//!
//! The suite lives under `conformance/` as data rather than Rust test code, so
//! another implementation can consume the same cases.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use deed_lsp::{Json, json};

const DEED: &str = env!("CARGO_BIN_EXE_deed");

#[derive(Clone, Copy)]
enum Mode {
    Check,
    Test,
    Run,
}

impl Mode {
    fn parse(value: &str) -> Self {
        match value {
            "check" => Self::Check,
            "test" => Self::Test,
            "run" => Self::Run,
            other => panic!("unknown mode `{other}`"),
        }
    }

    fn as_arg(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Test => "test",
            Self::Run => "run",
        }
    }
}

enum Expectation {
    Accept,
    Reject { code: String },
    Run { stdout: Vec<String> },
}

struct Case {
    name: String,
    mode: Mode,
    program: PathBuf,
    expect: Expectation,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn suite_root() -> PathBuf {
    repo_root().join("conformance").join("cases")
}

fn run(args: &[&str]) -> Output {
    Command::new(DEED)
        .args(args)
        .output()
        .expect("the deed binary should run")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("should exit normally")
}

fn load_cases() -> Vec<Case> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(suite_root())
        .expect("conformance/cases should exist")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    assert!(!dirs.is_empty(), "no conformance cases found");

    dirs.into_iter().map(|dir| load_case(&dir)).collect()
}

fn load_case(dir: &Path) -> Case {
    let text = std::fs::read_to_string(dir.join("case.txt"))
        .unwrap_or_else(|error| panic!("{}: {error}", dir.join("case.txt").display()));

    let mut mode = None;
    let mut expect = None;
    let mut expected_code = None;
    let mut path = None;
    let mut expected_stdout = Vec::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            panic!("{}: expected `key: value`, got `{line}`", dir.display());
        };
        let key = key.trim();

        match key {
            "mode" => mode = Some(Mode::parse(value.trim())),
            "expect" => expect = Some(value.trim().to_string()),
            "code" => expected_code = Some(value.trim().to_string()),
            "path" => path = Some(value.trim().to_string()),
            "stdout" => expected_stdout.push(value.strip_prefix(' ').unwrap_or(value).to_string()),
            other => panic!("{}: unknown key `{other}`", dir.display()),
        }
    }

    let mode = mode.unwrap_or_else(|| panic!("{}: missing `mode`", dir.display()));
    let expect = expect.unwrap_or_else(|| panic!("{}: missing `expect`", dir.display()));

    let program = match path {
        Some(relative) => repo_root().join(relative),
        None => dir.join("program.deed"),
    };
    assert!(
        program.exists(),
        "{}: program does not exist",
        program.display()
    );

    let expect = match expect.as_str() {
        "accept" => Expectation::Accept,
        "reject" => Expectation::Reject {
            code: expected_code
                .unwrap_or_else(|| panic!("{}: `expect: reject` needs `code`", dir.display())),
        },
        "run" => Expectation::Run {
            stdout: expected_stdout,
        },
        other => panic!("{}: unknown expectation `{other}`", dir.display()),
    };

    if matches!(expect, Expectation::Run { .. }) {
        assert!(
            matches!(mode, Mode::Run),
            "{}: `expect: run` needs `mode: run`",
            dir.display()
        );
    }

    Case {
        name: dir
            .file_name()
            .expect("case directory has a name")
            .to_string_lossy()
            .to_string(),
        mode,
        program,
        expect,
    }
}

fn error_codes(output: &Output) -> Vec<String> {
    let mut codes = Vec::new();

    for line in stdout(output).lines() {
        let Ok(message) = json::parse(line) else {
            panic!("expected JSON output line, got `{line}`");
        };
        if message.at(&["kind"]).and_then(Json::as_str) != Some("diagnostic") {
            continue;
        }
        if message
            .at(&["diagnostic", "severity"])
            .and_then(Json::as_str)
            != Some("error")
        {
            continue;
        }
        let code = message
            .at(&["diagnostic", "code"])
            .and_then(Json::as_str)
            .expect("diagnostic should carry a code");
        codes.push(code.to_string());
    }

    codes.sort();
    codes.dedup();
    codes
}

#[test]
fn external_conformance_cases_hold_for_the_cli() {
    let cases = load_cases();

    // A suite that walked an empty directory would satisfy every assertion
    // below without running anything, which is this repository's oldest way of
    // passing for free.
    assert!(
        cases.len() >= 20,
        "only {} cases; a suite this small says almost nothing about an implementation",
        cases.len()
    );

    let mut accepted = 0;
    let mut rejected = 0;
    let mut ran = 0;
    for case in &cases {
        match case.expect {
            Expectation::Accept => accepted += 1,
            Expectation::Reject { .. } => rejected += 1,
            Expectation::Run { .. } => ran += 1,
        }
    }
    assert!(
        accepted > 0 && rejected > 0 && ran > 0,
        "a conformance suite needs all three kinds: {accepted} accept, {rejected} reject, {ran} run"
    );

    for case in cases {
        let path = case.program.to_string_lossy().to_string();

        match case.expect {
            Expectation::Accept => {
                let output = run(&[case.mode.as_arg(), &path]);
                assert_eq!(
                    code(&output),
                    0,
                    "{} should be accepted:\nstdout:\n{}\nstderr:\n{}",
                    case.name,
                    stdout(&output),
                    stderr(&output)
                );
            }
            Expectation::Reject { code: expected } => {
                let output = run(&[case.mode.as_arg(), "--format", "json", &path]);
                assert_eq!(
                    code(&output),
                    1,
                    "{} should be rejected:\nstdout:\n{}\nstderr:\n{}",
                    case.name,
                    stdout(&output),
                    stderr(&output)
                );
                let found = error_codes(&output);
                assert!(
                    found.contains(&expected),
                    "{} should report {}, got {:?}",
                    case.name,
                    expected,
                    found
                );
            }
            Expectation::Run { stdout: expected } => {
                let output = run(&[case.mode.as_arg(), &path]);
                assert_eq!(
                    code(&output),
                    0,
                    "{} should run:\nstdout:\n{}\nstderr:\n{}",
                    case.name,
                    stdout(&output),
                    stderr(&output)
                );
                let got: Vec<String> = stdout(&output)
                    .lines()
                    .map(|line| line.trim_end().to_string())
                    .collect();
                let expected: Vec<String> = expected
                    .into_iter()
                    .map(|line| line.trim_end().to_string())
                    .collect();
                assert_eq!(got, expected, "{} produced unexpected output", case.name);
            }
        }
    }
}
