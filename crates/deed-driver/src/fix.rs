//! Applying the fixes diagnostics carry.
//!
//! P7 says a diagnostic carries an applicable patch where the fix is
//! unambiguous. Nothing applied them, so what P7 described was a data
//! structure rather than something a machine does. This is the part that does
//! it.
//!
//! A fix is usually a span and a replacement, so most of them are written
//! where the problem is found: the lexer, the parser and the resolver each
//! hand one over with the diagnostic. `DEED2012` is the one that is not, and
//! it is not by a little: turning `n as String` into `to_string(n)` is an
//! insertion in front of the value and a replacement behind it, so a fix is
//! taken or refused whole and never in pieces.
//!
//! The row diagnostics cannot write theirs where they are found, and they are
//! the ones that would pay, because they already say the words. `DEED5001`
//! names the effect, names the function and tells the reader to add it to the
//! `uses` clause. Saying that as a span means knowing about commas, about
//! indentation, and about a clause that may not exist yet, and the effect
//! checker has the answer and no business knowing any of that. So
//! [`crate::rows`] writes those, where the text, the tree and the one canonical
//! layout are all in scope at once. [`crate::imports`] is the same shape one
//! pass earlier: the resolver knows which import is unused and nothing about
//! the comma beside it.
//!
//! The type checker has one, and only one. There is no obvious repair for a
//! type that does not fit, which is the difference between a fix that is
//! missing and a fix that is not there to be had. `DEED4026` is the exception:
//! a statement whose value nobody reads has exactly one mechanical answer,
//! which is to say out loud that it is being dropped.
//!
//! Only [`Applicability::MachineApplicable`] fixes are applied. There is no
//! flag to apply the others, because a fix that is a guess is a fix a person
//! has to look at, and a flag for applying guesses is a flag someone turns on
//! once and then forgets about. Where a person looks at one is the editor:
//! `deed-lsp` offers every fix as a quick fix and marks the certain ones
//! preferred, which is the same distinction spelled the way an editor reads
//! it. The two warnings about something going nowhere, `DEED3009` and
//! `DEED4026`, are guesses for the same reason as each other: the other way to
//! arrive at either is a value that was supposed to be read, and papering over
//! that in bulk is the one thing they must not do.

use deed_diagnostics::{Applicability, Diagnostic, Fix, SourceMap, Span, SuggestedEdit};

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
    /// How many repairs went in, across every round.
    ///
    /// Repairs and not edits: one that wraps something is two edits and one
    /// answer to one diagnostic, and what a person counting wants is the
    /// number of things that were wrong.
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
        let repairs = collect(&diagnostics);
        if repairs.edits.is_empty() {
            return Fixed {
                source: current,
                applied,
                gave_up: false,
            };
        }

        let next = apply(&current, &repairs.edits);
        if next == current {
            // The edits were all no-ops, so another round would find the same
            // ones and change nothing again.
            return Fixed {
                source: current,
                applied,
                gave_up: false,
            };
        }

        applied += repairs.fixes;
        current = next;
    }

    Fixed {
        source: current,
        applied,
        gave_up: true,
    }
}

/// The edits worth applying, and how many repairs they came from.
struct Repairs {
    edits: Vec<SuggestedEdit>,
    fixes: usize,
}

/// The edits worth applying, with overlaps dropped.
///
/// Two fixes that touch the same range cannot both be right, and applying one
/// of them would leave the other's span pointing at text that moved. Dropping
/// both is the answer that does not depend on which order they came in.
///
/// A fix is refused whole. Most carry one edit, but a repair that wraps
/// something carries two, and half of that is not a smaller repair: it is
/// `to_string(n as String`, which is worse than the line it replaced.
fn collect(diagnostics: &[Diagnostic]) -> Repairs {
    let fixes: Vec<&Fix> = diagnostics
        .iter()
        .filter_map(|diagnostic| diagnostic.fix.as_ref())
        .filter(|fix| fix.applicability == Applicability::MachineApplicable)
        .collect();

    let mut edits: Vec<(usize, SuggestedEdit)> = fixes
        .iter()
        .enumerate()
        .flat_map(|(owner, fix)| fix.edits.iter().cloned().map(move |edit| (owner, edit)))
        .collect();

    edits.sort_by_key(|(_, edit)| (edit.span.start, edit.span.end));

    // Walk adjacent pairs only. A cluster is every run of edits where each
    // one overlaps the next; non-adjacent overlap is impossible after the
    // sort by start. A plain `for` over the pairs cannot hang: every mutant
    // of the old `index = last + 1` form either still advances or fails a
    // test, instead of spinning until the timeout.
    let mut refused = vec![false; fixes.len()];
    let mut run_start = 0;
    for i in 1..=edits.len() {
        let run_breaks = i == edits.len() || !overlaps(&edits[i - 1].1.span, &edits[i].1.span);
        if run_breaks {
            if i - run_start > 1 {
                for (owner, _) in &edits[run_start..i] {
                    refused[*owner] = true;
                }
            }
            run_start = i;
        }
    }

    Repairs {
        edits: edits
            .into_iter()
            .filter(|(owner, _)| !refused[*owner])
            .map(|(_, edit)| edit)
            .collect(),
        fixes: refused.iter().filter(|refused| !**refused).count(),
    }
}

/// Whether two start-sorted spans clash.
///
/// `collect` only asks about adjacent edits after sorting by start, so the
/// earlier span is always `a` and the later is always `b`. Under that order a
/// pure touch is `a.end == b.start`, and the only comparison that matters is
/// whether the earlier span ends past the later's start. Flip `>` to `>=` and
/// the touch pair in `fixes_that_only_touch_at_a_point_both_go_in` is refused.
fn overlaps(a: &Span, b: &Span) -> bool {
    a.end > b.start
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
        .map(|diagnostic| deed_diagnostics::render_human(sources, diagnostic))
        .collect::<Vec<_>>()
        .join("\n")
}
