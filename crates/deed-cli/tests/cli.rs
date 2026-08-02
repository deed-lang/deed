//! The `deed` binary, exercised by actually running it.
//!
//! Testing the library behind a command line tool and calling it done is how
//! tools ship with broken argument handling. These spawn the binary.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const DEED: &str = env!("CARGO_BIN_EXE_deed");
const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.deed");
const RUNNABLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/counter.deed");
const HELLO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/hello.deed");
const CONFIG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/config.deed");
const TODO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/todo.deed");
const JOURNAL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/journal.deed");
const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");

fn run(args: &[&str]) -> Output {
    Command::new(DEED)
        .args(args)
        .output()
        .expect("the deed binary should run")
}

#[test]
fn runtime_profile_is_only_for_run() {
    let output = run(&["check", "--profile-runtime", EXAMPLE]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("only for `deed run`"),
        "{}",
        stderr(&output)
    );
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn run_can_report_runtime_profile() {
    let scratch = Scratch::new("runtime-profile");
    let file = scratch.write(
        "profiled.deed",
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         effect Counter {\n\
         \x20 fn bump(by: Int) -> Int\n\
         }\n\n\
         handler InMemory implements Counter {\n\
         \x20 state count: Int\n\n\
         \x20 fn bump(by) -> Int {\n\
         \x20   count = count + by\n\
         \x20   count\n\
         \x20 }\n\
         }\n\n\
         fn needs(value: Positive) -> Int { value }\n\n\
         fn work() -> Int {\n\
         \x20 with InMemory { count: 0 } {\n\
         \x20   let n = Counter.bump(1)\n\
         \x20   needs(n)\n\
         \x20 }\n\
         }\n\n\
         fn main(sys: System) -> Int {\n\
         \x20 work()\n\
         \x20 0\n\
         }\n",
    );

    let output = run(&["run", "--profile-runtime", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}{}", stdout(&output), stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("runtime profile:"), "{text}");
    assert!(text.contains("contract"), "{text}");
    assert!(text.contains("handler"), "{text}");
    assert!(text.contains("needs"), "{text}");
    assert!(text.contains("bump"), "{text}");
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
        let dir = std::env::temp_dir().join(format!("deed-cli-{tag}-{nanos}"));
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
fn compiled_tests_report_the_same_complete_count_as_the_interpreter() {
    let output = run(&[
        "test",
        "--compiled",
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/diverge.deed"),
    ]);
    assert_eq!(code(&output), 0, "{}{}", stdout(&output), stderr(&output));
    assert!(
        stdout(&output).contains("1 passed, 0 failed"),
        "{}",
        stdout(&output)
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
    let file = scratch.write("broken.deed", MIXED);

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
    let file = scratch.write("broken.deed", MIXED);

    let text = stdout(&run(&["check", file.to_str().unwrap()]));
    let missing_fields = text
        .find("DEED4002")
        .expect("expected a missing-fields error");
    let unknown_name = text
        .find("DEED3001")
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
        "broken.deed",
        "module a\n\nfn broken(x: ) -> Int { 0 }\n\nfn g() -> Int { balanse() }\n",
    );

    let text = stdout(&run(&["check", file.to_str().unwrap()]));
    assert!(text.contains("DEED2001"), "expected a parse error:\n{text}");
    assert!(
        text.contains("DEED3001"),
        "the resolver should still have run:\n{text}"
    );
}

#[test]
fn warnings_alone_do_not_fail_the_check() {
    let scratch = Scratch::new("warnings");
    scratch.write(
        "other.deed",
        "module other\n\nrecord Used { n: Int }\n\nrecord Spare { n: Int }\n",
    );
    scratch.write(
        "warn.deed",
        "module a\n\nuse other.{Used, Spare}\n\nfn f() -> Used { Used { n: 0 } }\n",
    );

    let output = run(&["check", scratch.path().to_str().unwrap()]);
    assert_eq!(
        code(&output),
        0,
        "warnings should not fail the build:\n{}",
        stdout(&output)
    );
    assert!(stdout(&output).contains("0 errors, 1 warning"));
}

#[test]
fn an_import_of_a_module_that_is_not_there_is_an_error() {
    let scratch = Scratch::new("no-module");
    let file = scratch.write(
        "lonely.deed",
        "module a\n\nuse other.{Thing}\n\nfn f() -> Thing {}\n",
    );

    let output = run(&["check", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    assert!(stdout(&output).contains("DEED3007"), "{}", stdout(&output));
}

// -- output formats --------------------------------------------------------

#[test]
fn json_emits_one_object_per_line() {
    let scratch = Scratch::new("json");
    let file = scratch.write("broken.deed", MIXED);

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
fn a_directory_is_searched_for_deed_files() {
    let scratch = Scratch::new("dir");
    scratch.write("a.deed", "module a\n\nfn f() -> Int { 0 }\n");
    scratch.write("nested/b.deed", "module b\n\nfn g() -> Int { nope() }\n");
    scratch.write("notes.txt", "not deed source");

    let output = run(&["check", scratch.path().to_str().unwrap()]);
    assert_eq!(code(&output), 1, "the nested file has an error");
    assert!(stdout(&output).contains("DEED3001"));
}

#[test]
fn paths_are_printed_with_forward_slashes() {
    let scratch = Scratch::new("slashes");
    let file = scratch.write(
        "nested/broken.deed",
        "module a\n\nfn f() -> Int { nope() }\n",
    );

    let text = stdout(&run(&["check", file.to_str().unwrap()]));
    assert!(text.contains("nested/broken.deed"), "{text}");
    assert!(!text.contains("nested\\broken.deed"), "{text}");
}

// -- invocation errors -----------------------------------------------------

#[test]
fn a_missing_path_is_a_usage_error() {
    let output = run(&["check", "definitely-not-here.deed"]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("definitely-not-here.deed"));
}

#[test]
fn a_directory_with_no_deed_files_is_a_usage_error() {
    let scratch = Scratch::new("empty");
    scratch.write("readme.txt", "nothing here");

    let output = run(&["check", scratch.path().to_str().unwrap()]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("no `.deed` files"));
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
    assert!(stdout(&help).contains("--profile-runtime"));

    let version = run(&["--version"]);
    assert_eq!(code(&version), 0);
    assert!(stdout(&version).starts_with("deed "));
}

#[test]
fn no_arguments_at_all_explains_itself() {
    let output = run(&[]);
    assert_eq!(code(&output), 2);
    let text = stderr(&output);
    assert!(text.contains("there is no REPL"), "{text}");
    assert!(text.contains("scratch `.deed` file"), "{text}");
}

#[test]
fn repl_is_refused_with_a_whole_program_alternative() {
    let output = run(&["repl"]);
    assert_eq!(code(&output), 2);
    let text = stderr(&output);
    assert!(text.contains("there is no REPL"), "{text}");
    assert!(text.contains("playground"), "{text}");
}

// -- the language server ---------------------------------------------------

#[test]
fn lsp_speaks_the_protocol_on_stdin_and_stdout() {
    // The library has the session tests. This one is about the wiring: that
    // the subcommand exists, that it reads stdin, and that nothing else the
    // binary prints ends up on stdout, which is the protocol.
    let body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}";
    let input = format!("Content-Length: {}\r\n\r\n{body}", body.len());

    let mut child = Command::new(DEED)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the deed binary should run");

    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(input.as_bytes())
        .unwrap();

    let output = child.wait_with_output().expect("it should finish");
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = stdout(&output);
    assert!(text.starts_with("Content-Length: "), "{text}");
    assert!(text.contains("\"textDocumentSync\":1"), "{text}");
}

#[test]
fn lsp_takes_no_arguments() {
    // An editor starts it with no arguments. A path on the line means somebody
    // typed it by hand expecting `check`, and doing nothing about it would
    // look like a hang.
    let output = run(&["lsp", EXAMPLE]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("takes no arguments"),
        "{}",
        stderr(&output)
    );
}

// -- the agent surface -----------------------------------------------------

#[test]
fn mcp_speaks_the_protocol_on_stdin_and_stdout() {
    // Same shape as the language server test above and for the same reason:
    // the crate has the session tests, this one is about the wiring. The
    // framing is different, though, and that difference is the point of
    // checking it here. MCP is one JSON message a line, so a stray
    // `Content-Length` header would be a client that reads nothing.
    let initialize = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}";
    let list = "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}";
    let input = format!("{initialize}\n{list}\n");

    let mut child = Command::new(DEED)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the deed binary should run");

    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(input.as_bytes())
        .unwrap();

    let output = child.wait_with_output().expect("it should finish");
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = stdout(&output);
    assert!(!text.starts_with("Content-Length"), "{text}");
    assert_eq!(text.lines().count(), 2, "one answer a line: {text}");
    assert!(text.contains("\"serverInfo\""), "{text}");
    assert!(text.contains("\"deed_check\""), "{text}");
}

#[test]
fn mcp_takes_no_arguments() {
    let output = run(&["mcp", EXAMPLE]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("takes no arguments"),
        "{}",
        stderr(&output)
    );
}

// -- explain ---------------------------------------------------------------

#[test]
fn explain_prints_the_reasoning_for_a_known_code() {
    let output = run(&["explain", "DEED4025"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("DEED4025"), "{text}");
    assert!(text.contains("BROKEN_PRECONDITION"), "{text}");
    // The reasoning comes from the doc comment in typeck/src/codes.rs.
    assert!(text.contains("precondition"), "{text}");
}

#[test]
fn explain_accepts_a_constant_name_too() {
    let output = run(&["explain", "BROKEN_PRECONDITION"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("DEED4025"), "{}", stdout(&output));
}

#[test]
fn explain_unknown_code_is_a_usage_error() {
    let output = run(&["explain", "DEED9999"]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("no page for"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn explain_without_a_code_is_a_usage_error() {
    let output = run(&["explain"]);
    assert_eq!(code(&output), 2);
}

// -- running tests ---------------------------------------------------------

#[test]
fn every_example_passes_its_own_tests() {
    // The whole directory, not one file. Adding an example with a property
    // that fails used to be invisible here, and it happened: `hello.deed` had a
    // `twice` whose postcondition the generator broke by overflowing.
    let output = run(&["test", EXAMPLES]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));
    assert!(stdout(&output).contains("0 failed"), "{}", stdout(&output));
}

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
        "failing.deed",
        "module a\n\nfn double(n: Int) -> Int { n + n }\n\ntest \"wrong\" {\n  assert double(3) == 7\n}\n",
    );

    let output = run(&["test", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);

    let text = stdout(&output);
    assert!(text.contains("FAIL  wrong"), "{text}");
    assert!(text.contains("DEED6001"), "{text}");
    assert!(text.contains("left is 6, right is 7"), "{text}");
    assert!(text.contains("0 passed, 1 failed"), "{text}");
}

#[test]
fn nothing_is_run_when_the_file_does_not_check() {
    // Running code that does not check would answer a question nobody asked,
    // and the failure would be about the wrong thing.
    let scratch = Scratch::new("broken-test");
    let file = scratch.write(
        "broken.deed",
        "module a\n\nfn f() -> Int { nope() }\n\ntest \"never runs\" {\n  assert true\n}\n",
    );

    let output = run(&["test", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);

    let text = stdout(&output);
    assert!(text.contains("DEED3001"), "{text}");
    assert!(
        !text.contains("never runs"),
        "tests were run anyway:\n{text}"
    );
}

#[test]
fn a_file_with_no_tests_says_so() {
    let scratch = Scratch::new("no-tests");
    let file = scratch.write("quiet.deed", "module a\n\nfn f() -> Int { 0 }\n");

    let output = run(&["test", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0);
    assert!(stdout(&output).contains("no tests found"));
}

#[test]
fn check_does_not_run_tests() {
    let scratch = Scratch::new("check-only");
    let file = scratch.write(
        "failing.deed",
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

// -- running a program -----------------------------------------------------

#[test]
fn the_hello_example_runs() {
    let output = run(&["run", HELLO]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));

    let text = stdout(&output);
    assert!(text.contains("hello, "), "{text}");
    assert!(text.contains("world"), "{text}");
}

#[test]
fn nothing_is_run_when_the_program_does_not_check() {
    let scratch = Scratch::new("broken-run");
    let file = scratch.write(
        "broken.deed",
        "module a\n\nfn main(sys: System) -> Int\n  uses Io.write,\n{\n  Io.write(sys.console, nope())\n  0\n}\n",
    );

    let output = run(&["run", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    assert!(stdout(&output).contains("DEED3001"));
}

#[test]
fn a_failed_contract_in_main_exits_with_one() {
    let scratch = Scratch::new("main-contract");
    let file = scratch.write(
        "bad.deed",
        "module a\n\nfn main(sys: System) -> Int\n  ensures\n    ok => result > 100,\n{\n  1\n}\n",
    );

    let output = run(&["run", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    assert!(stdout(&output).contains("DEED6003"), "{}", stdout(&output));
}

#[test]
fn compiled_run_reports_the_same_precondition_location_and_message() {
    let scratch = Scratch::new("compiled-precondition");
    let file = scratch.write(
        "broken.deed",
        "module a\n\n\
fn halve(n: Int) -> Int\n\
  where n > 0,\n\
{ n / 2 }\n\n\
fn answer(n: Int) -> Int {\n\
    halve(n)\n\
}\n\n\
fn main() -> Int {\n\
    answer(-4)\n\
}\n",
    );

    let interpreted = run(&["run", file.to_str().unwrap()]);
    let compiled = run(&["run", "--compiled", file.to_str().unwrap()]);
    assert_eq!(code(&interpreted), 1, "{}", stderr(&interpreted));
    assert_eq!(code(&compiled), 1, "{}", stderr(&compiled));

    let interpreted = stdout(&interpreted);
    let compiled = stdout(&compiled);

    let interpreted_lines: Vec<&str> = interpreted.lines().collect();
    let compiled_lines: Vec<&str> = compiled.lines().collect();
    assert_eq!(
        compiled_lines.first(),
        interpreted_lines.first(),
        "{compiled}"
    );
    assert_eq!(
        compiled_lines.get(1),
        interpreted_lines.get(1),
        "{compiled}"
    );
    assert_eq!(
        compiled_lines.get(3),
        interpreted_lines.get(3),
        "{compiled}"
    );

    let interpreted_caret = interpreted_lines
        .get(4)
        .and_then(|line| line.find('^'))
        .expect("the interpreted diagnostic should contain a caret");
    let compiled_caret = compiled_lines
        .get(4)
        .and_then(|line| line.find('^'))
        .expect("the compiled diagnostic should contain a caret");
    assert_eq!(compiled_caret, interpreted_caret, "{compiled}");
}

#[test]
fn running_something_with_no_main_is_a_usage_error() {
    let scratch = Scratch::new("no-main");
    let file = scratch.write("quiet.deed", "module a\n\nfn f() -> Int { 0 }\n");

    let output = run(&["run", file.to_str().unwrap()]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("no `main`"));
}

#[test]
fn two_mains_is_a_question_not_a_choice() {
    let scratch = Scratch::new("two-mains");
    scratch.write(
        "one.deed",
        "module one\n\nfn main(sys: System) -> Int { 0 }\n",
    );
    scratch.write(
        "two.deed",
        "module two\n\nfn main(sys: System) -> Int { 0 }\n",
    );

    for arguments in [
        vec!["run", scratch.path().to_str().unwrap()],
        vec!["run", "--compiled", scratch.path().to_str().unwrap()],
    ] {
        let output = run(&arguments);
        assert_eq!(code(&output), 2);
        assert!(stderr(&output).contains("more than one `main`"));
    }
}

#[test]
fn a_program_cannot_write_without_being_handed_a_console() {
    let scratch = Scratch::new("no-console");
    let file = scratch.write(
        "sneaky.deed",
        "module a\n\nfn sneaky() -> ()\n  uses Io.write,\n{\n  Io.write(Console, \"hello\")\n}\n\nfn main(sys: System) -> Int { 0 }\n",
    );

    let output = run(&["run", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    let text = stdout(&output);
    assert!(text.contains("DEED4019"), "{text}");
    assert!(text.contains("only received"), "{text}");
}

#[test]
fn dir_decides_what_the_program_can_reach() {
    let scratch = Scratch::new("dir");
    scratch.write("secret.txt", "the outer one");
    let inner = scratch.path().join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    std::fs::write(inner.join("fine.txt"), "the inner one").unwrap();

    let program = scratch.write(
        "reader.deed",
        "module a\n\nfn main(sys: System) -> Int\n  uses\n    Io.write,\n    Io.read,\n{\n  match Io.read(sys.files, \"fine.txt\") {\n    ok(text) => Io.write(sys.console, text),\n    err(why) => Io.write(sys.console, why),\n  }\n  match Io.read(sys.files, \"secret.txt\") {\n    ok(text) => Io.write(sys.console, text),\n    err(why) => Io.write(sys.console, why),\n  }\n  0\n}\n",
    );

    // Rooted at `inner`, the same program reads one file and cannot see the
    // other, and nothing about the program changed.
    let output = run(&[
        "run",
        program.to_str().unwrap(),
        "--dir",
        inner.to_str().unwrap(),
    ]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));

    let text = stdout(&output);
    assert!(text.contains("the inner one"), "{text}");
    assert!(!text.contains("the outer one"), "escaped the root:\n{text}");
}

#[test]
fn the_config_example_runs_and_reaches_nothing_it_was_not_given() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let output = run(&["run", CONFIG, "--dir", dir]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));

    let text = stdout(&output);
    assert!(text.contains("found it"), "{text}");
    assert!(text.contains("no way out of a `Dir`"), "{text}");
    assert!(text.contains("used the fallback"), "{text}");
}

#[test]
fn the_todo_example_reads_a_real_file_and_reports_on_it() {
    // The end to end one. It reads a file, takes the lines apart, counts them,
    // collects some of them and prints a number, so every piece added this
    // week has to work at once for this to say anything.
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let output = run(&["run", TODO, "--dir", dir]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));

    let text = stdout(&output);
    assert!(text.contains("5 of 6 done"), "{text}");
    assert!(
        text.contains("still open: 6. work out what a trait is"),
        "{text}"
    );
    // A `\r` left on the end of a title would print the rest of the line over
    // the top of itself, which is how the line endings in the data file were
    // found in the first place. `trim` is what takes it off now, so this holds
    // whatever the file on disk happens to end its lines with.
    assert!(!text.contains('\r'), "a carriage return survived:\n{text}");
}

#[test]
fn the_journal_example_writes_a_file_and_cannot_write_outside_the_directory() {
    // The other half of the capability story. `config.deed` shows that reading
    // cannot escape a `Dir`; this shows that writing cannot either, and that
    // it is the same check rather than a second one that agrees today.
    let scratch = Scratch::new("journal");
    let dir = scratch.path().to_str().unwrap().to_string();

    let output = run(&["run", JOURNAL, "--dir", &dir]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("saved"), "{text}");
    assert!(text.contains("no way out of a `Dir`"), "{text}");
    assert!(!text.contains("which is a bug"), "{text}");

    // Making a directory hands back authority over somewhere the caller could
    // already reach, and what comes back is narrower rather than wider: `..`
    // out of the new one is refused by the same check that refuses it out of
    // the old one.
    assert!(text.contains("made a shelf, and"), "{text}");
    assert!(scratch.path().join("shelf").is_dir(), "{text}");

    let written = std::fs::read_to_string(scratch.path().join("journal.txt")).unwrap();
    // Stamped with the machine's clock, so the line is checked by shape rather
    // than by equality: `Io.epoch` is in the row precisely because it is the
    // thing that makes two runs differ.
    let stamp_of = |line: &str| -> i64 {
        let (millis, rest) = line
            .split_once(' ')
            .unwrap_or_else(|| panic!("a stamped line: {line:?}"));
        assert_eq!(rest, "wrote a file for the first time");
        millis
            .parse()
            .unwrap_or_else(|_| panic!("milliseconds: {millis:?}"))
    };
    assert!(stamp_of(&written) > 1_577_836_800_000, "{written}");

    // Running it again appends rather than replacing, which is the only thing
    // that makes it a journal.
    let output = run(&["run", JOURNAL, "--dir", &dir]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let written = std::fs::read_to_string(scratch.path().join("journal.txt")).unwrap();
    let lines: Vec<&str> = written.split('\n').collect();
    assert_eq!(lines.len(), 2, "{written}");
    assert!(stamp_of(lines[0]) <= stamp_of(lines[1]), "{written}");

    // And the shelf is still there rather than made again. "I made it" and "it
    // was already there" are different answers.
    assert!(
        stdout(&output).contains("`shelf` is already there"),
        "{}",
        stdout(&output)
    );

    // Nothing landed beside the directory it was given.
    let beside = scratch.path().parent().unwrap().join("escape.txt");
    assert!(!beside.exists(), "something was written outside the `Dir`");
}

#[test]
fn arguments_after_a_double_dash_go_to_the_program() {
    // The to-do example, finally able to do the other half of its job. A
    // separator rather than "whatever is left over", because a program's
    // arguments can look exactly like this tool's.
    let scratch = Scratch::new("todo-add");
    scratch.write("todo.txt", "[x] one\n[ ] two\n");
    let dir = scratch.path().to_str().unwrap().to_string();

    let output = run(&["run", TODO, "--dir", &dir, "--", "buy", "milk"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("added"), "{text}");
    assert!(text.contains("1 of 3 done"), "{text}");
    assert!(text.contains("still open: 2. two, 3. buy milk"), "{text}");

    assert_eq!(
        std::fs::read_to_string(scratch.path().join("todo.txt")).unwrap(),
        "[x] one\n[ ] two\n[ ] buy milk\n"
    );
}

#[test]
fn a_program_given_no_arguments_leaves_the_file_alone() {
    // Rewriting a file on every run touches it even when asked to do nothing,
    // and a to-do list whose timestamp moves for no reason is one nobody
    // trusts.
    let scratch = Scratch::new("todo-noop");
    scratch.write("todo.txt", "[x] one\n[ ] two\n");
    let dir = scratch.path().to_str().unwrap().to_string();

    let before = std::fs::read_to_string(scratch.path().join("todo.txt")).unwrap();
    let output = run(&["run", TODO, "--dir", &dir]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(!stdout(&output).contains("added"), "{}", stdout(&output));
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("todo.txt")).unwrap(),
        before
    );
}

#[test]
fn a_task_can_be_marked_done() {
    // The thing the example said for months it could not do. It is one `map`
    // over the tasks with one of them replaced, and what took so long was
    // being able to write that `map` once rather than once per element type.
    let scratch = Scratch::new("todo-done");
    scratch.write("todo.txt", "[x] one\n[ ] two\n[ ] three\n");
    let dir = scratch.path().to_str().unwrap().to_string();

    let output = run(&["run", TODO, "--dir", &dir, "--", "done", "two"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("done: two"), "{text}");
    assert!(text.contains("2 of 3 done"), "{text}");
    assert!(text.contains("still open: 3. three"), "{text}");

    assert_eq!(
        std::fs::read_to_string(scratch.path().join("todo.txt")).unwrap(),
        "[x] one\n[x] two\n[ ] three\n"
    );
}

#[test]
fn a_task_can_be_marked_done_by_position() {
    // Two tasks with the same title, which is the case a title cannot answer
    // because it is being used as a name and it is not one. The report prints
    // positions so that there is something to type.
    let scratch = Scratch::new("todo-done-at");
    scratch.write("todo.txt", "[ ] buy milk\n[ ] buy milk\n");
    let dir = scratch.path().to_str().unwrap().to_string();

    let output = run(&["run", TODO, "--dir", &dir, "--", "done", "2"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("done: 2"), "{text}");
    assert!(text.contains("1 of 2 done"), "{text}");
    assert!(text.contains("still open: 1. buy milk"), "{text}");

    assert_eq!(
        std::fs::read_to_string(scratch.path().join("todo.txt")).unwrap(),
        "[ ] buy milk\n[x] buy milk\n"
    );
}

#[test]
fn a_position_that_is_not_there_says_so_and_writes_nothing() {
    // Nothing bounds the number, so the walk that marks by position cannot
    // fail and comes back with the list it started with. Whether that meant
    // anything is asked separately, which is the whole shape of an index the
    // type system cannot hold in range.
    let scratch = Scratch::new("todo-done-at-missing");
    scratch.write("todo.txt", "[ ] two\n");
    let dir = scratch.path().to_str().unwrap().to_string();

    let before = std::fs::read_to_string(scratch.path().join("todo.txt")).unwrap();
    let output = run(&["run", TODO, "--dir", &dir, "--", "done", "9"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    assert!(
        stdout(&output).contains("no task at 9"),
        "{}",
        stdout(&output)
    );
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("todo.txt")).unwrap(),
        before
    );
}

#[test]
fn clearing_deletes_the_file_rather_than_emptying_it() {
    // There is a large difference between a file holding nothing and a file
    // that is not there, and only one of the two is what `clear` was asked
    // for.
    let scratch = Scratch::new("todo-clear");
    scratch.write("todo.txt", "[x] one\n[ ] two\n");
    let dir = scratch.path().to_str().unwrap().to_string();

    let output = run(&["run", TODO, "--dir", &dir, "--", "clear"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("cleared"), "{text}");
    assert!(text.contains("0 of 0 done"), "{text}");
    assert!(
        !scratch.path().join("todo.txt").exists(),
        "the file is still there"
    );
}

#[test]
fn clear_with_anything_after_it_is_a_task() {
    // A verb that swallowed whatever followed it would be the program deciding
    // it knows better than what was typed.
    let scratch = Scratch::new("todo-clear-task");
    scratch.write("todo.txt", "[ ] two\n");
    let dir = scratch.path().to_str().unwrap().to_string();

    let output = run(&["run", TODO, "--dir", &dir, "--", "clear", "the", "shed"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    assert!(
        stdout(&output).contains("added: clear the shed"),
        "{}",
        stdout(&output)
    );
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("todo.txt")).unwrap(),
        "[ ] two\n[ ] clear the shed\n"
    );
}

#[test]
fn finishing_a_task_that_is_not_there_says_so_and_writes_nothing() {
    // Silently doing nothing is how somebody finds out a week later that they
    // misspelled it.
    let scratch = Scratch::new("todo-done-missing");
    scratch.write("todo.txt", "[ ] two\n");
    let dir = scratch.path().to_str().unwrap().to_string();

    let before = std::fs::read_to_string(scratch.path().join("todo.txt")).unwrap();
    let output = run(&["run", TODO, "--dir", &dir, "--", "done", "nope"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    assert!(
        stdout(&output).contains("no task called: nope"),
        "{}",
        stdout(&output)
    );
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("todo.txt")).unwrap(),
        before
    );
}

#[test]
fn a_file_written_on_windows_reads_the_same_as_one_written_anywhere_else() {
    // The example used to print its own output backwards over itself, because
    // splitting on a newline leaves the carriage return on every piece and
    // there was no way to take one off. The only thing stopping it was the
    // repository forcing LF, which is a program depending on how the
    // repository holding it is configured.
    let scratch = Scratch::new("todo-crlf");
    scratch.write("todo.txt", "[x] one\r\n[ ] two\r\n");
    let dir = scratch.path().to_str().unwrap().to_string();

    let output = run(&["run", TODO, "--dir", &dir]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("1 of 2 done"), "{text}");
    assert!(text.contains("still open: 2. two"), "{text}");
    assert!(!text.contains('\r'), "a carriage return survived:\n{text}");
}

// -- finding the files an import needs ---------------------------------------
#[test]
fn a_program_that_imports_can_be_run_by_naming_its_own_file() {
    // The compiler only looks at the files it was handed, which is a rule
    // worth having. A module's name says where it lives, so the files it was
    // handed can say which other ones to pick up.
    //
    // Without this, `examples/todo.deed` could not be run at all once it
    // started using a library in another file, and the workaround was to name
    // every file the program transitively needs.
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let output = run(&["run", TODO, "--dir", dir]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("done"), "{}", stdout(&output));
}

#[test]
fn a_root_relative_import_is_still_found_from_the_named_file_alone() {
    // The half above stopped covering, because the library `todo.deed` imports
    // ships with the compiler now and is found without a root at all.
    // `examples/greeting.deed` imports two files beside it, so this is the
    // rule working out a root from the path it was given.
    let greeting = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/greeting.deed");
    let output = run(&["check", greeting]);
    assert_eq!(code(&output), 0, "{}{}", stdout(&output), stderr(&output));
}

#[test]
fn a_library_found_this_way_is_context_and_not_the_subject() {
    // `deed test` on a program should not run the tests of a library it
    // happens to import. You asked about one file.
    let scratch = Scratch::new("imports-subject");
    scratch.write(
        "lib.deed",
        "module lib\n\n\
         fn twice(n: Int) -> Int { n + n }\n\n\
         test \"the library has its own tests\" {\n\
         \x20 assert twice(2) == 4\n\
         }\n",
    );
    let app = scratch.write(
        "app.deed",
        "module app\n\n\
         use lib.{twice}\n\n\
         fn four() -> Int { twice(2) }\n\n\
         test \"the program has its own\" {\n\
         \x20 assert four() == 4\n\
         }\n",
    );

    let output = run(&["test", app.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("the program has its own"), "{text}");
    assert!(
        !text.contains("the library has its own tests"),
        "a library's tests are not the ones you asked to run:\n{text}"
    );
    assert!(text.contains("1 passed"), "{text}");
}

#[test]
fn an_error_in_a_library_is_still_an_error_in_the_program() {
    // The other half. A program that cannot compile its own dependency does
    // not compile, so hiding the library's diagnostics would be hiding the
    // reason.
    let scratch = Scratch::new("imports-errors");
    scratch.write(
        "lib.deed",
        "module lib\n\nfn twice(n: Int) -> String { n + n }\n",
    );
    let app = scratch.write(
        "app.deed",
        "module app\n\nuse lib.{twice}\n\nfn f() -> String { twice(2) }\n",
    );

    let output = run(&["check", app.to_str().unwrap()]);
    assert_eq!(code(&output), 1, "{}", stdout(&output));
    assert!(stdout(&output).contains("lib.deed"), "{}", stdout(&output));
}

#[test]
fn a_file_somewhere_its_name_does_not_say_finds_nothing() {
    // The root comes from taking a file's module path off the end of its own
    // file path. A file that is not where its name says cannot say where
    // anything else is, and the resolver has the message for the import that
    // then fails.
    let scratch = Scratch::new("imports-misplaced");
    scratch.write(
        "lib.deed",
        "module lib\n\nfn twice(n: Int) -> Int { n + n }\n",
    );
    let app = scratch.write(
        "elsewhere.deed",
        "module app\n\nuse lib.{twice}\n\nfn f() -> Int { twice(2) }\n",
    );

    let output = run(&["check", app.to_str().unwrap()]);
    assert_eq!(code(&output), 1, "{}", stdout(&output));
    assert!(stdout(&output).contains("DEED3007"), "{}", stdout(&output));
}

#[test]
fn an_import_is_followed_as_far_as_it_goes() {
    let scratch = Scratch::new("imports-deep");
    scratch.write("bottom.deed", "module bottom\n\nfn one() -> Int { 1 }\n");
    scratch.write(
        "middle.deed",
        "module middle\n\nuse bottom.{one}\n\nfn two() -> Int { one() + one() }\n",
    );
    let top = scratch.write(
        "top.deed",
        "module top\n\n\
         use middle.{two}\n\n\
         test \"all three\" {\n\
         \x20 assert two() == 2\n\
         }\n",
    );

    let output = run(&["test", top.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("1 passed"), "{}", stdout(&output));
}

// -- formatting ------------------------------------------------------------

#[test]
fn fmt_rewrites_a_file_in_place() {
    let scratch = Scratch::new("fmt");
    let file = scratch.write("cramped.deed", "module a\nfn   f( n:Int )->Int{n+n}\n");

    let output = run(&["fmt", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "module a\n\nfn f(n: Int) -> Int {\n    n + n\n}\n"
    );
    assert!(
        stdout(&output).contains("cramped.deed"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn fmt_leaves_a_canonical_file_alone() {
    let scratch = Scratch::new("fmt-noop");
    let canonical = "module a\n\nfn f(n: Int) -> Int {\n    n + n\n}\n";
    let file = scratch.write("fine.deed", canonical);

    let output = run(&["fmt", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output), "", "it reported a file it did not change");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), canonical);
}

#[test]
fn fmt_check_reports_without_writing() {
    let scratch = Scratch::new("fmt-check");
    let cramped = "module a\nfn   f( n:Int )->Int{n+n}\n";
    let file = scratch.write("cramped.deed", cramped);

    let output = run(&["fmt", "--check", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    assert!(stdout(&output).contains("cramped.deed"));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        cramped,
        "`--check` wrote to the file"
    );
}

#[test]
fn fmt_refuses_a_file_that_does_not_parse() {
    let scratch = Scratch::new("fmt-broken");
    let broken = "module a\n\nfn f( -> Int {\n";
    let file = scratch.write("broken.deed", broken);

    let output = run(&["fmt", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        broken,
        "it reshaped a file it could not parse"
    );
}

#[test]
fn the_examples_are_in_canonical_form() {
    let output = run(&["fmt", "--check", EXAMPLES]);
    assert_eq!(code(&output), 0, "run `deed fmt`:\n{}", stdout(&output));
}

#[test]
fn fmt_has_no_options_for_the_output() {
    // The point of P4 is that there is nothing to argue about, and a flag that
    // changes the output would be the first thing to argue about.
    let output = run(&["fmt", "--indent", "2", EXAMPLES]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("unknown option"));
}

// -- fixing ----------------------------------------------------------------

#[test]
fn fix_applies_a_certain_fix() {
    let scratch = Scratch::new("fix");
    let file = scratch.write(
        "typo.deed",
        "module a\n\nfn balance() -> Int { 0 }\n\nfn f() -> Int { balanse() }\n",
    );

    let output = run(&["fix", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("1 fix"), "{}", stdout(&output));

    let after = std::fs::read_to_string(&file).unwrap();
    assert!(after.contains("balance()"), "{after}");

    // And the file checks now, which is the only reason to have done it.
    let output = run(&["check", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));
}

#[test]
fn fix_leaves_a_guess_alone() {
    let scratch = Scratch::new("fix-guess");
    let source = "module a\n\nfn f() -> Int {\n    let s = \"a\\qb\"\n    0\n}\n";
    let file = scratch.write("guess.deed", source);

    let output = run(&["fix", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output), "", "it reported a change it did not make");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), source);
}

#[test]
fn fix_check_reports_without_writing() {
    let scratch = Scratch::new("fix-check");
    let source = "module a\n\nfn balance() -> Int { 0 }\n\nfn f() -> Int { balanse() }\n";
    let file = scratch.write("typo.deed", source);

    let output = run(&["fix", "--check", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    assert!(stdout(&output).contains("typo.deed"));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        source,
        "`--check` wrote to the file"
    );
}

#[test]
fn fix_is_quiet_when_there_is_nothing_to_do() {
    let scratch = Scratch::new("fix-noop");
    let source = "module a\n\nfn f() -> Int { 0 }\n";
    let file = scratch.write("fine.deed", source);

    let output = run(&["fix", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output), "");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), source);
}

#[test]
fn the_examples_have_nothing_left_to_fix() {
    let output = run(&["fix", "--check", EXAMPLES]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));
}

// -- the library that ships with the compiler ------------------------------

#[test]
fn a_program_anywhere_can_import_a_shipped_module() {
    // The whole point. This directory has no relationship to the repository
    // and there is nothing beside the binary to find.
    let scratch = Scratch::new("shipped");
    let file = scratch.write(
        "report.deed",
        "module scratch/report\n\n\
         use std/string.{pad_right}\n\n\
         test \"a column\" {\n\
         \x20 assert pad_right(\"ab\", 4) == \"ab  \"\n\
         }\n",
    );

    let output = run(&["test", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}{}", stdout(&output), stderr(&output));
    assert!(stdout(&output).contains("1 passed"), "{}", stdout(&output));
}

#[test]
fn a_program_anywhere_can_import_the_shipped_list_library() {
    // The list library used to live under `examples/`, so this same file had
    // to say `use examples/list` and only worked from inside a checkout of
    // this repository. It says `std/list` now and there is nothing to copy.
    let scratch = Scratch::new("shipped-list");
    let file = scratch.write(
        "report.deed",
        "module scratch/report\n\n\
         use std/list.{map, count_where}\n\n\
         test \"the library is there\" {\n\
         \x20 assert map([1, 2], |n: Int| n + n) == [2, 4]\n\
         \x20 assert count_where([1, 2, 3], |n: Int| n > 1) == 2\n\
         }\n",
    );

    let output = run(&["test", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}{}", stdout(&output), stderr(&output));
    assert!(stdout(&output).contains("1 passed"), "{}", stdout(&output));
}

#[test]
fn a_program_anywhere_can_import_the_shipped_table_library() {
    // The same move as the list library, and the one that carried tests with
    // it. A keyed collection is not in the language, so a program counting by
    // key needs this and used to need its own copy of `examples/table.deed`.
    let scratch = Scratch::new("shipped-table");
    let file = scratch.write(
        "report.deed",
        "module scratch/report\n\n\
         use std/table.{get, or_else, set}\n\n\
         test \"the library is there\" {\n\
         \x20 let counts = set([], \"a\", or_else([], \"a\", 0) + 1)\n\
         \x20 assert get(counts, \"a\") == ok(1)\n\
         \x20 assert or_else(counts, \"b\", 0) == 0\n\
         }\n",
    );

    let output = run(&["test", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}{}", stdout(&output), stderr(&output));
    assert!(stdout(&output).contains("1 passed"), "{}", stdout(&output));
}

#[test]
fn a_shipped_module_is_context_rather_than_subject() {
    // Its tests are not the ones you asked about, the same way an imported
    // file's are not. Otherwise every program that used the library would
    // report the library's tests as its own.
    let scratch = Scratch::new("shipped-context");
    let file = scratch.write(
        "one.deed",
        "module scratch/one\n\n\
         use std/string.{pad_left}\n\n\
         test \"only this one\" {\n\
         \x20 assert pad_left(\"a\", 2) == \" a\"\n\
         }\n",
    );

    let output = run(&["test", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("1 passed"), "{}", stdout(&output));
}

#[test]
fn a_file_of_their_own_wins_over_the_shipped_one() {
    // Shadowing, and it goes this way round because the file they can read is
    // the one they can change. The shipped module is only ever the answer when
    // nothing under their own root is.
    let scratch = Scratch::new("shipped-shadow");
    scratch.write(
        "std/string.deed",
        "module std/string\n\n\
         fn pad_right(text: String, width: Int) -> String {\n\
         \x20 \"theirs\"\n\
         }\n",
    );
    let file = scratch.write(
        "app.deed",
        "module app\n\n\
         use std/string.{pad_right}\n\n\
         test \"theirs\" {\n\
         \x20 assert pad_right(\"ab\", 4) == \"theirs\"\n\
         }\n",
    );

    let output = run(&["test", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}{}", stdout(&output), stderr(&output));
    assert!(stdout(&output).contains("1 passed"), "{}", stdout(&output));
}

#[test]
fn two_files_asking_for_the_same_one_pick_it_up_once() {
    // Their own file wins, and it has to keep winning for the second file that
    // asks. The name went into the wanted list twice, the first ask picked the
    // file up and the second ask found it already picked up, read that as not
    // found, and fell through to the compiler's table, so the module was there
    // twice and `DEED3002` said so.
    let scratch = Scratch::new("shipped-twice");
    scratch.write(
        "std/string.deed",
        "module std/string\n\n\
         fn pad_right(text: String, width: Int) -> String {\n\
         \x20 \"theirs\"\n\
         }\n",
    );
    let one = scratch.write(
        "one.deed",
        "module one\n\nuse std/string.{pad_right}\n\nfn a() -> String { pad_right(\"x\", 2) }\n",
    );
    let two = scratch.write(
        "two.deed",
        "module two\n\nuse std/string.{pad_right}\n\nfn b() -> String { pad_right(\"y\", 2) }\n",
    );

    // Named rather than handed the directory, so their `std/string.deed`
    // arrives through the import rule that had the hole in it.
    let output = run(&["check", one.to_str().unwrap(), two.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}{}", stdout(&output), stderr(&output));
}

#[test]
fn a_module_that_ships_with_nothing_is_still_unknown() {
    // The mechanism adds one place to look and not a fallback for everything.
    let scratch = Scratch::new("shipped-missing");
    let file = scratch.write(
        "app.deed",
        "module app\n\nuse std/nonesuch.{f}\n\nfn g() -> Int { 0 }\n",
    );

    let output = run(&["check", file.to_str().unwrap()]);
    assert_ne!(code(&output), 0);
    assert!(
        stdout(&output).contains("no module `std/nonesuch`"),
        "{}",
        stdout(&output)
    );
}

// -- every sentence, read --------------------------------------------------
//
// A sentence that nothing holds can be reworded into nonsense, or deleted, and
// the build stays green. One test per distinct sentence, asserting the words a
// person reads rather than only that something failed. The stream matters too:
// errors belong on stderr and results belong on stdout.
//
// Two sentences printed by `main.rs` cannot be reached from the command line
// today and so have no test below:
//
// - "<name>: still changing after several rounds, run `deed fix` again" (stdout)
//   requires a fix that oscillates. The driver comment says "nothing in the
//   compiler does this today", so no test input can trigger it.
//
// - "error: <name>: fixing made it worse, so nothing was written. This is a
//   compiler bug, please report it." (stderr) requires a fix that increases the
//   error count. No such fix exists in the compiler today.

/// "error: no `.deed` files found"
#[test]
fn no_deed_files_says_the_full_sentence() {
    let scratch = Scratch::new("sentence-no-deed");
    scratch.write("readme.txt", "nothing here");

    let output = run(&["check", scratch.path().to_str().unwrap()]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("error: no `.deed` files found"),
        "{}",
        stderr(&output)
    );
    assert_eq!(stdout(&output), "");
}

/// "error: no `main` found, so there is nothing to run"
///
/// "so there is nothing to run" is the part most likely to be trimmed away
/// on a refactor: it is the reason, not just the diagnosis.
#[test]
fn no_main_says_the_full_sentence() {
    let scratch = Scratch::new("sentence-no-main");
    let file = scratch.write("quiet.deed", "module a\n\nfn f() -> Int { 0 }\n");

    let output = run(&["run", file.to_str().unwrap()]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("no `main` found, so there is nothing to run"),
        "{}",
        stderr(&output)
    );
    assert_eq!(stdout(&output), "");
}

/// "error: more than one `main`, in <a> and <b>"
///
/// Both filenames are the point. An assertion that stops at "more than one"
/// would let the names disappear without anyone noticing.
#[test]
fn two_mains_names_both_files_in_the_sentence() {
    let scratch = Scratch::new("sentence-two-mains");
    scratch.write(
        "alpha.deed",
        "module alpha\n\nfn main(sys: System) -> Int { 0 }\n",
    );
    scratch.write(
        "beta.deed",
        "module beta\n\nfn main(sys: System) -> Int { 0 }\n",
    );

    let output = run(&["run", scratch.path().to_str().unwrap()]);
    assert_eq!(code(&output), 2);
    let text = stderr(&output);
    assert!(text.contains("more than one `main`, in"), "{text}");
    assert!(text.contains("alpha.deed"), "{text}");
    assert!(text.contains("beta.deed"), "{text}");
    assert_eq!(stdout(&output), "");
}

/// "error: <path>: <io error>"
///
/// The path is in the sentence so the reader knows which file caused the
/// problem. A test that only checks the exit code or "error:" would let
/// the path disappear.
#[test]
fn a_path_io_error_names_the_path() {
    let output = run(&["check", "not-a-real-path.deed"]);
    assert_eq!(code(&output), 2);
    let text = stderr(&output);
    assert!(text.starts_with("error: "), "{text}");
    assert!(text.contains("not-a-real-path.deed"), "{text}");
    assert_eq!(stdout(&output), "");
}

/// "deed <version>" goes to stdout, not stderr.
///
/// Version is a result, not an error, so it belongs on stdout where a
/// script can capture it.
#[test]
fn version_is_on_stdout_not_stderr() {
    let output = run(&["--version"]);
    assert_eq!(code(&output), 0);
    assert!(stdout(&output).starts_with("deed "), "{}", stdout(&output));
    assert_eq!(stderr(&output), "");
}

/// "no tests found" goes to stdout, not stderr.
///
/// Test results are on stdout so they can be piped. Mixing them with stderr
/// would break any pipeline that captures only one stream.
#[test]
fn no_tests_found_is_on_stdout() {
    let scratch = Scratch::new("sentence-no-tests");
    let file = scratch.write("quiet.deed", "module a\n\nfn f() -> Int { 0 }\n");

    let output = run(&["test", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0);
    assert!(
        stdout(&output).contains("no tests found"),
        "{}",
        stdout(&output)
    );
    assert_eq!(stderr(&output), "");
}

/// `deed fmt` on a file that does not parse writes the diagnostic to stdout.
///
/// The stream is stdout because `fmt` output goes there (the list of
/// rewritten files, and the diagnostics for files it could not rewrite).
/// Stderr is empty so a caller piping stdout sees the full picture.
#[test]
fn fmt_diagnostic_for_broken_file_is_on_stdout() {
    let scratch = Scratch::new("sentence-fmt-broken");
    let file = scratch.write("broken.deed", "module a\n\nfn f( -> Int {\n");

    let output = run(&["fmt", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    assert!(stdout(&output).contains("DEED"), "{}", stdout(&output));
    assert_eq!(stderr(&output), "");
}

/// "<path>: N fix[es]" goes to stdout, not stderr.
///
/// Fix results are on stdout so a script can distinguish them from errors.
/// "1 fix" rather than "1 fixes" checks the singular-plural branch too.
#[test]
fn fix_count_is_on_stdout() {
    let scratch = Scratch::new("sentence-fix-count");
    let file = scratch.write(
        "typo.deed",
        "module a\n\nfn balance() -> Int { 0 }\n\nfn f() -> Int { balanse() }\n",
    );

    let output = run(&["fix", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(stdout(&output).contains("1 fix"), "{}", stdout(&output));
    assert_eq!(stderr(&output), "");
}

/// `deed build` writes a WebAssembly module beside the file it was given.
///
/// The bytes are checked rather than only the exit code: a compiler whose
/// output nobody looks at is a compiler that can start writing anything.
#[test]
fn build_writes_a_module_next_to_the_source() {
    let scratch = Scratch::new("build");
    let source = scratch.write(
        "small.deed",
        "module small\n\nfn double(n: Int) -> Int { n + n }\n\nfn answer() -> Int { double(21) }\n",
    );

    let output = run(&["build", source.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let target = scratch.path().join("small.wasm");
    assert!(target.is_file(), "{}", stdout(&output));

    let bytes = std::fs::read(&target).unwrap();
    assert_eq!(&bytes[..4], b"\0asm", "the magic number");
    assert_eq!(&bytes[4..8], &1u32.to_le_bytes(), "the version");
}

/// What the backend cannot compile it names, and answers with a failure
/// rather than writing a module that would be wrong.
#[test]
fn build_says_what_it_could_not_compile() {
    let scratch = Scratch::new("build-refused");
    let source = scratch.write("hard.deed", REFUSED);

    let output = run(&["build", source.to_str().unwrap()]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert!(stdout(&output).contains("in memory"), "{}", stdout(&output));
    assert!(!scratch.path().join("hard.wasm").exists());
}

/// One file that compiles and one that does not, handed over together.
///
/// The half this pins is that a refusal does not take the build down with
/// it: what could be compiled is written, what could not is named, and the
/// answer is still a failure because something was asked for and not
/// delivered.
#[test]
fn build_writes_what_it_can_and_names_what_it_cannot() {
    let scratch = Scratch::new("build-mixed");
    scratch.write("fine.deed", "module fine\n\nfn answer() -> Int { 1 }\n");
    scratch.write("hard.deed", REFUSED);

    let output = run(&["build", scratch.path().to_str().unwrap()]);
    assert!(
        scratch.path().join("fine.wasm").is_file(),
        "{}",
        stdout(&output)
    );
    assert!(!scratch.path().join("hard.wasm").exists());
    assert!(stdout(&output).contains("in memory"), "{}", stdout(&output));
}

/// A program the backend does not compile, for the two tests above.
///
/// Comparing two records is structural in this language, and two addresses
/// being equal is not two records being equal, so the backend refuses rather
/// than answering the wrong question.
const REFUSED: &str = "module hard\n\nrecord Point {\n    x: Int,\n}\n\nfn same(one: Point, other: Point) -> Bool { one == other }\n";

/// A file that calls into another one builds, and the other one is not
/// written next to somebody else's source.
///
/// The backend is handed the files that came in because an import wanted
/// them, alongside the file somebody named. Handing it the named file twice
/// would compile the same way for anything with no imports in it, which is
/// most of what is tested elsewhere, so this asks for the case that tells
/// them apart.
#[test]
fn build_compiles_a_file_that_calls_into_another() {
    let scratch = Scratch::new("build-import");
    scratch.write(
        "tools.deed",
        "module tools\n\nfn twice(n: Int) -> Int { n + n }\n",
    );
    let source = scratch.write(
        "caller.deed",
        "module caller\n\nuse tools.{twice}\n\nfn answer() -> Int { twice(3) }\n",
    );

    let output = run(&["build", source.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}", stdout(&output));
    assert!(
        scratch.path().join("caller.wasm").is_file(),
        "{}",
        stdout(&output)
    );
    assert!(
        !scratch.path().join("tools.wasm").exists(),
        "a module that came in for an import is context, not something to write"
    );
}

// -- building a component --------------------------------------------------

/// `deed build --component` writes both a `.wasm` module and a `.wit` world.
///
/// The WIT file is the component's interface declaration. The bytes of the
/// module are checked the same way `build_writes_a_module_next_to_the_source`
/// checks them: a compiler whose output nobody reads can start writing anything.
#[test]
fn component_build_writes_wasm_and_wit() {
    let scratch = Scratch::new("component");
    let source = scratch.write(
        "math.deed",
        "module math\n\nfn double(n: Int) -> Int { n + n }\n\nfn negate(b: Bool) -> Bool { !b }\n",
    );

    let output = run(&["build", "--component", source.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    // Both files are named on stdout.
    let text = stdout(&output);
    assert!(text.contains("math.wasm"), "{text}");
    assert!(text.contains("math.wit"), "{text}");

    // The module is a real WebAssembly binary.
    let wasm = std::fs::read(scratch.path().join("math.wasm")).unwrap();
    assert_eq!(&wasm[..4], b"\0asm", "the magic number");
    assert_eq!(&wasm[4..8], &1u32.to_le_bytes(), "the version");

    // The WIT file declares the component package and world.
    let wit = std::fs::read_to_string(scratch.path().join("math.wit")).unwrap();
    assert!(wit.contains("package deed:math"), "{wit}");
    assert!(wit.contains("world component"), "{wit}");
    assert!(wit.contains("export double"), "{wit}");
    assert!(wit.contains("export negate"), "{wit}");
    assert!(wit.contains("s64"), "{wit}");
    assert!(wit.contains("bool"), "{wit}");
}

/// A module that declares `main` is a program, not a component.
///
/// `deed build --component` refuses it with a message that names both the
/// problem (`main`) and the alternative (`deed build`).
#[test]
fn component_build_refuses_a_program_with_main() {
    let scratch = Scratch::new("component-main");
    let source = scratch.write(
        "prog.deed",
        "module prog\n\nfn main(sys: System) -> Int { 0 }\n",
    );

    let output = run(&["build", "--component", source.to_str().unwrap()]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("`main`"), "{text}");
    assert!(text.contains("deed build"), "{text}");

    // Nothing is written.
    assert!(!scratch.path().join("prog.wasm").exists());
    assert!(!scratch.path().join("prog.wit").exists());
}

/// A function whose signature contains a capability has no world-level type
/// in WIT. `deed build --component` refuses the module and names the function.
#[test]
fn component_build_refuses_capability_in_signature() {
    let scratch = Scratch::new("component-cap");
    let source = scratch.write(
        "sneaky.deed",
        "module sneaky\n\nfn write_it(c: Console, msg: String) -> ()\n  uses Io.write,\n{ Io.write(c, msg) }\n",
    );

    let output = run(&["build", "--component", source.to_str().unwrap()]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));

    let text = stdout(&output);
    assert!(text.contains("write_it"), "{text}");
    assert!(text.contains("capability"), "{text}");
    assert!(text.contains("no world-level type"), "{text}");

    // Nothing is written when the module is refused.
    assert!(!scratch.path().join("sneaky.wasm").exists());
    assert!(!scratch.path().join("sneaky.wit").exists());
}

/// `deed build --component` tests are not part of the interface.
///
/// Tests live only in the source; they cannot be called from outside the
/// module, and so they are not exported in the WIT world.
#[test]
fn component_build_does_not_export_tests() {
    let scratch = Scratch::new("component-tests");
    let source = scratch.write(
        "lib.deed",
        "module lib\n\nfn double(n: Int) -> Int { n + n }\n\ntest \"two plus two\" {\n    assert double(2) == 4\n}\n",
    );

    let output = run(&["build", "--component", source.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let wit = std::fs::read_to_string(scratch.path().join("lib.wit")).unwrap();
    assert!(wit.contains("export double"), "{wit}");
    // Test blocks do not appear in the compiled output at all.
    assert!(!wit.contains("two plus two"), "{wit}");
}

// -- lock files: provenance, offline verification, and reproducibility -----

/// `deed build --lock <path>` writes a lock file listing every input.
///
/// The lock file must start with the deed header, and every local source
/// file must appear with a `sha256:` line. The format pins the content so
/// that a later `--locked` run can verify nothing changed.
#[test]
fn build_writes_a_lock_file_when_asked() {
    let scratch = Scratch::new("lock-write");
    let source = scratch.write(
        "small.deed",
        "module small\n\nfn double(n: Int) -> Int { n + n }\n",
    );
    let lock_path = scratch.path().join("small.lock");

    let output = run(&[
        "build",
        source.to_str().unwrap(),
        "--lock",
        lock_path.to_str().unwrap(),
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let lock_text = std::fs::read_to_string(&lock_path).expect("lock file should be written");
    assert!(lock_text.starts_with("deed lock v1\n"), "{lock_text}");
    // The source file appears with its hash.
    assert!(lock_text.contains("sha256:"), "{lock_text}");
    assert!(lock_text.contains("small.deed"), "{lock_text}");
}

/// `deed check --locked <path>` succeeds when inputs match the lock.
///
/// Write a lock for a file, then verify it immediately. The file has not
/// changed, so verification passes.
#[test]
fn locked_build_passes_when_inputs_match() {
    let scratch = Scratch::new("lock-pass");
    let source = scratch.write("match.deed", "module match_\n\nfn f() -> Int { 0 }\n");
    let lock_path = scratch.path().join("match.lock");

    // Write the lock.
    let write_output = run(&[
        "check",
        source.to_str().unwrap(),
        "--lock",
        lock_path.to_str().unwrap(),
    ]);
    assert_eq!(code(&write_output), 0, "{}", stderr(&write_output));

    // Verify the lock.
    let verify_output = run(&[
        "check",
        source.to_str().unwrap(),
        "--locked",
        lock_path.to_str().unwrap(),
    ]);
    assert_eq!(code(&verify_output), 0, "{}", stderr(&verify_output));
}

/// `deed check --locked <path>` refuses when a source file has been modified.
///
/// Write a lock, then tamper with the source file. The hash no longer
/// matches, so `--locked` must reject the build before compiling anything.
#[test]
fn locked_build_refuses_tampered_input() {
    let scratch = Scratch::new("lock-tamper");
    let source = scratch.write("tamper.deed", "module tamper\n\nfn f() -> Int { 0 }\n");
    let lock_path = scratch.path().join("tamper.lock");

    // Write the lock.
    let write_output = run(&[
        "check",
        source.to_str().unwrap(),
        "--lock",
        lock_path.to_str().unwrap(),
    ]);
    assert_eq!(code(&write_output), 0, "{}", stderr(&write_output));

    // Tamper: change the source file after locking.
    std::fs::write(&source, "module tamper\n\nfn f() -> Int { 999 }\n").unwrap();

    // --locked should now reject the build.
    let verify_output = run(&[
        "check",
        source.to_str().unwrap(),
        "--locked",
        lock_path.to_str().unwrap(),
    ]);
    assert_eq!(code(&verify_output), 1, "tampered build should fail");
    let text = stderr(&verify_output);
    assert!(
        text.contains("changed since the lock"),
        "should explain what went wrong:\n{text}"
    );
}

/// `deed check --locked <path>` refuses when the lock file names a file that
/// no longer exists.
///
/// This covers the "missing cached input" requirement: a vendored build that
/// is missing a file should be refused, not silently broken.
#[test]
fn locked_build_refuses_missing_input() {
    let scratch = Scratch::new("lock-missing");
    let source = scratch.write("gone.deed", "module gone\n\nfn f() -> Int { 0 }\n");
    let lock_path = scratch.path().join("gone.lock");

    // Write the lock.
    let write_output = run(&[
        "check",
        source.to_str().unwrap(),
        "--lock",
        lock_path.to_str().unwrap(),
    ]);
    assert_eq!(code(&write_output), 0, "{}", stderr(&write_output));

    // Delete the source file.
    std::fs::remove_file(&source).unwrap();

    // --locked should now refuse because the file is gone.
    let verify_output = run(&[
        "check",
        source.to_str().unwrap(),
        "--locked",
        lock_path.to_str().unwrap(),
    ]);
    // Either exit 1 (tamper) or exit 2 (usage/io error): both are non-zero.
    assert_ne!(code(&verify_output), 0, "missing-file build should fail");
}

/// Building the same source twice produces byte-identical WebAssembly output.
///
/// Same inputs must produce the same bytes. This is the strongest form of
/// reproducibility: not just the same behavior but the same artifact.
#[test]
fn same_inputs_produce_same_bytes() {
    let scratch = Scratch::new("repro");
    let source = scratch.write(
        "repro.deed",
        "module repro\n\nfn double(n: Int) -> Int { n + n }\n\nfn answer() -> Int { double(21) }\n",
    );

    let first = scratch.path().join("first.wasm");
    let second = scratch.path().join("second.wasm");

    // Build twice to separate output files so we can compare.
    let out1 = run(&["build", source.to_str().unwrap()]);
    assert_eq!(code(&out1), 0, "{}", stderr(&out1));
    let built = scratch.path().join("repro.wasm");
    std::fs::copy(&built, &first).unwrap();
    std::fs::remove_file(&built).unwrap();

    let out2 = run(&["build", source.to_str().unwrap()]);
    assert_eq!(code(&out2), 0, "{}", stderr(&out2));
    std::fs::copy(&built, &second).unwrap();

    let bytes1 = std::fs::read(&first).unwrap();
    let bytes2 = std::fs::read(&second).unwrap();
    assert_eq!(
        bytes1, bytes2,
        "two builds of the same source must be byte-identical"
    );
}

/// A multi-module build resolves all imports from the local file system with
/// no network access required. Both files compile together and all imports
/// resolve.
///
/// This is the "offline build" requirement: the build must complete from a
/// local checkout of the source tree with no network at all.
#[test]
fn offline_multi_module_build_needs_no_network() {
    let scratch = Scratch::new("offline");
    // A library module.
    scratch.write(
        "offline/lib.deed",
        "module offline/lib\n\nfn add(a: Int, b: Int) -> Int { a + b }\n",
    );
    // A main module that imports the library.
    let main = scratch.write(
        "offline/main.deed",
        "module offline/main\n\nuse offline/lib.{add}\n\nfn answer() -> Int { add(40, 2) }\n",
    );

    // The build must succeed without any network calls.  If it did reach the
    // network it would fail (no registry is running), so a passing test here
    // demonstrates the offline property.
    let output = run(&["check", main.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(stdout(&output), "", "a clean check should say nothing");
}

/// A lock file written by `deed check` enumerates all inputs including
/// transitive imports. The multi-module build produces entries for every
/// file that went into it.
#[test]
fn lock_file_enumerates_all_transitive_inputs() {
    let scratch = Scratch::new("lock-transitive");
    scratch.write(
        "tree/lib.deed",
        "module tree/lib\n\nfn one() -> Int { 1 }\n",
    );
    let main = scratch.write(
        "tree/main.deed",
        "module tree/main\n\nuse tree/lib.{one}\n\nfn two() -> Int { one() + one() }\n",
    );
    let lock_path = scratch.path().join("tree.lock");

    let output = run(&[
        "check",
        main.to_str().unwrap(),
        "--lock",
        lock_path.to_str().unwrap(),
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let lock_text = std::fs::read_to_string(&lock_path).expect("lock file should be written");
    // Both the named file and its import must appear.
    assert!(lock_text.contains("main.deed"), "{lock_text}");
    assert!(lock_text.contains("lib.deed"), "{lock_text}");
}
