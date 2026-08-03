//! Runtime invariants over systematically varied, generated programs.
//!
//! These are soundness properties: they claim something about every accepted
//! program. So each generated case is checked, run, and held to all three
//! invariants together.

use std::collections::BTreeSet;

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_driver::check_all;
use deed_interp::{Program, codes, run_tests};

#[derive(Clone, Copy)]
enum Expectation {
    Passes,
    GuardRefusal,
}

struct Case {
    name: String,
    source: String,
    expectation: Expectation,
    row_exemption: Option<&'static str>,
}

fn unknowns(sources: &SourceMap, checked: &deed_driver::Checked) -> Vec<String> {
    let text = sources.file(checked.file).text();
    let mut found: Vec<(u32, String)> = checked
        .types
        .unknowns()
        .map(|span| {
            let start = span.start as usize;
            let end = (span.end as usize).min(text.len());
            let snippet = text.get(start..end).unwrap_or("<bad span>");
            (span.start, format!("{}: `{snippet}`", span.start))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, line)| line).collect()
}

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

fn generated_cases() -> Vec<Case> {
    let mut cases = Vec::new();

    // Guarded obligations, varied by boundary and by failing value.
    for bad in [0, -1] {
        cases.push(Case {
            name: format!("argument_guard_{bad}"),
            source: format!(
                "module a\n\n\
                 type Positive = Int where value > 0\n\n\
                 fn take(n: Positive) -> Int {{ n }}\n\n\
                 fn indirect(n: Int) -> Int {{ take(n) }}\n\n\
                 test \"argument is guarded\" {{\n\
                 \x20 assert indirect({bad}) == {bad}\n\
                 }}\n"
            ),
            expectation: Expectation::GuardRefusal,
            row_exemption: None,
        });

        cases.push(Case {
            name: format!("return_guard_{bad}"),
            source: format!(
                "module a\n\n\
                 type Positive = Int where value > 0\n\n\
                 fn make(n: Int) -> Positive {{ n }}\n\n\
                 test \"return is guarded\" {{\n\
                 \x20 assert make({bad}) == {bad}\n\
                 }}\n"
            ),
            expectation: Expectation::GuardRefusal,
            row_exemption: None,
        });
    }

    cases.push(Case {
        name: "list_payload_guard_0".to_string(),
        source: "module a\n\
                 \n\
                 type Positive = Int where value > 0\n\
                 \n\
                 fn make(n: Int) -> List<Positive> { [1, n] }\n\
                 \n\
                 test \"list payload is guarded\" {\n\
                 \x20 assert make(0) == [1, 1]\n\
                 }\n"
        .to_string(),
        expectation: Expectation::GuardRefusal,
        row_exemption: None,
    });

    // Row-check exemptions: generated coverage for all three.
    cases.push(Case {
        name: "row_exemption_with_discharges".to_string(),
        source: "module a\n\
                 \n\
                 effect Log {\n\
                 \x20 fn note(message: String) -> Int\n\
                 }\n\
                 \n\
                 handler Counted implements Log {\n\
                 \x20 state seen: Int\n\
                 \x20 fn note(message) -> Int {\n\
                 \x20   seen = seen + 1\n\
                 \x20   seen\n\
                 \x20 }\n\
                 }\n\
                 \n\
                 fn logs(n: Int) -> Int uses Log.note { n + Log.note(\"x\") }\n\
                 \n\
                 fn quietly(n: Int) -> Int {\n\
                 \x20 with Counted { seen: 0 } {\n\
                 \x20   logs(n)\n\
                 \x20 }\n\
                 }\n\
                 \n\
                 test \"with discharges the effect\" {\n\
                 \x20 assert quietly(1) == 2\n\
                 }\n"
        .to_string(),
        expectation: Expectation::Passes,
        row_exemption: Some("with discharges"),
    });

    cases.push(Case {
        name: "row_exemption_installer_pays".to_string(),
        source: "module a\n\
                 \n\
                 effect Log {\n\
                 \x20 fn note(message: String) -> Int\n\
                 }\n\
                 \n\
                 effect Ticker {\n\
                 \x20 fn tick() -> Int\n\
                 }\n\
                 \n\
                 handler Silent implements Ticker {\n\
                 \x20 fn tick() -> Int { 7 }\n\
                 }\n\
                 \n\
                 handler Sneaky implements Log {\n\
                 \x20 fn note(message) -> Int uses Ticker.tick, { Ticker.tick() }\n\
                 }\n\
                 \n\
                 fn talks(n: Int) -> Int uses Log.note { n + Log.note(\"hi\") }\n\
                 \n\
                 fn installs(n: Int) -> Int uses Ticker.tick, {\n\
                 \x20 with Sneaky { talks(n) }\n\
                 }\n\
                 \n\
                 test \"installer pays for what the handler performs\" {\n\
                 \x20 with Silent {\n\
                 \x20   assert installs(1) == 8\n\
                 \x20 }\n\
                 }\n"
        .to_string(),
        expectation: Expectation::Passes,
        row_exemption: Some("handler belongs to installer"),
    });

    cases.push(Case {
        name: "row_exemption_contract".to_string(),
        source: "module a\n\
                 \n\
                 effect Log {\n\
                 \x20 fn note(message: String) -> Int\n\
                 }\n\
                 \n\
                 handler Counted implements Log {\n\
                 \x20 state seen: Int\n\
                 \x20 fn note(message) -> Int {\n\
                 \x20   seen = seen + 1\n\
                 \x20   seen\n\
                 \x20 }\n\
                 }\n\
                 \n\
                 fn counted(n: Int) -> Int\n\
                 \x20 uses Log.note,\n\
                 \x20 ensures\n\
                 \x20   ok  => Log.note(\"asking\") > 0,\n\
                 \x20   err => true,\n\
                 { n + Log.note(\"working\") }\n\
                 \n\
                 test \"contract effects are exempt from row accounting\" {\n\
                 \x20 with Counted { seen: 0 } {\n\
                 \x20   assert counted(1) == 2\n\
                 \x20 }\n\
                 }\n"
        .to_string(),
        expectation: Expectation::Passes,
        row_exemption: Some("contracts are exempt"),
    });

    cases
}

