//! Diagnostic codes produced by type checking.
//!
//! The type checker owns the `DEED4xxx` range. Codes are stable and never reused.

/// A value of one type used where another was required.
pub const TYPE_MISMATCH: &str = "DEED4001";

/// A record or variant literal that leaves fields out.
pub const MISSING_FIELDS: &str = "DEED4002";

/// A record or variant literal that names a field the type does not have.
pub const UNKNOWN_FIELD: &str = "DEED4003";

/// Field access on a type with no such field.
pub const NO_SUCH_FIELD: &str = "DEED4004";

/// A literal whose path is not a record or a variant.
pub const NOT_A_CONSTRUCTOR: &str = "DEED4005";

/// A `match` that does not cover every variant.
pub const NON_EXHAUSTIVE_MATCH: &str = "DEED4006";

/// A call with the wrong number of arguments.
pub const WRONG_ARITY: &str = "DEED4007";

/// A refinement that could not be discharged.
///
/// Not a rejection of the program so much as an admission of what the compiler
/// can currently prove. See the tier table in `design/02-syntax.md`.
pub const UNPROVEN_REFINEMENT: &str = "DEED4008";

/// A refinement the compiler proved false.
pub const VIOLATED_REFINEMENT: &str = "DEED4009";

/// A type alias that expands to itself.
pub const TYPE_ALIAS_CYCLE: &str = "DEED4010";

/// A name used in type position that does not name a type.
pub const NOT_A_TYPE: &str = "DEED4011";

/// A call on something that is not a function.
pub const NOT_CALLABLE: &str = "DEED4012";

/// Type arguments applied to a type that takes none.
pub const NOT_GENERIC: &str = "DEED4013";

/// A `match` on a choice with an arm that matches everything.
///
/// Exhaustiveness is only worth something if adding a variant breaks the code
/// that has to care. A wildcard arm makes that stop happening quietly.
pub const CATCH_ALL_ON_CHOICE: &str = "DEED4014";

/// An assignment to something that is not handler state.
///
/// State is the only mutable thing in Deed. Everything else is a name for a
/// value, and a name whose value changes partway through a function is the
/// same problem that made shadowing an error.
pub const NOT_ASSIGNABLE: &str = "DEED4015";

/// `?` applied to something that is not a `Result`.
pub const NOT_A_RESULT: &str = "DEED4016";

/// `?` inside a function that does not return a `Result`.
pub const TRY_NEEDS_RESULT_RETURN: &str = "DEED4017";

/// A pattern that cannot match the thing it is applied to.
pub const PATTERN_MISMATCH: &str = "DEED4018";

/// A name used in expression position that names a type, not a value.
///
/// This matters more than it looks. Without it a type name in expression
/// position has no type, and a type-less expression is compatible with
/// everything, so `Io.write(Console, "hi")` would type check and a program
/// could conjure authority by naming it.
pub const NOT_A_VALUE: &str = "DEED4019";

/// `<`, `<=`, `>` or `>=` on a type that has no order.
///
/// Ordering used to be accepted for anything, as long as both sides agreed, so
/// comparing two records passed the type checker and failed at runtime with a
/// message about the interpreter being incomplete. It was not incomplete: there
/// is nothing to implement, because there is nothing the comparison could mean.
pub const NOT_ORDERED: &str = "DEED4020";

/// A handler operation that does not line up with the effect it implements.
///
/// A handler operation writes no types, because the effect already declared
/// them. That only works if the effect is actually consulted, and it was not:
/// every parameter in every handler body was the unknown type, which agrees
/// with everything, so a handler was the least checked code in the language
/// while being the piece that holds the state and talks to the outside world.
pub const OPERATION_MISMATCH: &str = "DEED4021";

/// A list operation applied to something that is not a list.
pub const NOT_A_LIST: &str = "DEED4022";

/// A type parameter that appears in no parameter's type.
///
/// Every call has to be able to work out what a type parameter is from the
/// arguments alone, and the only place it can look is the parameter types.
/// The alternative is writing type arguments at the call site, which needs
/// `first<String>([])` to parse, which is the `f<a>(b)` versus `f < a > (b)`
/// ambiguity, and P2 has a budget for exactly this kind of thing.
///
/// It is also the same claim the rest of the language makes: a signature is
/// complete, and one with a hole a caller has to fill from somewhere else is
/// not.
///
/// Two things in the language break it and are allowed to, because they are
/// built in rather than declared. `ok(x)` is a `Result<T, unknown>` and
/// `err(e)` is a `Result<unknown, E>`, so each mentions a parameter no
/// argument decides and answers it with the unknown type, which absorbs. That
/// is the shortcut holding `Result` in the language: a declared `choice` can
/// already be built and matched with named fields, and what nobody can write
/// is the constructor. See the open question in `design/02-syntax.md`, where
/// letting the rule take that answer generally is one of the three ways out.
pub const UNDETERMINED_TYPE_PARAM: &str = "DEED4023";

