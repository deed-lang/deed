//! Diagnostic codes produced at runtime.
//!
//! The interpreter owns the `DEED6xxx` range. Codes are stable and never reused.
//!
//! Runtime failures are diagnostics like everything else. A program that fails
//! while running is not a different kind of problem from one that fails while
//! being checked, and P7 does not stop applying because the compiler finished.
//!
//! Ten codes, forty-eight messages. A code is not a message and the ratchet in
//! `crates/deed-driver/tests/codes.rs` only asks for one test per code, so the
//! doc comments below say what each code covers rather than quoting one of the
//! sentences behind it. Every one of the forty-eight that anything can reach is
//! rendered by a test; `crates/deed-interp/tests/messages.rs` is where most of
//! them are read, says where the rest are, and argues about the two nothing
//! can reach.

/// An `assert` that was not true, or an `assert refuses` that nothing refused.
///
/// Two messages. The second is the half that makes the first worth having: a
/// statement that passed whatever happened would be the same mistake as a test
/// that only feeds a guard values it accepts.
pub const ASSERTION_FAILED: &str = "DEED6001";

/// A `where` clause that did not hold on entry. A bug in the caller.
pub const PRECONDITION_FAILED: &str = "DEED6002";

/// An `ensures` obligation that did not hold on exit. A bug in the function.
pub const POSTCONDITION_FAILED: &str = "DEED6003";

/// A value that did not satisfy the refinement it was passed into.
///
/// This is the `Guarded` tier actually guarding something.
pub const REFINEMENT_FAILED: &str = "DEED6004";

/// An effect that could not be performed by anything installed.
///
/// Three messages, and only the first is about the program: no handler for the
/// effect at all. The other two are about the compiler. A handler that leaves
/// an operation out is refused by `DEED4029`, and an `Io` operation handed the
/// wrong kind of capability is a type error, so a run that meets either has
/// been given a file the checker would not have accepted.
pub const NO_HANDLER: &str = "DEED6005";

/// Something the run could not do.
///
/// Thirty-six messages, which is three quarters of everything in this range,
/// and they are not one thing. Thirty-two arrive through one helper and say
/// only that the run met a shape `deed check` refuses first, so meeting one
/// means the file was not checked or the check has a hole; one is a call into
/// a module whose code was never handed over, which is a gap in what this
/// library was given rather than in the library; one is `sys.files` in a
/// program that was not given a directory, which is an ordinary runtime fact
/// about a program that is right; and two name a particular part of the
/// compiler, a match that ran out of arms and a handler with a state field
/// nobody gave a value to.
///
/// Four notes say which of those a reader has. There used to be one, claiming
/// all thirty-six were the interpreter's own unfinished work.
///
/// Two of them said the opposite, and were the only two that could: handler
/// state read from a closure that outlived the handler operation it was
/// written in, which `deed check` accepted. It does not any more. `DEED4030`
/// refuses the closure where it is written, so those two joined the thirty
/// and the fifth note went away with the helper behind it.
pub const NOT_RUNNABLE: &str = "DEED6006";

/// Arithmetic that has no answer, such as overflow or division by zero.
pub const ARITHMETIC: &str = "DEED6007";

/// A generated property test that could not find enough usable inputs.
///
/// A property that only ever tested a handful of inputs is worse than no
/// property, because it looks like one.
pub const NOT_ENOUGH_CASES: &str = "DEED6008";

/// A call that went deeper than the interpreter is willing to go.
///
/// `Diverge` in a row says a function may not return. It does not make one
/// return, so the runtime needs an answer for the case where it does not, and
/// the answer is a diagnostic rather than the process dying.
pub const TOO_DEEP: &str = "DEED6009";

/// An effect was performed inside a function that did not declare it.
///
/// Not a mistake in the program. The file was accepted, so this says the
/// effect checker let something through, and the run is the thing that noticed.
/// The rows are the argument this language is making, and until this existed
/// the only thing that ever read one was the pass that wrote it.
pub const ROW_NOT_KEPT: &str = "DEED6010";

/// A computation was abandoned by its handler.
///
/// A handler operation executed `abandon` instead of returning a value. The
/// computation that performed the effect does not receive a return value;
/// instead the stack unwinds, running every `finally` clause it passes
/// through, and then the run stops with this code.
///
/// This is not a contract failure and `assert refuses` cannot catch it.
pub const ABANDONED: &str = "DEED6011";
