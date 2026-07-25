//! Capabilities.
//!
//! Most of these are about what a program cannot do, because that is the whole
//! claim. A capability system that only demonstrates the things that work has
//! demonstrated nothing.

use std::path::{Path, PathBuf};

use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_driver::{Checked, check_text};
use vow_interp::{Program, Run, run_main};

/// A directory nothing was granted, for programs that do not touch files.
fn nowhere() -> &'static Path {
    Path::new("")
}

/// The one checked module, as something the interpreter can run.
fn program_of(checked: &Checked) -> Program<'_> {
    let mut program = Program::new();
    program.add(checked.file, &checked.module, &checked.resolutions);
    program
}

fn check(src: &str) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.vow", src);
    (sources, checked)
}

fn check_ok(src: &str) -> (SourceMap, Checked) {
    let (sources, checked) = check(src);
    assert!(
        checked.diagnostics.is_empty(),
        "expected a clean check:\n{}",
        rendered(&sources, &checked.diagnostics)
    );
    (sources, checked)
}

fn run(src: &str) -> (SourceMap, Run) {
    run_in(src, nowhere())
}

fn run_in(src: &str, root: &Path) -> (SourceMap, Run) {
    let (sources, checked) = check_ok(src);
    let run = run_main(&program_of(&checked), checked.file, root).expect("there should be a main");
    (sources, run)
}

fn codes_of(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.code).collect()
}

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

// -- what a program cannot do ----------------------------------------------

