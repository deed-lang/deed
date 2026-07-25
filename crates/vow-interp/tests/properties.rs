//! Property tests generated from contracts.

use vow_diagnostics::{SourceMap, render_human};
use vow_interp::{Program, PropertyConfig, PropertyOutcome, codes, run_properties};
use vow_lexer::tokenize;
use vow_parser::parse;
use vow_resolve::{Universe, resolve};

fn properties(src: &str, config: PropertyConfig) -> (SourceMap, Vec<PropertyOutcome>) {
    properties_in(src, config, &Universe::new())
}

fn properties_in(
    src: &str,
    config: PropertyConfig,
    universe: &Universe,
) -> (SourceMap, Vec<PropertyOutcome>) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.vow", src);

    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "source should parse cleanly");
    let resolved = resolve(file, &parsed.module, universe);
    assert!(!resolved.has_errors(), "source should resolve cleanly");

    let mut program = Program::new();
    program.add(file, &parsed.module, &resolved.resolutions);
    let outcomes = run_properties(
        &program,
        file,
        &parsed.module,
        &resolved.resolutions,
        config,
    );
    (sources, outcomes)
}

/// A universe holding each of `modules`, parsed from source.
///
/// An import with nothing behind it is an error now, so a test about a name
/// from elsewhere needs a real module declaring it.
fn universe_of(modules: &[&str]) -> Universe {
    let mut universe = Universe::new();
    let mut sources = SourceMap::new();
    for (index, source) in modules.iter().enumerate() {
        let file = sources.add(format!("dep{index}.vow"), *source);
        let lexed = tokenize(file, sources.file(file).text());
        let parsed = parse(file, &lexed.tokens);
        universe.add(&parsed.module);
    }
    universe
}

fn only(src: &str) -> (SourceMap, PropertyOutcome) {
    let (sources, mut outcomes) = properties(src, PropertyConfig::default());
    assert_eq!(outcomes.len(), 1, "expected exactly one property");
    (sources, outcomes.remove(0))
}

// -- which functions get one -----------------------------------------------

#[test]
fn a_pure_function_with_a_postcondition_gets_a_property() {
    let (_, outcome) = only(
        "module a\n\n\
         fn identity(n: Int) -> Int\n\
         \x20 ensures ok => result == n,\n\
         { n }\n",
    );
    assert!(outcome.passed());
    assert_eq!(outcome.cases, 100);
    assert_eq!(outcome.function, "identity");
}

#[test]
fn it_finds_overflow_in_a_function_that_looks_obviously_correct() {
    // The first property I wrote was `double`, with the postcondition
    // `result == n + n`, and it failed. `Int` does not wrap, so doubling
    // overflows for large inputs and the function has no precondition saying
    // otherwise. The generated inputs found that on the first run.
    let (sources, outcome) = only(
        "module a\n\n\
         fn double(n: Int) -> Int\n\
         \x20 ensures ok => result == n + n,\n\
         { n + n }\n",
    );

    let failure = outcome.failure.as_ref().expect("doubling overflows");
    assert_eq!(failure.code, codes::ARITHMETIC);

    // And the shrinker finds the boundary rather than reporting whichever
    // enormous number happened to come out of the generator.
    let text = render_human(&sources, failure);
    assert!(
        text.contains("generated input: n = 4611686018427387904"),
        "expected the smallest overflowing input, got:\n{text}"
    );
}

#[test]
fn constraining_the_input_makes_the_same_function_pass() {
    let (_, outcome) = only(
        "module a\n\n\
         fn double(n: Int) -> Int\n\
         \x20 where n < 1000000, n > 0 - 1000000,\n\
         \x20 ensures ok => result == n + n,\n\
         { n + n }\n",
    );
    assert!(outcome.passed(), "{:?}", outcome.failure.map(|f| f.message));
}

#[test]
fn a_function_with_no_obligations_gets_nothing() {
    let (_, outcomes) = properties(
        "module a\n\nfn double(n: Int) -> Int { n + n }\n",
        PropertyConfig::default(),
    );
    assert!(outcomes.is_empty());
}

