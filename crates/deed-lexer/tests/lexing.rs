//! Lexer behaviour, described in terms of what a user would see.
//!
//! The awkward cases get more attention than the happy path on purpose. The
//! happy path is covered by lexing `examples/transfer.deed`, and everything else
//! here is a way the lexer could quietly do the wrong thing.
//!
//! The last section is the other half of that. Everything above it asks what
//! the lexer did; that one asks what it said, once per message, because a
//! message nobody has read is also a message nobody has judged.

use deed_diagnostics::{Applicability, Diagnostic, SourceMap, render_human, render_json};
use deed_lexer::{Keyword, Lexed, TokenKind, codes, tokenize};

fn lex(src: &str) -> (SourceMap, Lexed) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);
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
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.deed");
    let source = std::fs::read_to_string(path).expect("examples/transfer.deed should exist");

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
fn a_decimal_point_is_reported_once_and_the_whole_part_stands_in() {
    // Reporting it and then handing the parser an invalid token would earn a
    // second message in the same column saying an expression was expected.
    let kinds = kinds("1.5");
    assert_eq!(
        kinds,
        vec![TokenKind::Int(1)],
        "the dot and the fraction are absorbed, not left behind"
    );
    // Once, and under the code that says what it is. The text this renders is
    // read by `a_decimal_point_says_there_are_no_floats`; the wording and the
    // code it arrives under are two separate claims, and breaking either one
    // leaves the other standing.
    assert_eq!(
        codes_of(&lex("1.5").1.diagnostics),
        vec![codes::NO_FLOAT_LITERAL]
    );
}

#[test]
fn taking_the_decimal_point_leaves_the_shapes_around_it_alone() {
    // A field name is an identifier, so a dot before a digit was never a field
    // access, and a dot before a dot is not this at all.
    for src in ["40.try", "0..10", "1 . 5", "0x1f", "1_000"] {
        assert!(
            !codes_of(&lex(src).1.diagnostics).contains(&codes::NO_FLOAT_LITERAL),
            "`{src}` should not be read as a decimal number"
        );
    }
}

#[test]
fn identifiers_may_be_non_ascii() {
    assert_eq!(
        kinds("hesapNumarası"),
        vec![TokenKind::Ident("hesapNumarası".into())]
    );
}

