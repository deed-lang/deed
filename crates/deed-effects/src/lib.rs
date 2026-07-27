//! Checks what a Deed function is allowed to touch.
//!
//! ```
//! use std::collections::HashMap;
//!
//! use deed_diagnostics::SourceMap;
//! use deed_effects::analyse;
//! use deed_lexer::tokenize;
//! use deed_parser::parse;
//! use deed_resolve::{Universe, resolve};
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
//! let file = sources.add("demo.deed", source);
//! let lexed = tokenize(file, sources.file(file).text());
//! let parsed = parse(file, &lexed.tokens);
//! let resolved = resolve(file, &parsed.module, &Universe::new());
//!
//! // Nothing here hands a function around, so no value owes a row. The type
//! // checker is what works out which ones do.
//! let analysis = analyse(file, &parsed.module, &resolved.resolutions, &HashMap::new(), &HashMap::new());
//!
//! assert!(!analysis.has_errors());
//! ```

pub mod check;
pub mod codes;
pub mod cycles;
pub mod row;

pub use check::{Analysis, Effects, analyse};
pub use row::{EffectItem, Row};
