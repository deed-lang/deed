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
//! The other thing that belongs here is a library that was already written and
//! had nowhere to be. The list library and the table library both sat under
//! `examples/` for months, and a module's name says where it lives, so
//! importing one meant writing `use examples/list`, and a program outside this
//! repository had to copy the file to get one.
//!
//! These are checked like any other file. `crates/deed-driver/tests/shipped.rs`
//! runs their tests, and `deed fmt` reaches them through the repository walk
//! because they are also files here.
//!
//! Who reaches them is a rule rather than a habit, and it lives here because
//! it used to live in one place and be missing from the other. The command
//! line tool injected these modules and the language server did not, so the
//! editor put `DEED3007 UNKNOWN_MODULE` under a `use` line that `deed check`
//! was silent about, on files checked into this repository. [`take_shipped`]
//! is that rule, and both of them call it.

use std::collections::HashSet;

/// A module that ships with the compiler, by the name a `use` writes.
///
/// The name and the text, rather than a path, because at run time there is no
/// path. The file in this repository is where the text is edited and the
/// constant is what a program gets.
const SHIPPED: &[(&str, &str)] = &[
    ("std/string", include_str!("../../../std/string.deed")),
    ("std/list", include_str!("../../../std/list.deed")),
    ("std/table", include_str!("../../../std/table.deed")),
    ("std/map", include_str!("../../../std/map.deed")),
    ("std/ratio", include_str!("../../../std/ratio.deed")),
    ("std/date", include_str!("../../../std/date.deed")),
    ("std/task", include_str!("../../../std/task.deed")),
];

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

/// One round of asking the compiler's own table for modules nothing else
/// answered to.
///
/// `wanted` is every module name that has been asked for. A name already in
/// `have` is skipped, and that is the whole of the precedence rule: everything
/// a person can read is offered first, and this table is the last place
/// looked, so somebody's own `std/string.deed` is the one that wins. Nobody
/// should have to know which is which, and the one that is right there is the
/// one they can change.
///
/// What is taken goes on the end of `taken`, in the order it was asked for, so
/// that two runs over the same program produce the same list. `wanted` comes
/// back holding what those modules import in turn, which is the caller's turn
/// again: a shipped module imports the same way anything else does, and a
/// caller that is still looking for files gets its chance at those names
/// before this table does.
///
/// Answers whether anything was taken, which is a caller's fixpoint.
///
/// A `use` naming a module that is nowhere is left alone. The resolver has the
/// message for that, and it can point at the line.
pub fn take_shipped(
    have: &mut HashSet<String>,
    wanted: &mut Vec<String>,
    taken: &mut Vec<&'static str>,
) -> bool {
    let before = taken.len();

    for module in std::mem::take(wanted) {
        if have.contains(&module) || taken.contains(&module.as_str()) {
            continue;
        }
        if let Some(name) = shipped_modules().find(|name| *name == module) {
            taken.push(name);
        }
    }

    for module in &taken[before..] {
        let Some(text) = shipped_source(module) else {
            continue;
        };
        let Some((name, uses)) = crate::imports_of(text) else {
            continue;
        };
        have.insert(name);
        wanted.extend(uses);
    }

    taken.len() > before
}

/// The shipped modules a set of sources needs, when the looking is done.
///
/// The shape for a caller that already has every source it is ever going to
/// have: an editor knows the whole workspace before it checks any of it, where
/// the command line tool is still working out where files are and so drives
/// [`take_shipped`] itself, a round at a time.
///
/// `sources` is every text in hand. All of it answers first, for the reason
/// [`take_shipped`] gives.
pub fn shipped_for<'a>(sources: impl IntoIterator<Item = &'a str>) -> Vec<&'static str> {
    let mut have: HashSet<String> = HashSet::new();
    let mut wanted: Vec<String> = Vec::new();

    for text in sources {
        let Some((module, uses)) = crate::imports_of(text) else {
            continue;
        };
        have.insert(module);
        wanted.extend(uses);
    }

    let mut taken = Vec::new();
    while take_shipped(&mut have, &mut wanted, &mut taken) {}
    taken
}
