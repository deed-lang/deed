//! Name resolution behaviour.
//!
//! The interesting cases are the ambiguities the parser refused to settle, and
//! the diagnostics, since "cannot find X, did you mean Y" is the highest value
//! message in the compiler for what it costs.

use std::collections::HashSet;

use deed_ast::Item;
use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_lexer::tokenize;
use deed_parser::parse;
use deed_resolve::{DefKind, Dot, Resolutions, Resolved, Universe, codes, resolve};

fn resolve_source(src: &str) -> (SourceMap, deed_ast::Module, Resolved) {
    resolve_source_in(src, &Universe::new())
}

fn resolve_source_in(src: &str, universe: &Universe) -> (SourceMap, deed_ast::Module, Resolved) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);
    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "test source should parse cleanly");
    let resolved = resolve(file, &parsed.module, universe);
    (sources, parsed.module, resolved)
}

/// A universe holding each of `modules`, parsed from source.
///
/// Any test that touches an import needs something on the other side of it,
/// which is the whole point of this pass: an import with nothing behind it is
/// an error now rather than a name nobody checks.
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

fn resolve_ok(src: &str) -> (SourceMap, deed_ast::Module, Resolutions) {
    let (sources, module, resolved) = resolve_source(src);
    if !resolved.diagnostics.is_empty() {
        let rendered: Vec<String> = resolved
            .diagnostics
            .iter()
            .map(|d| render_human(&sources, d))
            .collect();
        panic!("expected a clean resolve:\n{}", rendered.join("\n"));
    }
    (sources, module, resolved.resolutions)
}

fn codes_of(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.code).collect()
}

/// Span of the nth occurrence of `needle`, counting from zero.
fn span_of(src: &str, needle: &str, occurrence: usize) -> deed_diagnostics::Span {
    let mut start = 0;
    for _ in 0..occurrence {
        let found = src[start..]
            .find(needle)
            .unwrap_or_else(|| panic!("fewer than {} occurrences of {needle:?}", occurrence + 1));
        start += found + needle.len();
    }
    let found = src[start..]
        .find(needle)
        .unwrap_or_else(|| panic!("fewer than {} occurrences of {needle:?}", occurrence + 1));
    let begin = start + found;
    deed_diagnostics::Span::new(begin as u32, (begin + needle.len()) as u32)
}

// -- the worked example ----------------------------------------------------

#[test]
fn the_worked_example_resolves_cleanly() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.deed");
    let source = std::fs::read_to_string(path).expect("examples/transfer.deed should exist");

    let mut sources = SourceMap::new();
    let file = sources.add("examples/transfer.deed", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors());

    let resolved = resolve(file, &parsed.module, &Universe::new());
    if !resolved.diagnostics.is_empty() {
        let rendered: Vec<String> = resolved
            .diagnostics
            .iter()
            .map(|d| render_human(&sources, d))
            .collect();
        panic!(
            "the worked example should resolve cleanly:\n{}",
            rendered.join("\n")
        );
    }
}