#[test]
fn a_function_with_no_console_has_no_way_to_name_one() {
    // Not forbidden by a rule. There is simply nothing to pass.
    let (sources, checked) = check(
        "module a\n\nfn sneaky() -> ()\n  uses Io.write,\n{\n  Io.write(console, \"hello\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_resolve::codes::UNKNOWN_NAME),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_capability_cannot_be_built_out_of_nothing() {
    let (_, checked) = check(
        "module a\n\nfn sneaky() -> ()\n  uses Io.write,\n{\n  Io.write(Console, \"hello\")\n}\n",
    );
    assert!(checked.has_errors());
}

#[test]
fn holding_the_wrong_capability_is_a_type_error() {
    let (sources, checked) = check(
        "module a\n\nfn sneaky(time: Clock) -> ()\n  uses Io.write,\n{\n  Io.write(time, \"hello\")\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::TYPE_MISMATCH),
        "{text}"
    );
    assert!(text.contains("expected `Console`, found `Clock`"), "{text}");
}

#[test]
fn writing_without_declaring_it_is_an_effect_error() {
    // Holding the capability is not enough. The row has to admit to it too.
    let (sources, checked) =
        check("module a\n\nfn quiet(out: Console) -> () {\n  Io.write(out, \"hello\")\n}\n");
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn system_carries_only_what_it_carries() {
    let (sources, checked) = check(
        "module a\n\nfn main(sys: System) -> Int\n  uses Io.write,\n{\n  Io.write(sys.network, \"hello\")\n  0\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::NO_SUCH_FIELD),
        "{text}"
    );
    assert!(text.contains("console"), "{text}");
    assert!(text.contains("files"), "{text}");
}

#[test]
fn declaring_a_capability_type_of_your_own_is_noticed() {
    // Otherwise a program could define its own `Console` and conjure one, and
    // none of the rest of this would mean anything.
    let (sources, checked) = check(
        "module a\n\nrecord Console { pretend: Int }\n\nfn f(c: Console) -> Int { c.pretend }\n",
    );
    assert!(
        !checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- what it can do --------------------------------------------------------

#[test]
fn a_function_handed_a_console_can_write_to_it() {
    check_ok(
        "module a\n\nfn greet(out: Console) -> ()\n  uses Io.write,\n{\n  Io.write(out, \"hello\")\n}\n",
    );
}

#[test]
fn main_receives_the_one_system_there_is() {
    let (_, run) = run(
        "module a\n\nfn main(sys: System) -> Int\n  uses Io.write,\n{\n  Io.write(sys.console, \"hello\")\n  0\n}\n",
    );
    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["hello".to_string()]);
}

#[test]
fn delegation_only_ever_narrows() {
    let (_, run) = run(
        "module a\n\nfn greet(out: Console) -> ()\n  uses Io.write,\n{\n  Io.write(out, \"narrowed\")\n}\n\nfn main(sys: System) -> Int\n  uses Io.write,\n{\n  greet(sys.console)\n  0\n}\n",
    );
    assert_eq!(run.output, vec!["narrowed".to_string()]);
}

#[test]
fn the_clock_is_deterministic() {
    // Wall clock time would make every run different, and P8 says the default
    // is deterministic.
    let source = "module a\n\nfn main(sys: System) -> Int\n  uses Io.now,\n{\n  Io.now(sys.clock) + Io.now(sys.clock)\n}\n";
    let (_, first) = run(source);
    let (_, second) = run(source);
    let one = first.result.expect("should run").to_string();
    let two = second.result.expect("should run").to_string();
    assert_eq!(one, two);
}

#[test]
fn contracts_still_apply_to_main() {
    let (sources, run) = run(
        "module a\n\nfn main(sys: System) -> Int\n  ensures\n    ok => result > 100,\n{\n  1\n}\n",
    );
    let failure = run
        .result
        .expect_err("the postcondition should have caught this");
    assert_eq!(failure.code, vow_interp::codes::POSTCONDITION_FAILED);
    let _ = render_human(&sources, &failure);
}

// -- directories -----------------------------------------------------------

/// A `main` whose whole job is to report what one operation did.
///
/// The row is spelled out per case rather than shared, because the effects
/// pass rejects declaring an operation the body never performs, which is the
/// rule doing its job.
fn report(uses: &[&str], body: &str) -> String {
    let row: String = uses
        .iter()
        .map(|name| format!("    Io.{name},\n"))
        .collect();
    format!("module a\n\nfn main(sys: System) -> Int\n  uses\n{row}{{\n{body}\n  0\n}}\n")
}

#[test]
fn a_dir_reads_a_file_inside_it() {
    let scratch = Scratch::new("read");
    scratch.write("note.txt", "the contents");

    let (_, run) = run_in(
        &report(
            &["write", "read"],
            "  match Io.read(sys.files, \"note.txt\") {\n    ok(text) => Io.write(sys.console, text),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );
    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["the contents".to_string()]);
}

#[test]
fn every_way_out_of_a_dir_is_refused() {
    let scratch = Scratch::new("escape");
    scratch.write("inside.txt", "fine");

    // Each of these is a real thing someone would try, and none of them are
    // rejected by a list of known bad strings. Written the way a Vow program
    // would write them, so a backslash is escaped.
    for attempt in [
        "..",
        ".",
        "../Cargo.toml",
        "/etc/passwd",
        "C:\\\\Windows\\\\System32",
        "a/b",
        "a\\\\b",
        "",
    ] {
        let (_, run) = run_in(
            &report(
                &["write", "read"],
                &format!(
                    "  match Io.read(sys.files, \"{attempt}\") {{\n    ok(text) => Io.write(sys.console, \"READ IT\"),\n    err(why) => Io.write(sys.console, why),\n  }}"
                ),
            ),
            scratch.path(),
        );
        assert!(run.result.is_ok());
        assert_ne!(
            run.output,
            vec!["READ IT".to_string()],
            "`{attempt}` got out of the directory"
        );
    }
}

#[test]
fn opening_narrows_and_there_is_no_way_back() {
    let scratch = Scratch::new("narrow");
    scratch.write("outer.txt", "the wider directory");
    std::fs::create_dir_all(scratch.path().join("inner")).unwrap();
    scratch.write("inner/inner.txt", "the narrower one");

    // The `Dir` handed to the inner branch reaches `inner` and nothing above
    // it, and there is no operation that would widen it again.
    let (_, run) = run_in(
        &report(
            &["write", "read", "open"],
            "  match Io.open(sys.files, \"inner\") {\n    ok(narrower) => match Io.read(narrower, \"inner.txt\") {\n      ok(text) => Io.write(sys.console, text),\n      err(why) => Io.write(sys.console, why),\n    },\n    err(why) => Io.write(sys.console, why),\n  }\n  match Io.open(sys.files, \"inner\") {\n    ok(narrower) => match Io.read(narrower, \"outer.txt\") {\n      ok(text) => Io.write(sys.console, \"REACHED THE PARENT\"),\n      err(why) => Io.write(sys.console, why),\n    },\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output[0], "the narrower one");
    assert!(
        !run.output[1].contains("REACHED THE PARENT"),
        "a narrowed `Dir` read its parent: {:?}",
        run.output
    );
}

#[test]
fn a_program_given_no_directory_has_none() {
    // `sys.console` still works. Only the part nothing granted is missing, and
    // the runtime does not quietly substitute the working directory.
    let (_, run) = run(&report(
        &["write", "read"],
        "  match Io.read(sys.files, \"anything\") {\n    ok(text) => Io.write(sys.console, text),\n    err(why) => Io.write(sys.console, why),\n  }",
    ));
    let failure = run.result.expect_err("there is no directory to reach");
    assert_eq!(failure.code, vow_interp::codes::NOT_RUNNABLE);
}

#[test]
fn a_function_holding_only_a_dir_cannot_write() {
    let (sources, checked) = check(
        "module a\n\nfn peek(files: Dir) -> ()\n  uses Io.write,\n{\n  Io.write(files, \"hello\")\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::TYPE_MISMATCH),
        "{text}"
    );
    assert!(text.contains("expected `Console`, found `Dir`"), "{text}");
}

// -- the examples ----------------------------------------------------------

#[test]
fn the_hello_example_runs() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/hello.vow");
    let source = std::fs::read_to_string(path).expect("examples/hello.vow should exist");

    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "examples/hello.vow", source);
    assert!(
        checked.diagnostics.is_empty(),
        "the example should check cleanly:\n{}",
        rendered(&sources, &checked.diagnostics)
    );

    let run =
        run_main(&program_of(&checked), checked.file, nowhere()).expect("the example has a main");
    assert!(run.result.is_ok());
    assert!(run.output.iter().any(|line| line.contains("world")));
}

#[test]
fn the_config_example_reads_itself_and_nothing_else() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let source = std::fs::read_to_string(format!("{dir}/config.vow"))
        .expect("examples/config.vow should exist");

    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "examples/config.vow", source);
    assert!(
        checked.diagnostics.is_empty(),
        "the example should check cleanly:\n{}",
        rendered(&sources, &checked.diagnostics)
    );

    let run = run_main(&program_of(&checked), checked.file, Path::new(dir))
        .expect("the example has a main");
    assert!(run.result.is_ok());
    assert_eq!(run.output[0], "found it");
    assert!(
        run.output[1..].iter().all(|line| line != "found it"),
        "something escaped: {:?}",
        run.output
    );
}

#[test]
fn a_file_with_no_main_is_not_runnable() {
    let (_, checked) = check_ok("module a\n\nfn f() -> Int { 0 }\n");
    assert!(run_main(&program_of(&checked), checked.file, nowhere()).is_none());
}

/// A scratch directory, named so parallel tests do not collide.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("vow-cap-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.0.join(name), contents).unwrap();
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}
