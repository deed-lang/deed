//! Turns Deed source text into tokens.
//!
//! ```
//! use deed_diagnostics::SourceMap;
//! use deed_lexer::{tokenize, TokenKind};
//!
//! let mut sources = SourceMap::new();
//! let file = sources.add("greeting.deed", "let name = \"world\"");
//! let lexed = tokenize(file, sources.file(file).text());
//!
//! assert!(!lexed.has_errors());
//! assert!(matches!(lexed.tokens[3].kind, TokenKind::Str(_)));
//! ```

pub mod codes;
pub mod lexer;
pub mod token;

pub use lexer::{Lexed, Trivia, TriviaKind, integer_out_of_range, tokenize};
pub use token::{Keyword, Token, TokenKind};