#[test]
fn unicode_digits_may_continue_identifiers() {
    assert_eq!(
        kinds("café໐test໑"),
        vec![TokenKind::Ident("café໐test໑".into())]
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

// -- where a line ended -----------------------------------------------------
//
// The parser needs this to tell a statement that begins with `(` or `-` from a
// continuation of the line above, which the token kinds alone cannot say.

#[test]
fn a_token_knows_whether_a_line_ended_before_it() {
    let (_, lexed) = lex("a b\nc");
    let starts: Vec<bool> = lexed.tokens.iter().map(|token| token.starts_line).collect();

    // `a`, `b`, `c`, end of file. The first token of the file has nothing
    // before it, so nothing ended before it either.
    assert_eq!(starts, vec![false, false, true, true]);
}

#[test]
fn a_comment_with_a_newline_in_it_ends_the_line() {
    // Measured over the text that was skipped rather than counted as it goes,
    // so a block comment spanning lines counts the way a reader would count it.
    let (_, lexed) = lex("a /* one\ntwo */ b");
    assert!(lexed.tokens[1].starts_line, "{:?}", lexed.tokens);

    let (_, lexed) = lex("a /* one two */ b");
    assert!(!lexed.tokens[1].starts_line, "{:?}", lexed.tokens);
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
fn an_unterminated_block_comment_offers_every_terminator_it_needs() {
    let (_, lexed) = lex("/* one /* two ");
    assert_eq!(
        codes_of(&lexed.diagnostics),
        vec![codes::UNTERMINATED_BLOCK_COMMENT]
    );

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
        deed_diagnostics::Applicability::MachineApplicable
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
    // All five shapes, because the code a message arrives under is a separate
    // claim from its wording, and the section at the bottom of this file only
    // reads the wording.
    let (_, lexed) = lex(r#""\u41" "\u{41" "\u{}" "\u{D800}" "\u{110000}""#);
    assert_eq!(
        codes_of(&lexed.diagnostics),
        vec![
            codes::UNKNOWN_ESCAPE,
            codes::UNKNOWN_ESCAPE,
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
fn a_literal_suffix_is_one_error_not_two_tokens() {
    let (_, lexed) = lex("100u8");
    assert_eq!(codes_of(&lexed.diagnostics), vec![codes::MALFORMED_NUMBER]);
    assert_eq!(
        lexed.tokens[0].kind,
        TokenKind::Int(100),
        "the digits before the bad one stand in, so the parser has an expression"
    );
    assert_eq!(lexed.tokens.len(), 2, "should not also emit an identifier");
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
        deed_diagnostics::Applicability::MachineApplicable
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

// -- every message, read ---------------------------------------------------
//
// The ratchet in `crates/deed-driver/tests/codes.rs` matches on the name of a
// code, so one test naming `UNKNOWN_ESCAPE` covered four messages and rendered
// none of them. Nineteen messages were written in this crate, one of them
// could not be reached at all, and four had ever been rendered by a test.
//
// So one test per message from here down, reading the words rather than the
// code, and no string read in two of them: breaking a sentence should name the
// test that owns it. `message` also asserts on the way past that one mistake
// produces one diagnostic, which is the thing #210 had to go back and fix.

/// Every diagnostic `src` produced, rendered the way a person reads them.
fn messages(src: &str) -> Vec<String> {
    let (sources, lexed) = lex(src);
    assert!(
        !lexed.diagnostics.is_empty(),
        "`{src}` was meant to produce a diagnostic and produced none"
    );
    lexed
        .diagnostics
        .iter()
        .map(|d| render_human(&sources, d))
        .collect()
}

/// The one message `src` produces, and the assertion that there is one.
fn message(src: &str) -> String {
    let mut all = messages(src);
    assert_eq!(
        all.len(),
        1,
        "`{src}` is one mistake and should be one message:\n{}",
        all.join("\n")
    );
    all.pop().unwrap()
}

fn fix_for(src: &str) -> deed_diagnostics::Fix {
    let (_, lexed) = lex(src);
    lexed.diagnostics[0]
        .fix
        .clone()
        .unwrap_or_else(|| panic!("`{src}` should carry a fix"))
}

// DEED1001, four ways.

#[test]
fn an_ampersand_is_told_which_operator_deed_has() {
    let text = message("a & b");
    assert!(text.contains("`&` is not an operator in Deed"), "{text}");
    assert!(text.contains("expected `&&`"), "{text}");
    assert!(text.contains("Deed has no bitwise operators"), "{text}");
    assert!(text.contains("help: use `&&`"), "{text}");
}

#[test]
fn a_character_that_starts_nothing_says_only_what_is_wrong() {
    let text = message("§");
    assert!(text.contains("unexpected character `§`"), "{text}");
    assert!(text.contains("not valid at the start of a token"), "{text}");
    // Nothing is suggested, because there is nothing this was likely to be.
    assert!(!text.contains("help:"), "{text}");
}

#[test]
fn a_character_pasted_in_from_a_document_is_named_as_such() {
    let text = message("\u{201C}");
    assert!(text.contains("pasted in from formatted text"), "{text}");
    assert!(text.contains("help: replace it with `\"`"), "{text}");
}

#[test]
fn a_curly_apostrophe_is_not_offered_a_replacement_that_fails_the_same_way() {
    // `'` is not a character in this language either, so the fix that used to
    // be here produced this same message again, and `deed fix` applied it
    // without asking.
    let text = message("\u{2019}");
    assert!(
        text.contains("Deed has no character literals, so text of any length goes between"),
        "{text}"
    );
    assert!(!text.contains("help:"), "{text}");
}

#[test]
fn a_no_break_space_separates_tokens_the_way_a_space_does() {
    // There was a fourth suggestion under this code, for the no-break spaces,
    // and nothing could reach it: `char::is_whitespace` is true for all three,
    // so the trivia skipper takes them before anything looks at them. Which is
    // the right answer, and now it is one somebody has checked.
    let source = "let\u{00A0}a\u{2007}=\u{202F}1";
    let (_, lexed) = lex(source);
    assert!(!lexed.has_errors());
    assert_eq!(
        kinds(source),
        vec![
            TokenKind::Keyword(Keyword::Let),
            TokenKind::Ident("a".into()),
            TokenKind::Eq,
            TokenKind::Int(1),
        ]
    );
}

// DEED1002, two ways.

#[test]
fn a_string_that_runs_off_the_line_says_which_end_it_reached() {
    let text = message("\"oops\nlet b = 1");
    assert!(
        text.contains("string literal reaches end of line before its closing quote"),
        "{text}"
    );
    assert!(text.contains("this string is never closed"), "{text}");
    assert!(text.contains("cannot span multiple lines"), "{text}");
}

#[test]
fn a_string_that_runs_off_the_file_says_which_end_it_reached() {
    let text = message("\"oops");
    assert!(
        text.contains("string literal reaches end of file before its closing quote"),
        "{text}"
    );
}

// DEED1003, two ways.

#[test]
fn one_missing_comment_terminator_is_counted_in_the_singular() {
    // This used to say "1 `*/` are still needed".
    let text = message("/* one ");
    assert!(text.contains("unterminated block comment"), "{text}");
    assert!(text.contains("this comment is never closed"), "{text}");
    assert!(text.contains("one `*/` is still needed"), "{text}");
}

#[test]
fn several_missing_comment_terminators_explain_the_nesting() {
    let text = message("/* one /* two ");
    assert!(
        text.contains("block comments nest, so 2 `*/`s are still needed"),
        "{text}"
    );
}

// DEED1004, seven ways. Three of them are about `\u{...}`, and the one certain
// thing about somebody typing `\u{...}` is that they were already unsure.

#[test]
fn an_unknown_escape_lists_the_ones_that_exist() {
    let text = message(r#""a\qb""#);
    assert!(text.contains("unknown escape sequence `\\q`"), "{text}");
    assert!(text.contains("not a recognised escape"), "{text}");
    assert!(
        text.contains("Deed defines `\\n`, `\\t`, `\\r`, `\\0`, `\\\\`, `\\\"` and `\\u{...}`"),
        "{text}"
    );
    assert!(
        text.contains("help: write a literal backslash as `\\\\q`"),
        "{text}"
    );
}

#[test]
fn a_backslash_u_with_no_brace_names_both_readings() {
    // The other way to get here is a Windows path or a regular expression,
    // where the backslash was meant to stand for itself, and that reading was
    // not mentioned at all.
    let text = message(r#""\u41""#);
    assert!(text.contains("expected `{` after `\\u`"), "{text}");
    assert!(text.contains("incomplete unicode escape"), "{text}");
    assert!(
        text.contains("unicode escapes are written `\\u{1F600}`"),
        "{text}"
    );
    assert!(
        text.contains("a backslash that stands for itself is written `\\\\`"),
        "{text}"
    );
    assert!(
        text.contains("help: write a literal backslash as `\\\\u`"),
        "{text}"
    );
}

#[test]
fn the_backslash_reading_is_offered_and_never_applied() {
    // Which of the two readings was meant is the reader's to choose, so this
    // is a suggestion. Braces around whatever follows would be the other
    // guess, and it would have to decide where the digits end.
    let fix = fix_for(r#""\u41""#);
    assert_eq!(fix.edits[0].replacement, "\\\\u");
    assert_eq!(fix.applicability, Applicability::MaybeIncorrect);
}

#[test]
fn an_escape_that_runs_into_the_end_of_the_string_is_closed_for_you() {
    let text = message(r#""\u{41" rest"#);
    assert!(
        text.contains("unicode escape is missing its closing `}`"),
        "{text}"
    );
    assert!(text.contains("this escape is never closed"), "{text}");
    assert!(text.contains("help: close the escape"), "{text}");

    let fix = fix_for(r#""\u{41" rest"#);
    assert_eq!(fix.edits[0].replacement, "}");
    assert_eq!(fix.applicability, Applicability::MachineApplicable);
}

#[test]
fn an_escape_stopped_by_a_stray_character_names_the_character() {
    let text = message(r#""\u{4G}""#);
    assert!(
        text.contains("`G` is not a hexadecimal digit, so the escape stops before it"),
        "{text}"
    );
}

#[test]
fn a_brace_is_not_applied_over_a_character_it_would_turn_into_text() {
    // `deed fix` would otherwise rewrite `"\u{4G}"` to `"\u{4}G}"` without
    // asking. The brace is inserted, not moved, so the original one stays and
    // becomes text as well: the error goes away and the string says something
    // else.
    let fix = fix_for(r#""\u{4G}""#);
    assert_eq!(fix.edits[0].replacement, "}");
    assert_eq!(fix.applicability, Applicability::MaybeIncorrect);
}

#[test]
fn an_escape_with_no_digits_and_no_brace_is_not_offered_a_brace() {
    // A `}` here would produce `\u{}`, which is the next message down rather
    // than a repair.
    let text = message(r#""\u{" rest"#);
    assert!(
        text.contains("there is no codepoint here yet either, so `}` alone would not finish it"),
        "{text}"
    );
    assert!(!text.contains("help:"), "{text}");
}

#[test]
fn an_empty_pair_of_braces_says_what_is_missing() {
    // This used to render as "`` is not a unicode scalar value", which names
    // nothing at all.
    let text = message(r#""\u{}""#);
    assert!(
        text.contains("unicode escape has no digits between its braces"),
        "{text}"
    );
    assert!(
        text.contains("expected at least one hexadecimal digit"),
        "{text}"
    );
}

#[test]
fn a_codepoint_that_is_not_a_scalar_value_says_which_ones_are() {
    let text = message(r#""\u{D800}""#);
    assert!(
        text.contains("`D800` is not a unicode scalar value"),
        "{text}"
    );
    assert!(text.contains("invalid unicode escape"), "{text}");
    assert!(
        text.contains("valid values are 0 to 10FFFF, excluding the surrogate range D800 to DFFF"),
        "{text}"
    );
}

// DEED1005, one way.

#[test]
fn an_oversized_integer_names_the_limit() {
    let text = message("99999999999999999999");
    assert!(
        text.contains("integer literal does not fit in `Int`"),
        "{text}"
    );
    assert!(text.contains("too large"), "{text}");
    assert!(
        text.contains("`Int` holds values up to 9223372036854775807"),
        "{text}"
    );
}

/// The digits of the smallest `Int` are one past the largest, so the lexer
/// hands them over and says nothing: whether a unary minus is in front of them
/// is a question about the grammar, and the parser is what answers it.
#[test]
fn the_negative_boundary_is_a_minus_and_a_digit_run_at_the_limit() {
    let (_, lexed) = lex("-9223372036854775808");
    assert_eq!(codes_of(&lexed.diagnostics), Vec::<&str>::new());
    assert_eq!(lexed.tokens[0].kind, TokenKind::Minus);
    assert_eq!(lexed.tokens[1].kind, TokenKind::IntAtLimit);
}

/// And on its own it is still the digit run at the limit rather than an error
/// here, because the lexer has no way to know what came before it.
#[test]
fn the_digit_run_one_past_the_largest_is_left_to_the_parser() {
    let (_, lexed) = lex("9223372036854775808");
    assert_eq!(codes_of(&lexed.diagnostics), Vec::<&str>::new());
    assert_eq!(lexed.tokens[0].kind, TokenKind::IntAtLimit);

    // A number that is merely too big has no reading at all, so it is the
    // lexer's to report.
    let text = message("99999999999999999999");
    assert!(text.contains("does not fit in `Int`"), "{text}");
}

/// One message per literal, which is what the stand-in tokens are for. An
/// error token would be an expression the parser cannot read, and it would say
/// so in the same column the lexer has just written in.
#[test]
fn a_literal_the_lexer_cannot_read_still_stands_in_for_one() {
    for source in ["99999999999999999999", "0x", "100u8", "0b12", "1.5"] {
        let (_, lexed) = lex(source);
        assert_eq!(
            lexed.diagnostics.len(),
            1,
            "{source} should be one mistake: {:?}",
            codes_of(&lexed.diagnostics)
        );
        assert!(
            matches!(lexed.tokens[0].kind, TokenKind::Int(_)),
            "{source} left {:?} where a number was written",
            lexed.tokens[0].kind
        );
    }
}

/// The same for a string: what was read before the line ended stands in for
/// it, so the quote that is missing is the only thing wrong with the line.
#[test]
fn an_unterminated_string_stands_in_for_the_string() {
    let (_, lexed) = lex("\"oops\n");
    assert_eq!(
        codes_of(&lexed.diagnostics),
        vec![codes::UNTERMINATED_STRING]
    );
    assert_eq!(lexed.tokens[0].kind, TokenKind::Str("oops".to_string()));
}

// DEED1006, five ways: no digits at all, and one note per radix.

#[test]
fn a_radix_prefix_with_no_digits_says_what_is_missing() {
    let text = message("0x");
    assert!(
        text.contains("numeric literal has no digits after `0x`"),
        "{text}"
    );
    assert!(text.contains("expected at least one digit"), "{text}");
}

#[test]
fn a_bad_binary_digit_says_which_two_there_are() {
    let text = message("0b12");
    assert!(
        text.contains("`2` is not a valid digit in base 2"),
        "{text}"
    );
    assert!(text.contains("invalid digit"), "{text}");
    assert!(text.contains("in this literal"), "{text}");
    assert!(
        text.contains("binary literals accept `0` and `1`"),
        "{text}"
    );
}

#[test]
fn a_bad_octal_digit_says_where_the_range_stops() {
    let text = message("0o18");
    assert!(
        text.contains("`8` is not a valid digit in base 8"),
        "{text}"
    );
    assert!(
        text.contains("octal literals accept `0` through `7`"),
        "{text}"
    );
}

#[test]
fn a_bad_hexadecimal_digit_admits_that_case_does_not_matter() {
    // `0xFF` lexes, and this note used to name the lowercase letters only.
    let text = message("0xZZ");
    assert!(
        text.contains("`Z` is not a valid digit in base 16"),
        "{text}"
    );
    assert!(
        text.contains(
            "hexadecimal literals accept `0` through `9` and `a` through `f`, in either case"
        ),
        "{text}"
    );
}

#[test]
fn a_literal_suffix_is_told_there_are_none() {
    let text = message("100u8");
    assert!(
        text.contains("`u` is not a valid digit in base 10"),
        "{text}"
    );
    assert!(
        text.contains("Deed has no literal suffixes, so `100u8` should be written `100`"),
        "{text}"
    );
}

// DEED1007, one way.

/// The decision not to have floats is in `design/02-syntax.md`, and before
/// #210 it was nowhere in the compiler. `1.5` came apart into `1`, a stray `.`
/// the parser called a missing expression, and a `5`, so the reader was told
/// about a dot they never thought of as separate.
#[test]
fn a_decimal_point_says_there_are_no_floats() {
    let text = message("1.5");
    assert!(
        text.contains("`1.5` has a decimal point, and there are no float literals"),
        "{text}"
    );
    assert!(text.contains("no literal has this shape"), "{text}");
    assert!(text.contains("counted in its smallest unit"), "{text}");
    assert!(
        text.contains("this is not `1` with a field after it either"),
        "{text}"
    );
}
