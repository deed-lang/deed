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

/// What is not lowered yet is named rather than approximated.
#[test]
fn a_shape_that_is_not_lowered_says_which_one() {
    let why = lowered("module a\n\neffect Log {\n    fn note(text: String)\n}\n\nhandler Quiet implements Log {\n    fn note(text: String) -> () { () }\n}\n\nfn f() -> Int uses Log.note {\n    Log.note(\"hi\")\n    1\n}\n")
        .expect_err("effects are not lowered yet");
    assert!(why.contains("not lowered yet"), "{why}");
}
