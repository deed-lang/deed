//! Runs Deed programs.
//!
//! `test` blocks, generated property tests, and `main`. A `main` receives a
//! `System`, which is the only place authority enters a program, and
//! [`sandbox`] is what makes a `Dir` mean something.
//!
//! ```
//! use deed_diagnostics::SourceMap;
//! use deed_interp::{DeclaredRows, Guards, OperatorCalls, Program, run_tests};
//! use deed_lexer::tokenize;
//! use deed_parser::parse;
//! use deed_resolve::{Universe, resolve};
//!
//! let source = "\
//! module demo
//!
//! fn double(n: Int) -> Int { n + n }
//!
//! test \"doubling\" {
//!     assert double(21) == 42
//! }
//! ";
//!
//! let mut sources = SourceMap::new();
//! let file = sources.add("demo.deed", source);
//! let lexed = tokenize(file, sources.file(file).text());
//! let parsed = parse(file, &lexed.tokens);
//! let resolved = resolve(file, &parsed.module, &Universe::new());
//!
//! // Every module that should be able to see the others goes in the program.
//! // A call that leaves one of them runs with that module's own names in
//! // scope, which is why the interpreter needs all of them at once.
//! //
//! // `Guards` is what the type checker could not settle. Nothing here is
//! // refined, so there is nothing to check at runtime. `DeclaredRows` is what
//! // each function promised, so that the run can hold it to that. Nothing
//! // here performs anything, so there is nothing to hold it to either.
//! // `OperatorCalls` is what each bound operator means, and nothing here
//! // binds one.
//! let mut program = Program::new();
//! program.add(
//!     file,
//!     &parsed.module,
//!     &resolved.resolutions,
//!     Guards::new(),
//!     DeclaredRows::new(),
//!     OperatorCalls::new(),
//! );
//!
//! let outcomes = run_tests(&program, file);
//! assert_eq!(outcomes.len(), 1);
//! assert!(outcomes[0].passed());
//! ```

pub mod codes;
pub mod interp;
pub mod property;
pub mod value;

/// Where a `Dir` capability's rules live now.
///
/// Re-exported rather than moved out of view, because the interpreter is
/// not the only host a compiled program can have and the rules have to be
/// the same for all of them. See `deed-rt`.
pub use deed_rt::sandbox;

pub use interp::{
    DeclaredRows, FunctionProfile, Guard, Guards, OperatorCalls, Program, RowItem, Run,
    RuntimeProfile, TestOutcome, run_main, run_main_profiled, run_main_profiled_reaching,
    run_main_reaching, run_tests,
};
pub use property::{
    GeneratedInputs, PropertyConfig, PropertyOutcome, generate_inputs, is_testable, run_properties,
    shrink_inputs,
};
pub use sandbox::Refused;
pub use value::{Capability, Fields, Value, VariantValue};
