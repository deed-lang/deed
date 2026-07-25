//! Lexer behaviour, described in terms of what a user would see.
//!
//! The awkward cases get more attention than the happy path on purpose. The
//! happy path is covered by lexing `examples/transfer.vow`, and everything else
//! here is a way the lexer could quietly do the wrong thing.

use vow_diagnostics::{Diagnostic, SourceMap, render_human, render_json};
use vow_lexer::{Keyword, Lexed, TokenKind, codes, tokenize};

fn lex(src: &str) -> (SourceMap, Lexed) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.vow", src);
    let lexed = tokenize(file, sources.file(file).text());
    (sources, lexed)
}

/// Token kinds with the trailing `Eof` dropped, which is noise in most assertions.
fn kinds(src: &str) -> Vec<TokenKind> {
    let (_, lexed) = lex(src);
    let mut kinds: Vec<TokenKind> = lexed.tokens.into_iter().map(|t| t.kind).collect();
    kinds.pop();
    kinds
}

fn codes_of(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.code).collect()
}

// -- the happy path --------------------------------------------------------

#[test]
fn the_worked_example_lexes_cleanly() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.vow");
    let source = std::fs::read_to_string(path).expect("examples/transfer.vow should exist");

    let (sources, lexed) = lex(&source);

    if lexed.has_errors() {
        let rendered: Vec<String> = lexed
            .diagnostics
            .iter()
            .map(|d| render_human(&sources, d))
            .collect();
        panic!(
            "the worked example should lex cleanly:\n{}",
            rendered.join("\n")
        );
    }

    // Sanity check that it produced something structural rather than a soup of
    // error tokens.
    assert!(lexed.tokens.len() > 200);
    assert!(!lexed.tokens.iter().any(|t| t.kind == TokenKind::Error));
    assert_eq!(lexed.tokens[0].kind, TokenKind::Keyword(Keyword::Module));
}

#[test]
fn contract_keywords_are_recognised() {
    assert_eq!(
        kinds("where uses ensures old unchanged"),
        vec![
            TokenKind::Keyword(Keyword::Where),
            TokenKind::Keyword(Keyword::Uses),
            TokenKind::Keyword(Keyword::Ensures),
            TokenKind::Keyword(Keyword::Old),
            TokenKind::Keyword(Keyword::Unchanged),
        ]
    );
}

#[test]
fn operators_prefer_the_longest_match() {
    assert_eq!(
        kinds("-> => == != <= >= && || = ! < > | -"),
        vec![
            TokenKind::Arrow,
            TokenKind::FatArrow,
            TokenKind::EqEq,
            TokenKind::BangEq,
            TokenKind::Le,
            TokenKind::Ge,
            TokenKind::AmpAmp,
            TokenKind::PipePipe,
            TokenKind::Eq,
            TokenKind::Bang,
            TokenKind::Lt,
            TokenKind::Gt,
            TokenKind::Pipe,
            TokenKind::Minus,
        ]
    );
}

#[test]
fn a_bare_underscore_is_not_an_identifier() {
    assert_eq!(
        kinds("_ _x"),
        vec![TokenKind::Underscore, TokenKind::Ident("_x".into())]
    );
}

#[test]
fn method_calls_on_integer_literals_are_unambiguous() {
    // There are no float literals, so `40.try` needs no lookahead to resolve.
    assert_eq!(
        kinds("40.try"),
        vec![
            TokenKind::Int(40),
            TokenKind::Dot,
            TokenKind::Ident("try".into())
        ]
    );
}

#[test]
fn identifiers_may_be_non_ascii() {
    assert_eq!(
        kinds("hesapNumarası"),
        vec![TokenKind::Ident("hesapNumarası".into())]
    );
}

// -- spans -----------------------------------------------------------------

#[test]
fn spans_cover_exactly_the_token() {
    let source = "let total = 42";
    let (_, lexed) = lex(source);

    let slices: Vec<&str> = lexed
        .tokens
        .iter()
        .take(4)
        .map(|t| &source[t.span.as_range()])
        .collect();

    assert_eq!(slices, vec!["let", "total", "=", "42"]);
}

#[test]
fn eof_is_an_empty_span_at_the_end() {
    let (_, lexed) = lex("fn");
    let eof = lexed.tokens.last().unwrap();
    assert_eq!(eof.kind, TokenKind::Eof);
    assert!(eof.span.is_empty());
    assert_eq!(eof.span.start, 2);
}

