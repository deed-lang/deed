//! The formatter.
//!
//! Most of these are properties rather than examples. A formatter is a pile of
//! layout decisions and testing each one by hand tests the decisions I thought
//! of, which is not the set that matters.

use vow_diagnostics::SourceMap;
use vow_fmt::format;

fn fmt(source: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add("test.vow", source);
    match format(file, source) {
        Ok(formatted) => formatted,
        Err(diagnostics) => panic!(
            "should have parsed: {}",
            diagnostics
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

// -- the properties --------------------------------------------------------

#[test]
fn formatting_is_idempotent() {
    for source in SOURCES {
        let once = fmt(source);
        let twice = fmt(&once);
        assert_eq!(once, twice, "second pass changed it:\n{once}");
    }
}

#[test]
fn formatting_does_not_change_what_the_program_means() {
    // The tree after formatting has to be the tree before it. A formatter that
    // reshapes a program is worse than no formatter, and the place this goes
    // wrong is parentheses, which the tree does not record.
    for source in SOURCES {
        let formatted = fmt(source);
        assert_eq!(
            shape(source),
            shape(&formatted),
            "the tree changed:\n{formatted}"
        );
    }
}

#[test]
fn every_comment_survives() {
    for source in SOURCES {
        let formatted = fmt(source);
        for comment in comments(source) {
            assert!(
                formatted.contains(&comment),
                "lost `{comment}` from:\n{formatted}"
            );
        }
    }
}

#[test]
fn output_ends_with_exactly_one_newline() {
    for source in SOURCES {
        let formatted = fmt(source);
        assert!(formatted.ends_with('\n'), "no trailing newline");
        assert!(
            !formatted.ends_with("\n\n"),
            "more than one trailing newline"
        );
    }
}

#[test]
fn no_line_has_trailing_whitespace() {
    for source in SOURCES {
        for line in fmt(source).lines() {
            assert_eq!(line, line.trim_end(), "trailing whitespace on `{line}`");
        }
    }
}

// -- parentheses -----------------------------------------------------------

#[test]
fn parentheses_come_back_where_precedence_needs_them() {
    // The tree carries no parentheses, so these have to be reconstructed. Each
    // one changes the answer if it is dropped.
    let cases = [
        ("(a + b) * c", "(a + b) * c"),
        ("a + b * c", "a + b * c"),
        ("a - (b - c)", "a - (b - c)"),
        ("(a - b) - c", "a - b - c"),
        ("(a || b) && c", "(a || b) && c"),
        ("a || b && c", "a || b && c"),
        ("!(a && b)", "!(a && b)"),
        ("(a + b).field", "(a + b).field"),
    ];

    for (input, expected) in cases {
        let formatted = fmt(&format!("module a\n\nfn f() -> Int {{ {input} }}\n"));
        assert!(
            formatted.contains(expected),
            "`{input}` became:\n{formatted}"
        );
    }
}

#[test]
fn redundant_parentheses_are_dropped() {
    let formatted = fmt("module a\n\nfn f() -> Int { ((a + b)) }\n");
    assert!(formatted.contains("    a + b\n"), "{formatted}");
}

// -- the shape of things ---------------------------------------------------

#[test]
fn whitespace_does_not_matter() {
    let cramped = "module a\nfn   f( n:Int )->Int{n+n}\n";
    let sprawling = "module a\n\n\n\nfn f(\n  n: Int\n)\n->\nInt\n{\n\n\n    n   +   n\n\n}\n";
    assert_eq!(fmt(cramped), fmt(sprawling));
}

#[test]
fn ok_is_padded_so_the_arrows_line_up() {
    let formatted =
        fmt("module a\n\nfn f(n: Int) -> Int\n  ensures\n    ok => result > 0,\n{\n  n\n}\n");
    assert!(formatted.contains("    ok  => result > 0,"), "{formatted}");
}

#[test]
fn a_contract_puts_the_brace_back_on_the_margin() {
    let formatted = fmt("module a\n\nfn f() -> Int uses E.op, { 1 }\n");
    assert!(formatted.contains("    E.op,\n{\n"), "{formatted}");
}

#[test]
fn an_empty_body_stays_on_one_line() {
    let formatted = fmt("module a\n\nfn f() -> () {\n}\n");
    assert!(formatted.contains("fn f() -> () {}"), "{formatted}");
}

#[test]
fn a_long_call_breaks_one_argument_per_line() {
    let long = "aaaaaaaaaaaaaaaaaaaa";
    let formatted = fmt(&format!(
        "module a\n\nfn f() -> Int {{ g({long}, {long}, {long}, {long}, {long}) }}\n"
    ));
    assert!(formatted.contains("g(\n"), "{formatted}");
    assert!(
        formatted.contains(&format!("        {long},\n")),
        "{formatted}"
    );
}

#[test]
fn a_short_list_stays_on_one_line() {
    let formatted = fmt("module a\n\nfn f() -> List<Int> {\n[1,\n2,\n3,]\n}\n");
    assert!(formatted.contains("    [1, 2, 3]\n"), "{formatted}");
}

#[test]
fn a_long_list_breaks_one_element_per_line() {
    let long = "aaaaaaaaaaaaaaaaaaaa";
    let formatted = fmt(&format!(
        "module a\n\nfn f() -> Int {{ [{long}, {long}, {long}, {long}] }}\n"
    ));
    assert!(formatted.contains("[\n"), "{formatted}");
    assert!(
        formatted.contains(&format!("        {long},\n")),
        "{formatted}"
    );
}

#[test]
fn a_for_keeps_its_head_on_one_line() {
    let formatted = fmt("module a\n\nfn f() -> Int {\nfor n in ns with sum=0 {sum+n}\n}\n");
    assert!(
        formatted.contains("    for n in ns with sum = 0 {\n"),
        "{formatted}"
    );
}

#[test]
fn a_trailing_comment_stays_on_its_line() {
    let formatted = fmt("module a\n\nfn f() -> Int {\n    let x = 1 // why\n    x\n}\n");
    assert!(formatted.contains("let x = 1 // why"), "{formatted}");
}

#[test]
fn at_most_one_blank_line_survives() {
    let formatted = fmt("module a\n\nfn f() -> Int {\n    let x = 1\n\n\n\n\n    x\n}\n");
    assert!(!formatted.contains("\n\n\n"), "{formatted}");
    assert!(formatted.contains("let x = 1\n\n"), "{formatted}");
}

#[test]
fn a_comment_inside_a_nested_block_stays_there() {
    // This is the one that was wrong first. Blocks inside an expression are
    // rendered into their own buffer, and a comment written straight to the
    // output surfaced after the enclosing statement instead.
    let formatted = fmt(
        "module a\n\ntest \"t\" {\n    with H { s: 0 } {\n        // inside\n        assert true\n    }\n}\n",
    );
    let inside = formatted.find("// inside").expect("the comment survived");
    let closing = formatted.rfind('}').unwrap();
    assert!(inside < closing, "{formatted}");
    assert!(formatted.contains("        // inside\n"), "{formatted}");
}

// -- broken input ----------------------------------------------------------

#[test]
fn a_file_that_does_not_parse_is_refused() {
    let source = "module a\n\nfn f( -> Int {\n";
    let mut sources = SourceMap::new();
    let file = sources.add("bad.vow", source);
    assert!(
        format(file, source).is_err(),
        "reshaping a broken file is guessing at what was meant"
    );
}

// -- helpers ---------------------------------------------------------------

/// A rough shape of the source, for comparing two spellings of one program.
///
/// Everything but the tokens themselves is dropped, which is enough to catch a
/// formatter that added, removed or reordered anything.
fn shape(source: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add("shape.vow", source);
    let lexed = vow_lexer::tokenize(file, source);
    lexed
        .tokens
        .iter()
        .map(|token| format!("{:?}", token.kind))
        .collect::<Vec<_>>()
        .join(" ")
}

fn comments(source: &str) -> Vec<String> {
    let mut sources = SourceMap::new();
    let file = sources.add("comments.vow", source);
    let lexed = vow_lexer::tokenize(file, source);
    lexed
        .trivia
        .iter()
        .map(|t| {
            source[t.span.start as usize..t.span.end as usize]
                .trim()
                .to_string()
        })
        .filter(|text| !text.contains('\n'))
        .collect()
}

const SOURCES: &[&str] = &[
    "module a\n",
    "module a\n\nfn f() -> Int { 0 }\n",
    "// leading\nmodule a\n\n// about f\nfn f() -> Int {\n    // inside\n    let x = 1 // trailing\n\n    x\n}\n",
    "module a\n\nuse b/c.{D, e}\n\nrecord R {\n    a: Int,\n    b: String,\n}\n",
    "module a\n\nchoice C {\n    One,\n    Two { x: Int, y: Int },\n}\n",
    "module a\n\ntype P = Int where value > 0\n",
    "module a\n\neffect E {\n    fn op(n: Int) -> Int\n}\n\nhandler H implements E {\n    state s: Int\n\n    fn op(n) -> Int { s + n }\n}\n",
    "module a\n\nfn f(n: Int) -> Result<Int, String>\n  where\n    n > 0,\n  ensures\n    ok => result > 0,\n    err => true,\n{\n    if n > 10 {\n        return err(\"too big\")\n    }\n    ok(n)\n}\n",
    "module a\n\nfn f(v: C) -> Int {\n    match v {\n        One => 1,\n        Two { x, y } => x + y,\n    }\n}\n",
    "module a\n\nfn f() -> Int { g(1)? + h(2)? }\n",
    "module a\n\nfn f() -> List<Int> { [1, 2, 3] }\n",
    "module a\n\nfn f(ns: List<Int>) -> Int {\n    for n in ns with sum = 0 {\n        sum + n\n    }\n}\n",
    "module a\n\nfn f(ns: List<Int>) -> () {\n    for n in ns {\n        g(n)\n    }\n}\n",
    "module a\n\nfn f() -> List<List<Int>> { [[], [1]] }\n",
    "module a\n\nfn f() -> Int { (1 + 2) * (3 - 4) / (5 % 6) }\n",
    "module a\n\ntest \"a test\" {\n    assert 1 == 1\n}\n",
];
