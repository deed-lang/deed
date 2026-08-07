//! Naming the module that has the name, when the compiler is carrying it.
//!
//! `DEED3001` says a name is not in scope, and the resolver is right about
//! that: nothing put it there. What the resolver cannot know is that the name
//! is sitting in [`crate::shipped`], compiled into the same binary that just
//! said it could not find it. The two are in this order on purpose, since a
//! module set is resolved from what the file imports and a file that never
//! wrote the `use` never asked for the module.
//!
//! So the answer is written here, the layer that has the shipped table, the
//! text and the one canonical layout in scope at once. Same reasoning and the
//! same shelf as [`crate::imports`] and [`crate::rows`].
//!
//! One fix for the whole `use` block rather than one per name, for the reason
//! [`crate::imports`] gives: separate edits over the same text overlap, and
//! [`crate::fix::collect`] drops overlapping edits, so a file with an obvious
//! repair would be told there was nothing to do.
//!
//! A name two shipped modules declare gets the sentence and no repair. Several
//! of them are real (`size`, `get` and `text` are each declared twice), and
//! picking one would be guessing on behalf of somebody who can read the two
//! names and decide. A machine-applicable fix gets applied without asking,
//! which is exactly why it has to be right.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use deed_ast::Module;
use deed_diagnostics::{Applicability, Diagnostic, SourceMap, Span};
use deed_lexer::Trivia;
use deed_resolve::codes;
use deed_typeck::codes as typeck_codes;

use crate::imports::{line_end, line_start};
use crate::shipped::{shipped_modules, shipped_source};

/// Every name a shipped module declares, to the modules that declare it.
///
/// Parsed rather than scanned for `fn`, because a second reading of the
/// grammar is a second grammar to keep in step, and this repository has paid
/// for one of those before. Built once: the table is a constant, so the answer
/// is the same on every call and on every file.
fn declared_by() -> &'static BTreeMap<String, Vec<&'static str>> {
    static TABLE: OnceLock<BTreeMap<String, Vec<&'static str>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table: BTreeMap<String, Vec<&'static str>> = BTreeMap::new();
        let mut sources = SourceMap::new();
        for module in shipped_modules() {
            let text = shipped_source(module).expect("a module that ships has a source");
            let file = sources.add(format!("{module}.deed"), text.to_string());
            let lexed = deed_lexer::tokenize(file, text);
            let parsed = deed_parser::parse(file, &lexed.tokens);
            for name in exported_names(&parsed.module) {
                table.entry(name).or_default().push(module);
            }
        }
        table
    })
}

/// What a module offers a `use` line.
///
/// There is no visibility marker in this language, so everything a shipped
/// module declares is its API. `design/02-syntax.md` says so and
/// `deed-driver/tests/shipped.rs` holds every one of them to having a test.
fn exported_names(module: &Module) -> Vec<String> {
    use deed_ast::Item;
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) => Some(function.sig.name.name.clone()),
            Item::Record(record) => Some(record.name.name.clone()),
            Item::Choice(choice) => Some(choice.name.name.clone()),
            Item::TypeAlias(alias) => Some(alias.name.name.clone()),
            Item::Effect(effect) => Some(effect.name.name.clone()),
            Item::Handler(handler) => Some(handler.name.name.clone()),
            Item::Deprecate(_) | Item::Operator(_) | Item::Test(_) => None,
        })
        .collect()
}

/// Says which shipped module has each missing name, and writes the `use`.
pub(crate) fn attach(
    diagnostics: &mut [Diagnostic],
    module: &Module,
    source: &str,
    trivia: &[Trivia],
) {
    // Asked before the table is built. Parsing the library is cheap and
    // happens once for the process, but a file with nothing unresolved in it
    // is the common case and should not pay for the answer to a question it
    // did not ask.
    if !diagnostics.iter().any(|diagnostic| {
        diagnostic.code == codes::UNKNOWN_NAME || diagnostic.code == typeck_codes::NO_SUCH_FIELD
    }) {
        return;
    }
    let table = declared_by();

    named_as_a_method(diagnostics, source, table);

    // Names to import, by module, and the diagnostics that asked for them.
    let mut wanted: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    let mut first_repairable: Option<usize> = None;

    for (index, diagnostic) in diagnostics.iter_mut().enumerate() {
        if diagnostic.code != codes::UNKNOWN_NAME {
            continue;
        }
        let span = diagnostic.primary.span;
        let Some(name) = source.get(span.start as usize..span.end as usize) else {
            continue;
        };
        let Some(homes) = table.get(name) else {
            continue;
        };

        // A name the resolver could point at somewhere in this file is a
        // question about scope rather than a missing import, and writing the
        // `use` line would answer a different question confidently. The
        // measured case is a walk's accumulator read after its walk: `sum` is
        // in `std/list`, and importing it there is worse than saying nothing.
        if !diagnostic.secondary.is_empty() {
            continue;
        }

        match homes.as_slice() {
            [home] => {
                *diagnostic = diagnostic.clone().with_note(format!(
                    "`{home}` declares `{name}` and ships with the compiler, and this file does \
                     not import it"
                ));
                wanted.entry(home).or_default().insert(name.to_string());
                first_repairable.get_or_insert(index);
            }
            homes => {
                let named: Vec<String> = homes.iter().map(|home| format!("`{home}`")).collect();
                *diagnostic = diagnostic.clone().with_note(format!(
                    "{} each declare `{name}` and ship with the compiler, so which one this \
                     file should import is a question about which one it means",
                    named.join(" and ")
                ));
            }
        }
    }

    let Some(index) = first_repairable else {
        return;
    };
    let Some((span, replacement)) = rewritten_block(module, source, trivia, &wanted) else {
        return;
    };

    let message = match wanted.values().flatten().collect::<Vec<_>>().as_slice() {
        [only] => format!("import `{only}`"),
        _ => "import them from the library that ships".to_string(),
    };
    diagnostics[index] = diagnostics[index].clone().with_fix(
        message,
        span,
        replacement,
        Applicability::MachineApplicable,
    );
}

