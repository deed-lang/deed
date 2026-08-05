//! Every message the parser can produce, read.
//!
//! The parser has twenty-one emission sites and twelve codes. A code is not a
//! message and the ratchet in `crates/deed-driver/tests/codes.rs` asks for one
//! test per code, so one tested shape satisfies it for every message behind the
//! same code. The sentences are what somebody stuck on a file reads; this file
//! reads them.
//!
//! # One test per emission site
//!
//! Every test calls `.under()`, which pins the code constant to the site it
//! came from. A code constant swapped for another fails here and nowhere else;
//! without this the constant can be changed silently and the error the reader
//! sees changes with it.
//!
//! # Recovery
//!
//! P7 says a single root cause should produce a single diagnostic. Several
//! messages in this file exist precisely because the old shape produced a
//! cascade; where that is true the test says how many diagnostics one mistake
//! costs, using `only_error`.

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_lexer::tokenize;
use deed_parser::{codes, parse};

struct Reported {
    code: &'static str,
    text: String,
    underlined: String,
}

impl Reported {
    /// Any substring of the rendered output.
    fn says(&self, needle: &str) -> &Self {
        assert!(
            self.text.contains(needle),
            "expected {needle:?} in:\n{}",
            self.text
        );
        self
    }

    /// The source text the primary caret is drawn over.
    fn underlines(&self, expected: &str) -> &Self {
        assert_eq!(self.underlined, expected, "in:\n{}", self.text);
        self
    }

    /// Which code this diagnostic arrived under.
    fn under(&self, code: &str) -> &Self {
        assert_eq!(self.code, code, "in:\n{}", self.text);
        self
    }
}

fn underlined(sources: &SourceMap, diagnostic: &Diagnostic) -> String {
    let span = diagnostic.primary.span;
    sources.file(diagnostic.file).text()[span.start as usize..span.end as usize].to_string()
}

/// The first error the source produces.
///
/// Asserts clean lexing so that a mistake in a test input shows up as a test
/// construction problem rather than an unexpected message.
fn message(src: &str) -> Reported {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);
    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    let first = parsed
        .diagnostics
        .iter()
        .find(|d| d.is_error())
        .expect("at least one error");
    Reported {
        code: first.code,
        underlined: underlined(&sources, first),
        text: render_human(&sources, first),
    }
}

/// Asserts the source produces exactly one error and returns it.
///
/// Used for the recovery tests: one mistake must cost one diagnostic.
fn only_error(src: &str) -> Reported {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);
    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    let errors: Vec<&Diagnostic> = parsed.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert_eq!(
        errors.len(),
        1,
        "expected exactly one error, got {}:\n{}",
        errors.len(),
        errors
            .iter()
            .map(|d| format!("  {}[{}]: {}", d.severity.as_str(), d.code, d.message))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let first = errors[0];
    Reported {
        code: first.code,
        underlined: underlined(&sources, first),
        text: render_human(&sources, first),
    }
}

// -- DEED2001, an unexpected token -------------------------------------------
//
// UNEXPECTED_TOKEN is used at seven distinct emission sites, each producing a
// different sentence. All seven are tested here with code-pinning so that a
// swap at any one site fails exactly the test for that site.

/// `expect()` is the generic helper. It fires whenever a specific token was
/// required but something else was found, and the message names both.
///
/// Sentence: "expected {expected} while parsing {context}, found {found}"
#[test]
fn expect_names_what_was_expected_and_what_was_found() {
    message("module a\n\ntype Foo Int\n")
        .under(codes::UNEXPECTED_TOKEN)
        .says("expected `=` while parsing a type alias, found")
        .says("expected `=`");
}

/// `expect_ident()` fires when a name is required and something else is there.
///
/// Sentence: "expected a name while parsing {context}, found {found}"
#[test]
fn expect_ident_names_the_context_and_what_was_found() {
    // `module` with no name at all: the parser sees EOF where a name should be.
    only_error("module\n")
        .under(codes::UNEXPECTED_TOKEN)
        .says("expected a name while parsing a module declaration, found end of file")
        .says("expected a name");
}

/// An effect body admits only `fn` signatures. Anything else gets its own
/// sentence rather than the generic "expected a token" one, because the set of
/// things that can appear there is a one-word answer.
///
/// Sentence: "expected an operation signature, found {found}"
#[test]
fn a_non_fn_inside_an_effect_says_what_was_expected() {
    only_error("module a\n\neffect E {\n  x: Int\n}\n")
        .under(codes::UNEXPECTED_TOKEN)
        .says("expected an operation signature, found identifier `x`")
        .says("expected `fn`")
        .says("an effect declares operations and nothing else");
}

