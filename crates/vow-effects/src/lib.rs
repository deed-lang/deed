//! Checks what a Vow function is allowed to touch.
//!
//! ```
//! use std::collections::HashMap;
//!
//! use vow_diagnostics::SourceMap;
//! use vow_effects::analyse;
//! use vow_lexer::tokenize;
//! use vow_parser::parse;
//! use vow_resolve::{Universe, resolve};
//!
//! let source = "\
//! module demo
//!
//! effect Clock {
//!     fn now() -> Int
//! }
//!
//! fn stamp() -> Int
//!   uses Clock.now,
//! { Clock.now() }
//!
//! fn pure_double(n: Int) -> Int { n + n }
//! ";
//!
//! let mut sources = SourceMap::new();
//! let file = sources.add("demo.vow", source);
//! let lexed = tokenize(file, sources.file(file).text());
//! let parsed = parse(file, &lexed.tokens);
//! let resolved = resolve(file, &parsed.module, &Universe::new());
//!
//! // Nothing here hands a function around, so no value owes a row. The type
//! // checker is what works out which ones do.
//! let analysis = analyse(file, &parsed.module, &resolved.resolutions, &HashMap::new());
//!
//! assert!(!analysis.has_errors());
//! ```

pub mod check;
pub mod codes;
pub mod cycles;
pub mod row;

pub use check::{Analysis, Effects, analyse};
pub use row::{EffectItem, Row};
