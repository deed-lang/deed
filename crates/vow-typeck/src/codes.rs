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

/// `?` applied to something that is not a `Result`.
pub const NOT_A_RESULT: &str = "VOW4016";

/// `?` inside a function that does not return a `Result`.
pub const TRY_NEEDS_RESULT_RETURN: &str = "VOW4017";

/// A pattern that cannot match the thing it is applied to.
pub const PATTERN_MISMATCH: &str = "VOW4018";

/// A name used in expression position that names a type, not a value.
///
/// This matters more than it looks. Without it a type name in expression
/// position has no type, and a type-less expression is compatible with
/// everything, so `Io.write(Console, "hi")` would type check and a program
/// could conjure authority by naming it.
pub const NOT_A_VALUE: &str = "VOW4019";

/// `<`, `<=`, `>` or `>=` on a type that has no order.
///
/// Ordering used to be accepted for anything, as long as both sides agreed, so
/// comparing two records passed the type checker and failed at runtime with a
/// message about the interpreter being incomplete. It was not incomplete: there
/// is nothing to implement, because there is nothing the comparison could mean.
pub const NOT_ORDERED: &str = "VOW4020";

/// A handler operation that does not line up with the effect it implements.
///
/// A handler operation writes no types, because the effect already declared
/// them. That only works if the effect is actually consulted, and it was not:
/// every parameter in every handler body was the unknown type, which agrees
/// with everything, so a handler was the least checked code in the language
/// while being the piece that holds the state and talks to the outside world.
pub const OPERATION_MISMATCH: &str = "VOW4021";

/// A list operation applied to something that is not a list.
pub const NOT_A_LIST: &str = "VOW4022";
