//! Imports, across more than one file.
//!
//! Everything here used to pass without anything being checked. A `use` of a
//! module that was not there, a name that module did not declare, a typo in an
//! effect operation: all accepted, all silent, and the name went on to absorb
//! every mistake made with it because an unknown type agrees with everything.

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_driver::{Checked, check_all};

/// Checks `sources` together, and returns the result for the first one.
fn check(files: &[&str]) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let ids: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(index, text)| sources.add(format!("file{index}.deed"), *text))
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
        codes_of(&checked.diagnostics).contains(&deed_resolve::codes::UNKNOWN_MODULE),
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
        codes_of(&checked.diagnostics).contains(&deed_resolve::codes::UNKNOWN_EXPORT),
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
        codes_of(&checked.diagnostics).contains(&deed_resolve::codes::UNKNOWN_MEMBER),
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
    assert!(codes_of(&checked.diagnostics).contains(&deed_resolve::codes::UNKNOWN_EXPORT));
}

#[test]
fn two_files_cannot_claim_one_module() {
    let mut sources = SourceMap::new();
    let ids = vec![
        sources.add("one.deed", "module same\n\nfn f() -> Int { 0 }\n"),
        sources.add("two.deed", "module same\n\nfn g() -> Int { 1 }\n"),
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
        codes_of(&checked.diagnostics).contains(&deed_resolve::codes::DUPLICATE_DEFINITION),
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
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::NO_SUCH_FIELD),
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
    let span = deed_diagnostics::Span::new(start, start + "t.n".len() as u32);
    assert_eq!(checked.types.type_of(span), Some(&deed_typeck::Ty::Int));
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
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::WRONG_ARITY),
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
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::TYPE_MISMATCH),
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
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::NON_EXHAUSTIVE_MATCH),
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
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::CATCH_ALL_ON_CHOICE),
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
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::TYPE_MISMATCH),
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
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::TYPE_MISMATCH),
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
    // module happened to mention it first. `Io` is in the prelude too, which
    // is why `a` declares it without importing anything.
    let (sources, checked) = check(&[
        "module a\n\nuse b.{shout}\n\nfn f(out: Console) -> ()\n  uses Io.write,\n{ shout(out) }\n",
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

// -- running across the boundary -------------------------------------------

/// Runs the tests of the first source with the rest loaded alongside it.
fn run_together(files: &[&str]) -> (SourceMap, Vec<deed_interp::TestOutcome>) {
    let mut sources = SourceMap::new();
    let ids: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(index, text)| sources.add(format!("file{index}.deed"), *text))
        .collect();

    let checks = check_all(&sources, &ids);
    for checked in &checks {
        assert!(
            checked.diagnostics.is_empty(),
            "{}",
            rendered(&sources, &checked.diagnostics)
        );
    }

    let mut program = deed_interp::Program::new();
    for checked in &checks {
        program.add(
            checked.file,
            &checked.module,
            &checked.resolutions,
            checked.guards(),
            checked.rows(),
        );
    }
    let outcomes = deed_interp::run_tests(&program, ids[0]);
    (sources, outcomes)
}

fn expect_pass(files: &[&str]) {
    let (sources, outcomes) = run_together(files);
    assert!(!outcomes.is_empty(), "nothing ran");
    for outcome in &outcomes {
        if let Some(failure) = &outcome.failure {
            panic!(
                "`{}` should pass:\n{}",
                outcome.name,
                render_human(&sources, failure)
            );
        }
    }
}

#[test]
fn a_test_can_call_into_another_module() {
    expect_pass(&[
        "module a\n\nuse b.{twice}\n\ntest \"it runs\" {\n  assert twice(21) == 42\n}\n",
        "module b\n\nfn twice(n: Int) -> Int { n + n }\n",
    ]);
}

#[test]
fn a_record_built_in_one_module_is_read_in_another() {
    expect_pass(&[
        "module a\n\nuse b.{Thing, make}\n\ntest \"fields survive\" {\n  let t = make(7)\n  assert t.n == 7\n  assert make(7) == Thing { n: 7 }\n}\n",
        "module b\n\nrecord Thing { n: Int }\n\nfn make(n: Int) -> Thing { Thing { n } }\n",
    ]);
}

#[test]
fn a_variant_is_one_variant_however_it_was_reached() {
    // Built in `b`, named in `a`, and they have to be the same value. A
    // `DefId` cannot express that, which is why a variant carries the module
    // that declared it instead.
    expect_pass(&[
        "module a\n\nuse b.{Loud, Plain, loudest}\n\ntest \"same value\" {\n  assert loudest() == Loud\n  assert loudest() != Plain\n}\n",
        "module b\n\nchoice Tone {\n  Plain,\n  Loud,\n}\n\nfn loudest() -> Tone { Loud }\n",
    ]);
}

#[test]
fn a_match_across_a_boundary_picks_the_right_arm() {
    expect_pass(&[
        "module a\n\nuse b.{Loud, Plain, Tone, loudest}\n\nfn describe(t: Tone) -> Int {\n  match t {\n    Plain => 1,\n    Loud => 2,\n  }\n}\n\ntest \"the right arm\" {\n  assert describe(loudest()) == 2\n}\n",
        "module b\n\nchoice Tone {\n  Plain,\n  Loud,\n}\n\nfn loudest() -> Tone { Loud }\n",
    ]);
}

#[test]
fn two_modules_with_a_same_named_variant_are_two_types() {
    // Comparing them never reaches the interpreter, because the checker
    // already knows a `Tone` from `one` is not a `Volume` from `two`. Worth a
    // test because the failure it prevents is silent: an `assert` passing for
    // the wrong reason.
    let (sources, checked) = check(&[
        "module a\n\nuse one.{first}\nuse two.{second}\n\nfn f() -> Bool { first() == second() }\n",
        "module one\n\nchoice Tone {\n  Quiet,\n  Loud,\n}\n\nfn first() -> Tone { Loud }\n",
        "module two\n\nchoice Volume {\n  Soft,\n  Loud,\n}\n\nfn second() -> Volume { Loud }\n",
    ]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::TYPE_MISMATCH),
        "{text}"
    );
}