#[test]
fn spans_are_byte_accurate_after_non_ascii_text() {
    let source = "\"çğü\" x";
    let (_, lexed) = lex(source);
    let x = &lexed.tokens[1];
    assert_eq!(x.kind, TokenKind::Ident("x".into()));
    assert_eq!(&source[x.span.as_range()], "x");
}

// -- comments --------------------------------------------------------------

#[test]
fn block_comments_nest() {
    assert_eq!(
        kinds("a /* outer /* inner */ still a comment */ b"),
        vec![TokenKind::Ident("a".into()), TokenKind::Ident("b".into())]
    );
}

#[test]
fn an_unterminated_block_comment_says_how_many_are_missing() {
    let (sources, lexed) = lex("/* one /* two ");
    assert_eq!(
        codes_of(&lexed.diagnostics),
        vec![codes::UNTERMINATED_BLOCK_COMMENT]
    );

    let rendered = render_human(&sources, &lexed.diagnostics[0]);
    assert!(rendered.contains("2 `*/`s are still needed"), "{rendered}");

    let fix = lexed.diagnostics[0].fix.as_ref().unwrap();
    assert_eq!(fix.edits[0].replacement, "*/*/");
}

#[test]
fn line_comments_end_at_the_newline() {
    assert_eq!(
        kinds("a // comment with \"quotes\" and /* stuff\nb"),
        vec![TokenKind::Ident("a".into()), TokenKind::Ident("b".into())]
    );
}

// -- strings ---------------------------------------------------------------

