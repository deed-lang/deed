//! Writing the repair an unused import can describe.
//!
//! `DEED3003` says a name was imported and never used, and there is exactly one
//! thing to do about it. The resolver knows which name, and that is all it
//! knows: taking a name out of `use a/b.{X, Y}` is a question about a comma,
//! and taking the last one out is a question about a line and the blank line
//! beside it. Same layering as [`crate::rows`], same answer, so it lives next
//! to it.
//!
//! One fix for the whole block rather than one per name. Two names out of the
//! same list are two edits over overlapping text, which [`crate::fix::collect`]
//! drops, and two whole lines removed separately each leave behind the blank
//! line that used to sit beside them, so the file ends up with a gap that no
//! longer separates anything. Rewriting the block once has neither problem,
//! and the replacement is what `deed fmt` would print for whatever is left.
//!
//! It declines on a comment anywhere in the block, for the reason
//! [`crate::rows`] does. A comment beside an import is usually about that
//! import, but it is not the compiler's to move and it is certainly not the
//! compiler's to delete.

use deed_ast::Module;
use deed_diagnostics::{Applicability, Diagnostic, Span};
use deed_lexer::Trivia;
use deed_resolve::codes;

/// Attaches the fix that takes out every unused import in one go.
pub(crate) fn attach_fixes(
    diagnostics: &mut [Diagnostic],
    module: &Module,
    source: &str,
    trivia: &[Trivia],
) {
    let (Some(first), Some(last)) = (module.uses.first(), module.uses.last()) else {
        return;
    };

    // Keyed by span rather than by name, because two modules can export the
    // same name and a diagnostic is about one of them.
    let unused: Vec<Span> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == codes::UNUSED_IMPORT)
        .map(|diagnostic| diagnostic.primary.span)
        .collect();
    if unused.is_empty() {
        return;
    }

    let start = line_start(source, first.span.start);
    let mut end = line_end(source, last.span.end);
    if trivia
        .iter()
        .any(|comment| comment.span.start >= start && comment.span.start < end)
    {
        return;
    }

    let mut replacement = String::new();
    let mut removed: Vec<&str> = Vec::new();
    for import in &module.uses {
        let mut kept: Vec<&str> = Vec::new();
        for name in &import.names {
            if unused.contains(&name.span) {
                removed.push(name.name.as_str());
            } else {
                kept.push(name.name.as_str());
            }
        }
        if kept.is_empty() {
            continue;
        }
        replacement.push_str("use ");
        replacement.push_str(&import.path.to_string_path());
        replacement.push_str(".{");
        replacement.push_str(&kept.join(", "));
        replacement.push_str("}\n");
    }

    // Nothing survives, so the block goes and one blank line goes with it. Two
    // blanks were separating the imports from what came before and from what
    // follows, and with the imports gone there is one gap to leave rather than
    // two.
    if replacement.is_empty()
        && source[..start as usize].ends_with("\n\n")
        && source[end as usize..].starts_with('\n')
    {
        end += 1;
    }

    if source.get(start as usize..end as usize) == Some(replacement.as_str()) {
        return;
    }

    let message = match removed.as_slice() {
        [only] => format!("remove `{only}`"),
        _ => "remove the imports nothing uses".to_string(),
    };
    let Some(target) = diagnostics
        .iter_mut()
        .find(|diagnostic| diagnostic.code == codes::UNUSED_IMPORT)
    else {
        return;
    };
    *target = target.clone().with_fix(
        message,
        Span::new(start, end),
        replacement,
        Applicability::MachineApplicable,
    );
}

/// The offset just past the newline that ends the line `offset` is on.
fn line_end(source: &str, offset: u32) -> u32 {
    match source[offset as usize..].find('\n') {
        Some(index) => offset + index as u32 + 1,
        None => source.len() as u32,
    }
}

/// The start of the line `offset` is on.
///
/// A `Use` span begins at the path rather than at the keyword, because that is
/// where the parser starts building one. The line is what has to be replaced.
fn line_start(source: &str, offset: u32) -> u32 {
    match source[..offset as usize].rfind('\n') {
        Some(index) => index as u32 + 1,
        None => 0,
    }
}
