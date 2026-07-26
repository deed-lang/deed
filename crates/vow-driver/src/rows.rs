//! Writing the repair a row diagnostic can describe.
//!
//! `VOW5001` names the effect, names the function and tells the reader to add
//! it to the `uses` clause. `VOW5002` names the entry that has to come out and
//! points at it. Neither carried a fix, and for a while a comment claimed
//! otherwise.
//!
//! The reason is worth stating, because it is a layering fact rather than an
//! oversight. A [`SuggestedEdit`] is a span and a replacement, and writing one
//! here means knowing about commas, about indentation, and about a clause that
//! may not exist yet. The effect checker has the answer and no business
//! knowing any of that. This module is where the answer, the text and the one
//! canonical layout are all in scope at once.
//!
//! What it will not do is guess. A contract holding a `where` or an `ensures`
//! is left alone, because the region between a signature and its body holds
//! all three and there is nothing in the tree that says where one clause stops
//! and the next starts. A comment inside that region is left alone for the
//! same reason: rewriting the block would eat it, and a machine-applicable fix
//! that deletes a comment is a fix nobody should have applied.

use vow_ast::{Item, Module};
use vow_diagnostics::{Applicability, Diagnostic, Span};
use vow_effects::{Effects, codes};
use vow_lexer::Trivia;
use vow_resolve::Resolutions;

/// Attaches a fix to the row diagnostics that can have one.
///
/// One fix per function, however many diagnostics it produced. A row that is
/// too narrow and too wide at once is one repair, and two edits over the same
/// span would be dropped as overlapping anyway.
pub(crate) fn attach_fixes(
    diagnostics: &mut [Diagnostic],
    module: &Module,
    resolutions: &Resolutions,
    effects: &Effects,
    source: &str,
    trivia: &[Trivia],
) {
    for item in &module.items {
        let Item::Function(function) = item else {
            continue;
        };
        // Only the region between the signature and the body, and only when
        // that region is nothing but a `uses` clause or nothing at all.
        if !function.contract.requires.is_empty() || !function.contract.ensures.is_empty() {
            continue;
        }
        let Some(def) = resolutions.resolution(function.sig.name.span) else {
            continue;
        };
        let Some(performed) = effects.performed(def) else {
            continue;
        };
        if effects.is_unverifiable(def) {
            continue;
        }

        let span = Span::new(function.sig.span.end, function.body.span.start);
        if trivia
            .iter()
            .any(|comment| comment.span.start >= span.start && comment.span.start < span.end)
        {
            continue;
        }

        let Some(target) = diagnostics.iter_mut().find(|diagnostic| {
            matches!(
                diagnostic.code,
                codes::UNDECLARED_EFFECT | codes::UNUSED_EFFECT
            ) && diagnostic.primary.span.start >= function.span.start
                && diagnostic.primary.span.start < function.span.end
        }) else {
            continue;
        };

        let mut entries: Vec<String> = performed
            .iter()
            .map(|item| match &item.operation {
                Some(operation) => format!("{}.{operation}", resolutions.def(item.effect).name),
                None => resolutions.def(item.effect).name.clone(),
            })
            .collect();
        // Sorted, because a generated row has no written order to preserve and
        // the same program has to produce the same text.
        entries.sort();

        let replacement = clause(&entries);
        if source.get(span.start as usize..span.end as usize) == Some(replacement.as_str()) {
            continue;
        }

        *target = target.clone().with_fix(
            match entries.len() {
                0 => "remove the `uses` clause".to_string(),
                1 => format!("declare `{}`", entries[0]),
                _ => "declare what this performs".to_string(),
            },
            span,
            replacement,
            Applicability::MachineApplicable,
        );
    }
}

/// The canonical text between a signature and its body.
///
/// One space when there is nothing to say, which is what `fn f() -> Int {` has
/// there, and the same shape `vow fmt` prints otherwise. Written out rather
/// than borrowed from the printer because the printer works on a whole file
/// and this is one region of one, and because the layout of a clause with
/// nothing in it but effect names is four lines of rules rather than a
/// dependency.
fn clause(entries: &[String]) -> String {
    if entries.is_empty() {
        return " ".to_string();
    }
    let mut text = "\n  uses\n".to_string();
    for entry in entries {
        text.push_str("    ");
        text.push_str(entry);
        text.push_str(",\n");
    }
    text
}
