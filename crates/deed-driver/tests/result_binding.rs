//! `result`, the value an obligation is about.

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_driver::check_text;
use deed_interp::run_tests;
use deed_lexer::tokenize;
use deed_parser::parse;
use deed_resolve::{Universe, resolve};

fn check(src: &str) -> (SourceMap, deed_driver::Checked) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.deed", src);
    (sources, checked)
}

fn check_ok(src: &str) {
    let (sources, checked) = check(src);
    if !checked.diagnostics.is_empty() {
        panic!(
            "expected a clean check:\n{}",
            rendered(&sources, &checked.diagnostics)
        );
    }
}

/// Runs the tests in a source that is expected to check cleanly.
fn run(src: &str) -> (SourceMap, Vec<deed_interp::TestOutcome>) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, &Universe::new());
    assert!(!resolved.has_errors(), "source should resolve cleanly");
    let mut program = deed_interp::Program::new();
    // Nothing here is refined, so there is nothing for the checker to have
    // given up on and nothing to guard.
    program.add(
        file,
        &parsed.module,
        &resolved.resolutions,
        deed_interp::Guards::new(),
        deed_interp::DeclaredRows::new(),
    );
    let outcomes = run_tests(&program, file);
    (sources, outcomes)
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

// -- typing ----------------------------------------------------------------

#[test]
fn a_pure_function_can_state_what_it_returns() {
    check_ok(
        "module a\n\n\
         fn double(n: Int) -> Int\n\
         \x20 ensures ok => result == n + n,\n\
         { n + n }\n",
    );
}

#[test]
fn the_ok_clause_sees_the_success_value_of_a_result() {
    check_ok(
        "module a\n\n\
         choice Failure { Empty }\n\n\
         fn one() -> Result<Int, Failure>\n\
         \x20 ensures ok => result > 0,\n\
         { ok(1) }\n",
    );
}

#[test]
fn the_err_clause_sees_the_error_value() {
    check_ok(
        "module a\n\n\
         record Problem { code: Int }\n\n\
         fn always() -> Result<Int, Problem>\n\
         \x20 ensures err => result.code == 7,\n\
         { err(Problem { code: 7 }) }\n",
    );
}

#[test]
fn the_two_clauses_do_not_see_the_same_thing() {
    // `result` in an `err` clause is the error, so treating it as the success
    // value is a type error rather than something noticed at 3am.
    let (sources, checked) = check(
        "module a\n\n\
         record Problem { code: Int }\n\n\
         fn always() -> Result<Int, Problem>\n\
         \x20 ensures err => result > 0,\n\
         { err(Problem { code: 7 }) }\n",
    );
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_typeck::codes::TYPE_MISMATCH]
    );
    assert!(rendered(&sources, &checked.diagnostics).contains("`Problem`"));
}

#[test]
fn using_result_outside_an_ensures_is_an_unknown_name() {
    let (_, checked) = check("module a\n\nfn f() -> Int { result }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_resolve::codes::UNKNOWN_NAME]
    );
}

#[test]
fn a_where_clause_cannot_see_it_either() {
    // A precondition is about the inputs. There is no result yet.
    let (_, checked) = check("module a\n\nfn f(n: Int) -> Int\n  where result > 0,\n{ n }\n");
    assert_eq!(
        codes_of(&checked.diagnostics),
        vec![deed_resolve::codes::UNKNOWN_NAME]
    );
}

// -- running ---------------------------------------------------------------

#[test]
fn an_obligation_about_the_return_value_is_checked() {
    let (sources, outcomes) = run("module a\n\n\
         fn wrong(n: Int) -> Int\n\
         \x20 ensures ok => result == n + n,\n\
         { n + 1 }\n\n\
         test \"catches it\" {\n\
         \x20 assert wrong(5) == 6\n\
         }\n");

    let failure = outcomes[0]
        .failure
        .as_ref()
        .expect("the postcondition should have caught this");
    assert_eq!(failure.code, deed_interp::codes::POSTCONDITION_FAILED);
    assert!(render_human(&sources, failure).contains("`wrong` did not keep this promise"));
}

#[test]
fn an_obligation_that_holds_passes_quietly() {
    let (_, outcomes) = run("module a\n\n\
         fn double(n: Int) -> Int\n\
         \x20 ensures ok => result == n + n,\n\
         { n + n }\n\n\
         test \"fine\" {\n\
         \x20 assert double(5) == 10\n\
         }\n");
    assert!(outcomes[0].failure.is_none());
}

#[test]
fn the_err_clause_sees_the_error_at_runtime() {
    let (sources, outcomes) = run("module a\n\n\
         record Problem { code: Int }\n\n\
         fn always() -> Result<Int, Problem>\n\
         \x20 ensures err => result.code == 1,\n\
         { err(Problem { code: 7 }) }\n\n\
         test \"wrong code\" {\n\
         \x20 assert always() == err(Problem { code: 7 })\n\
         }\n");

    let failure = outcomes[0]
        .failure
        .as_ref()
        .expect("the err obligation should have caught this");
    assert_eq!(failure.code, deed_interp::codes::POSTCONDITION_FAILED);
    let _ = render_human(&sources, failure);
}

#[test]
fn the_ok_clause_is_not_checked_on_the_failing_path() {
    // Otherwise a function that fails would be judged by a promise it never
    // made.
    let (_, outcomes) = run("module a\n\n\
         record Problem { code: Int }\n\n\
         fn always() -> Result<Int, Problem>\n\
         \x20 ensures ok => result > 1000,\n\
         { err(Problem { code: 7 }) }\n\n\
         test \"only the err path runs\" {\n\
         \x20 assert always() == err(Problem { code: 7 })\n\
         }\n");
    assert!(outcomes[0].failure.is_none());
}
