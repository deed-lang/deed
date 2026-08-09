//! Generated pages for every diagnostic code deed can produce.
//!
//! Every entry comes from the `///` doc-comment lines that already sit above
//! each `pub const` in a `codes.rs` file, together with a deed snippet taken
//! from an existing test and kept only because running it produces this code.
//! Nothing here is invented.
//!
//! The pages are generated, committed as `generated/pages.rs`, and shipped as
//! source.  A build script did the reading until it was measured against a
//! published crate: a `.crate` archive carries this directory and no
//! workspace, so the script would have found an empty tree, produced no pages,
//! and compiled.  `crates/deed-driver/tests/explain_pages.rs` reads the tree
//! now, where the tree is, and fails when the committed file has drifted from
//! it.  It sits over there because deciding that a program produces a code
//! takes a compiler, and this package depends on nothing.
//!
//! An `example` of `None` means no test offered a program that produces the
//! code.  Callers print nothing rather than something close.
//!
//! The public surface is small:
//! - [`all_pages`] returns every page in sorted order.
//! - [`page`] looks up a single code by its identifier (`"DEED4025"`) or its
//!   constant name (`"BROKEN_PRECONDITION"`).

/// One generated page per diagnostic code.
#[derive(Debug)]
pub struct Page {
    /// The code string, e.g. `"DEED4025"`.
    pub code: &'static str,
    /// The constant name, e.g. `"BROKEN_PRECONDITION"`.
    pub name: &'static str,
    /// The reasoning: doc-comment lines from `codes.rs`.
    pub text: &'static str,
    /// A deed program that produces this code, checked against the compiler
    /// when the pages were generated.  `None` when no test offered one.
    pub example: Option<&'static str>,
    /// The test file the example came from.
    pub example_source: Option<&'static str>,
}

include!("../generated/pages.rs");

/// All generated pages, sorted by code identifier.
pub fn all_pages() -> &'static [Page] {
    PAGES
}

/// The page for a code, looked up by either its identifier (`"DEED4025"`) or
/// its constant name (`"BROKEN_PRECONDITION"`).  Returns `None` if no page
/// exists for the given string.
pub fn page(query: &str) -> Option<&'static Page> {
    PAGES
        .iter()
        .find(|p| p.code.eq_ignore_ascii_case(query) || p.name.eq_ignore_ascii_case(query))
}
