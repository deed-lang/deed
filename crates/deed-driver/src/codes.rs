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

/// A `module` directive with nothing after it.
///
/// A location says where the bytes were and a hash says what they are. With
/// neither there is nothing to fetch and nothing to check it against.
pub const MISSING_MODULE_SOURCE: &str = "DEED7003";

/// A `module` directive with a location and no hash.
///
/// Separate from [`MISSING_MODULE_SOURCE`] because it is a different mistake
/// with a different answer: the reader knows where the module is and has to be
/// told that knowing is not enough. A dependency without a hash is one whose
/// bytes are whatever the other end felt like today, and a build that accepts
/// those cannot be repeated.
pub const MISSING_MODULE_HASH: &str = "DEED7004";

/// A hash that is not sixty-four lowercase hexadecimal digits after `sha256:`.
///
/// One code for every way of writing it wrong, because they are all the same
/// mistake seen from different angles and each one names what it found.
pub const BAD_MODULE_HASH: &str = "DEED7005";

/// Bytes that could not be retrieved, or that did not hash to what the
/// manifest said they would.
///
/// A mismatch is never a warning and never a retry. The cache is keyed by the
/// expected hash, so bytes that do not match are bytes for a different
/// dependency, and storing them would answer a later build with them.
pub const MODULE_NOT_FETCHED: &str = "DEED7006";
