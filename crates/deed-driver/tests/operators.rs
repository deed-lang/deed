//! Operators a module gave a meaning to.
//!
//! `design/decisions/2026-08-03-operators-bound-to-functions.md` is the
//! argument. What is held here is the shape of the thing and every refusal
//! that keeps it that shape, because a refusal nothing checks is a refusal
//! that goes away by accident.
//!
//! The two engines are held to the same answer. This repository's recurring
//! mistake is two consumers of one idea drifting apart, and an operator is a
//! call the checker resolved rather than one written down, so neither engine
//! can work it out on its own.

use deed_diagnostics::SourceMap;
use deed_driver::{Checked, check_all, shipped_for, shipped_source};
use deed_interp::{Program, run_tests};

/// Checks a program with whatever it imports from the shipped library.
fn checked(source: &str) -> (SourceMap, Vec<Checked>, deed_diagnostics::FileId) {
    let mut sources = SourceMap::new();
    let subject = sources.add("probe.deed".to_string(), source.to_string());
    let mut ids = vec![subject];

    for module in shipped_for([source]) {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("{module}.deed"), text.to_string()));
    }

    let checks = check_all(&sources, &ids);
    (sources, checks, subject)
}

/// The codes a program is refused with.
fn refused(source: &str) -> Vec<String> {
    let (_, checks, _) = checked(source);
    checks
        .iter()
        .flat_map(|one| one.diagnostics.iter())
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| diagnostic.code.to_string())
        .collect()
}

/// Runs a program's tests in the interpreter, insisting it checks first.
fn interpreted(source: &str) -> Vec<String> {
    let (sources, checks, subject) = checked(source);
    for one in &checks {
        if let Some(diagnostic) = one.diagnostics.iter().find(|d| d.is_error()) {
            panic!(
                "the probe should check cleanly:\n{}",
                deed_diagnostics::render_human(&sources, diagnostic)
            );
        }
    }

    let mut program = Program::new();
    for one in &checks {
        program.add(
            one.file,
            &one.module,
            &one.resolutions,
            one.guards(),
            one.rows(),
            one.operators(),
        );
    }

    let outcomes = run_tests(&program, subject);
    assert!(!outcomes.is_empty(), "the probe declared no tests");
    outcomes
        .into_iter()
        .filter(|outcome| outcome.failure.is_some())
        .map(|outcome| outcome.name)
        .collect()
}

/// Runs the same program's first test through the compiled backend.
///
/// `None` when the backend could not lower or compile it, which is a different
/// answer from a test that failed and is reported as one.
fn compiled(source: &str) -> Option<Result<(), String>> {
    let (_, checks, _) = checked(source);
    let subject = &checks[0];
    let alongside: Vec<deed_mir::Alongside<'_>> = checks[1..]
        .iter()
        .map(|one| deed_mir::Alongside {
            module: &one.module,
            resolutions: &one.resolutions,
            types: &one.types,
        })
        .collect();

    let lowered = deed_mir::lower_with_tests_alongside(
        &subject.module,
        &subject.resolutions,
        &subject.types,
        &alongside,
    )
    .ok()?;
    let module = deed_codegen::compile(&lowered).ok()?;
    let test = lowered.tests.first()?;
    Some(
        deed_codegen::call(&module, &test.body, &[])
            .map(|_| ())
            .map_err(|trap| trap.to_string()),
    )
}

const MONEY: &str = "module probe\n\n\
     record Money {\n\
     \x20   cents: Int,\n\
     }\n\n\
     operator + = added\n\n\
     operator - = subtracted\n\n\
     operator * = scaled\n\n\
     fn added(left: Money, right: Money) -> Money {\n\
     \x20   Money { cents: left.cents + right.cents }\n\
     }\n\n\
     fn subtracted(left: Money, right: Money) -> Money {\n\
     \x20   Money { cents: left.cents - right.cents }\n\
     }\n\n\
     fn scaled(left: Money, right: Money) -> Money {\n\
     \x20   Money { cents: left.cents * right.cents }\n\
     }\n\n\
     test \"arithmetic\" {\n\
     \x20   let a = Money { cents: 150 }\n\
     \x20   let b = Money { cents: 275 }\n\
     \x20   let c = Money { cents: 2 }\n\
     \x20   assert a + b == Money { cents: 425 }\n\
     \x20   assert a + b - a == b\n\
     \x20   assert a * c == Money { cents: 300 }\n\
     \x20   assert 1 + 2 == 3\n\
     }\n";

