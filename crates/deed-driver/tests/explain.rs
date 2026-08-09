//! Every diagnostic code has a page with non-empty content.
//!
//! The same ratchet as `codes.rs`, one level up.  `codes.rs` requires every
//! code to have a test that produces it; this file requires every code to have
//! a page in `deed-explain`.  A page without reasoning is as useless as a
//! code without a test: it tells the reader only that the error existed.
//!
//! The check is deliberately mechanical.  A doc-comment line of any kind
//! satisfies it.  Depth is not measured here; depth is what the author of the
//! codes.rs entry is responsible for.
//!
//! # Adding a code
//!
//! Add at least one `///` line above the `pub const` in `codes.rs`.  The
//! build script for `deed-explain` turns those lines into the page; this test
//! confirms the result is non-empty.  Everything else is automatic.

use std::collections::BTreeSet;
use std::path::PathBuf;

// Reuse the same enumeration logic as `codes.rs`.  Both files need the list
// of declared codes; extracting a shared helper keeps the two from drifting.

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn crates() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(root().join("crates"))
        .expect("crates/ should be there")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no crates found under crates/");
    found
}

/// Every `pub const NAME: &str = "DEEDnnnn"`, as the pair the page index uses.
fn declared() -> Vec<(String, String)> {
    let mut codes = Vec::new();
    for krate in crates() {
        let path = krate.join("src").join("codes.rs");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("pub const ") else {
                continue;
            };
            let Some((name, rest)) = rest.split_once(':') else {
                continue;
            };
            let Some(start) = rest.find("\"DEED") else {
                continue;
            };
            let Some(end) = rest[start + 1..].find('"') else {
                continue;
            };
            codes.push((
                name.trim().to_string(),
                rest[start + 1..start + 1 + end].to_string(),
            ));
        }
    }
    codes
}

#[test]
fn every_diagnostic_code_has_a_page() {
    let declared = declared();
    assert!(
        declared.len() > 50,
        "only {} codes found; the source was not read properly",
        declared.len()
    );

    let without_page: BTreeSet<&str> = declared
        .iter()
        .filter(|(name, code)| {
            match deed_explain::page(code).or_else(|| deed_explain::page(name)) {
                None => true,
                Some(p) => p.text.is_empty(),
            }
        })
        .map(|(_, code)| code.as_str())
        .collect();

    assert!(
        without_page.is_empty(),
        "these diagnostics have no page; add a doc comment in their codes.rs entry: {}",
        without_page.into_iter().collect::<Vec<_>>().join(", ")
    );
}

/// `all_pages` is a real listing, not a stand-in nothing reads: the test
/// above only ever calls `page()`, which reads the same generated table by a
/// different path, so a broken `all_pages` could return nothing and this
/// crate would still look fully covered.
#[test]
fn all_pages_lists_one_page_per_declared_code() {
    let declared = declared();
    let pages = deed_explain::all_pages();
    assert_eq!(
        pages.len(),
        declared.len(),
        "all_pages() should return exactly one page per declared code"
    );

    for (_, code) in &declared {
        assert!(
            pages.iter().any(|p| p.code == code.as_str()),
            "`{code}` is declared but all_pages() does not list it"
        );
    }
}

/// An example is copied out of a test, and a test is Rust, so an example can
/// be a `format!` template rather than a program (#780).
///
/// Thirty of the ninety-seven pages printed one: doubled braces, placeholders
/// and lone backslashes where Rust had joined two source lines. The tests
/// above all passed the whole time, because a template is a non-empty string
/// and that was the only question being asked.
///
/// This asks whether what came out could be a program. It cannot check that
/// the snippet checks, since most of them are meant not to, but Deed has no
/// syntax that produces any of these.
#[test]
fn no_example_is_a_rust_template_rather_than_a_program() {
    let mut wrong: Vec<String> = Vec::new();

    for page in deed_explain::all_pages() {
        let Some(example) = page.example else {
            continue;
        };

        // `{{` and `}}` are how a format string writes one brace. Deed writes
        // a brace as a brace, and never two.
        if example.contains("{{") || example.contains("}}") {
            wrong.push(format!("{}: doubled braces", page.code));
            continue;
        }

        // A line that is nothing but a backslash is Rust's line continuation
        // read as text.
        if example.lines().any(|line| line.trim() == "\\") {
            wrong.push(format!("{}: a line continuation", page.code));
        }
    }

    assert!(
        wrong.is_empty(),
        "these pages print Rust rather than Deed: {}",
        wrong.join(", ")
    );
}

/// The count is the point. A change to the extractor that quietly stopped
/// finding examples would leave every other test in this file green, because
/// the pages would still exist and still have reasoning in them.
///
/// The floor was 76 while an example was the first deed-looking string in a
/// test that mentioned the code, which let it be the control arm, or one
/// module of a two-module program: of the eighty-nine it reached, forty-three
/// did not produce the code. It is 75 now, and
/// `explain_pages.rs::every_example_produces_the_code_its_page_is_about` holds
/// every one of them. A higher number bought with programs that do not
/// produce the code is a worse number.
#[test]
fn the_pages_that_carry_an_example_keep_carrying_one() {
    let with_example = deed_explain::all_pages()
        .iter()
        .filter(|p| p.example.is_some())
        .count();

    assert!(
        with_example >= 75,
        "only {with_example} pages have an example; the extractor found fewer than it used to"
    );
}
