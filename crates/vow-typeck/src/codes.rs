//! Diagnostic codes produced by type checking.
//!
//! The type checker owns the `VOW4xxx` range. Codes are stable and never reused.

/// A value of one type used where another was required.
pub const TYPE_MISMATCH: &str = "VOW4001";

/// A record or variant literal that leaves fields out.
pub const MISSING_FIELDS: &str = "VOW4002";

/// A record or variant literal that names a field the type does not have.
pub const UNKNOWN_FIELD: &str = "VOW4003";

/// Field access on a type with no such field.
pub const NO_SUCH_FIELD: &str = "VOW4004";

/// A literal whose path is not a record or a variant.
pub const NOT_A_CONSTRUCTOR: &str = "VOW4005";

/// A `match` that does not cover every variant.
pub const NON_EXHAUSTIVE_MATCH: &str = "VOW4006";

/// A call with the wrong number of arguments.
pub const WRONG_ARITY: &str = "VOW4007";

/// A refinement that could not be discharged.
///
/// Not a rejection of the program so much as an admission of what the compiler
/// can currently prove. See the tier table in `design/02-syntax.md`.
pub const UNPROVEN_REFINEMENT: &str = "VOW4008";

/// A refinement the compiler proved false.
pub const VIOLATED_REFINEMENT: &str = "VOW4009";

/// A type alias that expands to itself.
pub const TYPE_ALIAS_CYCLE: &str = "VOW4010";

/// A name used in type position that does not name a type.
pub const NOT_A_TYPE: &str = "VOW4011";

/// A call on something that is not a function.
pub const NOT_CALLABLE: &str = "VOW4012";

/// Type arguments applied to a type that takes none.
pub const NOT_GENERIC: &str = "VOW4013";

/// A `match` on a choice with an arm that matches everything.
///
/// Exhaustiveness is only worth something if adding a variant breaks the code
/// that has to care. A wildcard arm makes that stop happening quietly.
pub const CATCH_ALL_ON_CHOICE: &str = "VOW4014";

/// An assignment to something that is not handler state.
///
/// State is the only mutable thing in Vow. Everything else is a name for a
/// value, and a name whose value changes partway through a function is the
/// same problem that made shadowing an error.
pub const NOT_ASSIGNABLE: &str = "VOW4015";