/// A bound operator is the function it was bound to, and `Int` still adds.
///
/// The last assertion is the one that would go quiet first. The lookup is by
/// the type of the operands, so a mistake that made every `+` go through the
/// table would still pass everything above it.
#[test]
fn an_operator_means_the_function_it_was_bound_to() {
    assert_eq!(interpreted(MONEY), Vec::<String>::new());
}

/// And the compiled program agrees.
#[test]
fn the_backend_gives_the_same_answer_as_the_interpreter() {
    match compiled(MONEY) {
        Some(Ok(())) => {}
        Some(Err(trap)) => panic!("the compiled program stopped: {trap}"),
        None => panic!("the backend refused a program the interpreter runs"),
    }
}

/// The binding travels with the type.
///
/// This is what the feature is for. A program that imports `Ratio` writes
/// `half + third` without importing `added`, because the meaning of `+` on a
/// type is part of the type rather than part of the file that declared it.
#[test]
fn an_operator_crosses_the_module_boundary_with_its_type() {
    let source = "module probe\n\n\
         use std/ratio.{Ratio, simplified}\n\n\
         fn half() -> Ratio {\n\
         \x20   simplified(1, 2)\n\
         }\n\n\
         fn third() -> Ratio {\n\
         \x20   simplified(1, 3)\n\
         }\n\n\
         test \"a sum of fractions\" {\n\
         \x20   assert half() + third() == simplified(5, 6)\n\
         \x20   assert half() - third() == simplified(1, 6)\n\
         \x20   assert half() * third() == simplified(1, 6)\n\
         }\n";
    assert_eq!(interpreted(source), Vec::<String>::new());
}

/// The function keeps its name, which is the reason this is a binding.
#[test]
fn the_function_an_operator_means_is_still_a_function() {
    let source = "module probe\n\n\
         use std/list.{fold}\n\
         use std/ratio.{Ratio, added, simplified, zero}\n\n\
         test \"the bound function is still a value\" {\n\
         \x20   let parts = [simplified(1, 2), simplified(1, 3)]\n\
         \x20   let total = fold(parts, zero(), |sum: Ratio, one: Ratio| added(sum, one))\n\
         \x20   assert total == simplified(5, 6)\n\
         }\n";
    assert_eq!(interpreted(source), Vec::<String>::new());
}

/// An operator that performs something is refused.
///
/// A contract clause can reach an operator, and a clause that performs
/// something is not a question about values.
#[test]
fn an_operator_that_performs_something_is_refused() {
    let codes = refused(
        "module probe\n\n\
         record Money {\n\
         \x20   cents: Int,\n\
         }\n\n\
         effect Log {\n\
         \x20   fn note(line: String) -> ()\n\
         }\n\n\
         operator + = loud\n\n\
         fn loud(left: Money, right: Money) -> Money\n\
         \x20 uses\n\
         \x20   Log.note,\n\
         {\n\
         \x20   Log.note(\"adding\")\n\
         \x20   Money { cents: left.cents + right.cents }\n\
         }\n",
    );
    assert!(
        codes.contains(&"DEED4031".to_string()),
        "an operator performed something: {codes:?}"
    );
}

/// An operator takes two of one type and answers with that type.
#[test]
fn an_operator_that_mixes_types_is_refused() {
    let codes = refused(
        "module probe\n\n\
         record Money {\n\
         \x20   cents: Int,\n\
         }\n\n\
         operator + = wider\n\n\
         fn wider(left: Money, right: Int) -> Money {\n\
         \x20   Money { cents: left.cents + right }\n\
         }\n",
    );
    assert!(
        codes.contains(&"DEED4031".to_string()),
        "an operator mixed its operand types: {codes:?}"
    );
}

