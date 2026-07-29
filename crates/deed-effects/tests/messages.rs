//! Every message the effect checker can produce, read.
//!
//! `crates/deed-driver/tests/codes.rs` matches on a code's name, so one tested
//! shape satisfies it for a code with several messages behind it. The effect
//! checker has eleven emission sites and eleven distinct sentences, and most of
//! them had never been rendered by a test. `effects.rs` tested eight of the
//! eleven codes but read the words of only some of the messages beneath them.
//!
//! # What is tested here and what is tested in effects.rs
//!
//! Sentences already read in `effects.rs`:
//!
//! - `DEED5001` normal: "`{name}` performs {described} without declaring it" —
//!   `performing_an_undeclared_effect_is_an_error`
//! - `DEED5002`: "{described} is declared but never performed" —
//!   `declaring_an_effect_that_is_never_performed_is_an_error`
//! - `DEED5003` imported: "`{name}` is {kind}, not an effect" and its note —
//!   `an_imported_name_that_is_not_an_effect_is_rejected_the_same_way`
//! - `DEED5003` local: "`{name}` is a {kind}, not an effect" —
//!   `a_uses_entry_naming_something_that_is_not_an_effect_is_rejected`
//! - `DEED5004` local `sys.*`: "grants everything that capability carries" —
//!   `granting_everything_a_capability_carries_is_reported`
//! - `DEED5004` local bare `sys`: "is a value, not an effect" —
//!   `a_capability_named_in_a_row_without_the_star_is_reported`
//! - `DEED5004` call site: "has a row that is not checked, so this call is not
//!   either" — `an_uncheckable_row_does_not_make_its_callers_look_pure`
//! - `DEED5006`: "`{effect}` is not in scope here" and the label —
//!   `an_effect_that_cannot_be_named_here_says_where_to_import_it`
//!
//! What is here because it was not read anywhere:
//!
//! - `DEED5001` Diverge: "`{name}` can reach itself, so it may not return"
//! - `DEED5003` local note: "a `uses` clause names effects declared with `effect`"
//! - `DEED5005`: "this test performs {described} with no handler for it"
//! - `DEED5006` note: "a function cannot declare an effect it has no name for"
//! - `DEED5007`: both shapes of "this performs {performed}, and {room}"
//! - `DEED5008`: "`{name}` is a row variable, and this is not a place a caller
//!   could work out what it stands for"
//! - `DEED5009`: "this performs {described}, and `{name}` does not mention
//!   `{effect}`"
//!
//! # Wording and code are two claims
//!
//! [`Checked::under`] appears on the two sites that had no test at all —
//! `DEED5007` and `DEED5008` — and not on the rest, which is deliberate.
//! Swapping the code constant at each of the eleven emission sites in turn
//! found two whose wording was read and whose code was held by nothing at all
//! in `deed-effects/tests/`, so those two also say which code they arrive
//! under. The nine remaining sites are already pinned by `effects.rs`.

use std::collections::HashMap;

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_effects::{analyse, codes};
use deed_lexer::tokenize;
use deed_parser::parse;
use deed_resolve::{Universe, resolve};

/// One diagnostic from the effect checker, as a reader meets it.
struct Checked {
    code: &'static str,
    text: String,
}

impl Checked {
    /// What the reader sees, anywhere in the rendering.
    fn says(&self, needle: &str) -> &Self {
        assert!(
            self.text.contains(needle),
            "expected `{needle}` in:\n{}",
            self.text
        );
        self
    }

    /// Which code it arrived under.
    fn under(&self, code: &str) -> &Self {
        assert_eq!(self.code, code, "in:\n{}", self.text);
        self
    }
}

/// The one diagnostic `src` produces.
fn message(src: &str) -> Checked {
    message_in(src, &Universe::new())
}

