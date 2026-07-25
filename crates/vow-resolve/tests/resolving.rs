//! Name resolution behaviour.
//!
//! The interesting cases are the ambiguities the parser refused to settle, and
//! the diagnostics, since "cannot find X, did you mean Y" is the highest value
//! message in the compiler for what it costs.

use std::collections::HashSet;

use vow_ast::Item;
use vow_diagnostics::{Diagnostic, SourceMap, render_human};
use vow_lexer::tokenize;
use vow_parser::parse;
use vow_resolve::{DefKind, Dot, Resolutions, Resolved, codes, resolve};

fn resolve_source(src: &str) -> (SourceMap, vow_ast::Module, Resolved) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.vow", src);
    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "test source should parse cleanly");
    let resolved = resolve(file, &parsed.module);
    (sources, parsed.module, resolved)
}

fn resolve_ok(src: &str) -> (SourceMap, vow_ast::Module, Resolutions) {
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
fn span_of(src: &str, needle: &str, occurrence: usize) -> vow_diagnostics::Span {
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
    vow_diagnostics::Span::new(begin as u32, (begin + needle.len()) as u32)
}

// -- the worked example ----------------------------------------------------

#[test]
fn the_worked_example_resolves_cleanly() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.vow");
    let source = std::fs::read_to_string(path).expect("examples/transfer.vow should exist");

    let mut sources = SourceMap::new();
    let file = sources.add("examples/transfer.vow", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors());

    let resolved = resolve(file, &parsed.module);
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
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/transfer.vow");
    let source = std::fs::read_to_string(path).unwrap();

    let mut sources = SourceMap::new();
    let file = sources.add("transfer.vow", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module);

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
    assert!(
        external.len() <= 12,
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
fn a_dot_after_an_import_is_left_alone() {
    let src = "module a\n\nuse other.{Thing}\n\nfn f() -> Int { Thing.whatever }\n";
    let (_, _, resolutions) = resolve_ok(src);

    let dot = span_of(src, "whatever", 0);
    assert_eq!(
        resolutions.dot(dot),
        Some(Dot::Foreign),
        "nothing can be said about the inside of a module we have not loaded"
    );
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
        vow_diagnostics::Applicability::MachineApplicable
    );
    assert!(render_human(&sources, &resolved.diagnostics[0]).contains("cannot find `balanse`"));
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
    let (_, _, resolved) =
        resolve_source("module a\n\nuse other.{Used, Spare}\n\nfn f() -> Used { }\n");
    assert_eq!(codes_of(&resolved.diagnostics), vec![codes::UNUSED_IMPORT]);
    assert!(!resolved.has_errors());
    assert!(resolved.diagnostics[0].message.contains("Spare"));
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
        "broken.vow",
        "module a\n\nfn good() -> Int { 0 }\n\nfn bad(x: ) -> Int { , }\n\nfn also_good() -> Int { good() }\n",
    );
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    assert!(parsed.has_errors());

    let resolved = resolve(file, &parsed.module);
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
        vow_resolve::PRELUDE.len()
    );
}
