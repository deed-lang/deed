//! Type checking behaviour.
//!
//! This used to open by saying two things got most of the attention: what the
//! checker refuses to say when it does not know, and refinements. Both were
//! the point when the sentence was written and neither is the largest thing
//! here now. Handlers are, on either way of counting, and they arrived long
//! after. Attention is not countable anyway, so the header no longer claims
//! any, and says something that can be read off the file instead.
//!
//! There are ninety-one tests. Eighty-eight of them hand the checker a module
//! and then either insist it is accepted or name diagnostic codes that should
//! come back, usually the whole list of them in order. The other three ask for
//! something no code carries: that a particular sentence is absent from the
//! rendering, the shape of a fix, and how many obligations landed in a tier.
//! None of them settles for asserting that something failed, which matters
//! because most of these rules were written to replace a worse message rather
//! than to replace silence: a test that only asked for failure would have
//! passed before the change it was written for.
//!
//! What that leaves out is what the reader sees. A code is not a sentence, so
//! the tests whose subject is the wording go on to render the diagnostic and
//! look for the phrase, and those are the ones to copy when adding a message
//! anybody is meant to act on.

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_lexer::tokenize;
use deed_parser::parse;
use deed_resolve::{Universe, resolve};
use deed_typeck::{Checked, Tier, Ty, Types, World, check, codes, surface};

/// The other modules a test source can see.
#[derive(Default)]
struct Deps {
    universe: Universe,
    world: World,
}

fn check_source(src: &str) -> (SourceMap, Checked) {
    check_source_in(src, &Deps::default())
}

fn check_source_in(src: &str, deps: &Deps) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);

    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "test source should parse cleanly");
    let resolved = resolve(file, &parsed.module, &deps.universe);
    assert!(
        !resolved.has_errors(),
        "test source should resolve cleanly: {:?}",
        resolved
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    );

    let checked = check(file, &parsed.module, &resolved.resolutions, &deps.world);
    (sources, checked)
}

/// The other modules, parsed and lowered.
///
/// An import with nothing behind it is an error now, and a name from elsewhere
/// has a type now, so a test about one needs a real module declaring it.
fn universe_of(modules: &[&str]) -> Deps {
    let mut deps = Deps::default();
    let mut sources = SourceMap::new();
    for (index, source) in modules.iter().enumerate() {
        let file = sources.add(format!("dep{index}.deed"), *source);
        let lexed = tokenize(file, sources.file(file).text());
        let parsed = parse(file, &lexed.tokens);
        deps.universe.add(&parsed.module);
    }

    // A second pass, because a module's surface needs its own names resolved
    // and that needs every module already registered.
    let mut sources = SourceMap::new();
    let mut surfaces = Vec::new();
    for (index, source) in modules.iter().enumerate() {
        let file = sources.add(format!("dep{index}.deed"), *source);
        let lexed = tokenize(file, sources.file(file).text());
        let parsed = parse(file, &lexed.tokens);
        let resolved = resolve(file, &parsed.module, &deps.universe);
        if let Some(name) = &parsed.module.name {
            surfaces.push((
                name.to_string_path(),
                surface(&parsed.module, &resolved.resolutions),
            ));
        }
    }
    deps.world = World::of(surfaces);

    deps
}

fn check_ok(src: &str) -> Types {
    check_ok_in(src, &Deps::default())
}

fn check_ok_in(src: &str, deps: &Deps) -> Types {
    let (sources, checked) = check_source_in(src, deps);
    if !checked.diagnostics.is_empty() {
        let rendered: Vec<String> = checked
            .diagnostics
            .iter()
            .map(|d| render_human(&sources, d))
            .collect();
        panic!("expected a clean check:\n{}", rendered.join("\n"));
    }
    checked.types
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

// -- the worked example ----------------------------------------------------

#[test]
fn the_worked_example_type_checks() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.deed");
    let source = std::fs::read_to_string(path).expect("examples/transfer.deed should exist");

    let mut sources = SourceMap::new();
    let file = sources.add("examples/transfer.deed", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, &Universe::new());
    assert!(!resolved.has_errors());

    let checked = check(file, &parsed.module, &resolved.resolutions, &World::new());
    if !checked.diagnostics.is_empty() {
        panic!(
            "the worked example should type check:\n{}",
            rendered(&sources, &checked.diagnostics)
        );
    }
}

#[test]
fn the_worked_example_proves_its_refinements() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.deed");
    let source = std::fs::read_to_string(path).unwrap();

    let mut sources = SourceMap::new();
    let file = sources.add("transfer.deed", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, &Universe::new());
    let checked = check(file, &parsed.module, &resolved.resolutions, &World::new());

    assert!(
        checked.types.obligations_at(Tier::Proven) > 0,
        "the example should discharge something statically"
    );
    assert_eq!(
        checked.types.obligations_at(Tier::Guarded),
        0,
        "nothing in the example should have needed a runtime check"
    );
}

// -- what the checker refuses to guess -------------------------------------

#[test]
fn a_name_from_another_module_has_a_type() {
    // A name that came from an import used to have no type at all, so nothing
    // done with it was ever checked. It is now identified by the module it
    // came from together with the name it was declared under, which needs no
    // numbering shared between modules.
    let src = "module a\n\n\
               use other.{Thing}\n\n\
               record R { field: Thing }\n\n\
               fn g(r: R) -> Thing { r.field }\n";
    let types = check_ok_in(
        src,
        &universe_of(&["module other\n\nrecord Thing { n: Int }\n"]),
    );

    let start = src.find("r.field").unwrap() as u32;
    let span = deed_diagnostics::Span::new(start, start + "r.field".len() as u32);
    assert_eq!(
        types.type_of(span),
        Some(&Ty::External {
            module: "other".into(),
            name: "Thing".into(),
            args: Vec::new(),
        })
    );
}

