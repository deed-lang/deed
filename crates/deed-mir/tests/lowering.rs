//! Lowering, asked directly rather than through the whole pipeline.
//!
//! `crates/deed-driver/tests/agreement.rs` checks that a compiled program
//! answers what an interpreted one does, which is the question that matters
//! and the slowest one to ask. These are the smaller ones: what a type comes
//! out as, what a copy of a generic function is called, and which shapes are
//! refused by name.

use deed_diagnostics::SourceMap;
use deed_lexer::tokenize;
use deed_mir::{Ty, lower};
use deed_parser::parse;
use deed_resolve::{Universe, resolve};

/// Lowers a module, or hands back what stopped it.
fn lowered(source: &str) -> Result<deed_mir::Program, String> {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", source);

    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "this source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "this source should parse cleanly");
    let universe = Universe::default();
    let resolved = resolve(file, &parsed.module, &universe);
    let checked = deed_typeck::check(
        file,
        &parsed.module,
        &resolved.resolutions,
        &deed_typeck::World::default(),
    );

    lower(&parsed.module, &resolved.resolutions, &checked.types).map_err(|why| why.to_string())
}

#[test]
fn a_written_type_comes_out_as_what_it_names() {
    let program = lowered(
        "module a\n\nfn f(n: Int, yes: Bool, text: String, items: List<Int>) -> () { () }\n",
    )
    .expect("this lowers");

    let function = program.function(program.find("f").expect("f is there"));
    assert_eq!(
        function.params,
        vec![Ty::Int, Ty::Bool, Ty::Str, Ty::List(Box::new(Ty::Int))]
    );
    assert_eq!(function.ret, Ty::Unit);
}

/// A refinement is a claim about a value rather than a shape it has, so what
/// is left to lay out is the base type.
#[test]
fn a_refinement_lowers_to_what_it_is_written_over() {
    let program = lowered(
        "module a\n\ntype Positive = Int where value > 0\n\nfn f(n: Positive) -> Positive { n }\n",
    )
    .expect("this lowers");

    let function = program.function(program.find("f").expect("f is there"));
    assert_eq!(function.params, vec![Ty::Int]);
    assert_eq!(function.ret, Ty::Int);
}

/// A record becomes a layout with one variant, since there is nothing to
/// tell apart, and a choice becomes one per variant.
#[test]
fn a_record_has_one_variant_and_a_choice_has_one_each() {
    let program = lowered(
        "module a\n\nrecord Pair {\n    left: Int,\n    right: Int,\n}\n\nchoice Tone {\n    Plain,\n    Loud,\n}\n\nfn f(p: Pair, t: Tone) -> () { () }\n",
    )
    .expect("this lowers");

    let pair = program
        .layouts
        .iter()
        .find(|layout| layout.name == "Pair")
        .expect("Pair is there");
    assert_eq!(pair.variants.len(), 1);
    assert!(!pair.is_tagged(), "a record has nothing to tell apart");
    assert_eq!(pair.variants[0].fields.len(), 2);

    let tone = program
        .layouts
        .iter()
        .find(|layout| layout.name == "Tone")
        .expect("Tone is there");
    assert_eq!(tone.variants.len(), 2);
    assert!(tone.is_tagged(), "a choice has to say which one it holds");
}

/// One declaration, two element types, two copies.
///
/// The names carry what they were lowered for, so a reader of the module
/// can tell them apart and two cannot collide.
#[test]
fn a_generic_function_gets_one_copy_per_set_of_type_arguments() {
    let program = lowered(
        "module a\n\nfn count_of<T>(items: List<T>) -> Int { length(items) }\n\nfn f() -> Int { count_of([1]) + count_of([true]) }\n",
    )
    .expect("this lowers");

    let copies: Vec<&str> = program
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .filter(|name| name.starts_with("count_of<"))
        .collect();

    assert_eq!(copies.len(), 2, "two element types, two copies: {copies:?}");
    assert_ne!(copies[0], copies[1], "the copies should be told apart");
    // The declaration itself is never lowered, since it has no one shape.
    assert!(program.find("count_of").is_none());
}