/// An operator sits between two values.
#[test]
fn an_operator_that_takes_three_arguments_is_refused() {
    let codes = refused(
        "module probe\n\n\
         record Money {\n\
         \x20   cents: Int,\n\
         }\n\n\
         operator + = three\n\n\
         fn three(a: Money, b: Money, c: Money) -> Money {\n\
         \x20   Money { cents: a.cents + b.cents + c.cents }\n\
         }\n",
    );
    assert!(
        codes.contains(&"DEED4031".to_string()),
        "an operator took three arguments: {codes:?}"
    );
}

/// A generic function cannot be an operator.
///
/// This is the trait question, and it is not this one: the operand types are
/// what choose the function, and a type parameter is not one of them yet.
#[test]
fn a_generic_function_cannot_be_an_operator() {
    let codes = refused(
        "module probe\n\n\
         operator + = joined\n\n\
         fn joined<T>(left: List<T>, right: List<T>) -> List<T> {\n\
         \x20   for one in right with all = left { push(all, one) }\n\
         }\n",
    );
    assert!(
        codes.contains(&"DEED4031".to_string()),
        "a generic function became an operator: {codes:?}"
    );
}

/// A module cannot bind an operator for a type it imported.
///
/// So that what `+` means on a type is decided in one file, which is the
/// reasoning that keeps module resolution free of a search path.
#[test]
fn a_module_cannot_bind_an_operator_for_a_type_it_imported() {
    let codes = refused(
        "module probe\n\n\
         use std/ratio.{Ratio}\n\n\
         operator - = backwards\n\n\
         fn backwards(left: Ratio, right: Ratio) -> Ratio {\n\
         \x20   right\n\
         }\n",
    );
    assert!(
        codes.contains(&"DEED4031".to_string()),
        "a module bound an operator for someone else's type: {codes:?}"
    );
}

/// One operator between two values means one thing.
#[test]
fn an_operator_cannot_be_bound_twice_for_one_type() {
    let codes = refused(
        "module probe\n\n\
         record Money {\n\
         \x20   cents: Int,\n\
         }\n\n\
         operator + = added\n\n\
         operator + = doubled\n\n\
         fn added(left: Money, right: Money) -> Money {\n\
         \x20   Money { cents: left.cents + right.cents }\n\
         }\n\n\
         fn doubled(left: Money, right: Money) -> Money {\n\
         \x20   Money { cents: left.cents + right.cents + right.cents }\n\
         }\n",
    );
    assert!(
        codes.contains(&"DEED4031".to_string()),
        "an operator was bound twice: {codes:?}"
    );
}

/// The codes the *importing* module is refused with, given a library that
/// wrote a binding of its own.
///
/// Only the importing module's, because the library's mistake is the
/// library's and is reported there. What this asks is whether a binding that
/// should not have been made survives the boundary, which is a different
/// question and one nothing else here can ask: a module trusts what it
/// imports, so what it trusts has to be worth trusting.
fn refused_importing(library: &str, app: &str) -> Vec<String> {
    let mut sources = SourceMap::new();
    let app_id = sources.add("app.deed".to_string(), app.to_string());
    let library_id = sources.add("lib.deed".to_string(), library.to_string());

    let checks = check_all(&sources, &[app_id, library_id]);
    checks
        .iter()
        .filter(|one| one.file == app_id)
        .flat_map(|one| one.diagnostics.iter())
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| diagnostic.code.to_string())
        .collect()
}

/// A binding the module that wrote it was refused for does not cross.
///
/// Each of these is refused where it is written, and the shape is read again
/// on the far side rather than trusted, because the surface carries what the
/// module said rather than what it was allowed to say. A binding that got
/// through would give the importing file an operator whose function cannot
/// take what the operator hands it.
#[test]
fn a_binding_that_does_not_hold_up_does_not_cross_the_boundary() {
    let app = "module app\n\n\
         use lib.{Money, coins}\n\n\
         test \"the operator is not there\" {\n\
         \x20   assert coins(1) + coins(2) == coins(3)\n\
         }\n";

    let cases = [
        (
            "three parameters",
            "fn combine(a: Money, b: Money, c: Money) -> Money { a }\n",
        ),
        (
            "a result of another type",
            "fn combine(a: Money, b: Money) -> Int { a.cents }\n",
        ),
        (
            "operands of two types",
            "fn combine(a: Money, b: Int) -> Money { a }\n",
        ),
        (
            "a generic function",
            "fn combine<T>(a: T, b: T) -> T { a }\n",
        ),
    ];

    for (what, combine) in cases {
        let library = format!(
            "module lib\n\n\
             record Money {{\n\
             \x20   cents: Int,\n\
             }}\n\n\
             operator + = combine\n\n\
             fn coins(cents: Int) -> Money {{\n\
             \x20   Money {{ cents: cents }}\n\
             }}\n\n\
             {combine}"
        );
        let codes = refused_importing(&library, app);
        assert!(
            !codes.is_empty(),
            "a binding to {what} reached the importing module and gave it an operator"
        );
    }
}

