//! The `vow` binary, exercised by actually running it.
//!
//! Testing the library behind a command line tool and calling it done is how
//! tools ship with broken argument handling. These spawn the binary.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const VOW: &str = env!("CARGO_BIN_EXE_vow");
const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.vow");
const RUNNABLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/counter.vow");
const HELLO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/hello.vow");
const CONFIG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/config.vow");
const TODO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/todo.vow");
const EXAMPLES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");

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
    scratch.write(
        "other.vow",
        "module other\n\nrecord Used { n: Int }\n\nrecord Spare { n: Int }\n",
    );
    scratch.write(
        "warn.vow",
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
        "lonely.vow",
        "module a\n\nuse other.{Thing}\n\nfn f() -> Thing {}\n",
    );

    let output = run(&["check", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    assert!(stdout(&output).contains("VOW3007"), "{}", stdout(&output));
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

// -- the language server ---------------------------------------------------

#[test]
fn lsp_speaks_the_protocol_on_stdin_and_stdout() {
    // The library has the session tests. This one is about the wiring: that
    // the subcommand exists, that it reads stdin, and that nothing else the
    // binary prints ends up on stdout, which is the protocol.
    let body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}";
    let input = format!("Content-Length: {}\r\n\r\n{body}", body.len());

    let mut child = Command::new(VOW)
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the vow binary should run");

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

// -- running tests ---------------------------------------------------------

#[test]
fn every_example_passes_its_own_tests() {
    // The whole directory, not one file. Adding an example with a property
    // that fails used to be invisible here, and it happened: `hello.vow` had a
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
        "broken.vow",
        "module a\n\nfn main(sys: System) -> Int\n  uses Io.write,\n{\n  Io.write(sys.console, nope())\n  0\n}\n",
    );

    let output = run(&["run", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    assert!(stdout(&output).contains("VOW3001"));
}

#[test]
fn a_failed_contract_in_main_exits_with_one() {
    let scratch = Scratch::new("main-contract");
    let file = scratch.write(
        "bad.vow",
        "module a\n\nfn main(sys: System) -> Int\n  ensures\n    ok => result > 100,\n{\n  1\n}\n",
    );

    let output = run(&["run", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    assert!(stdout(&output).contains("VOW6003"), "{}", stdout(&output));
}

#[test]
fn running_something_with_no_main_is_a_usage_error() {
    let scratch = Scratch::new("no-main");
    let file = scratch.write("quiet.vow", "module a\n\nfn f() -> Int { 0 }\n");

    let output = run(&["run", file.to_str().unwrap()]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("no `main`"));
}

#[test]
fn two_mains_is_a_question_not_a_choice() {
    let scratch = Scratch::new("two-mains");
    scratch.write(
        "one.vow",
        "module one\n\nfn main(sys: System) -> Int { 0 }\n",
    );
    scratch.write(
        "two.vow",
        "module two\n\nfn main(sys: System) -> Int { 0 }\n",
    );

    let output = run(&["run", scratch.path().to_str().unwrap()]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("more than one `main`"));
}

#[test]
fn a_program_cannot_write_without_being_handed_a_console() {
    let scratch = Scratch::new("no-console");
    let file = scratch.write(
        "sneaky.vow",
        "module a\n\nfn sneaky() -> ()\n  uses Io.write,\n{\n  Io.write(Console, \"hello\")\n}\n\nfn main(sys: System) -> Int { 0 }\n",
    );

    let output = run(&["run", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    let text = stdout(&output);
    assert!(text.contains("VOW4019"), "{text}");
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
        "reader.vow",
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
    assert!(text.contains("2 of 4 done"), "{text}");
    assert!(
        text.contains("still open: decide what a loop looks like, write a language server"),
        "{text}"
    );
    // A `\r` left on the end of a title would print the rest of the line over
    // the top of itself, which is how the line endings in the data file were
    // found in the first place.
    assert!(!text.contains('\r'), "a carriage return survived:\n{text}");
}

// -- formatting ------------------------------------------------------------

#[test]
fn fmt_rewrites_a_file_in_place() {
    let scratch = Scratch::new("fmt");
    let file = scratch.write("cramped.vow", "module a\nfn   f( n:Int )->Int{n+n}\n");

    let output = run(&["fmt", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "module a\n\nfn f(n: Int) -> Int {\n    n + n\n}\n"
    );
    assert!(
        stdout(&output).contains("cramped.vow"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn fmt_leaves_a_canonical_file_alone() {
    let scratch = Scratch::new("fmt-noop");
    let canonical = "module a\n\nfn f(n: Int) -> Int {\n    n + n\n}\n";
    let file = scratch.write("fine.vow", canonical);

    let output = run(&["fmt", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output), "", "it reported a file it did not change");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), canonical);
}

#[test]
fn fmt_check_reports_without_writing() {
    let scratch = Scratch::new("fmt-check");
    let cramped = "module a\nfn   f( n:Int )->Int{n+n}\n";
    let file = scratch.write("cramped.vow", cramped);

    let output = run(&["fmt", "--check", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    assert!(stdout(&output).contains("cramped.vow"));
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
    let file = scratch.write("broken.vow", broken);

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
    assert_eq!(code(&output), 0, "run `vow fmt`:\n{}", stdout(&output));
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
        "typo.vow",
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
    let file = scratch.write("guess.vow", source);

    let output = run(&["fix", file.to_str().unwrap()]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output), "", "it reported a change it did not make");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), source);
}

#[test]
fn fix_check_reports_without_writing() {
    let scratch = Scratch::new("fix-check");
    let source = "module a\n\nfn balance() -> Int { 0 }\n\nfn f() -> Int { balanse() }\n";
    let file = scratch.write("typo.vow", source);

    let output = run(&["fix", "--check", file.to_str().unwrap()]);
    assert_eq!(code(&output), 1);
    assert!(stdout(&output).contains("typo.vow"));
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
    let file = scratch.write("fine.vow", source);

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