/// P1 says a function body should be verifiable from its own signature.
///
/// This is the first pass that can put a number on that claim, so it does.
/// The number is not asserted tightly, since the point is to notice if it ever
/// starts climbing rather than to freeze it where it happens to be today.
#[test]
fn the_worked_example_has_a_small_context_radius() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.deed");
    let source = std::fs::read_to_string(path).unwrap();

    let mut sources = SourceMap::new();
    let file = sources.add("transfer.deed", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, &Universe::new());

    let function = parsed
        .module
        .items
        .iter()
        .find_map(|item| match item {
            Item::Function(f) if f.sig.name.name == "transfer" => Some(f),
            _ => None,
        })
        .expect("transfer should be there");

    let body = function.body.span;
    let signature = function
        .sig
        .span
        .to(function.contract.span.unwrap_or(function.sig.span));

    let mut local = 0usize;
    let mut external = HashSet::new();

    for (mention, def) in resolved.resolutions.names() {
        if !body.contains(mention.start) {
            continue;
        }
        let data = resolved.resolutions.def(def);
        // Anything declared in the signature, the contract, or the body itself
        // is inside the reader's field of view.
        if signature.contains(data.span.start) || body.contains(data.span.start) {
            local += 1;
        } else {
            external.insert(data.name.clone());
        }
    }

    assert!(local > 0, "the body should reference its own parameters");
    // The bound moved from 12 to 20 when the example stopped importing things.
    // That is worth being precise about rather than quietly retuning.
    //
    // The number grew because more names now resolve, not because the body got
    // more entangled. `Entry`, `Receipt`, `Debit` and the error variants used
    // to come from modules that could not be loaded, so they resolved to
    // nothing and were never counted. The body reaches for exactly what it
    // always did.
    //
    // Which exposes a weakness in the measurement: it counts every module level
    // name, including types the signature already mentions and variants of
    // those types. Reaching for `InsufficientFunds` when the signature says
    // `TransferError` is not a context radius problem. A sharper metric would
    // only count names unreachable from the signature, and that is harder than
    // it sounds. Left blunt on purpose, because a metric tuned until it passes
    // measures nothing.
    assert!(
        external.len() <= 20,
        "context radius is growing: the body reaches {} names declared outside it: {:?}",
        external.len(),
        external
    );
}

// -- the ambiguities the parser left open ----------------------------------

#[test]
fn a_dot_after_a_local_is_a_field_access() {
    let src = "module a\n\nrecord R { x: Int }\n\nfn f(r: R) -> Int { r.x }\n";
    let (_, _, resolutions) = resolve_ok(src);

    let dot = span_of(src, "x", 1);
    assert_eq!(
        resolutions.dot(dot),
        Some(Dot::Field),
        "`r.x` should defer to the type checker"
    );
}

