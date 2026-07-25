//! Turns Vow source text into tokens.
//!
//! ```
//! use vow_diagnostics::SourceMap;
//! use vow_lexer::{tokenize, TokenKind};
//!
//! let mut sources = SourceMap::new();
//! let file = sources.add("greeting.vow", "let name = \"world\"");
//! let lexed = tokenize(file, sources.file(file).text());
//!
//! assert!(!lexed.has_errors());
//! assert!(matches!(lexed.tokens[3].kind, TokenKind::Str(_)));
//! ```

pub mod codes;
pub mod lexer;
pub mod token;

pub use lexer::{Lexed, Trivia, TriviaKind, tokenize};
pub use token::{Keyword, Token, TokenKind};