fn assert_invariants(case: &Case) -> Option<&'static str> {
    let mut sources = SourceMap::new();
    let file = sources.add(format!("{}.deed", case.name), case.source.clone());
    let checked = check_all(&sources, &[file]).remove(0);

    let errors: Vec<Diagnostic> = checked
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .cloned()
        .collect();
    assert!(
        errors.is_empty(),
        "generated case `{}` should check cleanly:\n{}\n\n{}",
        case.name,
        rendered(&sources, &errors),
        case.source
    );

    let unknown = unknowns(&sources, &checked);
    assert!(
        checked.types.recorded() > 0,
        "generated case `{}` recorded no types, so the invariant would be vacuous\n\n{}",
        case.name,
        case.source
    );
    assert!(
        unknown.is_empty(),
        "generated case `{}` has {} unknown expression type(s):\n  {}\n\n{}",
        case.name,
        unknown.len(),
        unknown.join("\n  "),
        case.source
    );

    let guards = checked.guards();
    if matches!(case.expectation, Expectation::GuardRefusal) {
        assert!(
            !guards.is_empty(),
            "generated case `{}` should produce a guarded obligation, but none were recorded\n\n{}",
            case.name,
            case.source
        );
    }

    let mut program = Program::new();
    program.add(
        checked.file,
        &checked.module,
        &checked.resolutions,
        guards,
        checked.rows(),
        checked.operators(),
    );

    let outcomes = run_tests(&program, checked.file);
    assert!(
        !outcomes.is_empty(),
        "generated case `{}` ran no tests\n\n{}",
        case.name,
        case.source
    );

    let failures: Vec<Diagnostic> = outcomes
        .into_iter()
        .filter_map(|outcome| outcome.failure)
        .collect();

    let row_failures: Vec<Diagnostic> = failures
        .iter()
        .filter(|d| d.code == codes::ROW_NOT_KEPT)
        .cloned()
        .collect();
    if !row_failures.is_empty() {
        for failure in &row_failures {
            let text = render_human(&sources, failure);
            assert!(
                text.contains("hole in the effect checker"),
                "`DEED6010` should report a compiler bug:\n{text}"
            );
        }
        panic!(
            "generated case `{}` found a row soundness hole; copy this source into examples/ as a corpus entry:\n\n{}\n\n{}",
            case.name,
            case.source,
            rendered(&sources, &row_failures)
        );
    }

    match case.expectation {
        Expectation::Passes => {
            assert!(
                failures.is_empty(),
                "generated case `{}` should run cleanly:\n{}\n\n{}",
                case.name,
                rendered(&sources, &failures),
                case.source
            );
        }
        Expectation::GuardRefusal => {
            assert_eq!(
                failures.len(),
                1,
                "generated case `{}` should refuse exactly one value:\n{}\n\n{}",
                case.name,
                rendered(&sources, &failures),
                case.source
            );
            assert_eq!(
                failures[0].code,
                codes::REFINEMENT_FAILED,
                "generated case `{}` should fail as a guard refusal:\n{}\n\n{}",
                case.name,
                render_human(&sources, &failures[0]),
                case.source
            );
        }
    }

    case.row_exemption
}

#[test]
fn generated_programs_hold_the_runtime_invariants() {
    let mut seen = BTreeSet::new();

    for case in generated_cases() {
        if let Some(exemption) = assert_invariants(&case) {
            seen.insert(exemption);
        }
    }

    let expected = BTreeSet::from([
        "with discharges",
        "handler belongs to installer",
        "contracts are exempt",
    ]);
    assert_eq!(
        seen, expected,
        "generated cases should cover every row-check exemption"
    );
}