#[test]
fn an_operator_with_one_unknown_side_says_nothing() {
    // `balanse` does not resolve, so it has no type, and a type that agrees
    // with everything is what keeps one mistake from being reported twice.
    // The resolver has already said the name is missing.
    let mut sources = SourceMap::new();
    let file = sources.add(
        "test.deed",
        "module a\n\n\
         record Money { units: Int }\n\n\
         fn f(m: Money) -> Bool { balanse() < m }\n\n\
         fn g(m: Money) -> Bool { balanse() - m == balanse() }\n",
    );
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, &Universe::new());
    let checked = check(file, &parsed.module, &resolved.resolutions, &World::new());
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn an_operator_with_two_known_sides_is_checked() {
    let (sources, checked) = check_source(
        "module a\n\nrecord Money { units: Int }\n\nfn f(m: Money, n: Money) -> Int { m + n }\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&codes::TYPE_MISMATCH),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn the_question_mark_rejects_a_type_that_is_only_named_like_a_result() {
    // `?` went unchecked here while the `Result` in scope came from an import
    // and so had no type. It has one now, and a record another module happens
    // to have called `Result` is not the one the language provides.
    let (sources, checked) = check_source_in(
        "module a\n\nuse std/result.{Result}\n\nfn f(r: Result) -> Int { r? }\n",
        &universe_of(&["module std/result\n\nrecord Result { n: Int }\n"]),
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_A_RESULT]);
    assert!(rendered(&sources, &checked.diagnostics).contains("`?` needs a `Result`"));
}

// -- operators -------------------------------------------------------------

#[test]
fn plus_joins_two_strings() {
    check_ok("module a\n\nfn greet(name: String) -> String { \"hello, \" + name }\n");
}

#[test]
fn plus_will_not_join_a_number_to_a_string() {
    // There is no conversion between them, so guessing which one the author
    // meant would be guessing.
    let (sources, checked) =
        check_source("module a\n\nfn f(n: Int) -> String { \"count: \" + n }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
    assert!(rendered(&sources, &checked.diagnostics).contains("joined with this"));
}

#[test]
fn plus_still_adds_two_numbers() {
    check_ok("module a\n\nfn f(n: Int) -> Int { n + 1 }\n");
}

/// The half of the ordering rule that was kept rather than chosen.
///
/// The rule this test guards replaced one that asked only that both sides
/// agree, so text was comparable before anybody narrowed anything and the
/// narrowing left it in. It stays because it is the only thing in the language
/// that puts two pieces of text in an order and never ties two of them that
/// differ: `length` and `to_int` both rank text and both tie, `ab` with `ba`
/// and `007` with `7`, and `Io.list` sorts file names but wants a `Dir` and a
/// written file per comparison and ties whatever the filesystem does not tell
/// apart. A record is refused below and a caller who wants two of them ranked
/// passes the comparison in, which asks that caller for nothing it does not
/// already have to have, since the shape of a record does not say which field
/// decides the order and some of those fields have no order of their own.
/// `split(s, "")` reaches the characters, so a comparator over text is
/// writable too, but only over the characters it names, and everything else
/// ties.
#[test]
fn strings_are_ordered() {
    check_ok("module a\n\nfn f(a: String, b: String) -> Bool { a < b }\n");
}

#[test]
fn a_record_is_not_ordered() {
    // This used to pass here and fail at runtime, with a message blaming the
    // interpreter for not implementing something that has nothing to implement.
    let (sources, checked) = check_source(
        "module a\n\nrecord Point { x: Int }\n\nfn f(a: Point, b: Point) -> Bool { a < b }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_ORDERED]);
    assert!(rendered(&sources, &checked.diagnostics).contains("there is none on `Point`"));
}

#[test]
fn a_choice_is_not_ordered_either() {
    let (_, checked) = check_source(
        "module a\n\nchoice Tone { Plain, Loud }\n\nfn f(a: Tone, b: Tone) -> Bool { a >= b }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_ORDERED]);
}

/// The case that decides whether this language wants a bound.
///
/// A record and a choice are concrete: the checker can look at them and see
/// there is no order. A type parameter is the one where a trait system would
/// have something to say, because `T: Ord` is exactly the sentence being
/// refused here. Both tests above existed and this one did not, which left
/// the deciding case as the unmeasured one.
///
/// It refuses, and the refusal is the reason a generic sort cannot be written
/// without passing the comparison in. Anything that loosens this is choosing
/// bounds, and should have to come past this test to do it.
#[test]
fn a_type_parameter_is_not_ordered() {
    let (sources, checked) =
        check_source("module a\n\nfn bigger<T>(a: T, b: T) -> Bool { a > b }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_ORDERED]);
    assert!(
        rendered(&sources, &checked.diagnostics).contains("`T`, a type parameter"),
        "the message should name what it is, not just what it is not"
    );
}

/// And equality is not the same question, on the same type.
///
/// This is why the ordering refusal above costs so little. The bound people
/// reach for first is `Eq`, and here it is not a bound at all: `==` works on
/// a bare `T`, which is what lets `std/table` be a keyed table over any key
/// type without saying anything about it.
#[test]
fn a_type_parameter_is_still_comparable_for_equality() {
    check_ok("module a\n\nfn same<T>(a: T, b: T) -> Bool { a == b }\n");
}

#[test]
fn two_types_that_do_not_match_are_one_mistake_not_two() {
    // Saying the sides disagree and then that the type they do not share has
    // no ordering is two diagnostics for one edit.
    let (_, checked) = check_source(
        "module a\n\nrecord Point { x: Int }\n\nfn f(a: Point, b: Int) -> Bool { a < b }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn anything_can_still_be_compared_for_equality() {
    // Equality is structural and total. Wanting to know whether two records
    // are the same is reasonable; wanting to know which one is larger is not.
    check_ok(
        "module a\n\nrecord Point { x: Int }\n\nfn f(a: Point, b: Point) -> Bool { a == b }\n",
    );
}

#[test]
fn a_refinement_over_a_string_is_ordered_through_its_base() {
    check_ok(
        "module a\n\n\
         type Name = String where length(value) > 0\n\n\
         fn f(a: Name, b: Name) -> Bool { a < b }\n",
    );
}

#[test]
fn length_takes_a_string_and_gives_a_number() {
    check_ok("module a\n\nfn f(s: String) -> Int { length(s) }\n");
}

#[test]
fn length_will_not_measure_a_number() {
    // Its own code rather than a plain mismatch, because `length` takes more
    // than one type now and "expected a String" would be the wrong half of
    // the story.
    let (_, checked) = check_source("module a\n\nfn f(n: Int) -> Int { length(n) }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_A_LIST]);
}

#[test]
fn a_length_is_never_negative() {
    // The prelude says so, so a refinement that only needs that much is
    // Proven rather than checked again at runtime.
    let types = check_ok(
        "module a\n\n\
         type Counted = Int where value >= 0\n\n\
         fn f(s: String) -> Counted { length(s) }\n",
    );
    assert_eq!(types.obligations_at(Tier::Proven), 1);
}

// -- handlers ---------------------------------------------------------------

const COUNTER: &str = "module a\n\n\
     effect Counter {\n    fn set(to: Int) -> ()\n    fn value() -> Int\n}\n\n";

#[test]
fn a_handler_operation_takes_its_types_from_the_effect() {
    // Nothing here writes `Int` anywhere, and `to + 1` still has to be
    // arithmetic. Before this, every parameter in every handler body was the
    // unknown type, so nothing done with one was checked at all.
    check_ok(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: Int\n\n\
         \x20 fn set(to) -> () {{\n    count = to + 1\n  }}\n\n\
         \x20 fn value() -> Int {{\n    count\n  }}\n\
         }}\n"
    ));
}

#[test]
fn a_handler_operation_is_checked_against_those_types() {
    let (sources, checked) = check_source(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: String\n\n\
         \x20 fn set(to) -> () {{\n    count = to\n  }}\n\n\
         \x20 fn value() -> Int {{\n    1\n  }}\n\
         }}\n"
    ));
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
    assert!(rendered(&sources, &checked.diagnostics).contains("found `Int`"));
}

