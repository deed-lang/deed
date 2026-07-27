//! Structured diagnostics.
//!
//! P7 in `design/01-principles.md` says compiler output is an API. That has one
//! concrete consequence here: a diagnostic is data, and the human readable text
//! is a rendering of that data rather than the thing itself. Nothing in the
//! compiler is allowed to produce an error as a bare `String`.

use crate::source::FileId;
use crate::span::Span;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// A span with something to say about it.
#[derive(Clone, Debug)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

impl Label {
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

/// How much a tool should trust a fix.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Applicability {
    /// The fix is certainly what was meant and can be applied without asking.
    MachineApplicable,
    /// The fix is a guess. Offer it, do not apply it.
    MaybeIncorrect,
}

impl Applicability {
    pub fn as_str(self) -> &'static str {
        match self {
            Applicability::MachineApplicable => "machine-applicable",
            Applicability::MaybeIncorrect => "maybe-incorrect",
        }
    }
}

/// A single text replacement.
#[derive(Clone, Debug)]
pub struct SuggestedEdit {
    pub span: Span,
    pub replacement: String,
}

/// A set of edits that together resolve a diagnostic.
#[derive(Clone, Debug)]
pub struct Fix {
    pub message: String,
    pub edits: Vec<SuggestedEdit>,
    pub applicability: Applicability,
}

/// One problem, at one place, with one cause.
///
/// Cascades are deliberately not modelled. If a single root cause would produce
/// several diagnostics, the producer is expected to emit one diagnostic with
/// secondary labels instead, because a wall of derived errors buries the line
/// that actually needs editing.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    /// Stable identifier, for example `DEED1002`. Codes are never reused.
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub file: FileId,
    pub primary: Label,
    pub secondary: Vec<Label>,
    pub notes: Vec<String>,
    pub fix: Option<Fix>,
}

impl Diagnostic {
    pub fn error(code: &'static str, file: FileId, span: Span, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code,
            severity: Severity::Error,
            primary: Label::new(span, message.clone()),
            message,
            file,
            secondary: Vec::new(),
            notes: Vec::new(),
            fix: None,
        }
    }

    pub fn warning(
        code: &'static str,
        file: FileId,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        let mut diagnostic = Self::error(code, file, span, message);
        diagnostic.severity = Severity::Warning;
        diagnostic
    }

    /// Overrides the primary label, when the underline should say something
    /// shorter or more specific than the headline message.
    #[must_use]
    pub fn with_primary_label(mut self, message: impl Into<String>) -> Self {
        self.primary.message = message.into();
        self
    }

    #[must_use]
    pub fn with_secondary(mut self, span: Span, message: impl Into<String>) -> Self {
        self.secondary.push(Label::new(span, message));
        self
    }

    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    #[must_use]
    pub fn with_fix(
        mut self,
        message: impl Into<String>,
        span: Span,
        replacement: impl Into<String>,
        applicability: Applicability,
    ) -> Self {
        self.fix = Some(Fix {
            message: message.into(),
            edits: vec![SuggestedEdit {
                span,
                replacement: replacement.into(),
            }],
            applicability,
        });
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

#[cfg(test)]
mod tests {
    use super::{Applicability, Diagnostic, Severity};
    use crate::source::SourceMap;
    use crate::span::Span;

    #[test]
    fn primary_label_defaults_to_the_message() {
        let mut map = SourceMap::new();
        let file = map.add("t.deed", "let x = 1");
        let d = Diagnostic::error("DEED0001", file, Span::new(0, 3), "something went wrong");
        assert_eq!(d.primary.message, "something went wrong");
        assert!(d.is_error());
    }

    #[test]
    fn builders_compose() {
        let mut map = SourceMap::new();
        let file = map.add("t.deed", "let x = 1");
        let d = Diagnostic::warning("DEED0002", file, Span::new(0, 3), "headline")
            .with_primary_label("here")
            .with_secondary(Span::new(4, 5), "related")
            .with_note("a note")
            .with_fix(
                "try this",
                Span::new(0, 3),
                "val",
                Applicability::MachineApplicable,
            );

        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.primary.message, "here");
        assert_eq!(d.secondary.len(), 1);
        assert_eq!(d.notes, vec!["a note".to_string()]);
        assert_eq!(d.fix.unwrap().edits[0].replacement, "val");
    }
}
