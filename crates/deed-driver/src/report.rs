//! The JSON one obligation and one diagnostic look like, in one place.
//!
//! Written here rather than in `deed-cli` so that a second caller (the wasm
//! surface, see #587) reads the same shape rather than inventing a third one.
//! `deed-cli`'s own JSON output is this function, called and written to
//! stdout; nothing about the shape lives twice.

use deed_diagnostics::{SourceMap, render_json};

use crate::{Checked, ObligationReport};

/// One JSON object a line: every diagnostic from every file in `checks`, and
/// every obligation too when `obligations` is set.
///
/// The same two `kind`s `deed check --format json` has always written:
/// `"diagnostic"` and `"obligation"`. A caller wanting only one file's worth
/// passes a slice of one `Checked`.
pub fn json_report(sources: &SourceMap, checks: &[Checked], obligations: bool) -> String {
    let mut out = String::new();

    for checked in checks {
        for diagnostic in &checked.diagnostics {
            out.push_str(&format!(
                "{{\"kind\":\"diagnostic\",\"diagnostic\":{}}}\n",
                render_json(sources, diagnostic)
            ));
        }
    }

    if obligations {
        for checked in checks {
            let file = sources.file(checked.file);
            for ObligationReport {
                tier,
                span,
                subject,
                reason,
            } in &checked.obligations
            {
                let location = file.location(span.start);
                let reason = match reason {
                    Some(reason) => format!("\"{}\"", reason.text()),
                    None => "null".to_string(),
                };
                out.push_str(&format!(
                    "{{\"kind\":\"obligation\",\"tier\":\"{}\",\"file\":\"{}\",\"line\":{},\"column\":{},\"subject\":\"{}\",\"reason\":{}}}\n",
                    tier.name(),
                    file.name(),
                    location.line,
                    location.column,
                    subject,
                    reason
                ));
            }
        }
    }

    out
}
