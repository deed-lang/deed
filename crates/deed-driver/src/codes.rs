//! Diagnostic codes produced by the manifest parser.
//!
//! The manifest parser owns the `DEED7xxx` range. Codes are stable and never
//! reused.
//!
//! A `deed.manifest` file is plain text, not Deed source, so it has its own
//! parser and its own error range rather than reusing the Deed parser's range.

/// A line in the manifest that is not blank, not a comment, and not a
/// recognized directive.
///
/// The only directive today is `component <path>`. Anything else on a
/// non-blank, non-comment line is this error.
pub const UNKNOWN_DIRECTIVE: &str = "DEED7001";

/// A `component` directive with no path following it.
///
/// `component` on a line by itself, or followed only by whitespace, does not
/// say where the component lives and cannot be acted on.
pub const MISSING_COMPONENT_PATH: &str = "DEED7002";