#[test]
fn a_refined_state_field_is_an_obligation_like_any_other() {
    // This is the one that could not be written before. The parameter had no
    // type, unknown absorbs, and assigning it to a refined state field raised
    // nothing at all: no warning, no tier, no runtime check.
    let (_, checked) = check_source(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         effect Counter {\n    fn set(to: Int) -> ()\n}\n\n\
         handler InMemory implements Counter {\n\
         \x20 state count: Positive\n\n\
         \x20 fn set(to) -> () {\n    count = to\n  }\n\
         }\n",
    );
    assert!(!checked.has_errors());
    assert_eq!(checked.types.obligations_at(Tier::Guarded), 1);
}

#[test]
fn a_handler_operation_the_effect_does_not_have_is_an_error() {
    let (sources, checked) = check_source(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: Int\n\n\
         \x20 fn set(to) -> () {{\n    count = to\n  }}\n\n\
         \x20 fn value() -> Int {{\n    count\n  }}\n\n\
         \x20 fn nonsense() -> Int {{\n    1\n  }}\n\
         }}\n"
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::OPERATION_MISMATCH]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("does not declare an operation"));
}

#[test]
fn a_handler_operation_with_the_wrong_arity_is_an_error() {
    // Both parameters used to be accepted and both used to be unknown, so a
    // handler could disagree with its effect about the shape of a call and
    // nobody was told.
    let (sources, checked) = check_source(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: Int\n\n\
         \x20 fn set(to, extra) -> () {{\n    count = 1\n  }}\n\n\
         \x20 fn value() -> Int {{\n    count\n  }}\n\
         }}\n"
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::OPERATION_MISMATCH]
    );
    assert!(
        rendered(&sources, &checked.diagnostics).contains("takes 1 argument, and this takes 2")
    );
}

#[test]
fn a_handler_that_leaves_an_operation_out_is_an_error() {
    // The mirror of the test above, and the half that was never checked. A
    // `with` block discharges the effect rather than the operations written
    // inside the handler, so this used to typecheck, install, and fail at the
    // first call that reached `value`.
    let (sources, checked) = check_source(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: Int\n\n\
         \x20 fn set(to) -> () {{\n    count = to\n  }}\n\
         }}\n"
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::HANDLER_MISSING_OPERATION]
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("does not implement `value`"), "{text}");
    assert!(text.contains("one operation still to write"), "{text}");
}

#[test]
fn every_operation_left_out_is_named_at_once() {
    // One diagnostic rather than one per operation. The mistake is the
    // handler, not each of the things it did not write, and a reader fixing it
    // wants the list in front of them.
    let (sources, checked) = check_source(&format!(
        "{COUNTER}\
         handler Empty implements Counter {{\n\
         \x20 state count: Int\n\
         }}\n"
    ));
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::HANDLER_MISSING_OPERATION]
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("`set`") && text.contains("`value`"), "{text}");
    assert!(text.contains("2 operations still to write"), "{text}");
}