#[test]
fn an_effectful_function_gets_nothing() {
    // Running one needs a handler, and inventing a handler means inventing the
    // behaviour the property would then be checking against itself.
    let (_, outcomes) = properties(
        "module a\n\n\
         effect Clock {\n  fn now() -> Int\n}\n\n\
         fn stamp(n: Int) -> Int\n\
         \x20 uses Clock.now,\n\
         \x20 ensures ok => result > 0,\n\
         { Clock.now() + n }\n",
        PropertyConfig::default(),
    );
    assert!(outcomes.is_empty());
}

#[test]
fn a_parameter_the_generator_cannot_produce_stops_it() {
    let (_, outcomes) = properties_in(
        "module a\n\n\
         use other.{Thing}\n\n\
         fn f(t: Thing) -> Int\n\
         \x20 ensures ok => result == 0,\n\
         { 0 }\n",
        PropertyConfig::default(),
        &universe_of(&["module other\n\nrecord Thing { n: Int }\n"]),
    );
    assert!(outcomes.is_empty());
}

// -- catching a wrong contract ---------------------------------------------

#[test]
fn a_postcondition_that_does_not_hold_is_caught_by_generated_inputs() {
    let (sources, outcome) = only(
        "module a\n\n\
         fn double(n: Int) -> Int\n\
         \x20 ensures ok => result == n + n + 1,\n\
         { n + n }\n",
    );

    let failure = outcome.failure.as_ref().expect("should have been caught");
    assert_eq!(failure.code, codes::POSTCONDITION_FAILED);

    let text = render_human(&sources, failure);
    assert!(text.contains("generated input:"), "{text}");
    assert!(text.contains("so this reproduces"), "{text}");
}

#[test]
fn the_counterexample_is_shrunk_to_something_readable() {
    // Fails for n >= 50. A counterexample of 8419283 is a counterexample
    // nobody reads.
    let (sources, outcome) = only(
        "module a\n\n\
         fn small_double(n: Int) -> Int\n\
         \x20 where n > 0,\n\
         \x20 ensures ok => result < 100,\n\
         { n + n }\n",
    );

    let failure = outcome.failure.as_ref().expect("should have been caught");
    let text = render_human(&sources, failure);
    assert!(
        text.contains("generated input: n = 50"),
        "expected the smallest failing input, got:\n{text}"
    );
}

#[test]
fn running_twice_gives_the_same_counterexample() {
    let source = "module a\n\n\
                  fn f(n: Int, m: Int) -> Int\n\
                  \x20 ensures ok => result == n,\n\
                  { n + m }\n";

    let (first_sources, first) = only(source);
    let (second_sources, second) = only(source);

    let one = render_human(&first_sources, first.failure.as_ref().unwrap());
    let other = render_human(&second_sources, second.failure.as_ref().unwrap());
    assert_eq!(one, other);
}

#[test]
fn a_different_seed_is_still_a_failure() {
    let config = PropertyConfig {
        seed: 999,
        ..PropertyConfig::default()
    };
    let (_, outcomes) = properties(
        "module a\n\n\
         fn double(n: Int) -> Int\n\
         \x20 ensures ok => result == n + n + 1,\n\
         { n + n }\n",
        config,
    );
    assert!(!outcomes[0].passed());
    assert_eq!(outcomes[0].seed, 999);
}

// -- preconditions ---------------------------------------------------------

#[test]
fn inputs_violating_a_precondition_are_discarded_not_reported() {
    // The runtime already blames the caller for a `where` violation, and a
    // generator producing one is a bad caller. Reusing that code means there
    // is no second precondition check to disagree with the first.
    let (_, outcome) = only(
        "module a\n\n\
         fn halve(n: Int) -> Int\n\
         \x20 where n > 0,\n\
         \x20 ensures ok => result == n / 2,\n\
         { n / 2 }\n",
    );
    assert!(outcome.passed(), "{:?}", outcome.failure.map(|f| f.message));
    assert_eq!(outcome.cases, 100);
    assert!(
        outcome.rejected > 0,
        "roughly half of the inputs should be discarded"
    );
}

