//! Runs Vow programs.
//!
//! Only far enough to execute `test` blocks. There is no `main`, because a
//! `main` needs IO, which needs capabilities that can actually do something.
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
pub mod value;

pub use interp::{TestOutcome, run_tests};
pub use property::{PropertyConfig, PropertyOutcome, is_testable, run_properties};
pub use value::{Fields, Value, VariantValue};