#[test]
fn a_whole_handler_is_accepted() {
    // Worth having next to the two above: without it they would both pass on a
    // checker that rejected every handler there is.
    check_ok(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: Int\n\n\
         \x20 fn set(to) -> () {{\n    count = to\n  }}\n\n\
         \x20 fn value() -> Int {{\n    count\n  }}\n\
         }}\n"
    ));
}

#[test]
fn an_imported_effect_is_counted_the_same_way() {
    let (sources, checked) = check_source_in(
        "module a\n\n\
         use other.{Ledger}\n\n\
         handler InMemory implements Ledger {\n\
         \x20 state total: Int\n\n\
         \x20 fn post(amount) -> () {\n    total = total + amount\n  }\n\
         }\n",
        &universe_of(&[
            "module other\n\neffect Ledger {\n    fn post(amount: Int) -> ()\n    fn balance() -> Int\n}\n",
        ]),
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::HANDLER_MISSING_OPERATION]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("does not implement `balance`"));
}

#[test]
fn a_handler_for_an_imported_effect_gets_its_types_too() {
    check_ok_in(
        "module a\n\n\
         use other.{Ledger}\n\n\
         handler InMemory implements Ledger {\n\
         \x20 state total: Int\n\n\
         \x20 fn post(amount) -> () {\n    total = total + amount\n  }\n\
         }\n",
        &universe_of(&["module other\n\neffect Ledger {\n    fn post(amount: Int) -> ()\n}\n"]),
    );
}

#[test]
fn an_imported_handler_operation_is_checked_against_those_types() {
    let (_, checked) = check_source_in(
        "module a\n\n\
         use other.{Ledger}\n\n\
         handler InMemory implements Ledger {\n\
         \x20 state total: String\n\n\
         \x20 fn post(amount) -> () {\n    total = amount\n  }\n\
         }\n",
        &universe_of(&["module other\n\neffect Ledger {\n    fn post(amount: Int) -> ()\n}\n"]),
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn a_handler_literal_is_checked_against_its_state() {
    // `with InMemory { count: "hello" }` used to be accepted. The literal had
    // no type at all, so the field values were never compared with anything.
    let (sources, checked) = check_source(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: Int\n\n\
         \x20 fn set(to) -> () {{\n    count = to\n  }}\n\n\
         \x20 fn value() -> Int {{\n    count\n  }}\n\
         }}\n\n\
         test \"installing it\" {{\n\
         \x20 with InMemory {{ count: \"hello\" }} {{\n\
         \x20   assert Counter.value() == 0\n\
         \x20 }}\n\
         }}\n"
    ));
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
    assert!(rendered(&sources, &checked.diagnostics).contains("found `String`"));
}

#[test]
fn a_handler_literal_missing_state_says_which() {
    let (sources, checked) = check_source(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: Int\n\
         \x20 state limit: Int\n\n\
         \x20 fn set(to) -> () {{\n    count = to\n  }}\n\n\
         \x20 fn value() -> Int {{\n    count + limit\n  }}\n\
         }}\n\n\
         test \"installing it\" {{\n\
         \x20 with InMemory {{ count: 0 }} {{\n\
         \x20   assert Counter.value() == 0\n\
         \x20 }}\n\
         }}\n"
    ));
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::MISSING_FIELDS]);
    assert!(rendered(&sources, &checked.diagnostics).contains("limit"));
}

#[test]
fn a_handler_literal_from_another_module_is_checked_too() {
    let (_, checked) = check_source_in(
        "module a\n\n\
         use other.{Counter, InMemory}\n\n\
         test \"installing it\" {\n\
         \x20 with InMemory { count: \"hello\" } {\n\
         \x20   assert Counter.value() == 0\n\
         \x20 }\n\
         }\n",
        &universe_of(&["module other\n\n\
             effect Counter {\n    fn value() -> Int\n}\n\n\
             handler InMemory implements Counter {\n\
             \x20 state count: Int\n\n\
             \x20 fn value() -> Int { count }\n\
             }\n"]),
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn a_call_to_an_imported_effect_operation_is_checked() {
    // This one had no type at all, so the arguments, the arity and the result
    // were all unchecked the moment the effect came from another file.
    let (_, checked) = check_source_in(
        "module a\n\n\
         use other.{Sink}\n\n\
         fn f() -> Int\n\
         \x20 uses\n\
         \x20   Sink.emit,\n\
         {\n\
         \x20 Sink.emit(1)\n\
         \x20 0\n\
         }\n",
        &universe_of(&["module other\n\neffect Sink {\n    fn emit(line: String) -> ()\n}\n"]),
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn an_imported_effect_operation_call_gives_back_its_declared_type() {
    check_ok_in(
        "module a\n\n\
         use other.{Sink}\n\n\
         fn f() -> Int\n\
         \x20 uses\n\
         \x20   Sink.count,\n\
         {\n\
         \x20 Sink.count() + 1\n\
         }\n",
        &universe_of(&["module other\n\neffect Sink {\n    fn count() -> Int\n}\n"]),
    );
}

#[test]
fn an_imported_effect_operation_called_with_the_wrong_arity_is_an_error() {
    let (_, checked) = check_source_in(
        "module a\n\n\
         use other.{Sink}\n\n\
         fn f() -> Int\n\
         \x20 uses\n\
         \x20   Sink.count,\n\
         {\n\
         \x20 Sink.count(1)\n\
         }\n",
        &universe_of(&["module other\n\neffect Sink {\n    fn count() -> Int\n}\n"]),
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::WRONG_ARITY]);
}

// -- refinements -----------------------------------------------------------

#[test]
fn a_literal_satisfying_a_refinement_is_proven() {
    let types = check_ok(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn take(n: Positive) -> Int { 0 }\n\n\
         fn f() -> Int { take(40) }\n",
    );
    assert_eq!(types.obligations_at(Tier::Proven), 1);
    assert_eq!(types.obligations_at(Tier::Guarded), 0);
}

#[test]
fn a_literal_violating_a_refinement_is_an_error() {
    let (sources, checked) = check_source(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn take(n: Positive) -> Int { 0 }\n\n\
         fn f() -> Int { take(0) }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::VIOLATED_REFINEMENT]
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("does not satisfy `Positive`"), "{text}");
    assert!(text.contains("the predicate it has to satisfy"), "{text}");
}

#[test]
fn an_unprovable_refinement_says_so_out_loud() {
    // The design promises Proven, Tested or Guarded, and that a contract never
    // quietly degrades into a runtime check. This is that promise being kept.
    let (sources, checked) = check_source(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn take(n: Positive) -> Int { 0 }\n\n\
         fn f(n: Int) -> Int { take(n) }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::UNPROVEN_REFINEMENT]
    );
    assert!(!checked.has_errors(), "it is a warning, not a rejection");
    assert_eq!(checked.types.obligations_at(Tier::Guarded), 1);

    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("becomes a runtime check"), "{text}");
    assert!(text.contains("Guarded"), "{text}");
}

