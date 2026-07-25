//! Imports, across more than one file.
//!
//! Everything here used to pass without anything being checked. A `use` of a
//! module that was not there, a name that module did not declare, a typo in an
//! effect operation: all accepted, all silent, and the name went on to absorb
//! every mistake made with it because an unknown type agrees with everything.

use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_driver::{Checked, check_all};

/// Checks `sources` together, and returns the result for the first one.
fn check(files: &[&str]) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let ids: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(index, text)| sources.add(format!("file{index}.vow"), *text))
        .collect();

    let mut checks = check_all(&sources, &ids);
    (sources, checks.remove(0))
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

const NAMES: &str = "module other\n\n\
                     record Thing { n: Int }\n\n\
                     choice Tone {\n    Plain,\n    Loud,\n}\n\n\
                     effect Ledger {\n    fn balance(id: Int) -> Int\n}\n\n\
                     fn helper(n: Int) -> Int { n }\n";

// -- what is checked now ---------------------------------------------------

#[test]
fn an_import_of_a_module_that_is_not_there_is_an_error() {
    let (sources, checked) = check(&["module a\n\nuse nowhere.{Thing}\n\nfn f() -> Thing {}\n"]);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_resolve::codes::UNKNOWN_MODULE),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn an_import_of_a_name_the_module_does_not_declare_is_an_error() {
    let (sources, checked) = check(&[
        "module a\n\nuse other.{Missing}\n\nfn f() -> Missing {}\n",
        NAMES,
    ]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_resolve::codes::UNKNOWN_EXPORT),
        "{text}"
    );
    assert!(text.contains("`other` declares no `Missing`"), "{text}");
}

#[test]
fn a_near_miss_gets_a_suggestion() {
    let (sources, checked) = check(&["module a\n\nuse other.{Thin}\n\nfn f() -> Thin {}\n", NAMES]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("declares a `Thing`"), "{text}");
}

#[test]
fn an_operation_an_imported_effect_does_not_have_is_an_error() {
    // The one thing that crosses the boundary as more than a name. An effect's
    // operations are part of its declaration, so a typo in a `uses` row is
    // catchable without any cross module type machinery.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{Ledger}\n\nfn f() -> Int\n  uses Ledger.balence,\n{ 0 }\n",
        NAMES,
    ]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_resolve::codes::UNKNOWN_MEMBER),
        "{text}"
    );
    assert!(text.contains("balance"), "{text}");
}

#[test]
fn a_variant_of_an_imported_choice_can_be_imported() {
    // Variants are usable unqualified inside the module that declares them, so
    // they are exported in their own right. Importing one is written down like
    // any other name rather than arriving with the choice, because a `use` that
    // quietly brought in six more names is the wildcard import this language
    // does not have.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{Loud, Tone}\n\nfn f(t: Tone) -> Bool { t == Loud }\n",
        NAMES,
    ]);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_test_is_not_part_of_what_a_module_offers() {
    let (_, checked) = check(&[
        "module a\n\nuse other.{t}\n",
        "module other\n\ntest \"t\" {\n    assert true\n}\n",
    ]);
    assert!(codes_of(&checked.diagnostics).contains(&vow_resolve::codes::UNKNOWN_EXPORT));
}

#[test]
fn two_files_cannot_claim_one_module() {
    let mut sources = SourceMap::new();
    let ids = vec![
        sources.add("one.vow", "module same\n\nfn f() -> Int { 0 }\n"),
        sources.add("two.vow", "module same\n\nfn g() -> Int { 1 }\n"),
    ];

    let checks = check_all(&sources, &ids);
    let text: String = checks
        .iter()
        .map(|checked| rendered(&sources, &checked.diagnostics))
        .collect();
    assert!(text.contains("already declares"), "{text}");
}

#[test]
fn a_cycle_is_not_a_problem() {
    // Worth a test because the issue asked for cycle detection and there is
    // nothing to detect. What a module exports is a function of its syntax
    // alone, so neither file needs the other to have been resolved first. That
    // stops being true once an exported type has to be lowered, which is where
    // a cycle check will have to live.
    let (sources, checked) = check(&[
        "module a\n\nuse b.{B}\n\nrecord A { b: B }\n",
        "module b\n\nuse a.{A}\n\nrecord B { n: Int }\n\nfn f(a: A) -> Int { 0 }\n",
    ]);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_file_is_in_its_own_universe_and_importing_from_itself_is_a_duplicate() {
    // Not a rule anyone wrote, it falls out of a module being named by its own
    // `module` line: a file is one of the files being compiled, so it can see
    // itself. Doing it collides with the declaration it was trying to import.
    let (sources, checked) = check(&["module a\n\nuse a.{f}\n\nfn f() -> Int { 0 }\n"]);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_resolve::codes::DUPLICATE_DEFINITION),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- what is still not checked ---------------------------------------------

#[test]
fn an_imported_type_still_absorbs() {
    // Deliberate, and the reason there is a second issue. `Thing` crosses as a
    // name, not as a type, so nothing done with it is verified. This test
    // exists so that when that changes, something fails and says so.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{Thing}\n\nfn f(t: Thing) -> Int { t.anything_at_all }\n",
        NAMES,
    ]);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn an_imported_function_is_not_checked_against_its_signature() {
    let (sources, checked) = check(&[
        "module a\n\nuse other.{helper}\n\nfn f() -> Int { helper(1, 2, 3) }\n",
        NAMES,
    ]);
    assert!(
        checked.diagnostics.is_empty(),
        "arity across a module boundary is the second half of the work:\n{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- the example -----------------------------------------------------------

#[test]
fn the_two_module_example_checks() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let names = std::fs::read_to_string(format!("{dir}/names.vow")).unwrap();
    let greeting = std::fs::read_to_string(format!("{dir}/greeting.vow")).unwrap();

    let mut sources = SourceMap::new();
    let ids = vec![
        sources.add("examples/names.vow", names),
        sources.add("examples/greeting.vow", greeting),
    ];

    for checked in check_all(&sources, &ids) {
        assert!(
            checked.diagnostics.is_empty(),
            "{}",
            rendered(&sources, &checked.diagnostics)
        );
    }
}
