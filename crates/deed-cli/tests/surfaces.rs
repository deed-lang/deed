//! The command line and the artifact a page runs, asked about the same file.
//!
//! Three consumers read one compiler and there were ratchets holding two of
//! the three pairs: this file's neighbour holds the command line against the
//! language server, and `deed-mcp/tests/agreement.rs` holds the agent server
//! against the artifact it answers from. Nothing held the command line against
//! the artifact, and that is the pair that drifted, three separate ways in one
//! afternoon: the artifact ran a file the checker had rejected (#823), it
//! answered "no tests" with silence while the command line said so out loud
//! (#823), and it skipped the property tests a contract generates entirely
//! (#827). Each of those was found by reading rather than by anything failing.
//!
//! The observable is the decision rather than the words. One side writes lines
//! for a person and the other writes JSON for a program, so requiring the same
//! bytes would be requiring them to be the same tool. What has to match is
//! whether anything ran, what ran, and how it came out.
//!
//! Agreement on its own is not enough, since both of them agreeing on the
//! wrong answer is what this is here to catch. Every case below also pins what
//! the answer is.
//!
//! # What is out of scope, and why
//!
//! Capabilities. The command line has a filesystem behind it and a page does
//! not, so a program that reads a file is answered by one and refused by the
//! other, on purpose (#591). That difference is a decision rather than a
//! drift, and it is held by `deed-wasm`'s own tests. Every program here is one
//! a page can run.

use std::path::PathBuf;
use std::process::Command;

const DEED: &str = env!("CARGO_BIN_EXE_deed");

/// A scratch directory, named so parallel tests do not collide.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("deed-surfaces-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, contents: &str) -> PathBuf {
        let path = self.0.join("main.deed");
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What asking a surface to run the tests came to.
#[derive(Debug, PartialEq, Eq)]
enum Tested {
    /// Nothing ran, and why.
    Refused(&'static str),
    /// Nothing ran and nothing said so, which is the shape #823 was.
    SaidNothing,
    /// What ran, by name, and whether it passed. A property is named for the
    /// function it was generated from, since the case count is a
    /// configuration rather than an answer.
    Ran(Vec<(String, bool)>),
}

/// What asking a surface to run `main` came to.
#[derive(Debug, PartialEq, Eq)]
enum Run {
    Refused(&'static str),
    Ran { output: Vec<String>, ok: bool },
}

// -- the command line ------------------------------------------------------

fn cli(subcommand: &str, path: &PathBuf) -> (String, String, bool) {
    let output = Command::new(DEED)
        .arg(subcommand)
        .arg(path)
        .output()
        .expect("the binary should run");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.success(),
    )
}

/// Whether the command line would refuse to run this at all.
///
/// Asked of `deed check` rather than read out of the run, because a test that
/// fails prints a diagnostic too and the two are not the same refusal. This is
/// also the command line's own order: it checks, and only then runs.
fn cli_checks(path: &PathBuf) -> bool {
    let output = Command::new(DEED)
        .arg("check")
        .arg("--format")
        .arg("json")
        .arg(path)
        .output()
        .expect("the binary should run");
    !String::from_utf8_lossy(&output.stdout).contains("\"severity\":\"error\"")
}

fn cli_tested(source: &str) -> Tested {
    let scratch = Scratch::new("test");
    let path = scratch.write(source);
    if !cli_checks(&path) {
        return Tested::Refused("does not check");
    }
    let (stdout, _, _) = cli("test", &path);
    if stdout.contains("no tests found") {
        return Tested::Refused("nothing to test");
    }

    let mut ran = Vec::new();
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("  ") else {
            continue;
        };
        let (passed, name) = match rest.strip_prefix("ok    ") {
            Some(name) => (true, name),
            None => match rest.strip_prefix("FAIL  ") {
                Some(name) => (false, name),
                None => continue,
            },
        };
        ran.push((normalise(name), passed));
    }
    ran.sort();
    Tested::Ran(ran)
}