/// A handler body admits `state` fields, `fn` implementations and a `finally`
/// block. Anything else gets the same sentence with all three listed, and the
/// shape of a `state` declaration, because a model spent six turns guessing at
/// it from a message that only named the token it wanted.
///
/// Sentence: "expected `state`, `fn` or `finally` in a handler, found {found}"
#[test]
fn a_non_state_fn_inside_a_handler_says_what_was_expected() {
    only_error("module a\n\nhandler H implements E {\n  x: Int\n}\n")
        .under(codes::UNEXPECTED_TOKEN)
        .says("expected `state`, `fn` or `finally` in a handler, found identifier `x`")
        .says("expected `state`, `fn` or `finally`")
        .says("`state count: Int` here, and `with H { count: 0 } { .. }` there");
}

/// A `test` declaration needs a string name. An integer, identifier or
/// anything else gets its own sentence because "expected a string" is less
/// opaque than "expected `Str`".
///
/// Sentence: "expected a test name, found {found}"
#[test]
fn a_non_string_test_name_says_what_was_expected() {
    only_error("module a\n\ntest 42 { }\n")
        .under(codes::UNEXPECTED_TOKEN)
        .says("expected a test name, found integer literal")
        .says("expected a string")
        .says("tests are named with a sentence");
}

/// In primary expression position the parser says "expression" rather than
/// naming the specific token it was reaching for.
///
/// Sentence: "expected an expression, found {found}"
#[test]
fn an_unexpected_token_in_expression_position_says_expression() {
    message("module a\n\nfn f() -> Int {\n  let x = ,\n  0\n}\n")
        .under(codes::UNEXPECTED_TOKEN)
        .says("expected an expression, found `,`")
        .says("expected an expression");
}

/// An operator on a line of its own is reported as a bad expression and also
/// told why: an expression ends at end of line, so this started a new one.
#[test]
fn an_operator_starting_a_new_line_gets_a_note_explaining_why() {
    message("module t\n\nfn f() -> Int {\n  let a = 1\n  * 2\n  a\n}\n")
        .under(codes::UNEXPECTED_TOKEN)
        .says("expected an expression, found `*`")
        .says("an expression ends at the end of a line")
        .says("leave the operator on the line above");
}

/// In pattern position the parser says "pattern" rather than a specific token.
///
/// Sentence: "expected a pattern, found {found}"
#[test]
fn an_unexpected_token_in_pattern_position_says_pattern() {
    message("module a\n\nfn f(n: Int) -> Int {\n  match n {\n    + => 1,\n  }\n}\n")
        .under(codes::UNEXPECTED_TOKEN)
        .says("expected a pattern, found `+`")
        .says("expected a pattern");
}

// -- DEED2002, a file with no `module` declaration ---------------------------

/// Sentence: "every file must begin with a `module` declaration"
#[test]
fn a_file_without_a_module_declaration_says_so() {
    only_error("fn f() -> Int { 0 }\n")
        .under(codes::MISSING_MODULE_DECLARATION)
        .says("every file must begin with a `module` declaration")
        .says("expected `module` here")
        .says("the module path is the file's identity");
}

// -- DEED2003, a token that cannot begin a declaration -----------------------
//
// There are two emission sites: one for a non-keyword token at the top level
// (which also handles spelling corrections), and one for a keyword that begins
// a statement rather than a declaration. Both produce the same primary message
// but only the first adds contextual notes.

/// Site 1: a plain token or word from another language at the top level.
///
/// Sentence: "expected a declaration, found {found}"
#[test]
fn a_non_keyword_at_the_top_level_says_declaration_was_expected() {
    message("module a\n\n42\n")
        .under(codes::EXPECTED_DECLARATION)
        .says("expected a declaration, found integer literal")
        .says("not the start of a declaration");
}

/// Site 2: a keyword that cannot begin a declaration. The note set is smaller
/// because the word is already the right language's word, just in the wrong
/// position.
///
/// Sentence: "expected a declaration, found {found}"
#[test]
fn a_statement_keyword_at_the_top_level_says_declaration_was_expected() {
    only_error("module a\n\nlet x = 1\n")
        .under(codes::EXPECTED_DECLARATION)
        .says("expected a declaration, found keyword `let`")
        .says("not the start of a declaration");
}

// -- DEED2004, a contract clause given twice ---------------------------------