/// A generic function used as a value rather than called.
///
/// One expression has one type here, and a generic function named rather than
/// called has as many as there are ways to call it. What would make this work
/// is a polymorphic value, which is a much larger thing than substituting into
/// a signature at a call site.
pub const GENERIC_AS_VALUE: &str = "DEED4024";

/// A call the checker can see does not satisfy what the callee requires.
///
/// A precondition is the caller's job, so it is reported where the call was
/// written rather than inside the function that stated it. Only when the facts
/// at the call site settle the clause the wrong way; not knowing is the
/// ordinary case and leaves the runtime check to answer.
pub const BROKEN_PRECONDITION: &str = "DEED4025";

/// A statement whose value nobody reads.
///
/// A block's value is its tail, so any other expression in it is there for
/// what it does. One that produces a value has nowhere to put it, and the two
/// ways to get here are a result that was meant to be looked at and a line
/// that was meant to continue the one above it. The second became possible to
/// tell apart when an expression started ending at the end of a line.
pub const DISCARDED_VALUE: &str = "DEED4026";

/// A `for ... while ...` with nothing to be about.
///
/// The condition is read before each turn and sees the accumulator, so a walk
/// without one gives it nothing that changes. Such a condition either stops
/// the walk before the first turn or never stops it, and neither is a thing
/// anybody meant to write.
pub const WHILE_WITHOUT_ACCUMULATOR: &str = "DEED4027";

/// Type parameters on an alias that also carries a predicate.
///
/// An alias with no predicate is a name for a type and expands to it, so
/// parameters on one are the substitution a `record` already does.
/// A refinement is not that: it is a nominal type whose predicate is checked,
/// and a predicate about a value whose type is not decided yet has nothing it
/// can say. `length(value) > 0` over an unknown `T` is not a question with an
/// answer, so the parameters are refused rather than accepted and ignored.
pub const REFINEMENT_TYPE_PARAM: &str = "DEED4028";

/// A handler that leaves out an operation the effect it implements declares.
///
/// `implements` is a claim, and the check for it only ran one way: an
/// operation the effect does not declare was DEED4021, and an operation the
/// effect declares and the handler leaves out was nothing at all until the
/// missing one was reached at run time.
///
/// Half a handler is not a smaller handler. A `with` block discharges the
/// effect rather than the operations named inside it, so installing one is a
/// claim that every call underneath has somewhere to go, and a row saying
/// `uses Counter.total` is a caller taking that claim at its word.
pub const HANDLER_MISSING_OPERATION: &str = "DEED4029";

/// A closure that names the handler state around it.
///
/// A closure captures the frame by value, so everything else it sees is a
/// number it was handed. Handler state would be the one thing it saw through a
/// reference, and the reference is to something whose lifetime is a `with`
/// block while the closure's is not. The interpreter used to read it out of
/// whichever handler happened to be innermost when the call landed, so a
/// closure written in one handler and called under another quietly answered
/// out of the other one's table when the two shared a state name.
///
/// Capturing the handler instead was the other way out, and it is refused for
/// a reason that outlives the interpreter: the closure's type would not say it.
/// `Fn() -> Int` claims to take nothing and perform nothing, and a value that
/// is also a live window onto a particular handler's state carries an input
/// and a lifetime through a signature that mentions neither. A signature is
/// complete here, which is the same rule `DEED4023` and the row before the
/// arrow are both instances of.
///
/// What to write instead is the snapshot the rest of the language already
/// takes: read the state into a local and let the closure capture that. The
/// value is then a number, which is what a `Fn() -> Int` says it is.
pub const CLOSURE_OVER_STATE: &str = "DEED4030";

/// `operator + = added` where `added` is not something an operator can mean.
///
/// A binding says an operator, written between two values of one type, means a
/// function. The shape that makes that true is narrow and every part of it is
/// load-bearing:
///
/// - Two parameters and a result, all of one type, so `a + b + c` is written
///   the way it reads and the operator hands back what it was given.
/// - That type declared in this module, so the meaning of `+` on a type is
///   decided in one file. The same reasoning keeps module resolution free of a
///   search path.
/// - Nothing generic, because the operand types are what choose the function
///   and a type parameter is not a type yet. That is the trait question, and
///   it is not this one.
/// - An empty row. An operator is reachable from a contract clause, and a
///   clause that performs something is a clause that cannot be read as a
///   question about values.
///
/// One code and several sentences, because they are one mistake: the function
/// named is not the shape an operator is. See
/// `design/decisions/2026-08-03-operators-bound-to-functions.md`.
pub const OPERATOR_SHAPE: &str = "DEED4031";
