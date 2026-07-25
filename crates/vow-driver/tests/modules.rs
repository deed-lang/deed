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

#[test]
fn a_field_an_imported_record_does_not_have_is_an_error() {
    // `Thing` crosses the boundary as a type and not only as a name, so its
    // fields are known where it is used. A field it does not declare is a
    // mistake here rather than something an unknown type quietly absorbs.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{Thing}\n\nfn f(t: Thing) -> Int { t.anything_at_all }\n",
        NAMES,
    ]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::NO_SUCH_FIELD),
        "{text}"
    );
    assert!(text.contains("`Thing` from `other` has no field"), "{text}");
}

#[test]
fn a_field_an_imported_record_does_have_keeps_its_declared_type() {
    // The other half of the same thing. Catching a field that is not there is
    // only worth having if the ones that are there come across with the types
    // the other module gave them.
    let src = "module a\n\nuse other.{Thing}\n\nfn f(t: Thing) -> Int { t.n }\n";
    let (sources, checked) = check(&[src, NAMES]);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );

    let start = src.find("t.n").unwrap() as u32;
    let span = vow_diagnostics::Span::new(start, start + "t.n".len() as u32);
    assert_eq!(checked.types.type_of(span), Some(&vow_typeck::Ty::Int));
}

#[test]
fn an_imported_function_is_checked_against_its_signature() {
    // `helper` takes one `Int`, and that signature is readable from here, so a
    // call that does not match it is reported where the call is written.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{helper}\n\nfn f() -> Int { helper(1, 2, 3) }\n",
        NAMES,
    ]);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::WRONG_ARITY),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn an_argument_of_the_wrong_type_to_an_imported_function_is_an_error() {
    // Arity is the easy half. The parameter types have to cross as well, or a
    // call with the right number of wrong things still goes through.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{helper}\n\nfn f() -> Int { helper(\"one\") }\n",
        NAMES,
    ]);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::TYPE_MISMATCH),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- types across the boundary ---------------------------------------------

#[test]
fn a_choice_from_another_module_still_has_to_be_matched_exhaustively() {
    // Exhaustiveness is worth nothing if it stops at a file boundary, because
    // adding a variant would then break the modules that declare it and leave
    // the ones that use it silently wrong.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{Loud, Tone}\n\nfn f(t: Tone) -> Int {\n  match t {\n    Loud => 1,\n  }\n}\n",
        NAMES,
    ]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::NON_EXHAUSTIVE_MATCH),
        "{text}"
    );
    assert!(text.contains("Plain"), "{text}");
}

#[test]
fn a_catch_all_over_an_imported_choice_is_refused_too() {
    let (sources, checked) = check(&[
        "module a\n\nuse other.{Tone}\n\nfn f(t: Tone) -> Int {\n  match t {\n    _ => 1,\n  }\n}\n",
        NAMES,
    ]);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::CATCH_ALL_ON_CHOICE),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_record_from_another_module_can_be_built_and_is_checked() {
    let (sources, checked) = check(&[
        "module a\n\nuse other.{Thing}\n\nfn f() -> Thing { Thing { n: 1 } }\n",
        NAMES,
    ]);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );

    let (sources, checked) = check(&[
        "module a\n\nuse other.{Thing}\n\nfn f() -> Thing { Thing { n: \"one\" } }\n",
        NAMES,
    ]);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::TYPE_MISMATCH),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn two_modules_declaring_the_same_name_declare_two_types() {
    // Identity is the module path and the name together, so a `Thing` from one
    // module is not a `Thing` from another. Anything less would make the
    // checker agree with a mistake that is easy to make and hard to see.
    let (sources, checked) = check(&[
        "module a\n\nuse one.{Thing}\nuse two.{make}\n\nfn f() -> Thing { make() }\n",
        "module one\n\nrecord Thing { n: Int }\n",
        "module two\n\nrecord Thing { n: Int }\n\nfn make() -> Thing { Thing { n: 0 } }\n",
    ]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&vow_typeck::codes::TYPE_MISMATCH),
        "{text}"
    );
    assert!(text.contains("from `one`"), "{text}");
    assert!(text.contains("from `two`"), "{text}");
}

#[test]
fn a_refinement_stays_a_distinct_type_across_a_module_boundary() {
    // An `Int` does not fit where a `Positive` was wanted, wherever the
    // `Positive` was declared.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{Positive}\n\nfn f(n: Int) -> Positive { n }\n",
        "module other\n\ntype Positive = Int where value > 0\n",
    ]);
    assert!(
        checked.has_errors(),
        "an Int reached a Positive:\n{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_transparent_alias_is_still_transparent_from_outside() {
    // An alias with no predicate was not a distinct type where it was
    // declared, and crossing a module boundary does not make it one.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{Count}\n\nfn f(n: Int) -> Count { n }\n",
        "module other\n\ntype Count = Int\n",
    ]);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_type_from_a_third_module_keeps_its_own_identity() {
    // `b` re-exposes a type it imported from `c`. Lowering `b`'s surface must
    // not stamp `b` onto it, or the same type would compare unequal to itself
    // depending on which file the reader came through.
    let (sources, checked) = check(&[
        "module a\n\nuse b.{make}\nuse c.{Thing}\n\nfn f() -> Thing { make() }\n",
        "module b\n\nuse c.{Thing}\n\nfn make() -> Thing { Thing { n: 0 } }\n",
        "module c\n\nrecord Thing { n: Int }\n",
    ]);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_capability_is_the_same_capability_in_every_module() {
    // `Console` comes from the prelude, so it is not named after whichever
    // module happened to mention it first.
    let (sources, checked) = check(&[
        "module a\n\nuse b.{shout}\n\nfn f(out: Console) -> () { shout(out) }\n",
        "module b\n\nfn shout(out: Console) -> ()\n  uses Io.write,\n{\n  Io.write(out, \"hi\")\n}\n",
    ]);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_cycle_of_types_still_terminates() {
    // A type from elsewhere lowers to a module path and a name, both of which
    // are in the syntax, so neither module's lowering recurses into the other.
    let (sources, checked) = check(&[
        "module a\n\nuse b.{B}\n\nrecord A { b: B }\n\nfn f(x: A) -> B { x.b }\n",
        "module b\n\nuse a.{A}\n\nrecord B { n: Int }\n\nfn g(x: B) -> Int { x.n }\n",
    ]);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
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
