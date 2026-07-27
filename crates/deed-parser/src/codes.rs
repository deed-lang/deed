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

/// A word in front of a `let` name, such as `mut`, that the language has no
/// place for.
///
/// It has a code of its own because taking `mut` as the name is the reading
/// that produces the most and the least useful messages of any single word a
/// newcomer writes, and none of them names the word.
pub const NO_BINDING_MODIFIER: &str = "DEED2009";

/// A binding written without `let`, either behind another language's keyword
/// (`var n = 1`) or behind its type (`Int n = 1`).
///
/// Both used to arrive as a name nobody declared and an assignment to a name
/// nobody declared, which is two messages about the halves and none about the
/// line. The shape is safe to read because two names in a row on one line is
/// not a statement here.
pub const BINDING_WITHOUT_LET: &str = "DEED2010";