#[test]
fn escapes_are_decoded() {
    assert_eq!(
        kinds(r#""a\nb\tc\\d\"e\u{1F600}""#),
        vec![TokenKind::Str("a\nb\tc\\d\"e\u{1F600}".into())]
    );
}

#[test]
fn an_unterminated_string_does_not_swallow_the_next_line() {
    let (_, lexed) = lex("let a = \"oops\nlet b = 1");

    assert_eq!(
        codes_of(&lexed.diagnostics),
        vec![codes::UNTERMINATED_STRING]
    );
    // The point of leaving the newline alone: line two still lexes.
    assert_eq!(
        lexed
            .tokens
            .iter()
            .filter(|t| t.kind == TokenKind::Keyword(Keyword::Let))
            .count(),
        2
    );
    assert!(lexed.tokens.iter().any(|t| t.kind == TokenKind::Int(1)));
}

#[test]
fn an_unterminated_string_suggests_a_closing_quote() {
    let (_, lexed) = lex("\"oops");
    let fix = lexed.diagnostics[0].fix.as_ref().unwrap();
    assert_eq!(fix.edits[0].replacement, "\"");
    assert_eq!(
        fix.applicability,
        vow_diagnostics::Applicability::MachineApplicable
    );
}

#[test]
fn an_unknown_escape_is_reported_but_lexing_continues() {
    let (_, lexed) = lex(r#""a\qb" rest"#);
    assert_eq!(codes_of(&lexed.diagnostics), vec![codes::UNKNOWN_ESCAPE]);
    assert_eq!(lexed.tokens[1].kind, TokenKind::Ident("rest".into()));
}

#[test]
fn malformed_unicode_escapes_are_caught() {
    let (_, lexed) = lex(r#""\u41" "\u{D800}" "\u{110000}""#);
    assert_eq!(
        codes_of(&lexed.diagnostics),
        vec![
            codes::UNKNOWN_ESCAPE,
            codes::UNKNOWN_ESCAPE,
            codes::UNKNOWN_ESCAPE
        ]
    );
}

// -- numbers ---------------------------------------------------------------

#[test]
fn radix_prefixes_and_separators_are_decoded() {
    assert_eq!(
        kinds("0xFF 0b1010 0o755 1_000_000 0"),
        vec![
            TokenKind::Int(255),
            TokenKind::Int(10),
            TokenKind::Int(493),
            TokenKind::Int(1_000_000),
            TokenKind::Int(0),
        ]
    );
}

#[test]
fn an_oversized_integer_names_the_limit() {
    let (sources, lexed) = lex("99999999999999999999");
    assert_eq!(
        codes_of(&lexed.diagnostics),
        vec![codes::INTEGER_OUT_OF_RANGE]
    );
    assert!(render_human(&sources, &lexed.diagnostics[0]).contains("9223372036854775807"));
}

#[test]
fn a_literal_suffix_is_one_error_not_two_tokens() {
    let (sources, lexed) = lex("100u8");
    assert_eq!(codes_of(&lexed.diagnostics), vec![codes::MALFORMED_NUMBER]);
    assert_eq!(lexed.tokens[0].kind, TokenKind::Error);
    assert_eq!(lexed.tokens.len(), 2, "should not also emit an identifier");
    assert!(render_human(&sources, &lexed.diagnostics[0]).contains("no literal suffixes"));
}

#[test]
fn an_invalid_digit_points_at_the_digit() {
    let source = "0b1210";
    let (_, lexed) = lex(source);
    assert_eq!(codes_of(&lexed.diagnostics), vec![codes::MALFORMED_NUMBER]);
    let span = lexed.diagnostics[0].primary.span;
    assert_eq!(&source[span.as_range()], "2");
}

#[test]
fn a_radix_prefix_with_no_digits_is_reported() {
    let (_, lexed) = lex("0x");
    assert_eq!(codes_of(&lexed.diagnostics), vec![codes::MALFORMED_NUMBER]);
}

// -- recovery --------------------------------------------------------------

#[test]
fn one_bad_character_does_not_hide_the_rest() {
    // Four independent problems. Reporting only the first would cost three
    // extra round trips, which is the thing the design is trying to avoid.
    let (_, lexed) = lex("§ a\n\"unterminated\n0b12\n99999999999999999999");

    assert_eq!(
        codes_of(&lexed.diagnostics),
        vec![
            codes::UNKNOWN_CHARACTER,
            codes::UNTERMINATED_STRING,
            codes::MALFORMED_NUMBER,
            codes::INTEGER_OUT_OF_RANGE,
        ]
    );
    assert!(
        lexed
            .tokens
            .iter()
            .any(|t| t.kind == TokenKind::Ident("a".into()))
    );
}

#[test]
fn a_curly_quote_gets_a_fix_a_tool_can_apply() {
    let (sources, lexed) = lex("let s = \u{201C}hello\u{201D}");

    let first = &lexed.diagnostics[0];
    assert_eq!(first.code, codes::UNKNOWN_CHARACTER);
    let fix = first.fix.as_ref().unwrap();
    assert_eq!(fix.edits[0].replacement, "\"");
    assert_eq!(
        fix.applicability,
        vow_diagnostics::Applicability::MachineApplicable
    );

    let json = render_json(&sources, first);
    assert!(json.contains("\"applicability\":\"machine-applicable\""));
}

#[test]
fn a_single_ampersand_suggests_the_logical_operator() {
    let (_, lexed) = lex("a & b");
    assert_eq!(codes_of(&lexed.diagnostics), vec![codes::UNKNOWN_CHARACTER]);
    assert_eq!(
        lexed.diagnostics[0].fix.as_ref().unwrap().edits[0].replacement,
        "&&"
    );
    assert_eq!(lexed.tokens[2].kind, TokenKind::Ident("b".into()));
}

#[test]
fn a_byte_order_mark_is_not_a_problem() {
    // Windows editors write one without mentioning it. Rejecting it would mean
    // an "unexpected character" on a file the user typed nothing wrong into.
    let (_, lexed) = lex("\u{FEFF}module a\n");
    assert!(!lexed.has_errors());
    assert_eq!(lexed.tokens[0].kind, TokenKind::Keyword(Keyword::Module));
}

#[test]
fn a_byte_order_mark_does_not_shift_the_spans() {
    let source = "\u{FEFF}let x = 1";
    let (_, lexed) = lex(source);
    let slices: Vec<&str> = lexed
        .tokens
        .iter()
        .take(3)
        .map(|t| &source[t.span.as_range()])
        .collect();
    assert_eq!(slices, vec!["let", "x", "="]);
}

#[test]
fn empty_input_is_just_eof() {
    let (_, lexed) = lex("");
    assert_eq!(lexed.tokens.len(), 1);
    assert_eq!(lexed.tokens[0].kind, TokenKind::Eof);
    assert!(!lexed.has_errors());
}

#[test]
fn a_file_of_only_trivia_is_just_eof() {
    let (_, lexed) = lex("  // hi\n  /* there */\n\n");
    assert_eq!(lexed.tokens.len(), 1);
    assert_eq!(lexed.tokens[0].kind, TokenKind::Eof);
}