#[test]
fn a_precondition_nothing_can_satisfy_is_reported_rather_than_passed() {
    // A property that only tested a handful of inputs is worse than no
    // property, because it looks like one.
    let (sources, outcome) = only(
        "module a\n\n\
         fn impossible(n: Int) -> Int\n\
         \x20 where n > 0, n < 0,\n\
         \x20 ensures ok => result == n,\n\
         { n }\n",
    );

    let failure = outcome.failure.as_ref().expect("should not look green");
    assert_eq!(failure.code, codes::NOT_ENOUGH_CASES);
    assert_eq!(outcome.cases, 0);
    assert!(render_human(&sources, failure).contains("worse than no property"));
}

// -- generation ------------------------------------------------------------

#[test]
fn refined_parameters_only_ever_get_values_that_satisfy_them() {
    // If the generator produced a zero here, the refinement check inside the
    // call would fail and the property would report a failure that is really
    // the generator's fault.
    let (_, outcome) = only(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn identity(n: Positive) -> Int\n\
         \x20 ensures ok => result > 0,\n\
         { n }\n",
    );
    assert!(outcome.passed(), "{:?}", outcome.failure.map(|f| f.message));
    assert_eq!(outcome.cases, 100);
}

#[test]
fn records_and_choices_are_generated() {
    let (_, outcome) = only(
        "module a\n\n\
         choice Colour { Red, Green }\n\n\
         record Point { x: Int, y: Int }\n\n\
         fn describe(p: Point, c: Colour) -> Int\n\
         \x20 ensures ok => result == p.x + p.y,\n\
         { p.x + p.y }\n",
    );
    assert!(outcome.passed(), "{:?}", outcome.failure.map(|f| f.message));
}

#[test]
fn results_are_generated_on_both_sides() {
    let (_, outcome) = only(
        "module a\n\n\
         choice Failure { Empty }\n\n\
         fn passthrough(r: Result<Int, Failure>) -> Int\n\
         \x20 ensures ok => result == 0 || result == 1,\n\
         {\n\
         \x20 match r {\n\
         \x20   ok(n) => 0,\n\
         \x20   err(e) => 1,\n\
         \x20 }\n\
         }\n",
    );
    assert!(outcome.passed(), "{:?}", outcome.failure.map(|f| f.message));
}

#[test]
fn a_refinement_the_generator_cannot_satisfy_is_not_a_false_failure() {
    // Nothing satisfies this, so the generator gives up and the property
    // reports that it could not find inputs rather than inventing one.
    let (_, outcome) = only(
        "module a\n\n\
         type Impossible = Int where value > 0 && value < 0\n\n\
         fn f(n: Impossible) -> Int\n\
         \x20 ensures ok => result == n,\n\
         { n }\n",
    );
    assert_eq!(outcome.cases, 0);
    assert_eq!(
        outcome.failure.as_ref().map(|f| f.code),
        Some(codes::NOT_ENOUGH_CASES)
    );
}

// -- the examples ----------------------------------------------------------

#[test]
fn the_counter_example_has_a_property_and_it_passes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/counter.vow");
    let source = std::fs::read_to_string(path).expect("examples/counter.vow should exist");

    let mut sources = SourceMap::new();
    let file = sources.add("examples/counter.vow", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, &Universe::new());

    let outcomes = run_properties(
        &{
            let mut program = Program::new();
            program.add(file, &parsed.module, &resolved.resolutions);
            program
        },
        file,
        &parsed.module,
        &resolved.resolutions,
        PropertyConfig::default(),
    );

    assert!(
        !outcomes.is_empty(),
        "the example should have at least one property-testable function"
    );
    for outcome in &outcomes {
        if let Some(failure) = &outcome.failure {
            panic!(
                "property for `{}` should pass:\n{}",
                outcome.function,
                render_human(&sources, failure)
            );
        }
    }
}