fn message_in(src: &str, universe: &Universe) -> Checked {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);

    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "test source should parse cleanly");
    let resolved = resolve(file, &parsed.module, universe);
    assert!(!resolved.has_errors(), "test source should resolve cleanly");

    let analysis = analyse(
        file,
        &parsed.module,
        &resolved.resolutions,
        &HashMap::new(),
        &HashMap::new(),
    );

    assert_eq!(
        analysis.diagnostics.len(),
        1,
        "expected exactly one diagnostic:\n{}",
        rendered(&sources, &analysis.diagnostics)
    );
    let d = &analysis.diagnostics[0];
    Checked {
        code: d.code,
        text: render_human(&sources, d),
    }
}

/// The one diagnostic `src` produces, after also running the type checker.
///
/// `DEED5007` is only reachable when the type checker has filled in
/// `row_required`, so this is needed for those tests alone.
fn message_checked(src: &str) -> Checked {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);

    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "test source should parse cleanly");
    let resolved = resolve(file, &parsed.module, &Universe::new());
    assert!(!resolved.has_errors(), "test source should resolve cleanly");

    let typed = deed_typeck::check(
        file,
        &parsed.module,
        &resolved.resolutions,
        &deed_typeck::World::new(),
    );

    let analysis = analyse(
        file,
        &parsed.module,
        &resolved.resolutions,
        typed.types.row_required(),
        &typed.types.function_rows(),
    );

    assert_eq!(
        analysis.diagnostics.len(),
        1,
        "expected exactly one diagnostic:\n{}",
        rendered(&sources, &analysis.diagnostics)
    );
    let d = &analysis.diagnostics[0];
    Checked {
        code: d.code,
        text: render_human(&sources, d),
    }
}

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

// -- DEED5001, Diverge -------------------------------------------------------

/// The second shape, where the undeclared effect is `Diverge` rather than a
/// real operation. The normal shape reads "`{name}` performs {described}
/// without declaring it" and is tested in `effects.rs`.
#[test]
fn a_recursive_function_without_diverge_says_it_can_reach_itself() {
    message(
        "module a\n\n\
         fn factorial(n: Int) -> Int {\n\
         \x20 if n <= 1 {\n\
         \x20   1\n\
         \x20 } else {\n\
         \x20   n * factorial(n - 1)\n\
         \x20 }\n\
         }\n",
    )
    .says("`factorial` can reach itself, so it may not return")
    .says("nothing here proves termination, so any call cycle needs `Diverge`");
}

// -- DEED5003, local note ----------------------------------------------------

/// The note on the local shape. The sentence "is a {kind}, not an effect" is
/// read in `effects.rs`; this pins the note that was left out.
#[test]
fn a_non_effect_in_a_uses_clause_carries_a_note_about_the_keyword() {
    message(
        "module a\n\n\
         record Money { units: Int }\n\n\
         fn f() -> Int\n  uses Money,\n{ 0 }\n",
    )
    .says("a `uses` clause names effects declared with `effect`");
}

// -- DEED5005, unhandled effect -----------------------------------------------

/// The sentence, which names the test and the effect, and the note that says
/// what to do instead. The test in `effects.rs` only checks that the effect
/// name appears.
#[test]
fn an_unhandled_effect_says_what_performs_it_and_that_there_is_no_handler() {
    message(
        "module a\n\n\
         effect Ledger {\n\
         \x20 fn balance(id: Int) -> Int\n\
         }\n\n\
         test \"unhandled\" {\n\
         \x20 Ledger.balance(1)\n\
         }\n",
    )
    .says("this test performs `Ledger.balance` with no handler for it")
    .says("wrap the calls in a `with` block naming a handler for the effect");
}

// -- DEED5006, note ----------------------------------------------------------

/// The note. The sentence "`{effect}` is not in scope here" and the label are
/// read in `effects.rs`; this pins the note that was left out.
#[test]
fn an_effect_that_cannot_be_named_carries_a_note_about_declaring_it() {
    let logger = "module logger\n\n\
         effect Log {\n\
         \x20 fn note(message: String) -> ()\n\
         }\n\n\
         fn shout(message: String) -> Int\n\
         \x20 uses Log.note,\n\
         { Log.note(message)\n  1 }\n";

    message_in(
        "module a\n\nuse logger.{shout}\n\nfn f() -> Int { shout(\"hi\") }\n",
        &universe_of(&[logger]),
    )
    .says("a function cannot declare an effect it has no name for, and a row it cannot declare is one it cannot keep");
}

