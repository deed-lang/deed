//! Source management and structured diagnostics for the Vow compiler.
//!
//! Nothing here knows anything about the Vow language. It exists so that no
//! part of the compiler ever has a reason to return an error as a string.
//!
//! See `design/01-principles.md`, P7.

pub mod diagnostic;
pub mod render;
pub mod source;
pub mod span;

pub use diagnostic::{Applicability, Diagnostic, Fix, Label, Severity, SuggestedEdit};
pub use render::{render_human, render_json};
pub use source::{FileId, Location, SourceFile, SourceMap};
pub use span::Span;
