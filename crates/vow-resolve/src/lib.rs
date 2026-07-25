//! Binds every name in a Vow module to a declaration.
//!
//! ```
//! use vow_diagnostics::SourceMap;
//! use vow_lexer::tokenize;
//! use vow_parser::parse;
//! use vow_resolve::{Universe, resolve};
//!
//! let source = "module demo\n\nfn twice(n: Int) -> Int { n + n }\n";
//! let mut sources = SourceMap::new();
//! let file = sources.add("demo.vow", source);
//!
//! let lexed = tokenize(file, sources.file(file).text());
//! let parsed = parse(file, &lexed.tokens);
//! let resolved = resolve(file, &parsed.module, &Universe::new());
//!
//! assert!(!resolved.has_errors());
//! ```

pub mod codes;
pub mod defs;
pub mod exports;
pub mod resolver;

pub use defs::{DefData, DefId, DefKind, Dot, Resolutions};
pub use exports::{Export, ExportKind, Exports, Universe};
pub use resolver::{PRELUDE, Resolved, resolve};
