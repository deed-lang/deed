//! Builds a Vow syntax tree from tokens.
//!
//! ```
//! use vow_diagnostics::SourceMap;
//! use vow_lexer::tokenize;
//! use vow_parser::parse;
//!
//! let source = "module demo\n\nfn twice(n: Int) -> Int { n + n }\n";
//! let mut sources = SourceMap::new();
//! let file = sources.add("demo.vow", source);
//!
//! let lexed = tokenize(file, sources.file(file).text());
//! let parsed = parse(file, &lexed.tokens);
//!
//! assert!(!parsed.has_errors());
//! assert_eq!(parsed.module.items.len(), 1);
//! ```

pub mod codes;
pub mod parser;

pub use parser::{Parsed, parse};
