//! Capabilities.
//!
//! Most of these are about what a program cannot do, because that is the whole
//! claim. A capability system that only demonstrates the things that work has
//! demonstrated nothing.

use std::path::{Path, PathBuf};

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_driver::{Checked, check_text};
use deed_interp::{Program, Run, run_main};

/// A directory nothing was granted, for programs that do not touch files.
fn nowhere() -> &'static Path {
    Path::new("")
}

/// The one checked module, as something the interpreter can run.
fn program_of(checked: &Checked) -> Program<'_> {
    let mut program = Program::new();
    program.add(
        checked.file,
        &checked.module,
        &checked.resolutions,
        checked.guards(),
        checked.rows(),
    );
    program
}

fn check(src: &str) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.deed", src);
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
    run_with(src, root, &[])
}

fn run_with(src: &str, root: &Path, arguments: &[String]) -> (SourceMap, Run) {
    let (sources, checked) = check_ok(src);
    let run = run_main(&program_of(&checked), checked.file, root, arguments)
        .expect("there should be a main");
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
        codes_of(&checked.diagnostics).contains(&deed_resolve::codes::UNKNOWN_NAME),
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
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::TYPE_MISMATCH),
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
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
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
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::NO_SUCH_FIELD),
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

// -- installing a handler is a decision ------------------------------------
//
// A `with` block answers for the effect the handler implements. It does not
// answer for what the handler does to implement it. Those effects were charged
// to nobody, so a function holding a `Console` could install a handler that
// writes to it and still declare an empty row, which is the one claim an empty
// row is not allowed to make.
//
// Found by `rows_at_runtime.rs` on the day that test was written, from a
// program that checked clean and then wrote to the screen.

/// A `Log` effect and a handler that implements it by writing to a console.
const LOUD: &str = "effect Log {\n\
     \x20 fn note(message: String) -> ()\n\
     }\n\n\
     handler Loud implements Log {\n\
     \x20 state out: Console\n\n\
     \x20 fn note(message) -> ()\n\
     \x20   uses Io.write,\n\
     \x20 { Io.write(out, message) }\n\
     }\n\n\
     fn talks(n: Int) -> Int uses Log.note {\n\
     \x20 Log.note(\"hi\")\n\
     \x20 n\n\
     }\n\n";

#[test]
fn installing_a_handler_charges_what_the_handler_performs() {
    let (sources, checked) = check(&format!(
        "module a\n\n{LOUD}\
         fn looks_pure(n: Int, screen: Console) -> Int {{\n\
         \x20 with Loud {{ out: screen }} {{ talks(n) }}\n\
         }}\n"
    ));
    let text = rendered(&sources, &checked.diagnostics);
    assert!(checked.has_errors(), "this should not have been accepted");
    assert!(text.contains("DEED5001"), "{text}");
    assert!(text.contains("`Io.write`"), "{text}");
    assert!(text.contains("looks_pure"), "{text}");
}

#[test]
fn declaring_it_is_all_it_takes() {
    // The effect is still discharged. `Log.note` is what the handler is for,
    // so nothing has to declare that; `Io.write` is what it costs.
    check_ok(&format!(
        "module a\n\n{LOUD}\
         fn says_so(n: Int, screen: Console) -> Int\n\
         \x20 uses Io.write,\n\
         {{\n\
         \x20 with Loud {{ out: screen }} {{ talks(n) }}\n\
         }}\n"
    ));
}

#[test]
fn a_handler_that_performs_the_effect_it_implements_is_answered_by_itself() {
    // The handler's own row goes in with the body's row rather than straight
    // onto the function, so the `with` discharges it the same way. Otherwise a
    // handler that called its own effect would be uninstallable.
    check_ok(
        "module a\n\n\
         effect Log {\n\
         \x20 fn note(message: String) -> ()\n\
         \x20 fn warn(message: String) -> ()\n\
         }\n\n\
         handler Chatty implements Log {\n\
         \x20 fn note(message) -> () uses Log.warn, { Log.warn(message) }\n\
         \x20 fn warn(message) -> () { }\n\
         }\n\n\
         fn talks(n: Int) -> Int uses Log.note {\n\
         \x20 Log.note(\"hi\")\n\
         \x20 n\n\
         }\n\n\
         fn quiet(n: Int) -> Int {\n\
         \x20 with Chatty { talks(n) }\n\
         }\n",
    );
}

