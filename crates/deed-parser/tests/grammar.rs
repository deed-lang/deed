//! The editor grammar knows the same words the parser does.
//!
//! `editors/vscode/syntaxes/deed.tmLanguage.json` is a second copy of the
//! keyword set living outside the compiler, in a file no Rust code reads and
//! no test would otherwise open. A keyword added to the lexer and not to the
//! grammar is a word that stops being coloured, which nobody notices until
//! they are reading a program and one word is the wrong colour for a reason
//! they cannot see.
//!
//! So the grammar is held to the set rather than trusted to keep up. Both
//! directions: every keyword is coloured, and nothing is coloured that is not
//! a keyword.

use std::collections::BTreeSet;

use deed_lexer::Keyword;
use deed_parser::SOFT_KEYWORDS;

/// The words the grammar treats as keywords.
///
/// Every keyword group in the grammar is written `\b(one|two|three)\b`, so the
/// set is read out of the file rather than repeated here. The other patterns
/// match shapes rather than words and do not use that form, which is what
/// keeps numbers, type names and operators out of this.
fn coloured_words(grammar: &str) -> BTreeSet<&str> {
    // As they appear in the file: a JSON-escaped `\b(` and `)\b`.
    const OPEN: &str = "\\\\b(";
    const CLOSE: &str = ")\\\\b";

    let mut found = BTreeSet::new();
    let mut rest = grammar;
    while let Some(at) = rest.find(OPEN) {
        let after = &rest[at + OPEN.len()..];
        let Some(end) = after.find(CLOSE) else {
            break;
        };
        found.extend(after[..end].split('|'));
        rest = &after[end + CLOSE.len()..];
    }
    found
}

fn grammar() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../editors/vscode/syntaxes/deed.tmLanguage.json"
    );
    std::fs::read_to_string(path).expect("the editor grammar should be there")
}

#[test]
fn the_grammar_colours_every_keyword() {
    let grammar = grammar();
    let coloured = coloured_words(&grammar);

    // Without this the test passes on a grammar that lost every alternation,
    // which is the shape a bad edit takes.
    assert!(
        !coloured.is_empty(),
        "no `\\b(one|two)\\b` groups in the grammar, so nothing was compared"
    );

    let missing: Vec<&str> = Keyword::ALL
        .iter()
        .map(|kw| kw.as_str())
        .chain(SOFT_KEYWORDS)
        .filter(|word| !coloured.contains(word))
        .collect();

    assert!(
        missing.is_empty(),
        "the parser knows these and the grammar does not colour them: {}",
        missing.join(", ")
    );
}

#[test]
fn the_grammar_colours_nothing_else() {
    let grammar = grammar();
    let known: BTreeSet<&str> = Keyword::ALL
        .iter()
        .map(|kw| kw.as_str())
        .chain(SOFT_KEYWORDS)
        .collect();

    let invented: Vec<&str> = coloured_words(&grammar)
        .into_iter()
        .filter(|word| !known.contains(word))
        .collect();

    // A word coloured as a keyword and not read as one is worse than a missing
    // colour: it tells a reader the compiler cares about a name it has never
    // heard of.
    assert!(
        invented.is_empty(),
        "the grammar colours these and the parser does not know them: {}",
        invented.join(", ")
    );
}