/// The same type arguments twice is one copy, not two.
#[test]
fn calling_a_generic_function_twice_at_one_type_makes_one_copy() {
    let program = lowered(
        "module a\n\nfn count_of<T>(items: List<T>) -> Int { length(items) }\n\nfn f() -> Int { count_of([1]) + count_of([2, 3]) }\n",
    )
    .expect("this lowers");

    let copies = program
        .functions
        .iter()
        .filter(|function| function.name.starts_with("count_of<"))
        .count();
    assert_eq!(copies, 1, "one element type, one copy");
}

/// A closure becomes a function of its own, and the value that points at it
/// carries what the body reads from outside.
#[test]
fn a_closure_becomes_a_function_and_an_environment() {
    let program = lowered(
        "module a\n\nfn apply(f: Fn(Int) -> Int, n: Int) -> Int { f(n) }\n\nfn g() -> Int {\n    let by = 3\n    apply(|n: Int| n * by, 4)\n}\n",
    )
    .expect("this lowers");

    let lifted = program
        .functions
        .iter()
        .filter(|function| function.name.starts_with("closure"))
        .count();
    assert_eq!(lifted, 1, "one closure written, one function lifted");

    let environment = program
        .layouts
        .iter()
        .find(|layout| layout.name.starts_with("closure at"))
        .expect("a closure gets a layout for what it holds");
    // The code pointer, then the one name the body reads from outside.
    assert_eq!(environment.variants[0].fields.len(), 2);
    assert_eq!(environment.variants[0].fields[0].name, "code");
}

/// A closure that reads nothing carries nothing but its code pointer.
#[test]
fn a_closure_that_captures_nothing_holds_only_its_code() {
    let program = lowered(
        "module a\n\nfn apply(f: Fn(Int) -> Int, n: Int) -> Int { f(n) }\n\nfn g() -> Int { apply(|n: Int| n + 1, 4) }\n",
    )
    .expect("this lowers");

    let environment = program
        .layouts
        .iter()
        .find(|layout| layout.name.starts_with("closure at"))
        .expect("a closure gets a layout");
    assert_eq!(environment.variants[0].fields.len(), 1);
}

/// A handler is one set of bodies however many blocks install it, and each
/// operation is a function taking the state cell first.
#[test]
fn a_handler_becomes_one_function_per_operation() {
    let program = lowered(
        "module a\n\neffect Counter {\n    fn value() -> Int\n    fn bump(by: Int) -> ()\n}\n\nhandler InMemory implements Counter {\n    state count: Int\n\n    fn value() -> Int { count }\n\n    fn bump(by) -> () {\n        count = count + by\n    }\n}\n\nfn f() -> Int {\n    with InMemory { count: 0 } {\n        Counter.bump(1)\n        Counter.value()\n    }\n}\n\nfn g() -> Int {\n    with InMemory { count: 5 } {\n        Counter.value()\n    }\n}\n",
    )
    .expect("this lowers");

    assert_eq!(program.effects.len(), 1);
    assert_eq!(program.effect(deed_mir::EffectId(0)).name, "Counter");
    assert_eq!(
        program.effect(deed_mir::EffectId(0)).operations,
        vec!["value".to_string(), "bump".to_string()]
    );

    let value = program.find("InMemory.value").expect("value is lowered");
    let bump = program.find("InMemory.bump").expect("bump is lowered");
    // Two installations, one set of bodies.
    assert_eq!(
        program
            .functions
            .iter()
            .filter(|function| function.name.starts_with("InMemory."))
            .count(),
        2
    );

    // The state cell first, then what the effect declared.
    let state = program.function(value).params[0].clone();
    assert!(matches!(state, Ty::Aggregate(_)));
    assert_eq!(program.function(bump).params, vec![state, Ty::Int]);
    assert_eq!(program.function(bump).ret, Ty::Unit);
}

