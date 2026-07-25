//! Runs Vow programs.
//!
//! `test` blocks, generated property tests, and `main`. A `main` receives a
//! `System`, which is the only place authority enters a program, and
//! [`sandbox`] is what makes a `Dir` mean something.
//!
//! ```
//! use vow_diagnostics::SourceMap;
//! use vow_interp::run_tests;
//! use vow_lexer::tokenize;
//! use vow_parser::parse;
//! use vow_resolve::resolve;
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
//! let file = sources.add("demo.vow", source);
//! let lexed = tokenize(file, sources.file(file).text());
//! let parsed = parse(file, &lexed.tokens);
//! let resolved = resolve(file, &parsed.module);
//!
//! let outcomes = run_tests(file, &parsed.module, &resolved.resolutions);
//! assert_eq!(outcomes.len(), 1);
//! assert!(outcomes[0].passed());
//! ```

pub mod codes;
pub mod interp;
pub mod property;
pub mod sandbox;
pub mod value;

pub use interp::{Run, TestOutcome, run_main, run_tests};
pub use property::{PropertyConfig, PropertyOutcome, is_testable, run_properties};
pub use sandbox::Refused;
pub use value::{Capability, Fields, Value, VariantValue};