/// Sentence: "`{kw}` appears twice in one contract"
#[test]
fn a_duplicate_contract_clause_names_the_keyword() {
    only_error("module a\n\nfn f()\n  where true,\n  where false\n{ 0 }\n")
        .under(codes::DUPLICATE_CONTRACT_CLAUSE)
        .says("`where` appears twice in one contract")
        .says("second occurrence")
        .says("first one here")
        .says("write all of the obligations in a single clause");
}

// -- DEED2005, an `ensures` outcome that is neither `ok` nor `err` ----------

/// Sentence: "expected `ok` or `err`, found {found}"
#[test]
fn an_invalid_ensures_outcome_names_what_was_expected() {
    only_error("module a\n\nfn f() -> Int\n  ensures nope => true\n{ 0 }\n")
        .under(codes::INVALID_ENSURES_OUTCOME)
        .says("expected `ok` or `err`, found identifier `nope`")
        .says("not an outcome")
        .says("obligations are stated per outcome");
}

// -- DEED2006, contract clauses in the wrong order ---------------------------

/// Sentence: "`{kw}` must come before `{prev}`"
#[test]
fn an_out_of_order_contract_clause_names_both_keywords() {
    only_error("module a\n\nfn f()\n  uses E,\n  where true\n{ 0 }\n")
        .under(codes::CONTRACT_CLAUSE_ORDER)
        .says("`where` must come before `uses`")
        .says("out of order")
        .says("written after this")
        .says("contract clauses are always `where`, then `uses`, then `ensures`");
}

// -- DEED2007, a parameter written without a type ----------------------------
//
// There are two emission sites: one for function parameters and one for closure
// parameters. Both produce the same primary sentence but different primary
// labels and different notes, so both are tested.

/// Site 1: a function or effect operation parameter.
///
/// Sentence: "`{param}` has no type"
#[test]
fn a_function_parameter_without_a_type_says_so() {
    message("module a\n\nfn f(n) -> Int { n }\n")
        .under(codes::MISSING_PARAMETER_TYPE)
        .underlines("n")
        .says("`n` has no type")
        .says("a parameter needs a type")
        .says("a signature is what a reviewer is entitled to stop at");
}

/// Site 2: a closure parameter.
///
/// Same sentence as the function parameter case; the primary label and note
/// differ because a closure is not a boundary a reviewer reads, but the body
/// still has to be checked against something.
///
/// Sentence: "`{param}` has no type"
#[test]
fn a_closure_parameter_without_a_type_says_so() {
    message("module a\n\nfn f() -> Int {\n  let g = |x| x\n  0\n}\n")
        .under(codes::MISSING_PARAMETER_TYPE)
        .says("`x` has no type")
        .says("a closure parameter needs a type")
        .says("the unknown type agrees with everything");
}

// -- DEED2008, a choice variant with a positional payload -------------------

/// Sentence: "`{name}` carries its payload by position"
#[test]
fn a_positional_variant_names_the_variant() {
    only_error("module a\n\nchoice Shape {\n  Circle(Int),\n}\n")
        .under(codes::POSITIONAL_VARIANT)
        .says("`Circle` carries its payload by position")
        .says("a variant's fields are named")
        .says("`Variant { field: Type }`")
        .says("`ok` and `err` are the exception");
}

// -- DEED2009, a word in front of a `let` name that the language has no place for

/// `let mut n = 1` used to produce six cascading messages, none mentioning
/// `mut`. One error instead.
///
/// Sentence: "there is no `{word}`, and a `let` binds a name once"
#[test]
fn a_binding_modifier_names_the_word_and_says_let_binds_once() {
    only_error("module a\n\nfn f() -> Int {\n  let mut n = 1\n  n\n}\n")
        .under(codes::NO_BINDING_MODIFIER)
        .underlines("mut")
        .says("there is no `mut`, and a `let` binds a name once")
        .says("no such word")
        .says("handler's `state` field")
        .says("with sum = 0");
}

// -- DEED2010, a binding written without `let` --------------------------------
//
// Two sites: one for another language's binding keyword (`var`, `const`, `val`,
// `local`), one for a type written in front of the name.

/// Site 1: another language's binding keyword.
///
/// `var n = 1` used to produce two "name not found" messages and no mention of
/// `let`. One error instead.
///
/// Sentence: "there is no `{word}`, and a binding is written `let`"
#[test]
fn a_binding_keyword_names_the_word_and_says_let() {
    only_error("module a\n\nfn f() -> Int {\n  var n = 1\n  n\n}\n")
        .under(codes::BINDING_WITHOUT_LET)
        .says("there is no `var`, and a binding is written `let`")
        .says("no such word")
        .says("binds its name once");
}

