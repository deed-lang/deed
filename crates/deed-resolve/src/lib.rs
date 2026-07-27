//! Binds every name in a Deed module to a declaration.
//!
//! ```
//! use deed_diagnostics::SourceMap;
//! use deed_lexer::tokenize;
//! use deed_parser::parse;
//! use deed_resolve::{Universe, resolve};
//!
//! let source = "module demo\n\nfn twice(n: Int) -> Int { n + n }\n";
//! let mut sources = SourceMap::new();
//! let file = sources.add("demo.deed", source);
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
pub use exports::{Export, ExportKind, Exports, RowEntry, RowLowering, Universe};
pub use resolver::{IO_OPERATIONS, PRELUDE, PRELUDE_EFFECTS, PRELUDE_MODULE, Resolved, resolve};
