//! Applying the fixes diagnostics carry.
//!
//! P7 says a diagnostic carries an applicable patch where the fix is
//! unambiguous. Nothing applied them, so what P7 described was a data
//! structure rather than something a machine does. This is the part that does
//! it.
//!
//! What it can apply is what a single span and a replacement can express,
//! which is where every fix in the compiler comes from today: the lexer, the
//! parser and the resolver. The type checker and the effect checker have none,
//! and the second one is the one that hurts, because those diagnostics already
//! say the words. `VOW5001` names the effect, names the function and tells the
//! reader to add it to the `uses` clause, and then does not add it. A span
//! cannot say that: removing an entry has to take its comma and its line with
//! it, adding one has to match the indentation, and adding one where there is
//! no clause at all has to invent the whole block. The pass that knows what
//! the repair is does not know how to write it, and the passes that know how
//! to write it are this one and the printer. See issue #159.
//!
//! Only [`Applicability::MachineApplicable`] fixes are applied. There is no
//! flag to apply the others, because a fix that is a guess is a fix a person
//! has to look at, and a flag for applying guesses is a flag someone turns on
//! once and then forgets about.

use vow_diagnostics::{Applicability, Diagnostic, SourceMap, Span, SuggestedEdit};

/// How many rounds of fix-then-recheck to run before giving up.
///
/// Two fixes that undo each other would otherwise loop. A bound is the cheap
/// version of proving they cannot, and hitting it is reported rather than
/// hidden.
const ROUNDS: usize = 8;

/// What one file's fixing produced.
#[derive(Debug)]
pub struct Fixed {
    pub source: String,
    /// How many edits went in, across every round.
    pub applied: usize,
    /// True when the bound was reached with changes still pending, which means
    /// something is oscillating and the result should not be trusted as final.
    pub gave_up: bool,
}

impl Fixed {
    pub fn changed(&self) -> bool {
        self.applied > 0
    }
}

/// Applies every machine-applicable fix, re-checking between rounds.
///
/// `check` is handed the current text and returns the diagnostics for it. It
/// is a parameter rather than a call into the driver so this stays testable
/// without a whole pipeline, and so the caller decides what "checked" means.
pub fn fix(source: &str, mut check: impl FnMut(&str) -> Vec<Diagnostic>) -> Fixed {
    let mut current = source.to_string();
    let mut applied = 0;

    for _ in 0..ROUNDS {
        let diagnostics = check(&current);
        let edits = collect(&diagnostics);
        if edits.is_empty() {
            return Fixed {
                source: current,
                applied,
                gave_up: false,
            };
        }

        let next = apply(&current, &edits);
        if next == current {
            // The edits were all no-ops, so another round would find the same
            // ones and change nothing again.
            return Fixed {
                source: current,
                applied,
                gave_up: false,
            };
        }

        applied += edits.len();
        current = next;
    }

    Fixed {
        source: current,
        applied,
        gave_up: true,
    }
}

/// The edits worth applying, with overlaps dropped.
///
/// Two fixes that touch the same range cannot both be right, and applying one
/// of them would leave the other's span pointing at text that moved. Dropping
/// both is the answer that does not depend on which order they came in.
fn collect(diagnostics: &[Diagnostic]) -> Vec<SuggestedEdit> {
    let mut edits: Vec<SuggestedEdit> = diagnostics
        .iter()
        .filter_map(|diagnostic| diagnostic.fix.as_ref())
        .filter(|fix| fix.applicability == Applicability::MachineApplicable)
        .flat_map(|fix| fix.edits.iter().cloned())
        .collect();

    edits.sort_by_key(|edit| (edit.span.start, edit.span.end));

    let mut kept: Vec<SuggestedEdit> = Vec::with_capacity(edits.len());
    let mut index = 0;
    while index < edits.len() {
        let mut last = index;
        while last + 1 < edits.len() && overlaps(&edits[last].span, &edits[last + 1].span) {
            last += 1;
        }
        if last == index {
            kept.push(edits[index].clone());
        }
        index = last + 1;
    }
    kept
}

fn overlaps(a: &Span, b: &Span) -> bool {
    // Touching at a point is not overlapping: an insertion at the end of one
    // edit and an insertion at the start of the next are independent.
    a.end > b.start && b.end > a.start
}

/// Applies non-overlapping edits, back to front so earlier spans stay valid.
fn apply(source: &str, edits: &[SuggestedEdit]) -> String {
    let mut out = source.to_string();
    for edit in edits.iter().rev() {
        let start = edit.span.start as usize;
        let end = edit.span.end as usize;
        if start > end
            || end > out.len()
            || !out.is_char_boundary(start)
            || !out.is_char_boundary(end)
        {
            // A span that does not name a range of this text is a bug in
            // whoever produced it, and rewriting from it would corrupt the
            // file. Skipping is the only safe thing left.
            continue;
        }
        out.replace_range(start..end, &edit.replacement);
    }
    out
}

/// How many of `diagnostics` are errors, for the "never made it worse" check.
pub fn error_count(diagnostics: &[Diagnostic]) -> usize {
    diagnostics.iter().filter(|d| d.is_error()).count()
}

/// Renders diagnostics for a message, so callers do not each write this.
pub fn render_all(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| vow_diagnostics::render_human(sources, diagnostic))
        .collect::<Vec<_>>()
        .join("\n")
}