/// A `const` or `val` asks for exactly what `let` already is, so it gets no
/// note about mutability.
#[test]
fn a_const_binding_does_not_hear_about_state() {
    only_error("module a\n\nfn f() -> Int {\n  const n = 1\n  n\n}\n")
        .under(codes::BINDING_WITHOUT_LET)
        .says("there is no `const`, and a binding is written `let`");
}

/// Site 2: a type written in front of the name rather than after it.
///
/// `Int n = 1` used to produce two "name not found" messages. One error
/// instead, pointing at what needs to move.
///
/// Sentence: "`{word}` is the type of `{name}`, and a type is written after the name"
#[test]
fn a_type_before_a_name_names_the_type_and_the_name() {
    only_error("module a\n\nfn f() -> Int {\n  Int n = 1\n  n\n}\n")
        .under(codes::BINDING_WITHOUT_LET)
        .says("`Int` is the type of `n`, and a type is written after the name")
        .says("the type comes second")
        .says("let name: Type = value");
}

// -- DEED2011, a range -------------------------------------------------------

/// `for i in 0..10` used to leave the dots in place and produce six messages,
/// none of them about the dots. One error instead.
///
/// Sentence: "there is no range in this language"
#[test]
fn a_range_says_this_language_has_none() {
    only_error("module a\n\nfn f() -> () {\n  for i in 0..10 {\n    i\n  }\n}\n")
        .under(codes::NO_RANGE)
        .says("there is no range in this language")
        .says("no such operator")
        .says("walks a list that already exists")
        .says("`repeat(value, count)`");
}

/// The inclusive form is the same mistake with one more character.
#[test]
fn an_inclusive_range_is_reported_with_the_same_code() {
    only_error("module a\n\nfn f() -> () {\n  for i in 0..=10 {\n    i\n  }\n}\n")
        .under(codes::NO_RANGE)
        .says("there is no range in this language");
}

// -- DEED2012, a cast --------------------------------------------------------

/// `n as String` used to be answered with "cannot find `as` in this scope".
/// The conversion it was asking for is named instead.
///
/// Sentence: "there is no cast in this language"
#[test]
fn a_cast_to_string_names_the_conversion() {
    only_error("module a\n\nfn f(n: Int) -> String {\n  n as String\n}\n")
        .under(codes::NO_CAST)
        .says("there is no cast in this language")
        .says("no such operator")
        .says("a conversion is a call")
        .says("to_string(n)");
}

#[test]
fn a_cast_to_int_names_the_conversion() {
    only_error("module a\n\nfn f(s: String) -> Int {\n  s as Int\n}\n")
        .under(codes::NO_CAST)
        .says("there is no cast in this language")
        .says("to_int(s)")
        .says("gives a `Result`");
}

#[test]
fn a_cast_to_any_other_type_lists_the_available_conversions() {
    only_error("module a\n\nfn f(n: Int) -> Bool {\n  n as Bool\n}\n")
        .under(codes::NO_CAST)
        .says("there is no cast in this language")
        .says("`to_string` and `to_int` are the conversions the prelude has");
}

// -- DEED2014, a detached spawn ----------------------------------------------

/// `spawn(f())` used to be answered as an unknown name `spawn` or a runtime
/// failure. The structured-concurrency decision is stated instead.
///
/// Sentence: "there is no detached spawn in this language"
#[test]
fn a_detached_spawn_names_the_decision() {
    only_error("module a\n\nfn f() -> () {}\n\nfn g() -> () {\n  spawn(f())\n}\n")
        .under(codes::NO_DETACHED_SPAWN)
        .says("there is no detached spawn in this language")
        .says("no such construct")
        .says("tied to the block that started it");
}

// -- recovery ----------------------------------------------------------------
//
// P7: a single root cause produces a single diagnostic. The tests below verify
// the cases where a cascade used to follow before the parser learned to read
// and discard the shape.

/// A missing closing delimiter is the most common mistake. One `}` missing from
/// the end of a function body is one error; with no cascade suppression it was
/// everything from the unclosed brace to the end of the file.
#[test]
fn a_missing_closing_brace_is_one_error() {
    only_error("module a\n\nfn f() -> Int {\n  let x = 1\n").under(codes::UNEXPECTED_TOKEN);
}

