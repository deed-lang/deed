//! Diagnostic codes produced by effect checking.
//!
//! The effect checker owns the `VOW5xxx` range. Codes are stable and never
//! reused.

/// A body performs an effect its signature does not declare.
pub const UNDECLARED_EFFECT: &str = "VOW5001";

/// A signature declares an effect its body never performs.
///
/// This is the rule that decides whether any of this is worth having. If
/// over-declaring were allowed, every signature would drift towards listing
/// everything, and a row nobody can trust is a row nobody reads.
pub const UNUSED_EFFECT: &str = "VOW5002";

/// A `uses` entry naming something that is not an effect.
pub const NOT_AN_EFFECT: &str = "VOW5003";

/// A row the compiler cannot check.
///
/// Either it names an effect from a module that has not been loaded, or it
/// grants everything a capability carries. Both make the row vacuous, and the
/// compiler says so rather than reporting a clean check it did not perform.
pub const UNVERIFIABLE_ROW: &str = "VOW5004";

/// A test performs an effect with no handler in scope.
pub const UNHANDLED_EFFECT: &str = "VOW5005";

/// An effect that arrived through an imported call and cannot be named here.
///
/// A function cannot promise something it has no word for. Calling something
/// that logs means importing `Log`, which follows from the row being the review
/// surface: a row that could not name what it grants would not be one.
pub const EFFECT_NOT_IMPORTED: &str = "VOW5006";

/// A function value that performs more than the type it crosses into allows.
///
/// `Fn(Int) -> Int` promises no effects and
/// `Fn(Int) uses Log.note -> Int` promises no more than that. Leaving a row
/// off cannot mean "any row": a value that carried an unstated effect through
/// a signature would undo the point of having rows at all.
pub const IMPURE_FUNCTION_VALUE: &str = "VOW5007";

/// A row variable written somewhere a call site could not fill it in.
///
/// A row variable stands for whatever a callback performs, and the only thing
/// that knows what that was is the call that supplied the callback. So it has
/// to be readable off an argument: the row of a parameter whose type is a
/// function type, and the declaration's own `uses` clause. Written anywhere
/// else, such as a return type, it leaves the call site holding a value whose
/// row names something no caller has a word for, and an effect nobody can name
/// is an effect nobody declares.
///
/// The same shape as VOW4023, where a type parameter has to appear in a
/// parameter's type so that every call knows what it is.
pub const MISPLACED_ROW_VARIABLE: &str = "VOW5008";
