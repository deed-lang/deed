//! Diagnostic codes produced by the lexer.
//!
//! Codes are stable and never reused. If a check is removed, its code is
//! retired rather than recycled, because tooling and documentation elsewhere
//! may still refer to it.
//!
//! The lexer owns the `DEED1xxx` range.

/// A character that cannot begin any token.
pub const UNKNOWN_CHARACTER: &str = "DEED1001";

/// A string literal that reached a newline or end of file before its closing quote.
pub const UNTERMINATED_STRING: &str = "DEED1002";

/// A block comment that reached end of file before its closing delimiter.
pub const UNTERMINATED_BLOCK_COMMENT: &str = "DEED1003";

/// An escape sequence the language does not define.
pub const UNKNOWN_ESCAPE: &str = "DEED1004";

/// An integer literal that does not fit in `Int`.
pub const INTEGER_OUT_OF_RANGE: &str = "DEED1005";

/// A numeric literal with no digits, a digit invalid for its radix, or a suffix.
pub const MALFORMED_NUMBER: &str = "DEED1006";

/// A decimal point between two digits, which the language has no literal for.
pub const NO_FLOAT_LITERAL: &str = "DEED1007";