/// An operator the parser turned away does not cross either.
///
/// `/` cannot be bound, and a module that writes the binding anyway is told
/// so. What this holds is the other half: the file that imports it does not
/// quietly get a `/` the language does not have.
#[test]
fn an_operator_that_cannot_be_bound_does_not_cross_the_boundary() {
    let library = "module lib\n\n\
         record Money {\n\
         \x20   cents: Int,\n\
         }\n\n\
         operator / = combine\n\n\
         fn coins(cents: Int) -> Money {\n\
         \x20   Money { cents: cents }\n\
         }\n\n\
         fn combine(a: Money, b: Money) -> Money { a }\n";
    let app = "module app\n\n\
         use lib.{Money, coins}\n\n\
         test \"there is no such operator\" {\n\
         \x20   assert coins(6) / coins(2) == coins(3)\n\
         }\n";

    assert!(
        !refused_importing(library, app).is_empty(),
        "`/` crossed the boundary, and it is not an operator a module can bind"
    );
}

/// A binding for a type the binder did not declare does not cross either.
///
/// Written as three modules because that is the only shape that can tell the
/// difference: one declares the type, one binds an operator for it, and the
/// third asks what `+` means. The binding is refused where it is written, and
/// what this holds is that the third module does not get an operator anyway.
/// The table is keyed by the module that declares the type, so a binding from
/// somewhere else has to be turned away rather than filed under its author.
#[test]
fn a_binding_for_someone_elses_type_does_not_cross_the_boundary() {
    let mut sources = SourceMap::new();
    let app = sources.add(
        "app.deed".to_string(),
        "module app\n\n\
         use owner.{Money, coins}\n\n\
         test \"there is no such operator\" {\n\
         \x20   assert coins(1) + coins(2) == coins(3)\n\
         }\n"
        .to_string(),
    );
    let owner = sources.add(
        "owner.deed".to_string(),
        "module owner\n\n\
         record Money {\n\
         \x20   cents: Int,\n\
         }\n\n\
         fn coins(cents: Int) -> Money {\n\
         \x20   Money { cents: cents }\n\
         }\n"
        .to_string(),
    );
    let binder = sources.add(
        "binder.deed".to_string(),
        "module binder\n\n\
         use owner.{Money}\n\n\
         operator + = combine\n\n\
         fn combine(a: Money, b: Money) -> Money {\n\
         \x20   a\n\
         }\n"
        .to_string(),
    );

    let checks = check_all(&sources, &[app, owner, binder]);
    let refused: Vec<&str> = checks
        .iter()
        .filter(|one| one.file == app)
        .flat_map(|one| one.diagnostics.iter())
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| diagnostic.code)
        .collect();
    assert!(
        !refused.is_empty(),
        "a module gave `+` a meaning on somebody else's type and a third file got it"
    );
}

/// The operators that cannot be bound at all are still refused by name.
///
/// `/` and `%` are partial and this language spells a partial answer with a
/// `Result`; `==` is structural and total already; ordering is a separate
/// decision because it runs into generic sorting.
#[test]
fn only_the_total_arithmetic_operators_can_be_bound() {
    for spelled in ["/", "%", "==", "<", ">=", "&&"] {
        let codes = refused(&format!(
            "module probe\n\n\
             record Money {{\n\
             \x20   cents: Int,\n\
             }}\n\n\
             operator {spelled} = f\n\n\
             fn f(left: Money, right: Money) -> Money {{\n\
             \x20   left\n\
             }}\n"
        ));
        assert!(
            codes.contains(&"DEED2025".to_string()),
            "`{spelled}` was bindable: {codes:?}"
        );
    }
}