#[test]
fn a_dot_after_an_imported_record_is_left_alone() {
    // A record's fields are the type checker's business, and it does not know
    // about the other module's types yet, so the `.` is still classified as
    // foreign and nothing is asserted about it.
    let src = "module a\n\nuse other.{Thing}\n\nfn f() -> Int { Thing.whatever }\n";
    let universe = universe_of(&["module other\n\nrecord Thing { whatever: Int }\n"]);
    let (sources, _, resolved) = resolve_source_in(src, &universe);
    assert!(
        resolved.diagnostics.is_empty(),
        "{}",
        resolved
            .diagnostics
            .iter()
            .map(|d| render_human(&sources, d))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let dot = span_of(src, "whatever", 0);
    assert_eq!(
        resolved.resolutions.dot(dot),
        Some(Dot::Foreign),
        "a member of another module's type is not resolved here"
    );
}

#[test]
fn an_operation_an_imported_effect_does_not_have_is_an_error() {
    // The half that is checkable now. An effect's operations are part of its
    // declaration, so a typo in one crosses the module boundary and gets
    // caught, which it never did before.
    let src = "module a\n\nuse other.{Ledger}\n\nfn f() -> Int\n  uses Ledger.balence,\n{ 0 }\n";
    let universe =
        universe_of(&["module other\n\neffect Ledger {\n  fn balance(id: Int) -> Int\n}\n"]);
    let (sources, _, resolved) = resolve_source_in(src, &universe);

    assert_eq!(
        codes_of(&resolved.diagnostics),
        vec![codes::UNKNOWN_MEMBER],
        "{}",
        resolved
            .diagnostics
            .iter()
            .map(|d| render_human(&sources, d))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(render_human(&sources, &resolved.diagnostics[0]).contains("balance"));
}

#[test]
fn importing_from_a_module_that_is_not_there_is_an_error() {
    let (_, _, resolved) = resolve_source("module a\n\nuse other.{Thing}\n\nfn f() -> Thing {}\n");
    assert!(
        codes_of(&resolved.diagnostics).contains(&codes::UNKNOWN_MODULE),
        "an import with nothing behind it used to be accepted silently"
    );
}

#[test]
fn importing_a_name_the_module_does_not_declare_is_an_error() {
    let universe = universe_of(&["module other\n\nrecord Thing { n: Int }\n"]);
    let (sources, _, resolved) = resolve_source_in(
        "module a\n\nuse other.{Thng}\n\nfn f() -> Thng {}\n",
        &universe,
    );

    assert!(
        codes_of(&resolved.diagnostics).contains(&codes::UNKNOWN_EXPORT),
        "{}",
        resolved
            .diagnostics
            .iter()
            .map(|d| render_human(&sources, d))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let text = render_human(&sources, &resolved.diagnostics[0]);
    assert!(text.contains("declares a `Thing`"), "{text}");
}

#[test]
fn a_test_block_is_not_exported() {
    // Nothing outside a module can name its tests, because a test is not part
    // of what the module offers.
    let universe = universe_of(&["module other\n\ntest \"t\" {\n  assert true\n}\n"]);
    let (_, _, resolved) = resolve_source_in("module a\n\nuse other.{t}\n", &universe);
    assert!(codes_of(&resolved.diagnostics).contains(&codes::UNKNOWN_EXPORT));
}

#[test]
fn a_dot_after_a_local_effect_resolves_to_the_operation() {
    let src = "module a\n\n\
               effect Ledger {\n  fn balance(id: Int) -> Int\n}\n\n\
               fn f() -> Int\n  uses Ledger.balance,\n{ 0 }\n";
    let (_, _, resolutions) = resolve_ok(src);

    let mention = span_of(src, "balance", 1);
    let def = resolutions
        .resolution(mention)
        .expect("`Ledger.balance` should resolve");
    assert_eq!(resolutions.def(def).kind, DefKind::EffectOp);
}

#[test]
fn an_unknown_operation_on_a_known_effect_is_reported() {
    let (sources, _, resolved) = resolve_source(
        "module a\n\neffect Ledger {\n  fn balance(id: Int) -> Int\n}\n\n\
         fn f() -> Int\n  uses Ledger.teleport,\n{ 0 }\n",
    );
    assert_eq!(codes_of(&resolved.diagnostics), vec![codes::UNKNOWN_MEMBER]);
    assert!(render_human(&sources, &resolved.diagnostics[0]).contains("has no member `teleport`"));
}

#[test]
fn a_lowercase_pattern_binds_and_an_uppercase_one_must_exist() {
    let src = "module a\n\n\
               choice E { Empty, Full { count: Int } }\n\n\
               fn f(e: E) -> Int {\n\
               \x20 match e {\n\
               \x20   Empty => 0,\n\
               \x20   Full { count } => count,\n\
               \x20 }\n\
               }\n";
    let (_, _, resolutions) = resolve_ok(src);

    // `Empty` is a reference to the variant.
    let empty = span_of(src, "Empty", 1);
    let def = resolutions.resolution(empty).expect("Empty should resolve");
    assert_eq!(resolutions.def(def).kind, DefKind::Variant);

    // `count` in the pattern is a new binding, not a reference to anything.
    let binding = span_of(src, "count", 1);
    let def = resolutions
        .resolution(binding)
        .expect("the binding should be recorded");
    assert_eq!(resolutions.def(def).kind, DefKind::Local);
}

#[test]
fn an_unknown_uppercase_pattern_is_an_error_rather_than_a_binding() {
    let (_, _, resolved) = resolve_source(
        "module a\n\nchoice E { Empty }\n\nfn f(e: E) -> Int {\n  match e {\n    Emty => 0,\n  }\n}\n",
    );
    assert_eq!(codes_of(&resolved.diagnostics), vec![codes::UNKNOWN_NAME]);
    // Without the capitalisation rule this would silently become a binding that
    // matches everything, which is a bug nothing would ever mention.
    let fix = resolved.diagnostics[0].fix.as_ref().unwrap();
    assert_eq!(fix.edits[0].replacement, "Empty");
}

// -- ordering and scoping --------------------------------------------------

#[test]
fn declaration_order_does_not_matter() {
    resolve_ok("module a\n\nfn first() -> Int { second() }\n\nfn second() -> Int { 0 }\n");
}

#[test]
fn variants_are_usable_unqualified() {
    let src = "module a\n\nchoice E { Bare, Full { n: Int } }\n\nfn f() -> E { Full { n: 1 } }\n";
    let (_, _, resolutions) = resolve_ok(src);
    let mention = span_of(src, "Full", 1);
    let def = resolutions.resolution(mention).unwrap();
    assert_eq!(resolutions.def(def).kind, DefKind::Variant);
}

#[test]
fn value_is_in_scope_inside_a_refinement_and_nowhere_else() {
    resolve_ok("module a\n\ntype Positive = Int where value > 0\n");

    let (_, _, resolved) = resolve_source("module a\n\nfn f() -> Int { value }\n");
    assert_eq!(codes_of(&resolved.diagnostics), vec![codes::UNKNOWN_NAME]);
}

#[test]
fn an_initialiser_sees_the_outer_binding() {
    // `let x = x` reads the parameter, and then the new binding shadows it,
    // which is why this reports shadowing rather than an unknown name.
    let (_, _, resolved) =
        resolve_source("module a\n\nfn f(x: Int) -> Int {\n  let x = x\n  x\n}\n");
    assert_eq!(
        codes_of(&resolved.diagnostics),
        vec![codes::SHADOWED_BINDING]
    );
}

#[test]
fn handler_state_is_visible_to_its_operations() {
    resolve_ok(
        "module a\n\n\
         effect Ledger {\n  fn balance(id: Int) -> Int\n}\n\n\
         handler InMemory implements Ledger {\n\
         \x20 state holdings: Int\n\n\
         \x20 fn balance(id) -> Int { holdings }\n\
         }\n",
    );
}

// -- diagnostics -----------------------------------------------------------

#[test]
fn an_unknown_name_suggests_the_closest_one_in_scope() {
    let (sources, _, resolved) =
        resolve_source("module a\n\nfn balance() -> Int { 0 }\n\nfn f() -> Int { balanse() }\n");
    assert_eq!(codes_of(&resolved.diagnostics), vec![codes::UNKNOWN_NAME]);

    let fix = resolved.diagnostics[0].fix.as_ref().unwrap();
    assert_eq!(fix.edits[0].replacement, "balance");
    assert_eq!(
        fix.applicability,
        deed_diagnostics::Applicability::MachineApplicable
    );
    assert!(render_human(&sources, &resolved.diagnostics[0]).contains("cannot find `balanse`"));
}

/// The suggester works on edit distance, so a name nobody here could have
/// meant used to be answered with whatever short name happened to be nearby.
/// `null` was told there is a `f` in scope.
#[test]
fn a_name_from_another_language_is_answered_instead_of_guessed_at() {
    let (sources, _, resolved) = resolve_source("module a\n\nfn f() -> Int { null }\n");
    assert_eq!(codes_of(&resolved.diagnostics), vec![codes::UNKNOWN_NAME]);

    let text = render_human(&sources, &resolved.diagnostics[0]);
    assert!(text.contains("there is no empty value"), "{text}");
    assert!(!text.contains("there is a `f` in scope"), "{text}");
}

#[test]
fn a_word_for_an_operator_is_answered_with_the_operator() {
    for (word, op) in [("and", "&&"), ("or", "||"), ("not", "!")] {
        let source = format!("module a\n\nfn f(a: Bool, b: Bool) -> Bool {{ a {word} b }}\n");
        let (sources, _, resolved) = resolve_source(&source);
        let named = resolved
            .diagnostics
            .iter()
            .find(|d| d.message.contains(&format!("cannot find `{word}`")))
            .unwrap_or_else(|| panic!("`{word}` should not resolve"));

        let text = render_human(&sources, named);
        assert!(text.contains(&format!("this is spelled `{op}`")), "{text}");
        assert_eq!(named.fix.as_ref().expect("a fix").edits[0].replacement, op);
    }
}

/// Nothing here shadows anything. A name that resolves never reaches the
/// place this is decided, so somebody who declared a function called `and`
/// still has it.
#[test]
fn a_declared_name_is_not_taken_for_a_word_from_elsewhere() {
    let (_, _, resolved) = resolve_source(
        "module a\n\nfn and(a: Bool, b: Bool) -> Bool { a }\n\nfn f() -> Bool { and(true, false) }\n",
    );
    assert!(
        !codes_of(&resolved.diagnostics).contains(&codes::UNKNOWN_NAME),
        "{:?}",
        codes_of(&resolved.diagnostics)
    );
}

#[test]
fn a_name_with_no_close_match_gets_no_suggestion() {
    let (_, _, resolved) =
        resolve_source("module a\n\nfn balance() -> Int { 0 }\n\nfn f() -> Int { zzzzzzz() }\n");
    assert!(
        resolved.diagnostics[0].fix.is_none(),
        "a wrong fix is worse than none, because it gets applied"
    );
}

/// Two letters in the wrong order, in a name too short to have afforded it
/// under plain Levenshtein.
///
/// The threshold is one for anything up to five characters and a transposition
/// costs two edits under that metric, so `bmup` used to get no suggestion at
/// all while `lenght` got one purely because `length` is longer. Short names
/// are most of the names in a prelude, so that was the common case going
/// unanswered.
#[test]
fn two_letters_in_the_wrong_order_still_finds_the_name() {
    let (_, _, resolved) =
        resolve_source("module a\n\nfn bump() -> Int { 0 }\n\nfn f() -> Int { bmup() }\n");
    assert_eq!(codes_of(&resolved.diagnostics), vec![codes::UNKNOWN_NAME]);

    let fix = resolved.diagnostics[0].fix.as_ref().expect("a suggestion");
    assert_eq!(fix.edits[0].replacement, "bump");
    assert_eq!(
        fix.applicability,
        deed_diagnostics::Applicability::MachineApplicable
    );
}

/// Being closer to human mistakes is not the same as being looser. A name two
/// transpositions away is still two edits and still gets nothing.
#[test]
fn one_transposition_is_not_a_licence_for_two() {
    let (_, _, resolved) =
        resolve_source("module a\n\nfn abcd() -> Int { 0 }\n\nfn f() -> Int { badc() }\n");
    assert_eq!(codes_of(&resolved.diagnostics), vec![codes::UNKNOWN_NAME]);
    assert!(resolved.diagnostics[0].fix.is_none());
}

#[test]
fn an_ambiguous_suggestion_is_withheld() {
    // `fee` is one edit from both `fee1`-alikes, so neither wins.
    let (_, _, resolved) = resolve_source(
        "module a\n\nfn fees() -> Int { 0 }\n\nfn feed() -> Int { 0 }\n\nfn f() -> Int { fee() }\n",
    );
    assert_eq!(codes_of(&resolved.diagnostics), vec![codes::UNKNOWN_NAME]);
    assert!(resolved.diagnostics[0].fix.is_none());
}

#[test]
fn a_duplicate_declaration_points_at_both() {
    let (sources, _, resolved) =
        resolve_source("module a\n\nfn thing() -> Int { 0 }\n\nrecord thing { x: Int }\n");
    assert_eq!(
        codes_of(&resolved.diagnostics),
        vec![codes::DUPLICATE_DEFINITION]
    );
    let rendered = render_human(&sources, &resolved.diagnostics[0]);
    assert!(rendered.contains("redeclared as a record"), "{rendered}");
    assert!(
        rendered.contains("first declared as a function"),
        "{rendered}"
    );
}

#[test]
fn an_unused_import_is_a_warning_not_an_error() {
    let universe =
        universe_of(&["module other\n\nrecord Used { n: Int }\n\nrecord Spare { n: Int }\n"]);
    let (_, _, resolved) = resolve_source_in(
        "module a\n\nuse other.{Used, Spare}\n\nfn f() -> Used { }\n",
        &universe,
    );
    assert_eq!(codes_of(&resolved.diagnostics), vec![codes::UNUSED_IMPORT]);
    assert!(!resolved.has_errors());
    assert!(resolved.diagnostics[0].message.contains("Spare"));
}

// -- a name nobody reads -------------------------------------------------

#[test]
fn a_let_nobody_reads_is_a_warning() {
    let (sources, _, resolved) =
        resolve_source("module a\n\nfn f() -> Int {\n  let spare = 1\n  2\n}\n");
    assert_eq!(codes_of(&resolved.diagnostics), vec![codes::UNUSED_BINDING]);
    assert!(!resolved.has_errors());
    let rendered = render_human(&sources, &resolved.diagnostics[0]);
    assert!(rendered.contains("nothing reads `spare`"), "{rendered}");
    assert!(rendered.contains("write `_spare`"), "{rendered}");
}

#[test]
fn a_leading_underscore_says_it_was_meant() {
    let (_, _, resolved) =
        resolve_source("module a\n\nfn f() -> Int {\n  let _spare = 1\n  2\n}\n");
    assert!(resolved.diagnostics.is_empty());
}

/// A guess, and about intent rather than spelling: the other answer is that
/// something was supposed to read this and reads the wrong thing instead.
#[test]
fn the_underscore_is_offered_as_a_fix_and_never_applied() {
    let (_, _, resolved) = resolve_source("module a\n\nfn f() -> Int {\n  let spare = 1\n  2\n}\n");
    let fix = resolved.diagnostics[0]
        .fix
        .as_ref()
        .expect("the warning should carry a fix");
    assert_eq!(
        fix.applicability,
        deed_diagnostics::Applicability::MaybeIncorrect
    );
    assert_eq!(fix.edits.len(), 1);
    assert_eq!(fix.edits[0].replacement, "_spare");
    assert_eq!(
        span_of(
            "module a\n\nfn f() -> Int {\n  let spare = 1\n  2\n}\n",
            "spare",
            0
        ),
        fix.edits[0].span
    );
}

#[test]
fn a_binding_read_once_is_read() {
    let (_, _, resolved) = resolve_source("module a\n\nfn f() -> Int {\n  let n = 1\n  n\n}\n");
    assert!(resolved.diagnostics.is_empty());
}

/// A pattern is there to match, so its binders name what the shape holds
/// whether or not the arm goes on to look. `err(why)` reads better than
/// `err(_)` and neither one is a mistake.
#[test]
fn a_pattern_binder_is_left_alone() {
    let (_, _, resolved) = resolve_source(
        "module a\n\nfn f(r: Result<Int, String>) -> Int {\n  match r {\n    ok(n) => n,\n    err(why) => 0,\n  }\n}\n",
    );
    assert!(resolved.diagnostics.is_empty());
}

/// Same reason one level up: the shape of a parameter list is the signature,
/// and a handler's signature belongs to the effect it implements.
#[test]
fn a_parameter_is_left_alone() {
    let (_, _, resolved) = resolve_source("module a\n\nfn f(n: Int) -> Int { 0 }\n");
    assert!(resolved.diagnostics.is_empty());
}

#[test]
fn a_for_binder_is_left_alone() {
    let (_, _, resolved) = resolve_source(
        "module a\n\nfn f(ns: List<Int>) -> Int {\n  for n in ns with count = 0 {\n    count + 1\n  }\n}\n",
    );
    assert!(resolved.diagnostics.is_empty());
}

/// A `let` that destructures is a pattern like any other. What it takes apart
/// is the program saying which pieces it is talking about, not planning to
/// read every one of them.
#[test]
fn a_let_that_destructures_is_a_pattern() {
    let (_, _, resolved) = resolve_source(
        "module a\n\nrecord Pair { left: Int, right: Int }\n\nfn f(p: Pair) -> Int {\n  let Pair { left, right } = p\n  left\n}\n",
    );
    assert!(resolved.diagnostics.is_empty());
}

#[test]
fn a_capitalised_pattern_is_not_a_binding() {
    let (_, _, resolved) = resolve_source(
        "module a\n\nchoice Flag { On, Off }\n\nfn f(x: Flag) -> Int {\n  let On = x\n  0\n}\n",
    );
    assert!(resolved.diagnostics.is_empty());
}

#[test]
fn shadowing_a_declaration_only_warns() {
    let (_, _, resolved) = resolve_source(
        "module a\n\nfn total() -> Int { 0 }\n\nfn f() -> Int {\n  let total = 1\n  total\n}\n",
    );
    assert_eq!(
        codes_of(&resolved.diagnostics),
        vec![codes::SHADOWED_DECLARATION]
    );
    assert!(!resolved.has_errors());
}

#[test]
fn a_parameter_hiding_a_declaration_warns() {
    let (_, _, resolved) = resolve_source(
        "module a\n\nfn total() -> Int { 0 }\n\nfn f(total: Int) -> Int { total }\n",
    );
    assert_eq!(
        codes_of(&resolved.diagnostics),
        vec![codes::SHADOWED_DECLARATION]
    );
    assert!(!resolved.has_errors());
}

#[test]
fn a_capitalised_let_pattern_is_a_variant_match_not_a_binding() {
    // A consequence of the capitalisation rule that is worth pinning down:
    // `let Int = 1` does not introduce a binding, it mentions `Int`.
    let src = "module a\n\nfn f() -> Int {\n  let Int = 1\n  Int\n}\n";
    let (_, _, resolutions) = resolve_ok(src);

    let mention = span_of(src, "Int", 2);
    let def = resolutions.resolution(mention).unwrap();
    assert_eq!(resolutions.def(def).kind, DefKind::Builtin);
}

// -- robustness ------------------------------------------------------------

#[test]
fn error_nodes_do_not_derail_resolution() {
    // Deliberately broken, so the parser produces error nodes. The resolver
    // must still walk what is left and must not add noise about it.
    let mut sources = SourceMap::new();
    let file = sources.add(
        "broken.deed",
        "module a\n\nfn good() -> Int { 0 }\n\nfn bad(x: ) -> Int { , }\n\nfn also_good() -> Int { good() }\n",
    );
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    assert!(parsed.has_errors());

    let resolved = resolve(file, &parsed.module, &Universe::new());
    // Whatever it says, it must not invent unknown-name errors for the holes.
    for diagnostic in &resolved.diagnostics {
        assert_ne!(
            diagnostic.code,
            codes::UNKNOWN_NAME,
            "resolver reported a name error caused by a parse error: {}",
            render_human(&sources, diagnostic)
        );
    }
}

#[test]
fn an_empty_module_resolves_to_nothing() {
    let (_, _, resolved) = resolve_source("module a\n");
    assert!(resolved.diagnostics.is_empty());
    // Only the prelude.
    assert_eq!(
        resolved
            .resolutions
            .defs()
            .filter(|(_, d)| d.kind == DefKind::Builtin)
            .count(),
        deed_resolve::PRELUDE.len()
    );
}

// -- alternatives in a match arm -------------------------------------------

#[test]
fn an_alternative_that_would_bind_is_refused() {
    let (sources, _, resolved) = resolve_source(
        "module a\n\n\
         choice E { A { n: Int }, B }\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   A { n } | B => n,\n\
         \x20 }\n\
         }\n",
    );
    assert_eq!(
        codes_of(&resolved.diagnostics),
        vec![codes::BINDING_IN_AN_ALTERNATIVE]
    );
    let text: Vec<String> = resolved
        .diagnostics
        .iter()
        .map(|d| render_human(&sources, d))
        .collect();
    assert!(text.join("\n").contains("cannot bind a name"));
}

#[test]
fn the_body_still_resolves_after_that() {
    // One mistake, one complaint. The name is bound anyway, so the body that
    // reads it does not raise a second error about a line that is not wrong.
    let (_, _, resolved) = resolve_source(
        "module a\n\n\
         choice E { A { n: Int }, B }\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   A { n } | B => n,\n\
         \x20 }\n\
         }\n",
    );
    assert!(
        !codes_of(&resolved.diagnostics).contains(&codes::UNKNOWN_NAME),
        "the refused binding cascaded into a name error"
    );
}

#[test]
fn alternatives_that_bind_nothing_resolve() {
    resolve_ok(
        "module a\n\n\
         choice E { A { n: Int }, B, C }\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   A | B => 1,\n\
         \x20   C => 2,\n\
         \x20 }\n\
         }\n",
    );
}