// -- DEED5007, impure function value -----------------------------------------

/// The first shape: the function type the value is crossing into has no row at
/// all, so any effect is too many.
#[test]
fn an_impure_closure_passed_where_a_pure_type_is_expected_says_no_row_was_promised() {
    message_checked(
        "module a\n\n\
         effect Log {\n\
         \x20 fn note(message: String) -> ()\n\
         }\n\n\
         fn apply(f: Fn(Int) -> Int, n: Int) -> Int { f(n) }\n\n\
         fn go(n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         {\n\
         \x20 apply(|x: Int| { Log.note(\"hi\") x }, n)\n\
         }\n",
    )
    .under(codes::IMPURE_FUNCTION_VALUE)
    .says("a function type with no row promises nothing")
    .says("write the effect into the function type, as in `Fn(Int) uses Log.note -> Int`");
}

/// The second shape: the function type wrote down a row, and the value
/// performs something it does not leave room for.
#[test]
fn an_impure_closure_passed_where_a_typed_row_is_expected_says_what_there_was_room_for() {
    message_checked(
        "module a\n\n\
         effect Log {\n\
         \x20 fn note(message: String) -> ()\n\
         \x20 fn warn(message: String) -> ()\n\
         }\n\n\
         fn apply(f: Fn(Int) uses Log.note -> Int, n: Int) -> Int\n\
         \x20 uses Log.note,\n\
         { f(n) }\n\n\
         fn go(n: Int) -> Int\n\
         \x20 uses Log,\n\
         {\n\
         \x20 apply(|x: Int| { Log.warn(\"hi\") x }, n)\n\
         }\n",
    )
    .under(codes::IMPURE_FUNCTION_VALUE)
    .says("this function type leaves room only for `Log.note`")
    .says("write the effect into the function type, as in `Fn(Int) uses Log.note -> Int`");
}

// -- DEED5008, misplaced row variable ----------------------------------------

/// A row variable in a return type: the only place a call site cannot fill it
/// in from. The valid position is the row of a parameter whose type is a
/// function type; written anywhere else it reaches a caller as an effect the
/// caller has no name for.
#[test]
fn a_row_variable_in_a_return_type_names_what_the_sentence_says() {
    message(
        "module a\n\n\
         fn wrap<uses r>(f: Fn(Int) uses r -> Int) -> Fn(Int) uses r -> Int\n\
         { f }\n",
    )
    .under(codes::MISPLACED_ROW_VARIABLE)
    .says("`r` is a row variable, and this is not a place a caller could work out what it stands for")
    .says("nothing at the call site says what this is")
    .says("a row variable stands for whatever a callback performs, so it belongs in the row of a parameter that is one");
}

// -- DEED5009, contract effect not declared ----------------------------------

/// The sentence names the operation, the function, and the effect the
/// signature is silent about. The notes say why the signature has to mention
/// it and why naming the effect is enough.
#[test]
fn a_contract_performing_an_undeclared_effect_names_the_function_and_the_gap() {
    message(
        "module a\n\n\
         effect Ledger {\n\
         \x20 fn balance(id: Int) -> Int\n\
         }\n\n\
         fn f() -> Int\n\
         \x20 ensures ok => Ledger.balance(1) > 0,\n\
         { 0 }\n",
    )
    .says("this performs `Ledger.balance`, and `f` does not mention `Ledger`")
    .says("`Ledger` is not in the `uses` clause")
    .says("the signature a caller reads")
    .says("a handler is installed by the caller, and the signature is the only place a caller learns one is needed")
    .says("naming the effect is enough");
}

// -- helpers -----------------------------------------------------------------

fn universe_of(modules: &[&str]) -> Universe {
    let mut universe = Universe::new();
    let mut sources = SourceMap::new();
    for (index, source) in modules.iter().enumerate() {
        let file = sources.add(format!("dep{index}.deed"), *source);
        let lexed = tokenize(file, sources.file(file).text());
        let parsed = parse(file, &lexed.tokens);
        universe.add(&parsed.module);
    }
    universe
}