#[test]
fn a_callee_reads_its_own_names() {
    // Both modules declare `limit`. A call into `b` has to see `b`'s.
    expect_pass(&[
        "module a\n\nuse b.{capped}\n\nfn limit() -> Int { 1 }\n\ntest \"its own\" {\n  assert capped() == 100\n}\n",
        "module b\n\nfn limit() -> Int { 100 }\n\nfn capped() -> Int { limit() }\n",
    ]);
}

#[test]
fn an_effect_from_another_module_finds_its_handler() {
    // The last thing that did not cross. `Counter.bump` resolves to an
    // operation of the imported effect, and `with InMemory { .. }` installs a
    // handler whose operations are written in the other module and run there.
    expect_pass(&[
        "module a\n\nuse b.{Counter, InMemory}\n\ntest \"handled\" {\n  with InMemory { count: 0 } {\n    Counter.bump()\n    assert Counter.value() == 1\n  }\n}\n",
        "module b\n\neffect Counter {\n  fn value() -> Int\n  fn bump() -> ()\n}\n\nhandler InMemory implements Counter {\n  state count: Int\n\n  fn value() -> Int { count }\n  fn bump() -> () { count = count + 1 }\n}\n",
    ]);
}

#[test]
fn a_row_over_an_imported_effect_still_has_to_be_tight() {
    let (sources, checked) = check(&[
        "module a\n\nuse b.{Counter}\n\nfn f() -> Int\n  uses\n    Counter.value,\n    Counter.bump,\n{\n  Counter.value()\n}\n",
        "module b\n\neffect Counter {\n  fn value() -> Int\n  fn bump() -> ()\n}\n",
    ]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNUSED_EFFECT),
        "{text}"
    );
    assert!(
        text.contains("`Counter.bump` is declared but never performed"),
        "{text}"
    );
}

#[test]
fn a_handler_from_another_module_only_discharges_its_own_effect() {
    // A `with` used to discharge everything when the handler came from
    // elsewhere, because the compiler could not see which effect it
    // implements. It can now, so an undeclared effect is still reported.
    let (sources, checked) = check(&[
        "module a\n\nuse b.{Counter, InMemory, Other}\n\nfn f() -> Int {\n  with InMemory { count: 0 } {\n    Other.ping()\n    Counter.value()\n  }\n}\n",
        "module b\n\neffect Counter {\n  fn value() -> Int\n}\n\neffect Other {\n  fn ping() -> ()\n}\n\nhandler InMemory implements Counter {\n  state count: Int\n\n  fn value() -> Int { count }\n}\n",
    ]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{text}"
    );
    assert!(text.contains("Other.ping"), "{text}");
}

// -- the example -----------------------------------------------------------

#[test]
fn the_multi_module_example_runs_its_tests() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");

    let mut sources = SourceMap::new();
    let ids: Vec<_> = ["names.deed", "sink.deed", "greeting.deed"]
        .iter()
        .map(|name| {
            let text = std::fs::read_to_string(format!("{dir}/{name}")).unwrap();
            sources.add(format!("examples/{name}"), text)
        })
        .collect();

    let checks = check_all(&sources, &ids);
    for checked in &checks {
        assert!(
            checked.diagnostics.is_empty(),
            "{}",
            rendered(&sources, &checked.diagnostics)
        );
    }

    let mut program = deed_interp::Program::new();
    for checked in &checks {
        program.add(
            checked.file,
            &checked.module,
            &checked.resolutions,
            checked.guards(),
            checked.rows(),
        );
    }

    let outcomes = deed_interp::run_tests(&program, ids[2]);
    assert!(!outcomes.is_empty(), "greeting.deed should have tests");
    for outcome in &outcomes {
        if let Some(failure) = &outcome.failure {
            panic!(
                "`{}` should pass:\n{}",
                outcome.name,
                render_human(&sources, failure)
            );
        }
    }
}
