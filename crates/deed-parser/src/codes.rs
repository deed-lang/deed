//! Diagnostic codes produced by the parser.
//!
//! The parser owns the `DEED2xxx` range. Codes are stable and never reused.

/// A token that cannot appear where it was found.
pub const UNEXPECTED_TOKEN: &str = "DEED2001";

/// A file that does not begin with a `module` declaration.
pub const MISSING_MODULE_DECLARATION: &str = "DEED2002";

/// A token at the top level that cannot begin a declaration.
pub const EXPECTED_DECLARATION: &str = "DEED2003";

/// A contract clause given twice on one function.
pub const DUPLICATE_CONTRACT_CLAUSE: &str = "DEED2004";

/// An `ensures` obligation whose outcome is neither `ok` nor `err`.
pub const INVALID_ENSURES_OUTCOME: &str = "DEED2005";

/// Contract clauses written in an order other than `where`, `uses`, `ensures`.
///
/// P4 says there is one canonical form. Clause order is part of it: a signature
/// is the review surface, and it should read the same way every time.
pub const CONTRACT_CLAUSE_ORDER: &str = "DEED2006";

/// A function parameter written without a type.
///
/// P5 says nothing implicit crosses a boundary, and a parameter is the
/// boundary. An untyped one used to become the unknown type, which agrees with
/// everything, so every mistake made with it was invisible and a closure could
/// carry any effect through it into a function that declared none.
pub const MISSING_PARAMETER_TYPE: &str = "DEED2007";

/// A choice variant given its payload by position rather than by name.
///
/// `Circle(Int)` is what anyone arriving from a language with tuple variants
/// writes first, and it is refused. Saying so as "expected `}`" reads as a
/// typo in a line that has none, so it has a code of its own.
///
/// Whether it should be refused at all is open. `ok` and `err` carry a value
/// positionally and are built in, which is the shortcut `design/02-syntax.md`
/// records under what holds `Result` in the language.
pub const POSITIONAL_VARIANT: &str = "DEED2008";