/// A positional variant skips its payload whole, so the surrounding choice and
/// any function that follows both survive.
#[test]
fn a_positional_variant_does_not_swallow_what_follows() {
    // The choice keeps the variant and the function after it stays.
    let mut sources = SourceMap::new();
    let file = sources.add(
        "test.deed",
        "module a\n\nchoice C {\n  A(Int),\n}\n\nfn f() -> Int { 0 }\n",
    );
    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors());
    let parsed = parse(file, &lexed.tokens);
    let errors: Vec<&deed_diagnostics::Diagnostic> =
        parsed.diagnostics.iter().filter(|d| d.is_error()).collect();
    assert_eq!(errors.len(), 1, "one positional variant is one error");
    assert_eq!(
        parsed.module.items.len(),
        2,
        "choice and function both survive"
    );
}

/// A range inside a `for` loop used to produce six cascading diagnostics.
#[test]
fn a_range_in_a_for_loop_does_not_cascade() {
    only_error(
        "module a\n\nfn f() -> () {\n  for i in 0..10 {\n    i\n  }\n}\n\nfn g() -> Int { 1 }\n",
    )
    .under(codes::NO_RANGE);
}

/// A binding modifier such as `mut` used to produce six cascading diagnostics.
#[test]
fn a_binding_modifier_does_not_cascade() {
    only_error("module a\n\nfn f() -> Int {\n  let mut n = 1\n  n\n}\n")
        .under(codes::NO_BINDING_MODIFIER);
}

/// `var n = 1` used to produce two "name not found" diagnostics.
#[test]
fn a_binding_without_let_does_not_cascade() {
    only_error("module a\n\nfn f() -> Int {\n  var n = 1\n  n\n}\n")
        .under(codes::BINDING_WITHOUT_LET);
}

/// A literal the lexer could not read used to produce a second message here.
///
/// The lexer hands back what was written instead of an error token, so this
/// pass has an expression and nothing to say. The claim belongs here rather
/// than beside the lexer's own tests: what was wrong with an error token was
/// the message this pass then wrote in the same column.
#[test]
fn a_literal_the_lexer_reported_is_not_reported_again() {
    for literal in ["99999999999999999999", "0x", "100u8", "0b12", "1.5"] {
        let source = format!("module a\n\nfn f() -> Int {{\n  {literal}\n}}\n");
        let mut sources = SourceMap::new();
        let file = sources.add("test.deed", &source);
        let lexed = tokenize(file, sources.file(file).text());
        assert!(lexed.has_errors(), "{literal} should not lex");

        let parsed = parse(file, &lexed.tokens);
        assert!(
            parsed.diagnostics.is_empty(),
            "{literal} was reported twice:\n{}",
            parsed
                .diagnostics
                .iter()
                .map(|d| render_human(&sources, d))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

/// The digits of the smallest `Int` are one literal with the minus in front.
///
/// The lexer says nothing about them, because whether the minus is the unary
/// one is a question about the grammar. So this pass is the only one that
/// reports them, and only when nothing put a minus there.
#[test]
fn the_smallest_int_is_a_literal_and_the_digits_alone_are_not() {
    for (source, want) in [
        (
            "module a\n\nfn f() -> Int {\n  -9223372036854775808\n}\n",
            None,
        ),
        (
            "module a\n\nfn f() -> Int {\n  9223372036854775808\n}\n",
            Some(deed_lexer::codes::INTEGER_OUT_OF_RANGE),
        ),
        (
            "module a\n\nfn f(n: Int) -> Int {\n  n - 9223372036854775808\n}\n",
            Some(deed_lexer::codes::INTEGER_OUT_OF_RANGE),
        ),
    ] {
        let mut sources = SourceMap::new();
        let file = sources.add("test.deed", source);
        let lexed = tokenize(file, sources.file(file).text());
        assert!(!lexed.has_errors(), "the lexer has nothing to say here");

        let parsed = parse(file, &lexed.tokens);
        let codes: Vec<&str> = parsed.diagnostics.iter().map(|d| d.code).collect();
        match want {
            Some(code) => assert_eq!(codes, vec![code], "{source}"),
            None => assert!(codes.is_empty(), "{source} said {codes:?}"),
        }
    }
}

/// The same for a string that never closes.
#[test]
fn an_unterminated_string_is_not_reported_again() {
    let source = "module a\n\nfn f() -> String {\n  \"oops\n}\n";
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", source);
    let lexed = tokenize(file, sources.file(file).text());
    assert!(lexed.has_errors(), "the source should not lex");

    let parsed = parse(file, &lexed.tokens);
    assert!(
        parsed.diagnostics.is_empty(),
        "reported twice:\n{}",
        parsed
            .diagnostics
            .iter()
            .map(|d| render_human(&sources, d))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
