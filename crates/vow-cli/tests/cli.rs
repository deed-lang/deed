//! The `vow` binary, exercised by actually running it.
//!
//! Testing the library behind a command line tool and calling it done is how
//! tools ship with broken argument handling. These spawn the binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const VOW: &str = env!("CARGO_BIN_EXE_vow");
const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.vow");
const RUNNABLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/counter.vow");

fn run(args: &[&str]) -> Output {
    Command::new(VOW)
        .args(args)
        .output()
        .expect("the vow binary should run")
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

/// A scratch directory, named so parallel tests do not collide.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("vow-cli-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Two problems found by two different passes, the type error written first.
const MIXED: &str = "\
module a

record R { a: Int }

fn f() -> R { R { } }

fn g() -> Int { balanse() }
";

// -- the happy path --------------------------------------------------------

#[test]
fn checking_the_worked_example_is_silent_and_succeeds() {
    let output = run(&["check", EXAMPLE]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "",
        "a clean check should say nothing at all"
    );
}

#[test]
fn obligations_are_reported_with_their_tier() {
    let output = run(&["check", RUNNABLE, "--obligations"]);
    assert_eq!(code(&output), 0);

    let text = stdout(&output);
    assert!(text.contains("proven"), "{text}");
    assert!(text.contains("Positive"), "{text}");
    // All three tiers exist now. A `0 tested` line used to need a footnote
    // explaining that the tier was unbuilt rather than empty.
    assert!(text.contains("tested"), "{text}");
    assert!(text.contains("guarded"), "{text}");
    assert!(
        !text.contains("does not exist yet"),
        "the footnote should be gone:\n{text}"
    );
}

// -- reporting problems ----------------------------------------------------

#[test]
fn errors_exit_with_one() {
    let scratch = Scratch::new("errors");
    let file = scratch.write("broken.vow", MIXED);

    let output = run(&["check", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    assert!(stdout(&output).contains("2 errors, 0 warnings"));
}

#[test]
fn diagnostics_are_ordered_by_position_not_by_pass() {
    // The missing-fields error comes from the type checker and the unknown-name
    // error from resolution, and the type error is written first in the file.
    // A reader works down the file; which pass noticed is not their problem.
    let scratch = Scratch::new("order");
    let file = scratch.write("broken.vow", MIXED);

    let text = stdout(&run(&["check", file.to_str().unwrap()]));
    let missing_fields = text
        .find("VOW4002")
        .expect("expected a missing-fields error");
    let unknown_name = text
        .find("VOW3001")
        .expect("expected an unknown-name error");
    assert!(
        missing_fields < unknown_name,
        "diagnostics came out in pass order:\n{text}"
    );
}

#[test]
fn every_pass_runs_even_after_an_earlier_one_fails() {
    // A parse error must not hide what the later passes would have found.
    let scratch = Scratch::new("allpasses");
    let file = scratch.write(
        "broken.vow",
        "module a\n\nfn broken(x: ) -> Int { 0 }\n\nfn g() -> Int { balanse() }\n",
    );

    let text = stdout(&run(&["check", file.to_str().unwrap()]));
    assert!(text.contains("VOW2001"), "expected a parse error:\n{text}");
    assert!(
        text.contains("VOW3001"),
        "the resolver should still have run:\n{text}"
    );
}

#[test]
fn warnings_alone_do_not_fail_the_check() {
    let scratch = Scratch::new("warnings");
    let file = scratch.write(
        "warn.vow",
        "module a\n\nuse other.{Used, Spare}\n\nfn f() -> Used { }\n",
    );

    let output = run(&["check", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "warnings should not fail the build");
    assert!(stdout(&output).contains("0 errors, 1 warning"));
}

// -- output formats --------------------------------------------------------

#[test]
fn json_emits_one_object_per_line() {
    let scratch = Scratch::new("json");
    let file = scratch.write("broken.vow", MIXED);

    let output = run(&["check", "--format", "json", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);

    let text = stdout(&output);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    for line in lines {
        assert!(line.starts_with("{\"kind\":\"diagnostic\""), "{line}");
        assert!(line.ends_with('}'), "{line}");
        assert!(line.contains("\"severity\":\"error\""), "{line}");
    }
    // No human rendering leaking into the machine format.
    assert!(!text.contains("-->"), "{text}");
}

#[test]
fn json_obligations_are_distinguishable_from_diagnostics() {
    let output = run(&["check", "--format=json", "--obligations", EXAMPLE]);
    assert_eq!(code(&output), 0);

    let text = stdout(&output);
    assert!(text.contains("{\"kind\":\"obligation\""), "{text}");
    assert!(text.contains("\"tier\":\"proven\""), "{text}");
}

// -- paths -----------------------------------------------------------------

#[test]
fn a_directory_is_searched_for_vow_files() {
    let scratch = Scratch::new("dir");
    scratch.write("a.vow", "module a\n\nfn f() -> Int { 0 }\n");
    scratch.write("nested/b.vow", "module b\n\nfn g() -> Int { nope() }\n");
    scratch.write("notes.txt", "not vow source");

    let output = run(&["check", scratch.path().to_str().unwrap()]);
    assert_eq!(code(&output), 1, "the nested file has an error");
    assert!(stdout(&output).contains("VOW3001"));
}

#[test]
fn paths_are_printed_with_forward_slashes() {
    let scratch = Scratch::new("slashes");
    let file = scratch.write(
        "nested/broken.vow",
        "module a\n\nfn f() -> Int { nope() }\n",
    );

    let text = stdout(&run(&["check", file.to_str().unwrap()]));
    assert!(text.contains("nested/broken.vow"), "{text}");
    assert!(!text.contains("nested\\broken.vow"), "{text}");
}

// -- invocation errors -----------------------------------------------------

#[test]
fn a_missing_path_is_a_usage_error() {
    let output = run(&["check", "definitely-not-here.vow"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("definitely-not-here.vow"));
}

#[test]
fn a_directory_with_no_vow_files_is_a_usage_error() {
    let scratch = Scratch::new("empty");
    scratch.write("readme.txt", "nothing here");

    let output = run(&["check", scratch.path().to_str().unwrap()]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("no `.vow` files"));
}

#[test]
fn an_unknown_option_is_a_usage_error_with_the_usage_text() {
    let output = run(&["check", "--oblgations", EXAMPLE]);
    assert_eq!(code(&output), 2);
    let text = stderr(&output);
    assert!(text.contains("unknown option"), "{text}");
    assert!(text.contains("Usage:"), "{text}");
}

#[test]
fn check_with_no_paths_is_a_usage_error() {
    let output = run(&["check"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("at least one path"));
}

// -- meta ------------------------------------------------------------------

#[test]
fn help_and_version_succeed() {
    let help = run(&["--help"]);
    assert_eq!(code(&help), 0);
    assert!(stdout(&help).contains("Usage:"));
    assert!(stdout(&help).contains("--obligations"));

    let version = run(&["--version"]);
    assert_eq!(code(&version), 0);
    assert!(stdout(&version).starts_with("vow "));
}

#[test]
fn no_arguments_at_all_explains_itself() {
    let output = run(&[]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("vow check"));
}

// -- running tests ---------------------------------------------------------

#[test]
fn the_runnable_example_passes() {
    let output = run(&["test", RUNNABLE]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));

    let text = stdout(&output);
    assert!(text.contains("0 failed"), "{text}");
    assert!(text.contains("ok    bumping twice adds twice"), "{text}");
    assert!(
        text.contains("ok    the question mark stops the rest of the body"),
        "{text}"
    );
    assert!(!text.contains("FAIL"), "{text}");
}

#[test]
fn a_failing_test_exits_with_one_and_shows_why() {
    let scratch = Scratch::new("failing");
    let file = scratch.write(
        "failing.vow",
        "module a\n\nfn double(n: Int) -> Int { n + n }\n\ntest \"wrong\" {\n  assert double(3) == 7\n}\n",
    );

    let output = run(&["test", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);

    let text = stdout(&output);
    assert!(text.contains("FAIL  wrong"), "{text}");
    assert!(text.contains("VOW6001"), "{text}");
    assert!(text.contains("left is 6, right is 7"), "{text}");
    assert!(text.contains("0 passed, 1 failed"), "{text}");
}

#[test]
fn nothing_is_run_when_the_file_does_not_check() {
    // Running code that does not check would answer a question nobody asked,
    // and the failure would be about the wrong thing.
    let scratch = Scratch::new("broken-test");
    let file = scratch.write(
        "broken.vow",
        "module a\n\nfn f() -> Int { nope() }\n\ntest \"never runs\" {\n  assert true\n}\n",
    );

    let output = run(&["test", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);

    let text = stdout(&output);
    assert!(text.contains("VOW3001"), "{text}");
    assert!(
        !text.contains("never runs"),
        "tests were run anyway:\n{text}"
    );
}

#[test]
fn a_file_with_no_tests_says_so() {
    let scratch = Scratch::new("no-tests");
    let file = scratch.write("quiet.vow", "module a\n\nfn f() -> Int { 0 }\n");

    let output = run(&["test", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0);
    assert!(stdout(&output).contains("no tests found"));
}

#[test]
fn check_does_not_run_tests() {
    let scratch = Scratch::new("check-only");
    let file = scratch.write(
        "failing.vow",
        "module a\n\ntest \"would fail\" {\n  assert false\n}\n",
    );

    let output = run(&["check", file.to_str().unwrap()]);
    assert_eq!(
        code(&output),
        0,
        "checking should not care that it would fail"
    );
    assert_eq!(stdout(&output), "");
}