/// Two effects in one module get told apart, and by position.
///
/// A `Perform` names its effect by number, so a program with two of them is
/// what says the numbering is real.
#[test]
fn each_effect_gets_its_own_number() {
    let program = lowered(
        "module a\n\neffect Counter {\n    fn value() -> Int\n}\n\neffect Clock {\n    fn now() -> Int\n}\n\nhandler Fixed implements Counter {\n    state count: Int\n\n    fn value() -> Int { count }\n}\n\nhandler Frozen implements Clock {\n    state at: Int\n\n    fn now() -> Int { at }\n}\n\nfn f() -> Int {\n    with Fixed { count: 1 } {\n        with Frozen { at: 2 } {\n            Counter.value() + Clock.now()\n        }\n    }\n}\n",
    )
    .expect("this lowers");

    assert_eq!(program.effects.len(), 2);
    assert_eq!(program.effect(deed_mir::EffectId(0)).name, "Counter");
    assert_eq!(program.effect(deed_mir::EffectId(1)).name, "Clock");
}

/// Each installation carries the function that answers each operation, and
/// they are in the order the effect declared them.
///
/// The frame holds these as code pointers, so an off-by-one here is a
/// program that calls the wrong body.
#[test]
fn an_installation_points_at_the_right_body_for_each_operation() {
    let program = lowered(
        "module a\n\neffect Counter {\n    fn value() -> Int\n    fn bump(by: Int) -> ()\n}\n\nhandler InMemory implements Counter {\n    state count: Int\n\n    fn value() -> Int { count }\n\n    fn bump(by) -> () {\n        count = count + by\n    }\n}\n\nfn f() -> Int {\n    with InMemory { count: 0 } {\n        Counter.bump(1)\n        Counter.value()\n    }\n}\n",
    )
    .expect("this lowers");

    let f = program.function(program.find("f").expect("f is there"));
    let deed_mir::Expr::Install { operations, .. } = &f.body.value else {
        panic!("`f` should be an installation, not {:?}", f.body.value);
    };

    let named: Vec<&str> = operations
        .iter()
        .map(|id| program.function(*id).name.as_str())
        .collect();
    assert_eq!(named, vec!["InMemory.value", "InMemory.bump"]);
}

/// `Result` is a choice nobody wrote down, so the shape is built when a type
/// names one and found the second time.
#[test]
fn each_pair_of_types_a_result_holds_gets_its_own_layout() {
    let program = lowered(
        "module a\n\nfn number() -> Result<Int, String> { ok(1) }\n\nfn again() -> Result<Int, String> { ok(2) }\n\nfn text() -> Result<String, String> { ok(\"x\") }\n",
    )
    .expect("this lowers");

    let results: Vec<&str> = program
        .layouts
        .iter()
        .map(|layout| layout.name.as_str())
        .filter(|name| name.starts_with("Result<"))
        .collect();

    // Two pairs, two layouts. Two functions at the same pair share one.
    assert_eq!(results.len(), 2, "{results:?}");
    assert_ne!(results[0], results[1]);

    let held = program
        .layouts
        .iter()
        .find(|layout| layout.name.starts_with("Result<"))
        .expect("there is one");
    assert!(held.is_tagged(), "a Result has to say which half it holds");
    assert_eq!(held.variants[0].name, "ok");
    assert_eq!(held.variants[1].name, "err");
}

/// What is not lowered yet is named rather than approximated.
#[test]
fn a_shape_that_is_not_lowered_says_which_one() {
    let why = lowered("module a\n\nfn f() -> () {\n    assert refuses 1\n}\n")
        .expect_err("`assert refuses` is not lowered yet");
    assert!(why.contains("not lowered yet"), "{why}");
}
