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
fn modules_in_different_editions_interoperate() {
    let (sources, checked) = check(&[
        "module a edition 2024\n\nuse other.{helper}\n\nfn f() -> Int { helper(1) }\n",
        "module other edition 2025\n\nuse a.{f};\n\nfn helper(n: Int) -> Int { n }\n",
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
fn an_alias_a_signature_was_written_with_crosses_as_what_it_names() {
    // The other direction of the same rule, and the one that was missing. `b`
    // does not declare `Count`, it imports it, so lowering `b`'s surface could
    // not see what it names and sent the bare name across. `a` writes the
    // alias out, because it can see it, and then the two disagreed about a
    // type and its own definition.
    //
    // Found by pointing a benchmark at `examples/logs.deed`: `length` refused
    // a table that came back from a function, and took the same table when it
    // was written out by hand two lines above.
    let (sources, checked) = check(&[
        "module a\n\nuse b.{make}\n\nfn f() -> Int { make() + 1 }\n",
        "module b\n\nuse c.{Count}\n\nfn make() -> Count { 1 }\n",
        "module c\n\ntype Count = Int\n",
    ]);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn an_alias_that_carries_parameters_crosses_the_same_way() {
    // The shape it was found in. `Table` takes two parameters and names a list
    // of something, so the far side has to end up with a list rather than with
    // a name, or nothing that takes a list will take one of these.
    let (sources, checked) = check(&[
        "module a\n\nuse b.{counts}\n\nfn f() -> Int { length(counts()) }\n",
        "module b\n\nuse c.{Pair, Table}\n\nfn counts() -> Table<String, Int> {\n    [Pair { key: \"one\", value: 1 }]\n}\n",
        "module c\n\nrecord Pair<K, V> { key: K, value: V }\n\ntype Table<K, V> = List<Pair<K, V>>\n",
    ]);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_refinement_reached_through_a_third_module_stays_a_distinct_type() {
    // The half that must not move. Writing an alias out is right because it
    // was never a type of its own; a predicate makes one, and it stays one
    // however many modules it is read through.
    let (sources, checked) = check(&[
        "module a\n\nuse b.{make}\n\nfn f(n: Int) -> Int { make(n) }\n",
        "module b\n\nuse c.{Positive}\n\nfn make(n: Positive) -> Int { n }\n",
        "module c\n\ntype Positive = Int where value > 0\n",
    ]);
    assert!(
        checked.has_errors(),
        "an Int reached a Positive through two modules:\n{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn two_aliases_that_name_each_other_still_terminate() {
    // Nothing has refused this before it gets here. The surface pass runs over
    // whatever files it was handed, so a cycle it walked into would be a hang
    // rather than a diagnostic, and a hang in a language server is worse than
    // any message.
    let (_, checked) = check(&[
        "module a\n\nuse b.{Y}\n\ntype X = Y\n\nfn f(x: X) -> X { x }\n",
        "module b\n\nuse a.{X}\n\ntype Y = X\n",
    ]);
    // What it says is not the point. That it says anything at all is.
    assert!(checked.diagnostics.len() < 100);
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

// -- pointing across the boundary ------------------------------------------
//
// A call into another module used to say less than the same call at home. The
// declaration's span was filled with `Span::at(0)` and the label then declined
// to draw itself, because a `Label` carried no file and would have landed on
// whatever sat at those offsets here. A surface carries where a function is
// written now.

/// The callee, written longer than its caller so that reading one file's
/// offsets out of the other cannot land on the same words by accident.
const DECLARES_ARITY: &str = "module other\n\n\
     // Longer than the file that calls into it, on purpose.\n\n\
     fn arity(a: Int, b: Int) -> Int {\n    a + b\n}\n";

#[test]
fn the_wrong_number_of_arguments_points_at_the_declaration_in_the_other_file() {
    let (sources, checked) = check(&[
        "module a\n\nuse other.{arity}\n\nfn f() -> Int {\n    arity(1)\n}\n",
        DECLARES_ARITY,
    ]);
    let [problem] = checked.diagnostics.as_slice() else {
        panic!("{}", rendered(&sources, &checked.diagnostics));
    };
    assert_eq!(problem.code, deed_typeck::codes::WRONG_ARITY);

    // The name too. It came from the signature's span, so losing the span lost
    // the name with it and the message read `this takes` instead.
    assert!(problem.message.starts_with("`arity` takes"), "{problem:?}");

    let [declared] = problem.secondary.as_slice() else {
        panic!("{}", render_human(&sources, problem));
    };
    let file = sources.file(declared.file_or(problem.file));
    assert_ne!(declared.file_or(problem.file), problem.file);
    assert_eq!(
        &file.text()[declared.span.as_range()],
        "fn arity(a: Int, b: Int) -> Int"
    );
    assert!(render_human(&sources, problem).contains(file.name()));
}

#[test]
fn an_argument_of_the_wrong_type_points_at_the_parameter_in_the_other_file() {
    let (sources, checked) = check(&[
        "module a\n\nuse other.{arity}\n\nfn f() -> Int {\n    arity(\"no\", 2)\n}\n",
        DECLARES_ARITY,
    ]);
    let [problem] = checked.diagnostics.as_slice() else {
        panic!("{}", rendered(&sources, &checked.diagnostics));
    };
    assert_eq!(problem.code, deed_typeck::codes::TYPE_MISMATCH);

    let [param] = problem.secondary.as_slice() else {
        panic!("{}", render_human(&sources, problem));
    };
    assert_eq!(param.message, "the parameter it is passed to");
    let file = sources.file(param.file_or(problem.file));
    assert_ne!(param.file_or(problem.file), problem.file);
    assert_eq!(&file.text()[param.span.as_range()], "a: Int");
}

#[test]
fn the_same_call_at_home_says_the_same_thing_without_naming_a_second_file() {
    // The other 23 label sites. A label about the file being checked may not
    // grow a header, and the wording may not change with the boundary.
    let (sources, checked) = check(&[
        "module a\n\nfn arity(a: Int, b: Int) -> Int {\n    a + b\n}\n\n\
         fn f() -> Int {\n    arity(1)\n}\n",
    ]);
    let [problem] = checked.diagnostics.as_slice() else {
        panic!("{}", rendered(&sources, &checked.diagnostics));
    };
    assert!(problem.message.starts_with("`arity` takes"), "{problem:?}");
    assert_eq!(problem.secondary[0].file, None);
    assert_eq!(render_human(&sources, problem).matches("-->").count(), 1);
}

#[test]
fn a_field_of_the_wrong_type_points_at_the_declaration_in_the_other_file() {
    // The third of the labels #298 unblocked. A record's fields arrive through
    // `FieldTy`, which the runtime type table shares, so this one waited for a
    // file on that too.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{Point}\n\nfn f() -> Point {\n    Point { x: \"no\", y: 2 }\n}\n",
        "module other\n\n// Longer than the file that uses it, on purpose.\n\n\
         record Point {\n    x: Int,\n    y: Int,\n}\n",
    ]);
    let [problem] = checked.diagnostics.as_slice() else {
        panic!("{}", rendered(&sources, &checked.diagnostics));
    };
    assert_eq!(problem.code, deed_typeck::codes::TYPE_MISMATCH);

    let [field] = problem.secondary.as_slice() else {
        panic!("{}", render_human(&sources, problem));
    };
    assert_eq!(field.message, "the field it is assigned to");
    let file = sources.file(field.file_or(problem.file));
    assert_ne!(field.file_or(problem.file), problem.file);
    assert_eq!(&file.text()[field.span.as_range()], "x: Int");
}

#[test]
fn a_variant_from_another_module_can_be_built() {
    // The positive half, and the one nothing held: `cargo mutants` deleted the
    // arm of `imported_literal` that builds one and every test stayed green.
    //
    // Silence is not the claim. Without that arm the literal gets no type at
    // all, and an unknown type agrees with everything, so nothing is reported
    // and nothing was checked. What the type came out as is the claim.
    let source =
        "module a\n\nuse other.{Tone, Loud}\n\nfn f() -> Tone {\n    Loud { volume: 3 }\n}\n";
    let (sources, checked) = check(&[
        source,
        "module other\n\nchoice Tone {\n    Plain,\n    Loud { volume: Int },\n}\n",
    ]);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );

    let at = source
        .find("Loud { volume: 3 }")
        .expect("the literal is written") as u32;
    let (_, ty) = checked
        .types
        .at(at)
        .expect("the literal should have a type");
    assert_eq!(checked.types.describe(ty), "`Tone` from `other`");
}

#[test]
fn a_variant_field_points_at_the_declaration_in_the_other_file() {
    let (sources, checked) = check(&[
        "module a\n\nuse other.{Tone, Loud}\n\nfn f() -> Tone {\n    Loud { volume: \"no\" }\n}\n",
        "module other\n\n// Longer than the file that uses it, on purpose.\n\n\
         choice Tone {\n    Plain,\n    Loud { volume: Int },\n}\n",
    ]);
    let mismatch = checked
        .diagnostics
        .iter()
        .find(|d| d.code == deed_typeck::codes::TYPE_MISMATCH)
        .unwrap_or_else(|| panic!("{}", rendered(&sources, &checked.diagnostics)));

    let [field] = mismatch.secondary.as_slice() else {
        panic!("{}", render_human(&sources, mismatch));
    };
    assert_eq!(field.message, "the field it is assigned to");
    let file = sources.file(field.file_or(mismatch.file));
    assert_ne!(field.file_or(mismatch.file), mismatch.file);
    assert_eq!(&file.text()[field.span.as_range()], "volume: Int");
}

#[test]
fn a_handler_state_field_points_at_the_declaration_in_the_other_file() {
    // The other half. A `with` block installing an imported handler is still
    // writing a literal, and a literal nobody points at is one whose
    // declaration a reader has to go and find.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{Tally, Counter, count}\n\n\
         fn f() -> Int {\n    with Tally { seen: \"no\" } {\n        count()\n    }\n}\n",
        "module other\n\n// Longer than the file that uses it, on purpose.\n\n\
         effect Counter {\n    fn count() -> Int\n}\n\n\
         handler Tally implements Counter {\n    state seen: Int\n\n\
         \x20   fn count() -> Int {\n        seen\n    }\n}\n",
    ]);
    let mismatch = checked
        .diagnostics
        .iter()
        .find(|d| d.code == deed_typeck::codes::TYPE_MISMATCH)
        .unwrap_or_else(|| panic!("{}", rendered(&sources, &checked.diagnostics)));

    let [field] = mismatch.secondary.as_slice() else {
        panic!("{}", render_human(&sources, mismatch));
    };
    assert_eq!(field.message, "the field it is assigned to");
    let file = sources.file(field.file_or(mismatch.file));
    assert_ne!(field.file_or(mismatch.file), mismatch.file);
    assert_eq!(&file.text()[field.span.as_range()], "seen: Int");
}

// -- what a caller in another file has to answer for ------------------------
//
// A `where` clause is checked at the call site when it can be settled there,
// which is what `design/02-syntax.md` says and what #312 finished at home. It
// stopped at the module boundary: an imported signature arrived with no
// clauses on it at all, so `halve(0 - 5)` was a refused mistake when `halve`
// was written in the same file and silence when it was written next door.
//
// A precondition is not a proof the callee did, it is a question the caller
// has to answer, and the caller is the only one who can answer it. So unlike a
// refinement predicate the clause crosses whole, along with what every name in
// it refers to, worked out on the side that has the resolution to work it out.

/// A function nobody may call with a negative number, in another module.
///
/// Written longer than the files that call into it, so that reading one file's
/// offsets out of the other cannot land on the same words by accident.
const HALVE: &str = "module other\n\n\
     // Longer than the files that call into it, on purpose.\n\
     // Longer than the files that call into it, on purpose.\n\n\
     fn halve(n: Int) -> Int\n\
     \x20 where\n\
     \x20   n >= 0,\n\
     {\n\
     \x20 n\n\
     }\n";

/// The tiers of every precondition the first file's calls left behind.
fn precondition_tiers(checked: &Checked) -> Vec<deed_typeck::Tier> {
    checked
        .obligations
        .iter()
        .filter(|obligation| obligation.subject.ends_with(" requires"))
        .map(|obligation| obligation.tier)
        .collect()
}

#[test]
fn a_call_into_another_module_that_breaks_a_precondition_is_refused() {
    let (sources, checked) = check(&[
        "module a\n\nuse other.{halve}\n\nfn f() -> Int {\n    halve(0 - 5)\n}\n",
        HALVE,
    ]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(checked.has_errors(), "this should not have been accepted");

    let [problem] = checked.diagnostics.as_slice() else {
        panic!("{text}");
    };
    assert_eq!(problem.code, deed_typeck::codes::BROKEN_PRECONDITION);
    assert_eq!(
        problem.message,
        "this call does not satisfy what `halve` requires"
    );

    // And the clause it broke is drawn against the file that declares it,
    // rather than against these bytes of this one.
    let [clause] = problem.secondary.as_slice() else {
        panic!("{text}");
    };
    let file = sources.file(clause.file_or(problem.file));
    assert_ne!(clause.file_or(problem.file), problem.file);
    assert_eq!(&file.text()[clause.span.as_range()], "n >= 0");
    assert!(text.contains(file.name()), "{text}");
}

#[test]
fn a_caller_in_another_module_that_can_show_it_holds_proves_it() {
    // The half that says the clause was read rather than merely carried. A
    // clause nobody could make sense of is `Unknown`, which is `Guarded`, and
    // `Guarded` is also what you get for doing nothing.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{halve}\n\n\
         fn f(n: Int) -> Int\n\
         \x20 where\n\
         \x20   n > 3,\n\
         {\n\
         \x20 halve(n)\n\
         }\n",
        HALVE,
    ]);
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    // `f`'s own clause is checked by its callers, of which it has none here,
    // so the only precondition standing is the one the call to `halve` left.
    assert_eq!(
        precondition_tiers(&checked),
        vec![deed_typeck::Tier::Proven]
    );
}

#[test]
fn a_caller_in_another_module_that_cannot_tell_is_guarded() {
    let (sources, checked) = check(&[
        "module a\n\nuse other.{halve}\n\nfn f(n: Int) -> Int {\n    halve(n)\n}\n",
        HALVE,
    ]);
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(
        precondition_tiers(&checked),
        vec![deed_typeck::Tier::Guarded]
    );
}

#[test]
fn a_clause_from_another_module_is_read_by_role_rather_than_by_offset() {
    // The reason the roles cross at all. A clause's spans are offsets into the
    // file that declares it, so asking this module's resolution about them
    // answers about whatever this module happens to have written at the same
    // offsets. Here that is a different `n`: the caller has one of its own,
    // known to be negative, sitting where the callee's is.
    //
    // If the clause were read against this file, `n >= 0` would be read as a
    // claim about the caller's `n` and the call would be refused. It is not
    // the caller's `n` that is passed.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{halve}\n\n\
         fn f(n: Int) -> Int\n\
         \x20 where\n\
         \x20   n < 0,\n\
         {\n\
         \x20 halve(0 - n)\n\
         }\n",
        HALVE,
    ]);
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(
        precondition_tiers(&checked),
        vec![deed_typeck::Tier::Proven]
    );
}

#[test]
fn length_in_a_clause_from_another_module_still_means_length() {
    // The second of the two things a clause can name. `length` is a definition
    // rather than a spelling, and the definition it is on the far side is not
    // one this module has, so it crosses as a role like a parameter does.
    let callee = "module other\n\n\
         // Longer than the file that calls into it, on purpose.\n\
         // Longer than the file that calls into it, on purpose.\n\n\
         fn head(items: List<Int>) -> Int\n\
         \x20 where\n\
         \x20   length(items) > 0,\n\
         {\n\
         \x20 0\n\
         }\n";
    let (sources, checked) = check(&[
        "module a\n\nuse other.{head}\n\nfn f() -> Int {\n    head([])\n}\n",
        callee,
    ]);
    let text = rendered(&sources, &checked.diagnostics);
    let [problem] = checked.diagnostics.as_slice() else {
        panic!("{text}");
    };
    assert_eq!(problem.code, deed_typeck::codes::BROKEN_PRECONDITION);

    // And a list that is long enough settles it.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{head}\n\nfn f() -> Int {\n    head([1, 2])\n}\n",
        callee,
    ]);
    assert!(
        !checked.has_errors(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert_eq!(
        precondition_tiers(&checked),
        vec![deed_typeck::Tier::Proven]
    );
}

#[test]
fn an_assert_refuses_across_the_boundary_is_the_statement_being_right() {
    // A contract the checker can see will be broken is what the statement
    // claims, so it is not also a mistake. The boundary does not change that.
    let (sources, checked) = check(&[
        "module a\n\nuse other.{halve}\n\n\
         test \"negative is refused\" {\n    assert refuses halve(0 - 5)\n}\n",
        HALVE,
    ]);
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
    assert!(precondition_tiers(&checked).is_empty());
}
