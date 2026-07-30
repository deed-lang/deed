//! Generated pages for every diagnostic code deed can produce.
//!
//! Every entry comes from the `///` doc-comment lines that already sit above
//! each `pub const` in a `codes.rs` file, together with the smallest deed
//! snippet that triggers it extracted from an existing test.  Nothing here is
//! invented: the build script (`build.rs`) reads both sources and writes the
//! data into this crate.
//!
//! The public surface is small:
//! - [`all_pages`] returns every page in sorted order.
//! - [`page`] looks up a single code by its identifier (`"DEED4025"`) or its
//!   constant name (`"BROKEN_PRECONDITION"`).

include!(concat!(env!("OUT_DIR"), "/pages.rs"));

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
