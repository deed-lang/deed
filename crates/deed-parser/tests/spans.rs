//! What a parse tree says about where it came from, when the source is broken.
//!
//! A declaration ends at a closing token, and the parser used to record that
//! by taking the span sitting there before asking for the token. When the
//! closer is missing that span belongs to whatever comes next, so a signature
//! could end after its own body began. The fuzzer found it through
//! `deed fix`, which subtracts one from the other to find the region a `uses`
//! clause goes in, and `Span::new` refuses a range that runs backwards.
//!
//! The invariant is about order rather than about any one construct: the
//! signature, then the contract, then the body, each starting where the last
//! one stopped or later, and all of them inside the declaration. A parser that
//! reads past what it consumed breaks that no matter which token went missing.
//!
//! Broken sources are made rather than written: a handful of programs that
//! ship are taken apart one token at a time. Deleting a single token is what
//! the fuzzer did in the input that found this, and unlike the fuzzer it needs
//! no nightly toolchain and produces the same set of programs on every
//! machine. The sample is named rather than the whole corpus because every
//! deleted token is another parse and the whole corpus is a minute of them;
//! between them these five write every declaration form the language has.

use std::path::{Path, PathBuf};

use deed_ast::Item;
use deed_diagnostics::{SourceMap, Span};
use deed_lexer::{TokenKind, tokenize};
use deed_parser::parse;

/// Between them: contracts, effects, handlers, generics, records, choices,
/// matches, closures and `Result`.
const SAMPLE: [&str; 5] = [
    "examples/transfer.deed",
    "examples/generic_types.deed",
    "examples/config.deed",
    "examples/tic_tac_toe.deed",
    "examples/generator.deed",
];

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels below the repository root")
        .to_path_buf()
}

fn programs() -> Vec<(String, String)> {
    let root = repository();
    SAMPLE
        .iter()
        .map(|name| {
            let text = std::fs::read_to_string(root.join(name))
                .unwrap_or_else(|_| panic!("{name} should be readable"));
            assert!(!text.is_empty(), "{name} is empty");
            (name.to_string(), text)
        })
        .collect()
}

/// Each source with one token removed, in order.
fn with_one_token_missing(source: &str) -> Vec<(Span, String)> {
    let mut sources = SourceMap::new();
    let file = sources.add("whole.deed", source);
    let lexed = tokenize(file, sources.file(file).text());
    lexed
        .tokens
        .iter()
        .filter(|token| !matches!(token.kind, TokenKind::Eof))
        .map(|token| {
            let mut damaged = String::with_capacity(source.len());
            damaged.push_str(&source[..token.span.start as usize]);
            damaged.push_str(&source[token.span.end as usize..]);
            (token.span, damaged)
        })
        .collect()
}

/// Reports the first ordering the tree gets wrong, if any.
fn out_of_order(source: &str) -> Option<String> {
    let mut sources = SourceMap::new();
    let file = sources.add("damaged.deed", source);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);

    for item in &parsed.module.items {
        let Item::Function(function) = item else {
            continue;
        };
        let sig = function.sig.span;
        let body = function.body.span;
        let whole = function.span;

        let complain = |what: &str, first: Span, second: Span| {
            Some(format!(
                "`{}`: {what}, {first:?} against {second:?}",
                function.sig.name.name
            ))
        };

        if let Some(contract) = function.contract.span {
            if sig.end > contract.start {
                return complain("the signature reaches into the contract", sig, contract);
            }
            if contract.end > body.start {
                return complain("the contract reaches into the body", contract, body);
            }
        } else if sig.end > body.start {
            return complain("the signature reaches into the body", sig, body);
        }

        if whole.start > sig.start || whole.end < body.end {
            return complain("the declaration does not cover its own parts", whole, body);
        }
        if body.end as usize > source.len() {
            return complain("a span runs past the end of the file", body, whole);
        }
    }
    None
}

#[test]
fn a_signature_never_reaches_into_the_body_it_belongs_to() {
    let mut damaged = 0usize;
    for (name, source) in programs() {
        for (removed, text) in with_one_token_missing(&source) {
            damaged += 1;
            if let Some(problem) = out_of_order(&text) {
                panic!(
                    "{name} with the token at {removed:?} removed parses into a tree that is out \
                     of order: {problem}"
                );
            }
        }
    }
    assert!(
        damaged > 1_000,
        "this should be taking real programs apart, and it only made {damaged} of them"
    );
}

/// The shapes that found this, kept by hand as well.
///
/// The sweep above would catch them, but it says nothing about which mistake
/// it was, and these three are the ones a person actually makes: a parameter
/// list left open, a return type that never got written, and both at once.
#[test]
fn a_signature_that_never_closed_stops_where_the_parser_stopped() {
    let sources = [
        "module m\n\nfn f(x: Int\n    x\n}, y: Int) -> I\n    x\n}\n",
        "module m\n\nfn f() -> { 1 }\n",
        "module m\n\nfn f(a: Int {\n    a\n}\n",
        "module m\n\nfn f() ->\n",
        "module m\n\nfn f() -> List<Int { 1 }\n",
    ];
    for source in sources {
        assert!(
            out_of_order(source).is_none(),
            "{source:?} parses into a tree that is out of order: {:?}",
            out_of_order(source)
        );
    }
}
