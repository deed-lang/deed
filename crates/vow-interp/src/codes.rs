//! Diagnostic codes produced at runtime.
//!
//! The interpreter owns the `VOW6xxx` range. Codes are stable and never reused.
//!
//! Runtime failures are diagnostics like everything else. A program that fails
//! while running is not a different kind of problem from one that fails while
//! being checked, and P7 does not stop applying because the compiler finished.

/// An `assert` that was not true.
pub const ASSERTION_FAILED: &str = "VOW6001";

/// A `where` clause that did not hold on entry. A bug in the caller.
pub const PRECONDITION_FAILED: &str = "VOW6002";

/// An `ensures` obligation that did not hold on exit. A bug in the function.
pub const POSTCONDITION_FAILED: &str = "VOW6003";

/// A value that did not satisfy the refinement it was passed into.
///
/// This is the `Guarded` tier actually guarding something.
pub const REFINEMENT_FAILED: &str = "VOW6004";

/// An effect performed with no handler installed for it.
pub const NO_HANDLER: &str = "VOW6005";

/// Something the interpreter cannot run yet.
///
/// Also covers a call into a module whose code was never handed over, which is
/// not a gap in the interpreter but a gap in what it was given.
pub const NOT_RUNNABLE: &str = "VOW6006";

/// Arithmetic that has no answer, such as overflow or division by zero.
pub const ARITHMETIC: &str = "VOW6007";

/// A generated property test that could not find enough usable inputs.
///
/// A property that only ever tested a handful of inputs is worse than no
/// property, because it looks like one.
pub const NOT_ENOUGH_CASES: &str = "VOW6008";
