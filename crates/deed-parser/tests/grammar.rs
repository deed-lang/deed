//! The editor grammar knows the same words the parser does.
//!
//! `editors/vscode/syntaxes/deed.tmLanguage.json` and
//! `editors/tree-sitter-deed/grammar.js` are second copies of the keyword set
//! living outside the compiler, in files no Rust code reads and no test would
//! otherwise open. A keyword added to the lexer and not to either grammar is a
//! word that stops being coloured, which nobody notices until they are reading
//! a program and one word is the wrong colour for a reason they cannot see.
//!
//! Two sets are held rather than one. `Keyword::ALL` is what the lexer
//! reserves and `SOFT_KEYWORDS` is what the parser reads by name in a single
//! position, and somebody reading a file cannot tell those apart, so the
//! grammar owes both a colour. Its own groups are no help in telling them
//! apart either: they are named for what a word does rather than for whether
//! it is reserved, which is why `state` sits among the declaration keywords
//! and `at` among the contract ones.
//!
//! A word in either set is coloured wherever it appears, because a TextMate
//! grammar has no positions to ask about. `SOFT_KEYWORDS` says what that
//! costs and why the set is drawn where it is.
//!
//! So the grammar is held to both sets rather than trusted to keep up. Both
//! directions: every word in them is coloured, and nothing is coloured that is
//! in neither.

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

    // Here rather than in one of the tests, because both of them are satisfied
    // by an empty set: one has nothing missing from it and the other has
    // nothing invented in it. Losing every alternation is the shape a bad edit
    // to the grammar takes, and it is the shape neither test would report.
    assert!(
        !found.is_empty(),
        "no `\\b(one|two)\\b` groups in the grammar, so nothing was compared"
    );

    found
}

fn vscode_grammar() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../editors/vscode/syntaxes/deed.tmLanguage.json"
    );
    std::fs::read_to_string(path).expect("the editor grammar should be there")
}

fn tree_sitter_coloured_words(grammar: &str) -> BTreeSet<&str> {
    let open = "const KEYWORDS = [";
    let close = "];";
    let start = grammar
        .find(open)
        .expect("tree-sitter grammar should declare KEYWORDS");
    let after_open = &grammar[start + open.len()..];
    let end = after_open
        .find(close)
        .expect("tree-sitter grammar KEYWORDS should close");
    let list = &after_open[..end];

    let mut found = BTreeSet::new();
    for line in list.lines() {
        let token = line.trim().trim_end_matches(',');
        if let Some(word) = token.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            found.insert(word);
        }
    }

    assert!(
        !found.is_empty(),
        "no keywords found in tree-sitter KEYWORDS list"
    );
    found
}

fn tree_sitter_grammar() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../editors/tree-sitter-deed/grammar.js"
    );
    std::fs::read_to_string(path).expect("the tree-sitter grammar should be there")
}

fn known_words() -> BTreeSet<&'static str> {
    Keyword::ALL
        .iter()
        .map(|kw| kw.as_str())
        .chain(SOFT_KEYWORDS)
        .collect()
}

fn assert_colours_every_keyword(coloured: &BTreeSet<&str>, grammar_name: &str) {
    let missing: Vec<&str> = known_words()
        .into_iter()
        .filter(|word| !coloured.contains(word))
        .collect();

    assert!(
        missing.is_empty(),
        "{grammar_name} does not colour these parser words: {}",
        missing.join(", ")
    );
}

fn assert_colours_nothing_else(coloured: BTreeSet<&str>, grammar_name: &str) {
    let known = known_words();
    let invented: Vec<&str> = coloured
        .into_iter()
        .filter(|word| !known.contains(word))
        .collect();

    assert!(
        invented.is_empty(),
        "{grammar_name} colours these words the parser does not know: {}",
        invented.join(", ")
    );
}

#[test]
fn vscode_grammar_colours_every_keyword() {
    let grammar = vscode_grammar();
    let coloured = coloured_words(&grammar);
    assert_colours_every_keyword(&coloured, "vscode grammar");
}

#[test]
fn vscode_grammar_colours_nothing_else() {
    let grammar = vscode_grammar();
    let coloured = coloured_words(&grammar);
    assert_colours_nothing_else(coloured, "vscode grammar");
}

#[test]
fn tree_sitter_grammar_colours_every_keyword() {
    let grammar = tree_sitter_grammar();
    let coloured = tree_sitter_coloured_words(&grammar);
    assert_colours_every_keyword(&coloured, "tree-sitter grammar");
}

#[test]
fn tree_sitter_grammar_colours_nothing_else() {
    let grammar = tree_sitter_grammar();
    let coloured = tree_sitter_coloured_words(&grammar);
    assert_colours_nothing_else(coloured, "tree-sitter grammar");
}
