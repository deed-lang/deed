//! Source management and structured diagnostics for the Deed compiler.
//!
//! Nothing here knows anything about the Deed language. It exists so that no
//! part of the compiler ever has a reason to return an error as a string.
//!
//! [`hashing`] is the odd one out and it is here because [`Span`] is: every
//! pass keys a map by one, and the hash they are looked up with is a property
//! of the key rather than of the pass.
//!
//! See `design/01-principles.md`, P7.

pub mod diagnostic;
pub mod hashing;
pub mod render;
pub mod source;
pub mod span;

pub use diagnostic::{Applicability, Diagnostic, Fix, Label, Severity, SuggestedEdit};
pub use hashing::ByNumber;
pub use render::{json_string, render_human, render_json};
pub use source::{FileId, Location, SourceFile, SourceMap};
pub use span::Span;
