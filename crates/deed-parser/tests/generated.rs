//! A deterministic generated corpus for the first parser-fuzzing step.
//!
//! Coverage is stated on purpose. This step only produces arithmetic, `if`,
//! `let`, and calls inside one function body, because that is enough to start
//! exercising the parser over accepted programs without claiming more than it
//! checks. It does not yet generate loops, matches, contracts, effects,
//! handlers, records, generics, or top-level declarations beyond the helper
//! functions wrapped around each body.

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_lexer::tokenize;
use deed_parser::{codes, parse};

fn wrap(body: &str) -> String {
    format!(
        "module generated\n\
         \n\
         fn inc(n: Int) -> Int {{ n + 1 }}\n\
         \n\
         fn add(a: Int, b: Int) -> Int {{ a + b }}\n\
         \n\
         fn is_zero(n: Int) -> Bool {{ n == 0 }}\n\
         \n\
         fn choose(flag: Bool, when_true: Int, when_false: Int) -> Int {{\n\
         \x20   if flag {{ when_true }} else {{ when_false }}\n\
         }}\n\
         \n\
         fn sample(n: Int, m: Int) -> Int {{\n\
         {body}\n\
         }}\n"
    )
}

fn int_expressions() -> Vec<String> {
    let atoms = ["0", "1", "n", "m", "inc(n)", "add(n, m)"];
    let mut exprs: Vec<String> = atoms.iter().map(ToString::to_string).collect();

    for left in &atoms[..3] {
        for right in &atoms[3..] {
            exprs.push(format!("{left} + {right}"));
            exprs.push(format!("{left} - {right}"));
        }
    }

    exprs
}

fn bool_expressions(ints: &[String]) -> Vec<String> {
    let mut exprs = vec![
        "true".to_string(),
        "false".to_string(),
        "is_zero(n)".to_string(),
        "n == m".to_string(),
    ];

    for expr in ints.iter().take(4) {
        exprs.push(format!("is_zero({expr})"));
    }

    for left in ints.iter().take(2) {
        for right in ints.iter().skip(2).take(2) {
            exprs.push(format!("{left} == {right}"));
        }
    }

    exprs
}

fn generated_programs() -> Vec<(String, String)> {
    let ints = int_expressions();
    let bools = bool_expressions(&ints);
    let mut programs = Vec::new();

    for (index, expr) in ints.iter().take(12).enumerate() {
        programs.push((format!("arithmetic_{index}"), wrap(expr)));
        programs.push((
            format!("let_value_{index}"),
            wrap(&format!("let value = {expr};\nvalue")),
        ));
    }

    for (index, cond) in bools.iter().take(8).enumerate() {
        let when_true = &ints[index % ints.len()];
        let when_false = &ints[(index + 1) % ints.len()];

        programs.push((
            format!("if_expression_{index}"),
            wrap(&format!("if {cond} {{ {when_true} }} else {{ {when_false} }}")),
        ));
        programs.push((
            format!("call_and_let_{index}"),
            wrap(&format!(
                "let chosen = choose({cond}, {when_true}, {when_false});\nadd(chosen, inc(n))"
            )),
        ));
    }

    programs
}

fn missing_equals_programs() -> Vec<(String, String)> {
    int_expressions()
        .into_iter()
        .take(8)
        .enumerate()
        .map(|(index, expr)| {
            (
                format!("missing_equals_{index}"),
                wrap(&format!("let value {expr};\nvalue")),
            )
        })
        .collect()
}

fn render_all(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| render_human(sources, diagnostic))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn step_1_generated_programs_parse_cleanly() {
    let programs = generated_programs();
    assert!(programs.len() >= 40, "the generated corpus should be non-trivial");

    for (name, source) in programs {
        let mut sources = SourceMap::new();
        let file = sources.add(format!("{name}.deed"), source.clone());
        let lexed = tokenize(file, sources.file(file).text());
        assert!(
            !lexed.has_errors(),
            "{name} should lex cleanly:\n{}",
            render_all(&sources, &lexed.diagnostics)
        );

        let parsed = parse(file, &lexed.tokens);
        assert!(
            !parsed.has_errors(),
            "{name} should parse cleanly:\n{}",
            render_all(&sources, &parsed.diagnostics)
        );
    }
}

#[test]
fn step_2_generated_missing_equals_programs_report_the_same_parser_error() {
    for (name, source) in missing_equals_programs() {
        let mut sources = SourceMap::new();
        let file = sources.add(format!("{name}.deed"), source.clone());
        let lexed = tokenize(file, sources.file(file).text());
        assert!(
            !lexed.has_errors(),
            "{name} should stay a parser test:\n{}",
            render_all(&sources, &lexed.diagnostics)
        );

        let parsed = parse(file, &lexed.tokens);
        let errors: Vec<&Diagnostic> = parsed.diagnostics.iter().filter(|d| d.is_error()).collect();
        assert_eq!(
            errors.len(),
            1,
            "{name} should stay one parser error:\n{}",
            render_all(&sources, &parsed.diagnostics)
        );
        assert_eq!(
            errors[0].code,
            codes::UNEXPECTED_TOKEN,
            "{name} should keep reporting the missing `=` as an unexpected token"
        );
    }
}
