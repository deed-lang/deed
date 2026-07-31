//! Discovering the complete input set for a Deed program.
//!
//! One rule, three callers.
//!
//! [`resolve_inputs`] is the shared entry point. It takes the texts the caller
//! already holds and a closure that can find more sources by module name. The
//! caller decides what "more" means:
//!
//! - `deed check` passes a closure that walks the filesystem from the roots
//!   the named files imply.
//! - The language server passes `|_| None`, because it has already loaded the
//!   whole workspace. It cannot fetch missing modules without blocking a
//!   keystroke on the network, and it does not try.
//! - A future dependency command will pass a closure that reads a local cache,
//!   populated by a prior fetch that can block freely.
//!
//! Shipped modules are added last. A caller whose workspace contains its own
//! `std/list.deed` gets that one; the binary's copy is only reached when the
//! workspace has nothing.
//!
//! # What the editor shows for a dependency that is not yet cached
//!
//! When the caller is the language server, `find` always returns `None`. If a
//! `use` names a module that is neither in the workspace nor shipped with the
//! compiler, the resolver reports `DEED3007`. That is the same report `deed
//! check` would give before any fetch has run, and it is the correct answer
//! for code that is genuinely wrong. The editor never blocks on the network
//! because it never asks the network.

use std::collections::HashSet;

/// Finds every source a Deed program needs beyond what the caller already
/// holds.
///
/// `seeds` are the source texts the caller already has. For each module any
/// seed imports that is not yet accounted for, `find` is called with the
/// module name. When `find` returns `Some((display_name, source_text))`, the
/// new source is added and its own imports are queued. When it returns `None`,
/// the module is left for the shipped-module table and, if not found there
/// either, for the resolver to report as `DEED3007`.
///
/// Shipped modules are consulted after `find` is exhausted for all outstanding
/// imports. A seed or a source `find` provides that carries the same name as a
/// shipped module takes precedence over the binary's copy.
///
/// Returns the additional sources `find` provided and the names of the shipped
/// modules the program needs, in the order they were discovered.
pub fn resolve_inputs<F>(
    seeds: impl IntoIterator<Item = impl AsRef<str>>,
    mut find: F,
) -> (Vec<(String, String)>, Vec<&'static str>)
where
    F: FnMut(&str) -> Option<(String, String)>,
{
    let mut have: HashSet<String> = HashSet::new();
    let mut wanted: Vec<String> = Vec::new();
    let mut extras: Vec<(String, String)> = Vec::new();

    let seed_texts: Vec<String> = seeds.into_iter().map(|s| s.as_ref().to_owned()).collect();

    for text in &seed_texts {
        if let Some((module, uses)) = crate::imports_of(text) {
            have.insert(module);
            wanted.extend(uses);
        }
    }

    // Call `find` in rounds until nothing new comes back. Each new source may
    // import more, so the loop continues as long as any round adds something.
    loop {
        let pending = std::mem::take(&mut wanted);
        let mut any = false;

        for module in pending {
            if have.contains(&module) {
                continue;
            }

            match find(&module) {
                Some((name, text)) => {
                    if let Some((m, uses)) = crate::imports_of(&text) {
                        have.insert(m);
                        wanted.extend(uses);
                    }
                    extras.push((name, text));
                    any = true;
                }
                None => {
                    // Not available from this caller. The shipped-module table
                    // below will be checked next, and the resolver will report
                    // DEED3007 for anything neither provides.
                }
            }
        }

        if !any {
            break;
        }
    }

    // Shipped modules, after `find` has had its turn at every outstanding
    // import. `shipped_for` runs the same transitive closure that `find` did,
    // this time over the compiler's own table.
    let all_texts: Vec<&str> = seed_texts
        .iter()
        .map(String::as_str)
        .chain(extras.iter().map(|(_, t)| t.as_str()))
        .collect();
    let shipped = crate::shipped_for(all_texts);

    (extras, shipped)
}
