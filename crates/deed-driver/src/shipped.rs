//! The modules that ship inside the compiler.
//!
//! A module's name says where it lives, the root comes off the file that was
//! named, and there is no search path. That rule is what makes a program's
//! imports readable without a manifest, and none of it changes here: `std/x`
//! lives in the compiler, which is as determinate as a directory and does not
//! need looking for.
//!
//! Embedded rather than installed. The compiler is distributed as one binary,
//! so a library that is part of the binary is a library that is there the
//! moment the binary is, with nothing to fetch, nothing to version and no
//! second thing that can be missing. The alternative is a directory beside the
//! executable, which is a search path with one entry and the beginning of the
//! argument this project has already refused.
//!
//! What belongs here is what the prelude test turns away. A thing that can be
//! written in Deed is written in Deed, and until now there was nowhere for the
//! result to go, so `design/02-syntax.md` listed the string operations as
//! missing for as long as they were writable.
//!
//! These are checked like any other file. `crates/deed-driver/tests/shipped.rs`
//! runs their tests, and `deed fmt` reaches them through the repository walk
//! because they are also files here.

/// A module that ships with the compiler, by the name a `use` writes.
///
/// The name and the text, rather than a path, because at run time there is no
/// path. The file in this repository is where the text is edited and the
/// constant is what a program gets.
const SHIPPED: &[(&str, &str)] = &[("std/string", include_str!("../../../std/string.deed"))];

/// Every module that ships, in the order they are declared.
pub fn shipped_modules() -> impl Iterator<Item = &'static str> {
    SHIPPED.iter().map(|(name, _)| *name)
}

/// The source of a module that ships with the compiler.
///
/// `None` for everything else, which is every module a program writes and
/// every name that is simply wrong. A `use` naming neither is reported by the
/// resolver, which is the pass that can point at the line.
pub fn shipped_source(module: &str) -> Option<&'static str> {
    SHIPPED
        .iter()
        .find(|(name, _)| *name == module)
        .map(|(_, text)| *text)
}