#[test]
fn a_refinement_widens_to_its_base_without_an_obligation() {
    let types = check_ok(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn f(n: Positive) -> Int { n + 1 }\n",
    );
    assert!(types.obligations().is_empty());
}

#[test]
fn an_alias_without_a_predicate_is_transparent() {
    // It adds no information, so it should not add a distinction either.
    check_ok("module a\n\ntype Count = Int\n\nfn f(n: Count) -> Int { n }\n");
}

#[test]
fn a_self_referential_alias_is_reported_once() {
    let (_, checked) = check_source("module a\n\ntype Loop = Loop\n\nfn f(x: Loop) -> Int { 0 }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::TYPE_ALIAS_CYCLE]
    );
}

// -- records and choices ---------------------------------------------------

#[test]
fn a_literal_missing_fields_lists_them() {
    let (sources, checked) = check_source(
        "module a\n\nrecord R { a: Int, b: Int, c: Int }\n\nfn f() -> R { R { a: 1 } }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::MISSING_FIELDS]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("missing `b` and `c`"), "{text}");
}

#[test]
fn a_literal_with_an_unknown_field_lists_the_real_ones() {
    let (sources, checked) =
        check_source("module a\n\nrecord R { a: Int }\n\nfn f() -> R { R { a: 1, z: 2 } }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::UNKNOWN_FIELD]);
    assert!(rendered(&sources, &checked.diagnostics).contains("it has `a`"));
}

#[test]
fn the_shorthand_form_is_type_checked() {
    let (sources, checked) =
        check_source("module a\n\nrecord R { a: Int }\n\nfn f(a: Bool) -> R { R { a } }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
    assert!(rendered(&sources, &checked.diagnostics).contains("the field it is assigned to"));
}

/// A literal is the one form whose head has to be a declaration rather than a
/// value, and the head is written exactly like a name being read.
///
/// So the mistake is easy: a function is a value with a type, `g { a: 1 }`
/// parses, and what it says is that the answer of calling `g` has fields. The
/// message names the type it found rather than saying the shape was wrong,
/// because the shape was fine and the head was not.
#[test]
fn a_function_is_not_something_a_literal_can_build() {
    let (sources, checked) = check_source(
        "module a\n\nfn g() -> Int { 0 }\n\nfn f() -> Int {\n    let a = g { x: 1 }\n    0\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&codes::NOT_A_CONSTRUCTOR),
        "{:?}",
        codes_of(&checked.diagnostics)
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("`Fn() -> Int` is not a record or a variant"),
        "{text}"
    );
    assert!(text.contains("cannot be built with a literal"), "{text}");
}

/// The same for an ordinary value, which is the shape somebody reaches for
/// when they have forgotten which of two names is the type.
#[test]
fn a_value_is_not_something_a_literal_can_build_either() {
    let (sources, checked) =
        check_source("module a\n\nfn f(n: Int) -> Int {\n    let b = n { x: 1 }\n    0\n}\n");
    assert!(
        codes_of(&checked.diagnostics).contains(&codes::NOT_A_CONSTRUCTOR),
        "{:?}",
        codes_of(&checked.diagnostics)
    );
    assert!(
        rendered(&sources, &checked.diagnostics).contains("`Int` is not a record or a variant")
    );
}

/// A type name in that position is a different mistake with a message of its
/// own, and this one has to stay out of its way. `Int` is not a value at all,
/// so saying it is not a record would be answering the second question first.
#[test]
fn a_type_name_gets_the_message_about_being_a_type() {
    let (_, checked) =
        check_source("module a\n\nfn f() -> Int {\n    let c = Int { x: 1 }\n    0\n}\n");
    assert!(
        !codes_of(&checked.diagnostics).contains(&codes::NOT_A_CONSTRUCTOR),
        "{:?}",
        codes_of(&checked.diagnostics)
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&codes::NOT_A_VALUE),
        "{:?}",
        codes_of(&checked.diagnostics)
    );
}

#[test]
fn field_access_on_a_missing_field_lists_what_is_there() {
    let (sources, checked) =
        check_source("module a\n\nrecord R { alpha: Int }\n\nfn f(r: R) -> Int { r.beta }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NO_SUCH_FIELD]);
    assert!(rendered(&sources, &checked.diagnostics).contains("it has `alpha`"));
}

/// `xs.length()` is what somebody writes on their first day here, because
/// every language they came from has methods. `length` exists, so "no such
/// field" left them looking for a field.
#[test]
fn a_method_call_says_the_function_is_called_the_other_way_round() {
    let (sources, checked) =
        check_source("module a\n\nfn f(xs: List<Int>) -> Int { xs.length() }\n");
    assert!(
        codes_of(&checked.diagnostics).contains(&codes::NO_SUCH_FIELD),
        "{:?}",
        codes_of(&checked.diagnostics)
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("there are no methods"), "{text}");
    assert!(
        text.contains("`length(x)` rather than `x.length()`"),
        "{text}"
    );
}

/// Only for names that are actually functions. A misspelled field is still a
/// misspelled field, and telling its author to call it would be worse than
/// saying nothing.
#[test]
fn a_missing_field_that_is_not_a_function_is_not_called_a_method() {
    let (sources, checked) =
        check_source("module a\n\nrecord R { alpha: Int }\n\nfn f(r: R) -> Int { r.beta }\n");
    assert!(
        !rendered(&sources, &checked.diagnostics).contains("there are no methods"),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_variant_literal_has_the_type_of_its_choice() {
    let types = check_ok(
        "module a\n\nchoice E { Full { n: Int }, Bare }\n\nfn f() -> E { Full { n: 1 } }\n",
    );
    let _ = types;
}

#[test]
fn a_variant_literal_checks_its_fields() {
    let (_, checked) = check_source(
        "module a\n\nchoice E { Full { n: Int } }\n\nfn f() -> E { Full { n: true } }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

// -- matches ---------------------------------------------------------------

#[test]
fn a_match_missing_variants_names_them() {
    let (sources, checked) = check_source(
        "module a\n\n\
         choice E { A, B, C }\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   A => 1,\n\
         \x20 }\n\
         }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::NON_EXHAUSTIVE_MATCH]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("cover `B` and `C`"));
}

#[test]
fn a_wildcard_arm_on_a_choice_is_rejected() {
    let (sources, checked) = check_source(
        "module a\n\nchoice E { A, B, C }\n\nfn f(e: E) -> Int {\n  match e {\n    A => 1,\n    _ => 2,\n  }\n}\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::CATCH_ALL_ON_CHOICE]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("matches every variant of `E`"));
}

#[test]
fn a_bare_binding_arm_on_a_choice_is_rejected_too() {
    let (_, checked) = check_source(
        "module a\n\nchoice E { A, B }\n\nfn f(e: E) -> Int {\n  match e {\n    A => 1,\n    other => 2,\n  }\n}\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::CATCH_ALL_ON_CHOICE]
    );
}

#[test]
fn a_wildcard_is_fine_where_the_cases_cannot_be_listed() {
    // The rule is about choices. An `Int` has no variants to enumerate.
    check_ok("module a\n\nfn f(n: Int) -> Int {\n  match n {\n    0 => 1,\n    _ => 2,\n  }\n}\n");
}

// -- an arm naming more than one variant -----------------------------------

#[test]
fn one_arm_can_name_several_variants() {
    check_ok(
        "module a\n\n\
         choice E { A, B, C }\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   A | B => 1,\n\
         \x20   C => 2,\n\
         \x20 }\n\
         }\n",
    );
}

#[test]
fn naming_several_does_not_make_a_match_complete() {
    // The point of the whole rule. An arm covers what it names and no more,
    // so leaving `C` out is as much an error as it was when each variant had
    // its own arm.
    let (sources, checked) = check_source(
        "module a\n\n\
         choice E { A, B, C }\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   A | B => 1,\n\
         \x20 }\n\
         }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::NON_EXHAUSTIVE_MATCH]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("cover `C`"));
}

#[test]
fn a_wildcard_inside_an_alternative_is_still_a_catch_all() {
    // `A | _` reaches every variant, and standing next to a name it is easier
    // to miss than a wildcard on its own, which is the reason to check it.
    let (_, checked) = check_source(
        "module a\n\n\
         choice E { A, B, C }\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   A | _ => 1,\n\
         \x20 }\n\
         }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![codes::CATCH_ALL_ON_CHOICE]
    );
}

#[test]
fn a_variant_with_fields_can_be_named_among_alternatives() {
    // Matched by name, without the braces. This is what makes binding nothing
    // cost nothing: the arms that wanted this were binding fields they never
    // read.
    check_ok(
        "module a\n\n\
         choice E { A { n: Int }, B, C }\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   A | B => 1,\n\
         \x20   C => 2,\n\
         \x20 }\n\
         }\n",
    );
}

#[test]
fn alternatives_work_across_a_module_boundary() {
    // A second exhaustiveness walk, matching by name rather than by
    // definition, and it had to learn the same thing.
    check_ok_in(
        "module a\n\n\
         use other.{E, A, B, C}\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   A | B => 1,\n\
         \x20   C => 2,\n\
         \x20 }\n\
         }\n",
        &universe_of(&["module other\n\nchoice E { A, B, C }\n"]),
    );
}

#[test]
fn all_arms_must_agree() {
    let (_, checked) = check_source(
        "module a\n\nchoice E { A, B }\n\nfn f(e: E) -> Int {\n  match e {\n    A => 1,\n    B => true,\n  }\n}\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn a_variant_pattern_binds_its_fields_with_types() {
    check_ok(
        "module a\n\n\
         choice E { Full { n: Int }, Bare }\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   Full { n } => n + 1,\n\
         \x20   Bare => 0,\n\
         \x20 }\n\
         }\n",
    );
}

// -- calls and returns -----------------------------------------------------

#[test]
fn a_wrong_argument_points_at_the_parameter() {
    let (sources, checked) =
        check_source("module a\n\nfn take(n: Int) -> Int { n }\n\nfn f() -> Int { take(true) }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("expected `Int`, found `Bool`"), "{text}");
    assert!(text.contains("the parameter it is passed to"), "{text}");
}

#[test]
fn the_wrong_number_of_arguments_points_at_the_declaration() {
    let (sources, checked) = check_source(
        "module a\n\nfn take(n: Int, m: Int) -> Int { n }\n\nfn f() -> Int { take(1) }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::WRONG_ARITY]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("takes 2 arguments, but 1 was given"),
        "{text}"
    );
    assert!(text.contains("declared here"), "{text}");
}

#[test]
fn calling_something_that_is_not_a_function_is_reported() {
    let (_, checked) = check_source("module a\n\nfn f(n: Int) -> Int { n(1) }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_CALLABLE]);
}

#[test]
fn a_body_must_produce_the_declared_return_type() {
    let (sources, checked) = check_source("module a\n\nfn f() -> Int { true }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
    assert!(rendered(&sources, &checked.diagnostics).contains("the declared return type"));
}

#[test]
fn a_body_ending_in_return_is_accepted() {
    check_ok("module a\n\nfn f(n: Int) -> Int {\n  return n\n}\n");
}

#[test]
fn a_return_is_checked_against_the_signature() {
    let (_, checked) = check_source("module a\n\nfn f() -> Int {\n  return true\n}\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn an_if_without_an_else_must_produce_nothing() {
    check_ok("module a\n\nfn f(b: Bool, n: Int) -> Int {\n  if b {\n    return n\n  }\n  0\n}\n");
}

#[test]
fn both_branches_of_an_if_must_agree() {
    let (_, checked) =
        check_source("module a\n\nfn f(b: Bool) -> Int {\n  if b { 1 } else { true }\n}\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

// -- contracts -------------------------------------------------------------

#[test]
fn contract_clauses_must_be_conditions() {
    let (_, checked) = check_source("module a\n\nfn f(n: Int) -> Int\n  where n,\n{ n }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::TYPE_MISMATCH]);
}

#[test]
fn a_contract_may_talk_about_the_parameters() {
    check_ok(
        "module a\n\n\
         fn f(n: Int, m: Int) -> Int\n\
         \x20 where n > 0, m > 0,\n\
         \x20 ensures ok => old(n) == n,\n\
         { n + m }\n",
    );
}

#[test]
fn unchanged_is_a_condition() {
    check_ok(
        "module a\n\n\
         effect Ledger {\n  fn post(n: Int) -> Int\n}\n\n\
         fn f() -> Int\n\
         \x20 uses Ledger.post,\n\
         \x20 ensures err => unchanged(Ledger),\n\
         { 0 }\n",
    );
}

// -- values nobody reads ---------------------------------------------------
//
// A block's value is its tail, so every other expression in it is there for
// what it does. One that produces a value has nowhere to put it, and the two
// ways to get there are a result that was meant to be looked at and a line
// that was meant to belong to the one above it. The second one only became
// possible to tell apart when an expression started ending at the end of a
// line.

#[test]
fn a_statement_that_produces_a_value_says_nobody_reads_it() {
    let (sources, checked) = check_source(
        "module a\n\n\
         fn twice(n: Int) -> Int { n + n }\n\n\
         fn f(n: Int) -> Int {\n\
         \x20 twice(n)\n\
         \x20 n\n\
         }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::DISCARDED_VALUE]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("produces `Int` and nothing reads it"),
        "{text}"
    );
    assert!(text.contains("write `let _ = ...`"), "{text}");
}

/// The first fix the type checker has ever carried. A type that does not fit
/// has no obvious repair, which is why there were none; this one has exactly
/// one mechanical answer and it is still a guess, because the other way to
/// arrive here is a value that was supposed to be read.
#[test]
fn saying_you_meant_it_is_offered_as_a_fix_and_never_applied() {
    let source = "module a\n\n\
         fn twice(n: Int) -> Int { n + n }\n\n\
         fn f(n: Int) -> Int {\n\
         \x20 twice(n)\n\
         \x20 n\n\
         }\n";
    let (_, checked) = check_source(source);
    let fix = checked.diagnostics[0]
        .fix
        .as_ref()
        .expect("the warning should carry a fix");
    assert_eq!(
        fix.applicability,
        deed_diagnostics::Applicability::MaybeIncorrect
    );
    assert_eq!(fix.edits.len(), 1);
    assert_eq!(fix.edits[0].replacement, "let _ = ");
    // An insertion, so the span has no width and takes nothing out.
    assert_eq!(fix.edits[0].span.start, fix.edits[0].span.end);
    assert_eq!(
        fix.edits[0].span.start as usize,
        source.rfind("twice(n)").expect("the statement is in there")
    );
}

#[test]
fn a_line_that_was_meant_to_continue_the_one_above_says_so() {
    // The reason this is worth having. `let a = 1` with `- 2` under it is two
    // statements, which is the honest reading and still leaves a line doing
    // nothing. Saying so is the difference between a rule that is right and a
    // rule that helps.
    let (_, checked) = check_source(
        "module a\n\n\
         fn f(n: Int) -> Int {\n\
         \x20 let a = 1\n\
         \x20 - 2\n\
         \x20 a + n\n\
         }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::DISCARDED_VALUE]);
}

#[test]
fn dropping_a_result_says_the_failure_goes_with_it() {
    // Worth its own sentence. Dropping an `Int` wastes a line and dropping a
    // `Result` loses the failure, which is the thing the type was carrying.
    let (sources, checked) = check_source(
        "module a\n\n\
         fn boom(n: Int) -> Result<Int, String> { ok(n) }\n\n\
         fn f(n: Int) -> Int {\n\
         \x20 boom(n)\n\
         \x20 n\n\
         }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::DISCARDED_VALUE]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("the failure case goes with it"), "{text}");
}

#[test]
fn a_question_mark_is_a_statement_about_the_failure() {
    // `f()?` drops the success case and keeps the error, which is the one
    // worth not losing, so a statement written as `?` is saying what it means.
    check_ok(
        "module a\n\n\
         fn boom(n: Int) -> Result<Int, String> { ok(n) }\n\n\
         fn f(n: Int) -> Result<Int, String> {\n\
         \x20 boom(n)?\n\
         \x20 ok(n)\n\
         }\n",
    );
}

#[test]
fn a_statement_that_produces_nothing_is_the_ordinary_case() {
    check_ok(
        "module a\n\n\
         effect Log {\n  fn note(message: String) -> ()\n}\n\n\
         fn f(n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         {\n\
         \x20 Log.note(\"hi\")\n\
         \x20 n\n\
         }\n",
    );
}

#[test]
fn saying_you_meant_it_is_enough() {
    // A warning rather than an error, and `let _ = ...` is how a program says
    // it meant to throw the value away. Anything else would mean rewriting
    // working code to keep compiling.
    check_ok(
        "module a\n\n\
         fn twice(n: Int) -> Int { n + n }\n\n\
         fn f(n: Int) -> Int {\n\
         \x20 let _ = twice(n)\n\
         \x20 n\n\
         }\n",
    );
}

#[test]
fn an_expression_that_never_produced_a_value_is_not_discarded() {
    // `Never` is not a value nobody read, it is an expression that did not
    // come back, and `Unknown` is a type the checker does not have rather than
    // one it worked out.
    check_ok(
        "module a\n\n\
         fn f(n: Int) -> Result<Int, String> {\n\
         \x20 if n < 0 {\n\
         \x20   return err(\"no\")\n\
         \x20 }\n\
         \x20 ok(n)\n\
         }\n",
    );
}

// -- generics and robustness -----------------------------------------------

#[test]
fn a_type_declared_with_no_parameters_takes_none() {
    let (sources, checked) =
        check_source("module a\n\nrecord R { n: Int }\n\nfn f(x: R<Int>) -> Int { 0 }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_GENERIC]);
    assert!(
        rendered(&sources, &checked.diagnostics)
            .contains("`R` takes no type arguments, and 1 was given")
    );
}

#[test]
fn a_generic_type_takes_exactly_as_many_as_it_declared() {
    // In both directions. A signature is complete, so `Pair` written bare is
    // as much a hole in one as a parameter with no type, and filling it in
    // with unknowns would make every use of it agree with everything.
    let (sources, checked) = check_source(
        "module a\n\nrecord Pair<A, B> { left: A, right: B }\n\nfn f(x: Pair<Int>) -> Int { 0 }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_GENERIC]);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("`Pair` takes 2 type arguments, and 1 was given"),
        "{text}"
    );

    let (sources, checked) = check_source(
        "module a\n\nrecord Pair<A, B> { left: A, right: B }\n\nfn f(x: Pair) -> Int { 0 }\n",
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_GENERIC]);
    assert!(
        rendered(&sources, &checked.diagnostics).contains("and 0 were given"),
        "a bare generic type is a hole in a signature"
    );
}

#[test]
fn an_imported_type_is_checked_against_what_it_declared() {
    // This used to be accepted, and only because an imported name had no type,
    // so there was nothing for the arguments to be applied to. Which module a
    // type was declared in does not change how many arguments it takes.
    let (sources, checked) = check_source_in(
        "module a\n\nuse other.{Thing}\n\nfn f(x: Thing<Int>) -> Int { 0 }\n",
        &universe_of(&["module other\n\nrecord Thing { n: Int }\n"]),
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_GENERIC]);
    assert!(rendered(&sources, &checked.diagnostics).contains("`Thing` takes no type arguments"));
}

#[test]
fn an_imported_generic_type_takes_the_arguments_it_declared() {
    let (_, checked) = check_source_in(
        "module a\n\nuse other.{Box}\n\nfn f(x: Box<Int>) -> Int { 0 }\n",
        &universe_of(&["module other\n\nrecord Box<T> { value: T }\n"]),
    );
    assert_eq!(codes_of(&checked.diagnostics), Vec::<&str>::new());

    let (_, checked) = check_source_in(
        "module a\n\nuse other.{Box}\n\nfn f(x: Box) -> Int { 0 }\n",
        &universe_of(&["module other\n\nrecord Box<T> { value: T }\n"]),
    );
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_GENERIC]);
}

#[test]
fn a_value_used_as_a_type_is_reported() {
    let (_, checked) =
        check_source("module a\n\nfn thing() -> Int { 0 }\n\nfn f(x: thing) -> Int { 0 }\n");
    assert_eq!(codes_of(&checked.diagnostics), vec![codes::NOT_A_TYPE]);
}

#[test]
fn broken_input_does_not_panic_and_does_not_pile_on() {
    let mut sources = SourceMap::new();
    let file = sources.add(
        "broken.deed",
        "module a\n\nfn good() -> Int { 0 }\n\nfn bad(x: ) -> Int { , }\n\nfn worse() -> Nope { missing() }\n",
    );
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, &Universe::new());
    let checked = check(file, &parsed.module, &resolved.resolutions, &World::new());

    // Names that failed to resolve become Unknown, and Unknown agrees with
    // everything, so the type checker adds nothing to a mess it did not make.
    assert!(
        checked.diagnostics.is_empty(),
        "type checker piled on after earlier errors:\n{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn expression_types_are_recorded() {
    let src = "module a\n\nfn f() -> Int { 1 + 2 }\n";
    let types = check_ok(src);
    let start = src.find("1 + 2").unwrap() as u32;
    let span = deed_diagnostics::Span::new(start, start + 5);
    assert_eq!(types.type_of(span), Some(&Ty::Int));
}