#[test]
fn a_handler_costs_the_same_from_another_module() {
    // The effect system used to stop at the module boundary, which is where
    // most calls are. An imported handler carries what it performs in its
    // export, the same way an imported function carries its row.
    let mut sources = SourceMap::new();
    let ids = [
        sources.add("loud.deed", format!("module loud\n\n{LOUD}").as_str()),
        sources.add(
            "app.deed",
            "module app\n\n\
             use loud.{Log, Loud, talks}\n\n\
             fn looks_pure(n: Int, screen: Console) -> Int {\n\
             \x20 with Loud { out: screen } { talks(n) }\n\
             }\n",
        ),
    ];
    let checked = deed_driver::check_all(&sources, &ids);
    let said: Vec<Diagnostic> = checked
        .iter()
        .flat_map(|one| one.diagnostics.iter().cloned())
        .collect();
    let text = rendered(&sources, &said);
    assert!(said.iter().any(Diagnostic::is_error), "{text}");
    assert!(text.contains("DEED5001"), "{text}");
    assert!(text.contains("`Io.write`"), "{text}");
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

/// The default is the right one and it is not the only one anybody needs. A
/// program that wants the actual time says so, and what it says is a row entry
/// rather than a second kind of clock, which is the same answer `save` and
/// `read` get about the same `Dir`.
#[test]
fn the_real_clock_is_a_different_entry_in_the_row() {
    let (sources, checked) = check(
        "module a\n\nfn ticking(clock: Clock) -> Int\n  uses Io.now,\n{\n  Io.epoch(clock)\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("performs `Io.epoch` without declaring it"),
        "{text}"
    );

    // And the other way round, since a split that only holds in one direction
    // is not a split.
    let (sources, checked) = check(
        "module a\n\nfn stamped(clock: Clock) -> Int\n  uses Io.epoch,\n{\n  Io.now(clock)\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("performs `Io.now` without declaring it"),
        "{text}"
    );
}

#[test]
fn the_real_clock_reads_the_machine() {
    // Any machine whose clock is set after 2020. Asserting on the number
    // itself would be asserting on when the test was run, which is the whole
    // reason this is a separate entry from `now`.
    let (_, run) = run(
        "module a\n\nfn main(sys: System) -> Int\n  uses Io.epoch,\n{\n  Io.epoch(sys.clock)\n}\n",
    );
    let stamped = run.result.expect("should run").to_string();
    let millis: i64 = stamped.parse().expect("milliseconds");
    assert!(millis > 1_577_836_800_000, "{millis}");
}

#[test]
fn contracts_still_apply_to_main() {
    let (sources, run) = run(
        "module a\n\nfn main(sys: System) -> Int\n  ensures\n    ok => result > 100,\n{\n  1\n}\n",
    );
    let failure = run
        .result
        .expect_err("the postcondition should have caught this");
    assert_eq!(failure.code, deed_interp::codes::POSTCONDITION_FAILED);
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
    // rejected by a list of known bad strings. Written the way a Deed program
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
    assert_eq!(failure.code, deed_interp::codes::NOT_RUNNABLE);
}

#[test]
fn a_function_holding_only_a_dir_cannot_write() {
    let (sources, checked) = check(
        "module a\n\nfn peek(files: Dir) -> ()\n  uses Io.write,\n{\n  Io.write(files, \"hello\")\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::TYPE_MISMATCH),
        "{text}"
    );
    assert!(text.contains("expected `Console`, found `Dir`"), "{text}");
}

// -- writing ---------------------------------------------------------------

#[test]
fn a_dir_writes_a_file_inside_it() {
    let scratch = Scratch::new("save");

    let (_, run) = run_in(
        &report(
            &["write", "save"],
            "  match Io.save(sys.files, \"note.txt\", \"the contents\") {\n    ok(nothing) => Io.write(sys.console, \"saved\"),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["saved".to_string()]);
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("note.txt")).unwrap(),
        "the contents"
    );
}

#[test]
fn every_way_out_of_a_dir_is_refused_for_writing_too() {
    // The same list as the reading case, because writing goes through the same
    // check. A second implementation that agreed today would stop agreeing the
    // first time one of them was edited.
    let scratch = Scratch::new("save-escape");
    let outside = scratch.path().parent().unwrap().to_path_buf();

    for attempt in [
        "..",
        ".",
        "../escaped.txt",
        "/etc/passwd",
        "C:\\\\Windows\\\\System32",
        "a/b",
        "a\\\\b",
        "",
    ] {
        let (_, run) = run_in(
            &report(
                &["write", "save"],
                &format!(
                    "  match Io.save(sys.files, \"{attempt}\", \"escaped\") {{\n    ok(nothing) => Io.write(sys.console, \"WROTE IT\"),\n    err(why) => Io.write(sys.console, why),\n  }}"
                ),
            ),
            scratch.path(),
        );
        assert!(run.result.is_ok());
        assert_ne!(
            run.output,
            vec!["WROTE IT".to_string()],
            "`{attempt}` got out of the directory"
        );
    }

    assert!(
        !outside.join("escaped.txt").exists(),
        "something was written beside the directory"
    );
}

#[test]
fn saving_without_declaring_it_is_an_effect_error() {
    // Holding a `Dir` is not permission to write to it. The row has to say so,
    // which is what keeps `Io.read` and `Io.save` different authorities over
    // the same capability.
    let (sources, checked) = check(
        "module a\n\nfn sneak(files: Dir) -> Result<(), String>\n  uses Io.read,\n{\n  Io.save(files, \"note.txt\", \"hello\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_function_with_no_dir_has_nothing_to_save_into() {
    let (sources, checked) = check(
        "module a\n\nfn sneak() -> Result<(), String>\n  uses Io.save,\n{\n  Io.save(Dir, \"note.txt\", \"hello\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::NOT_A_VALUE),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- deleting --------------------------------------------------------------
//
// Reading, listing and writing all leave what was there. Deleting does not,
// and the difference is not one of degree: a program that writes the wrong
// bytes can be put back from what it overwrote, and one that deletes the wrong
// file cannot be put back from anything.
//
// The claim being tested is that this needed no new mechanism. It is a fourth
// entry in the row over the same `Dir`, the same way `Io.save` and `Io.list`
// were, and holding the directory still says nothing about which of the four a
// function may do.

#[test]
fn a_dir_removes_a_file_inside_it() {
    let scratch = Scratch::new("remove");
    scratch.write("note.txt", "the contents");

    let (_, run) = run_in(
        &report(
            &["write", "remove"],
            "  match Io.remove(sys.files, \"note.txt\") {\n    ok(nothing) => Io.write(sys.console, \"removed\"),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["removed".to_string()]);
    assert!(!scratch.path().join("note.txt").exists());
}

#[test]
fn removing_something_that_is_not_there_is_an_error_rather_than_a_success() {
    // "It was already gone" and "I removed it" are different answers, and a
    // program that cannot tell them apart has a bug waiting.
    let scratch = Scratch::new("remove-missing");

    let (_, run) = run_in(
        &report(
            &["write", "remove"],
            "  match Io.remove(sys.files, \"nothing.txt\") {\n    ok(nothing) => Io.write(sys.console, \"REMOVED IT\"),\n    err(why) => Io.write(sys.console, \"refused\"),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["refused".to_string()]);
}

#[test]
fn a_directory_is_not_a_file_to_remove() {
    // Files only, like `list`. Removing a directory is a different operation
    // with a different blast radius and nothing here wants it.
    let scratch = Scratch::new("remove-dir");
    std::fs::create_dir(scratch.path().join("inside")).unwrap();

    let (_, run) = run_in(
        &report(
            &["write", "remove"],
            "  match Io.remove(sys.files, \"inside\") {\n    ok(nothing) => Io.write(sys.console, \"REMOVED IT\"),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_ne!(run.output, vec!["REMOVED IT".to_string()]);
    assert!(scratch.path().join("inside").exists());
}

#[test]
fn every_way_out_of_a_dir_is_refused_for_removing_too() {
    // The same list as reading and writing, because removing goes through the
    // same check rather than through a second one that agrees today.
    let scratch = Scratch::new("remove-escape");
    let outside = scratch.path().parent().unwrap().to_path_buf();
    std::fs::write(outside.join("target.txt"), "still here").unwrap();

    for attempt in [
        "..",
        ".",
        "../target.txt",
        "/etc/passwd",
        "C:\\\\Windows\\\\System32",
        "a/b",
        "a\\\\b",
        "",
    ] {
        let (_, run) = run_in(
            &report(
                &["write", "remove"],
                &format!(
                    "  match Io.remove(sys.files, \"{attempt}\") {{\n    ok(nothing) => Io.write(sys.console, \"REMOVED IT\"),\n    err(why) => Io.write(sys.console, why),\n  }}"
                ),
            ),
            scratch.path(),
        );
        assert!(run.result.is_ok());
        assert_ne!(
            run.output,
            vec!["REMOVED IT".to_string()],
            "`{attempt}` got out of the directory"
        );
    }

    assert!(
        outside.join("target.txt").exists(),
        "something outside the directory was deleted"
    );
    std::fs::remove_file(outside.join("target.txt")).ok();
}

#[test]
fn removing_without_declaring_it_is_an_effect_error() {
    // The whole argument for not inventing a second kind of `Dir`. This
    // function holds one and may write to it, and it still cannot delete
    // anything, because which of the two it is doing lives in the row.
    let (sources, checked) = check(
        "module a\n\nfn sneak(files: Dir) -> Result<(), String>\n  uses Io.save,\n{\n  Io.remove(files, \"note.txt\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn declaring_removal_does_not_grant_anything_else() {
    // The other direction, and the one that would make the row decorative if
    // it failed. Saying `uses Io.remove` is not a way to read the file first.
    let (sources, checked) = check(
        "module a\n\nfn sneak(files: Dir) -> Result<String, String>\n  uses Io.remove,\n{\n  Io.read(files, \"note.txt\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- making a place --------------------------------------------------------
//
// The one that looks like it breaks the rule and does not. `Io.make` hands
// back a `Dir`, and a `Dir` is authority, so it reads like authority being
// made. It is not: the directory it names is inside the one it was given, so
// what comes back reaches strictly less than what went in. Which paths happen
// to exist was never what a `Dir` granted, which is why `Io.save` writing a
// file that was not there is not authority creation either.

#[test]
fn a_dir_makes_a_directory_inside_it() {
    let scratch = Scratch::new("make");

    let (_, run) = run_in(
        &report(
            &["write", "make"],
            "  match Io.make(sys.files, \"shelf\") {\n    ok(inside) => Io.write(sys.console, \"made\"),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["made".to_string()]);
    assert!(scratch.path().join("shelf").is_dir());
}

#[test]
fn what_comes_back_reaches_less_than_what_went_in() {
    // The claim, tested rather than asserted in a comment. The new `Dir` is a
    // `Dir` like any other, so climbing out of it is refused by the same rule
    // that refuses climbing out of the one it came from, however many times
    // this is done.
    let scratch = Scratch::new("make-narrower");
    let outside = scratch.path().parent().unwrap().to_path_buf();

    let (_, run) = run_in(
        &report(
            &["write", "make", "save"],
            "  match Io.make(sys.files, \"shelf\") {\n    ok(inside) => match Io.save(inside, \"..\", \"escaped\") {\n      ok(nothing) => Io.write(sys.console, \"ESCAPED\"),\n      err(why) => Io.write(sys.console, why),\n    },\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_ne!(run.output, vec!["ESCAPED".to_string()]);
    assert!(!outside.join("escaped").exists());
}

#[test]
fn a_name_that_is_already_there_is_an_error_rather_than_a_success() {
    // "I made it" and "it was already there" are different answers, which is
    // the same reasoning that makes a missing file an error for `Io.remove`.
    let scratch = Scratch::new("make-twice");
    scratch.write("shelf", "a file, not a directory");

    let (_, run) = run_in(
        &report(
            &["write", "make"],
            "  match Io.make(sys.files, \"shelf\") {\n    ok(inside) => Io.write(sys.console, \"MADE IT\"),\n    err(why) => Io.write(sys.console, \"refused\"),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["refused".to_string()]);
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("shelf")).unwrap(),
        "a file, not a directory"
    );
}

#[test]
fn every_way_out_of_a_dir_is_refused_for_making_too() {
    let scratch = Scratch::new("make-escape");
    let outside = scratch.path().parent().unwrap().to_path_buf();

    for attempt in [
        "..",
        ".",
        "../escaped",
        "/etc/passwd",
        "C:\\\\Windows\\\\System32",
        "a/b",
        "a\\\\b",
        "",
    ] {
        let (_, run) = run_in(
            &report(
                &["write", "make"],
                &format!(
                    "  match Io.make(sys.files, \"{attempt}\") {{\n    ok(inside) => Io.write(sys.console, \"MADE IT\"),\n    err(why) => Io.write(sys.console, why),\n  }}"
                ),
            ),
            scratch.path(),
        );
        assert!(run.result.is_ok());
        assert_ne!(
            run.output,
            vec!["MADE IT".to_string()],
            "`{attempt}` got out of the directory"
        );
    }

    assert!(
        !outside.join("escaped").exists(),
        "something was made beside the directory"
    );
}

#[test]
fn making_without_declaring_it_is_an_effect_error() {
    let (sources, checked) = check(
        "module a\n\nfn sneak(files: Dir) -> Result<Dir, String>\n  uses Io.save,\n{\n  Io.make(files, \"shelf\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn declaring_making_does_not_grant_anything_else() {
    let (sources, checked) = check(
        "module a\n\nfn sneak(files: Dir) -> Result<(), String>\n  uses Io.make,\n{\n  Io.save(files, \"note.txt\", \"hello\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- arguments -------------------------------------------------------------

#[test]
fn a_program_reads_what_it_was_invoked_with() {
    let (_, run) = run_with(
        &report(
            &["write", "args"],
            "  Io.write(sys.console, join(Io.args(sys), \" and \"))",
        ),
        nowhere(),
        &["one".to_string(), "two".to_string()],
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["one and two".to_string()]);
}

#[test]
fn a_program_invoked_with_nothing_gets_an_empty_list() {
    // Not a `Result` and not an error. Being given no arguments is an ordinary
    // way to invoke a program, so there is nothing to fail about.
    let (_, run) = run_in(
        &report(
            &["write", "args"],
            "  Io.write(sys.console, to_string(length(Io.args(sys))))",
        ),
        nowhere(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["0".to_string()]);
}

#[test]
fn reading_the_arguments_without_declaring_it_is_an_effect_error() {
    // Arguments are not authority, but they are input from outside, and every
    // other way of getting input from outside says so in the signature.
    let (sources, checked) =
        check("module a\n\nfn peek(sys: System) -> List<String> {\n  Io.args(sys)\n}\n");
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn reading_the_arguments_takes_the_root_capability() {
    // A narrower capability is not enough, which keeps `Io.args` where it
    // belongs: near `main`, with everything below it handed the values rather
    // than the means to read them again.
    let (sources, checked) = check(
        "module a\n\nfn peek(files: Dir) -> List<String>\n  uses Io.args,\n{\n  Io.args(files)\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::TYPE_MISMATCH),
        "{text}"
    );
    assert!(text.contains("expected `System`, found `Dir`"), "{text}");
}

// -- the examples ----------------------------------------------------------

#[test]
fn the_hello_example_runs() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/hello.deed");
    let source = std::fs::read_to_string(path).expect("examples/hello.deed should exist");

    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "examples/hello.deed", source);
    assert!(
        checked.diagnostics.is_empty(),
        "the example should check cleanly:\n{}",
        rendered(&sources, &checked.diagnostics)
    );

    let run = run_main(&program_of(&checked), checked.file, nowhere(), &[])
        .expect("the example has a main");
    assert!(run.result.is_ok());
    assert!(run.output.iter().any(|line| line.contains("world")));
}

#[test]
fn the_config_example_reads_itself_and_nothing_else() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let source = std::fs::read_to_string(format!("{dir}/config.deed"))
        .expect("examples/config.deed should exist");

    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "examples/config.deed", source);
    assert!(
        checked.diagnostics.is_empty(),
        "the example should check cleanly:\n{}",
        rendered(&sources, &checked.diagnostics)
    );

    let run = run_main(&program_of(&checked), checked.file, Path::new(dir), &[])
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
    assert!(run_main(&program_of(&checked), checked.file, nowhere(), &[]).is_none());
}

// -- listing, which is not reading -------------------------------------------

#[test]
fn listing_needs_its_own_entry_in_the_row() {
    // The claim `design/04-capabilities.md` makes, tested at the first place
    // it can be. Holding a `Dir` and declaring `Io.read` means you may read
    // the file somebody told you about. Finding out what is there is strictly
    // more, and the row is what separates them.
    let (sources, checked) = check(
        "module a\n\n\
         fn sneak(files: Dir) -> Int\n\
         \x20 uses Io.read,\n\
         {\n\
         \x20 match Io.list(files) {\n\
         \x20   ok(found) => length(found),\n\
         \x20   err(why) => 0,\n\
         \x20 }\n\
         }\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("performs `Io.list` without declaring it"),
        "{text}"
    );
}

#[test]
fn a_program_that_declares_it_can_see_what_is_there() {
    let scratch = Scratch::new("list");
    scratch.write("one.txt", "1");
    scratch.write("two.txt", "2");

    let (_, run) = run_in(
        "module a\n\n\
         fn main(sys: System) -> Int\n\
         \x20 uses Io.list, Io.write,\n\
         {\n\
         \x20 match Io.list(sys.files) {\n\
         \x20   ok(names) => Io.write(sys.console, join(names, \",\")),\n\
         \x20   err(why) => Io.write(sys.console, why),\n\
         \x20 }\n\
         \x20 0\n\
         }\n",
        scratch.path(),
    );

    // Sorted, because a caller that depends on the order the filesystem felt
    // like today is a caller with a bug that appears on somebody else's
    // machine.
    assert_eq!(run.output, vec!["one.txt,two.txt".to_string()]);
}

#[test]
fn listing_sees_only_what_is_directly_in_the_directory() {
    // Files only. A list holding two kinds of thing with no way to tell them
    // apart is the sort of thing that turns into a bug in the caller, and
    // `Io.open` already needs a name from somewhere.
    let scratch = Scratch::new("list-nested");
    scratch.write("top.txt", "1");
    std::fs::create_dir_all(scratch.path().join("inner")).unwrap();
    std::fs::write(scratch.path().join("inner").join("deep.txt"), "2").unwrap();

    let (_, run) = run_in(
        "module a\n\n\
         fn main(sys: System) -> Int\n\
         \x20 uses Io.list, Io.write,\n\
         {\n\
         \x20 match Io.list(sys.files) {\n\
         \x20   ok(names) => Io.write(sys.console, join(names, \",\")),\n\
         \x20   err(why) => Io.write(sys.console, why),\n\
         \x20 }\n\
         \x20 0\n\
         }\n",
        scratch.path(),
    );

    assert_eq!(run.output, vec!["top.txt".to_string()]);
}

#[test]
fn listing_a_narrowed_directory_sees_only_that_one() {
    // `Io.open` narrows, and listing the result sees inside the narrower one
    // and nothing above it. Authority only ever shrinks on the way down, and
    // enumerating does not go back up.
    let scratch = Scratch::new("list-narrowed");
    scratch.write("outside.txt", "1");
    std::fs::create_dir_all(scratch.path().join("inner")).unwrap();
    std::fs::write(scratch.path().join("inner").join("deep.txt"), "2").unwrap();

    let (_, run) = run_in(
        "module a\n\n\
         fn main(sys: System) -> Int\n\
         \x20 uses Io.open, Io.list, Io.write,\n\
         {\n\
         \x20 match Io.open(sys.files, \"inner\") {\n\
         \x20   ok(inner) => show(sys.console, inner),\n\
         \x20   err(why) => Io.write(sys.console, why),\n\
         \x20 }\n\
         \x20 0\n\
         }\n\n\
         fn show(out: Console, files: Dir) -> ()\n\
         \x20 uses Io.list, Io.write,\n\
         {\n\
         \x20 match Io.list(files) {\n\
         \x20   ok(names) => Io.write(out, join(names, \",\")),\n\
         \x20   err(why) => Io.write(out, why),\n\
         \x20 }\n\
         }\n",
        scratch.path(),
    );

    assert_eq!(run.output, vec!["deep.txt".to_string()]);
}

/// A scratch directory, named so parallel tests do not collide.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("deed-cap-{tag}-{nanos}"));
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