fn cli_run(source: &str) -> Run {
    let scratch = Scratch::new("run");
    let path = scratch.write(source);
    if !cli_checks(&path) {
        return Run::Refused("does not check");
    }
    let (stdout, stderr, ok) = cli("run", &path);
    if stderr.contains(deed_driver::NOTHING_TO_RUN) {
        return Run::Refused("nothing to run");
    }
    Run::Ran {
        // What the program printed comes before the failure that ended it, so
        // the first diagnostic is where the program's own output stops.
        output: stdout
            .lines()
            .take_while(|line| !line.starts_with("error["))
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
        ok,
    }
}

/// `property twice (9 cases)` and `{"function":"twice","cases":9}` are the
/// same answer, and the count is a setting rather than a result.
fn normalise(name: &str) -> String {
    match name.strip_prefix("property ") {
        Some(rest) => format!("property {}", rest.split(" (").next().unwrap_or(rest)),
        None => name.to_string(),
    }
}

// -- the artifact ----------------------------------------------------------

/// One field out of one JSON line, which is all this needs and is why there is
/// no parser here.
fn field(line: &str, key: &str) -> Option<String> {
    let start = line.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = &line[start..];
    Some(match rest.strip_prefix('"') {
        Some(quoted) => quoted[..quoted.find('"')?].to_string(),
        None => rest[..rest.find([',', '}'])?].to_string(),
    })
}

fn wasm_tested(source: &str) -> Tested {
    let json = deed_wasm::test_source(source);
    if json.contains("\"kind\":\"refused\"") {
        return Tested::Refused("does not check");
    }

    let mut ran = Vec::new();
    for line in json.lines() {
        if line.contains("\"kind\":\"test\"") {
            let name = field(line, "name").expect("a test line names itself");
            ran.push((name, field(line, "passed").as_deref() == Some("true")));
        } else if line.contains("\"kind\":\"property\"") {
            let name = field(line, "function").expect("a property names its function");
            ran.push((
                format!("property {name}"),
                field(line, "passed").as_deref() == Some("true"),
            ));
        }
    }

    // Read out of what it said rather than out of what it left out. Counting
    // the lines and calling zero of them "no tests" would let the surface go
    // back to answering with silence and still look like it agreed.
    let Some(summary) = json
        .lines()
        .find(|line| line.contains("\"kind\":\"summary\""))
    else {
        return Tested::SaidNothing;
    };
    let counted: usize = ["passed", "failed"]
        .iter()
        .filter_map(|key| field(summary, key)?.parse::<usize>().ok())
        .sum();
    assert_eq!(
        counted,
        ran.len(),
        "the summary disagrees with the lines above it: {json}"
    );
    if ran.is_empty() {
        return Tested::Refused("nothing to test");
    }
    ran.sort();
    Tested::Ran(ran)
}

fn wasm_run(source: &str) -> Run {
    let json = deed_wasm::run_source(source);
    if json.contains("\"kind\":\"refused\"") {
        return Run::Refused("does not check");
    }
    if json.contains(deed_driver::NOTHING_TO_RUN) {
        return Run::Refused("nothing to run");
    }
    Run::Ran {
        output: json
            .lines()
            .filter(|line| line.contains("\"kind\":\"output\""))
            .map(|line| field(line, "line").expect("an output line carries one"))
            .collect(),
        ok: json.contains("\"ok\":true"),
    }
}

// -- the cases -------------------------------------------------------------

fn agree_on_tests(source: &str, expected: Tested) {
    let from_cli = cli_tested(source);
    let from_wasm = wasm_tested(source);
    assert_eq!(from_cli, from_wasm, "the two surfaces disagree:\n{source}");
    assert_eq!(from_cli, expected, "and they agree on the wrong thing");
}

fn agree_on_running(source: &str, expected: Run) {
    let from_cli = cli_run(source);
    let from_wasm = wasm_run(source);
    assert_eq!(from_cli, from_wasm, "the two surfaces disagree:\n{source}");
    assert_eq!(from_cli, expected, "and they agree on the wrong thing");
}