/// `table.set(key, value)`, where `set` is a name the library has.
///
/// The type checker already answers this for the prelude, because it can ask
/// whether the name is a builtin. It cannot ask about `std/table`, which is a
/// module the driver resolves and the checker only ever sees through imports
/// this file did not write. So the sentence is finished here, on the same
/// shelf and off the same table as the `use` line above.
///
/// A note and no repair. Adding the import leaves `x.set(k, v)` exactly as
/// broken as it was, and rewriting the call is a change to the shape of the
/// line rather than an edit to a name, so a fix here would either do nothing
/// or guess.
fn named_as_a_method(
    diagnostics: &mut [Diagnostic],
    source: &str,
    table: &BTreeMap<String, Vec<&'static str>>,
) {
    for diagnostic in diagnostics.iter_mut() {
        if diagnostic.code != typeck_codes::NO_SUCH_FIELD {
            continue;
        }
        let span = diagnostic.primary.span;
        let Some(name) = source.get(span.start as usize..span.end as usize) else {
            continue;
        };
        let Some(homes) = table.get(name) else {
            continue;
        };
        let named: Vec<String> = homes.iter().map(|home| format!("`{home}`")).collect();
        *diagnostic = diagnostic.clone().with_note(format!(
            "there are no methods, and {} {} `{name}`, which takes the value as its first \
             argument: `{name}(x, ..)` rather than `x.{name}(..)`",
            named.join(" and "),
            if homes.len() == 1 {
                "declares"
            } else {
                "each declare"
            },
        ));
    }
}

/// The `use` block this file should have, and the span it replaces.
///
/// Merged into whatever is already there rather than appended after it, so the
/// result is what `deed fmt` would print: one line per module, names in the
/// order they are written, and the file's own imports left where they were.
fn rewritten_block(
    module: &Module,
    source: &str,
    trivia: &[Trivia],
    wanted: &BTreeMap<&'static str, BTreeSet<String>>,
) -> Option<(Span, String)> {
    let mut lines: Vec<(String, Vec<String>)> = module
        .uses
        .iter()
        .map(|import| {
            (
                import.path.to_string_path(),
                import
                    .names
                    .iter()
                    .map(|name| name.name.clone())
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    for (home, names) in wanted {
        match lines.iter_mut().find(|(path, _)| path == home) {
            Some((_, existing)) => {
                for name in names {
                    if !existing.contains(name) {
                        existing.push(name.clone());
                    }
                }
                existing.sort();
            }
            None => lines.push(((*home).to_string(), names.iter().cloned().collect())),
        }
    }

    let mut replacement = String::new();
    for (path, names) in &lines {
        replacement.push_str("use ");
        replacement.push_str(path);
        replacement.push_str(".{");
        replacement.push_str(&names.join(", "));
        replacement.push_str("}\n");
    }

    match (module.uses.first(), module.uses.last()) {
        (Some(first), Some(last)) => {
            let start = line_start(source, first.span.start);
            let end = line_end(source, last.span.end);
            let block = Span::new(start, end);
            // A comment among the imports is about one of them, and moving it
            // is not the compiler's to do. Same refusal as `crate::imports`.
            if trivia
                .iter()
                .any(|comment| block.contains(comment.span.start))
            {
                return None;
            }
            Some((block, replacement))
        }
        // No imports yet, so the block goes under the `module` line with a
        // blank line on each side. Without a `module` line there is nowhere
        // to put it that is not a guess, and that file has a bigger problem.
        _ => {
            let name = module.name.as_ref()?;
            let after = line_end(source, name.span.end);
            // Whether the next line is already blank, asked of the line rather
            // than of the byte: a file written on Windows has a `\r` in front
            // of the newline and would otherwise be given a second blank line.
            let rest = &source[after as usize..];
            let already_blank = rest.trim_start_matches('\r').starts_with('\n');
            let blank = if already_blank { "" } else { "\n" };
            Some((Span::new(after, after), format!("\n{replacement}{blank}")))
        }
    }
}
