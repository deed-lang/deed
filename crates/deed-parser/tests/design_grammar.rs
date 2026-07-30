//! The grammar document lists the same words the parser does.
//!
//! `design/06-grammar.md` states the `keyword` and `soft-keyword` productions
//! in double-quoted form. This file holds the grammar document to those two
//! productions in both directions, so a word added to or removed from the
//! parser cannot go unnoticed here.
//!
//! The same move is made for the editor grammar in `grammar.rs`: both sets
//! `Keyword::ALL` and `SOFT_KEYWORDS` are checked against the external
//! document rather than trusted to stay in sync.

use std::collections::BTreeSet;

use deed_lexer::Keyword;
use deed_parser::SOFT_KEYWORDS;

fn grammar_doc() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../design/06-grammar.md");
    std::fs::read_to_string(path).expect("the grammar document should be there")
}

/// Extracts double-quoted lowercase words from the lines of a named BNF rule.
///
/// Finds the first line that contains `{rule_name}  ::=` and reads lines until
/// a blank line or until a new production name is encountered. Every `"word"`
/// where `word` consists only of lowercase ASCII letters and underscores is
/// collected and returned.
///
/// Both directions -- every word in the rule is known and every known word is
/// in the rule -- are what the tests below check. An empty set satisfies both
/// halves and would miss every word, so the function panics when it finds
/// nothing, the same guard the editor grammar test uses.
fn rule_words(doc: &str, rule_name: &str) -> BTreeSet<String> {
    let marker = format!("{rule_name}  ::=");

    // Find the production and collect it until the next blank line.
    let start = doc.find(&marker).unwrap_or_else(|| {
        panic!("production `{rule_name}  ::=` not found in the grammar document")
    });

    let text: String = doc[start..]
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    let mut found = BTreeSet::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find('"') {
        rest = &rest[at + 1..];
        if let Some(end) = rest.find('"') {
            let word = &rest[..end];
            if !word.is_empty() && word.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                found.insert(word.to_string());
            }
            rest = &rest[end + 1..];
        } else {
            break;
        }
    }

    assert!(
        !found.is_empty(),
        "no quoted words found in the `{rule_name}  ::=` production; \
         the document may have been edited in a way that removes all entries"
    );

    found
}

#[test]
fn the_document_lists_every_keyword() {
    let doc = grammar_doc();
    let listed = rule_words(&doc, "keyword");

    let missing: Vec<&str> = Keyword::ALL
        .iter()
        .map(|kw| kw.as_str())
        .filter(|word| !listed.contains(*word))
        .collect();

    assert!(
        missing.is_empty(),
        "the parser knows these and the grammar document does not list them: {}",
        missing.join(", ")
    );
}

#[test]
fn the_document_lists_nothing_else_as_a_keyword() {
    let doc = grammar_doc();
    let listed = rule_words(&doc, "keyword");
    let known: BTreeSet<&str> = Keyword::ALL.iter().map(|kw| kw.as_str()).collect();

    let invented: Vec<String> = listed
        .into_iter()
        .filter(|word| !known.contains(word.as_str()))
        .collect();

    assert!(
        invented.is_empty(),
        "the grammar document lists these as keywords but the parser does not know them: {}",
        invented.join(", ")
    );
}

#[test]
fn the_document_lists_every_soft_keyword() {
    let doc = grammar_doc();
    let listed = rule_words(&doc, "soft-keyword");

    let missing: Vec<&str> = SOFT_KEYWORDS
        .iter()
        .copied()
        .filter(|word| !listed.contains(*word))
        .collect();

    assert!(
        missing.is_empty(),
        "the parser knows these soft keywords and the grammar document does not list them: {}",
        missing.join(", ")
    );
}

#[test]
fn the_document_lists_nothing_else_as_a_soft_keyword() {
    let doc = grammar_doc();
    let listed = rule_words(&doc, "soft-keyword");
    let known: BTreeSet<&str> = SOFT_KEYWORDS.iter().copied().collect();

    let invented: Vec<String> = listed
        .into_iter()
        .filter(|word| !known.contains(word.as_str()))
        .collect();

    assert!(
        invented.is_empty(),
        "the grammar document lists these as soft keywords but the parser does not know them: {}",
        invented.join(", ")
    );
}
