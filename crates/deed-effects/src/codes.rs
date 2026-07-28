//! Diagnostic codes produced by effect checking.
//!
//! The effect checker owns the `DEED5xxx` range. Codes are stable and never
//! reused.

/// A body performs an effect its signature does not declare.
pub const UNDECLARED_EFFECT: &str = "DEED5001";

/// A signature declares an effect its body never performs.
///
/// This is the rule that decides whether any of this is worth having. If
/// over-declaring were allowed, every signature would drift towards listing
/// everything, and a row nobody can trust is a row nobody reads.
pub const UNUSED_EFFECT: &str = "DEED5002";

/// A `uses` entry naming something that is not an effect.
///
/// An imported name lands here too. Which kind of thing it is comes off the
/// export rather than off a declaration in this file, but it comes off
/// something either way, so the module boundary does not soften the answer.
pub const NOT_AN_EFFECT: &str = "DEED5003";

/// A row the compiler cannot check.
///
/// Three shapes reach it, and each says something different: `sys.*` grants
/// everything a capability carries, a bare capability in a `uses` clause names
/// a value where an effect belongs, and a call into a function whose own row is
/// one of those inherits the hole. All three make the row vacuous, and the
/// compiler says so rather than reporting a clean check it did not perform.
///
/// A `uses` entry that is simply the wrong kind of name is not here. That is
/// DEED5003, whether the name is local or imported: the compiler knows what it
/// is, so there is nothing it failed to verify.
pub const UNVERIFIABLE_ROW: &str = "DEED5004";

/// A test performs an effect with no handler in scope.
pub const UNHANDLED_EFFECT: &str = "DEED5005";

/// An effect that arrived through an imported call and cannot be named here.
///
/// A function cannot promise something it has no word for. Calling something
/// that logs means importing `Log`, which follows from the row being the review
/// surface: a row that could not name what it grants would not be one.
pub const EFFECT_NOT_IMPORTED: &str = "DEED5006";

/// A function value that performs more than the type it crosses into allows.
///
/// `Fn(Int) -> Int` promises no effects and
/// `Fn(Int) uses Log.note -> Int` promises no more than that. Leaving a row
/// off cannot mean "any row": a value that carried an unstated effect through
/// a signature would undo the point of having rows at all.
pub const IMPURE_FUNCTION_VALUE: &str = "DEED5007";

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
/// The same shape as DEED4023, where a type parameter has to appear in a
/// parameter's type so that every call knows what it is.
pub const MISPLACED_ROW_VARIABLE: &str = "DEED5008";

/// A contract performs an effect the signature does not mention.
///
/// A contract does not contribute to a row, which is why the row may be
/// narrower than the clauses: `examples/transfer.deed` reads `Ledger.total()`
/// in an `ensures` clause and never declares it. What the row cannot be is
/// silent about the effect altogether. Installing a handler is the caller's
/// job, and the signature is the only place a caller can find out that one is
/// needed, so a clause performing an effect nothing above the brace names is a
/// call that type checks and then cannot run.
///
/// The granularity is the effect rather than the operation, because that is
/// what a handler is installed for, and because asking for the operation would
/// mean asking for a row entry that DEED5002 then rejects as unused.
pub const CONTRACT_EFFECT_NOT_DECLARED: &str = "DEED5009";