#[test]
fn a_test_that_passes_is_reported_by_both() {
    agree_on_tests(
        "module main\n\ntest \"one is one\" {\n    assert 1 == 1\n}\n",
        Tested::Ran(vec![("one is one".to_string(), true)]),
    );
}

#[test]
fn a_test_that_fails_is_reported_by_both() {
    agree_on_tests(
        "module main\n\ntest \"never\" {\n    assert 1 == 2\n}\n",
        Tested::Ran(vec![("never".to_string(), false)]),
    );
}

/// #823. The artifact answered this with an empty string, which on that
/// surface is what a well formed program looks like.
#[test]
fn a_file_with_no_tests_is_told_so_by_both() {
    agree_on_tests(
        "module main\n\nfn f() -> Int {\n    1\n}\n",
        Tested::Refused("nothing to test"),
    );
}

/// #823. The artifact ran the test and said it passed, in a file the checker
/// had already turned down.
#[test]
fn a_file_that_does_not_check_is_refused_by_both() {
    agree_on_tests(
        "module main\n\nfn f() -> Int {\n    nonesuch\n}\n\ntest \"t\" {\n    assert 1 == 1\n}\n",
        Tested::Refused("does not check"),
    );
}

/// #827. The property is the test nobody wrote, and the artifact was not
/// running it. This one fails: `n + n` overflows near the top of the range,
/// so the `ensures` is not true for every `n > 0`.
#[test]
fn a_property_a_contract_generates_is_run_by_both() {
    agree_on_tests(
        "module main\n\nfn twice(n: Int) -> Int\n  where\n    n > 0,\n  ensures\n    \
         ok  => result > n,\n{\n    n + n\n}\n\ntest \"twice doubles\" {\n    \
         assert twice(3) == 6\n}\n",
        Tested::Ran(vec![
            ("property twice".to_string(), false),
            ("twice doubles".to_string(), true),
        ]),
    );
}

#[test]
fn a_property_that_holds_is_reported_by_both() {
    agree_on_tests(
        "module main\n\nfn keep(n: Int) -> Int\n  ensures\n    ok  => result == n,\n{\n    n\n}\n",
        Tested::Ran(vec![("property keep".to_string(), true)]),
    );
}

#[test]
fn what_main_printed_is_the_same_on_both() {
    agree_on_running(
        "module main\n\nfn main(sys: System) -> Int\n  uses\n    Io.write,\n{\n    \
         Io.write(sys.console, \"hello\")\n    0\n}\n",
        Run::Ran {
            output: vec!["hello".to_string()],
            ok: true,
        },
    );
}

#[test]
fn a_library_is_told_there_is_nothing_to_run_by_both() {
    agree_on_running(
        "module main\n\nfn f() -> Int {\n    1\n}\n",
        Run::Refused("nothing to run"),
    );
}

#[test]
fn a_main_that_does_not_check_is_refused_by_both() {
    agree_on_running(
        "module main\n\nfn main() -> Int {\n    nonesuch\n}\n",
        Run::Refused("does not check"),
    );
}

/// A run that ends in a contract turning a value down is a failure on both,
/// and it is the case where something did run before it went wrong.
///
/// The argument comes from the clock rather than from a literal. A literal
/// would be settled at the call site and the file would be turned down before
/// anything ran, which is a different answer to a different question (#145).
#[test]
fn a_contract_that_refuses_at_run_time_fails_on_both() {
    agree_on_running(
        "module main\n\nfn halve(n: Int) -> Int\n  where\n    n > 0,\n{\n    n / 2\n}\n\n\
         fn main(sys: System) -> Int\n  uses\n    Io.write,\n    Io.now,\n{\n    \
         Io.write(sys.console, \"before\")\n    halve(0 - Io.now(sys.clock) - 1)\n}\n",
        Run::Ran {
            output: vec!["before".to_string()],
            ok: false,
        },
    );
}
