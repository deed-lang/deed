//! Diagnostic codes produced by the parser.
//!
//! The parser owns the `VOW2xxx` range. Codes are stable and never reused.

/// A token that cannot appear where it was found.
pub const UNEXPECTED_TOKEN: &str = "VOW2001";

/// A file that does not begin with a `module` declaration.
pub const MISSING_MODULE_DECLARATION: &str = "VOW2002";

/// A token at the top level that cannot begin a declaration.
pub const EXPECTED_DECLARATION: &str = "VOW2003";

/// A contract clause given twice on one function.
pub const DUPLICATE_CONTRACT_CLAUSE: &str = "VOW2004";

/// An `ensures` obligation whose outcome is neither `ok` nor `err`.
pub const INVALID_ENSURES_OUTCOME: &str = "VOW2005";

/// Contract clauses written in an order other than `where`, `uses`, `ensures`.
///
/// P4 says there is one canonical form. Clause order is part of it: a signature
/// is the review surface, and it should read the same way every time.
pub const CONTRACT_CLAUSE_ORDER: &str = "VOW2006";
