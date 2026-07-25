//! Tokens.
//!
//! The keyword set is deliberately small. P2 in `design/01-principles.md` puts
//! a budget on the size of the specification, and every keyword spends some of
//! it, so anything that could be an ordinary function is not a keyword.
//!
//! `old` and `unchanged` are the exception. They read like calls but they
//! cannot be functions: `old(e)` evaluates `e` in the state on entry, and
//! `unchanged(E)` takes an effect rather than a value. Neither is expressible
//! as a normal call, so both are keywords.

use vow_diagnostics::Span;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TokenKind {
    Keyword(Keyword),
    /// An identifier. A bare `_` lexes as [`TokenKind::Underscore`] instead.
    Ident(String),
    /// An integer literal, already decoded. Radix and digit separators are a
    /// lexical concern and nothing downstream should have to care.
    Int(i64),
    /// A string literal with escapes already resolved.
    Str(String),

    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,

    Comma,
    Dot,
    Colon,
    Semi,
    Question,
    Underscore,
    Pipe,

    /// `->`
    Arrow,
    /// `=>`
    FatArrow,

    Eq,
    EqEq,
    BangEq,
    Bang,
    Lt,
    Le,
    Gt,
    Ge,

    Plus,
    Minus,
    Star,
    Slash,
    Percent,

    AmpAmp,
    PipePipe,

    /// Emitted where a token could not be formed. Always accompanied by a
    /// diagnostic. It exists so the parser sees a placeholder instead of a
    /// hole, which keeps recovery from cascading.
    Error,

    Eof,
}

impl TokenKind {
    /// A short name for use in diagnostics.
    pub fn describe(&self) -> String {
        match self {
            TokenKind::Keyword(kw) => format!("keyword `{}`", kw.as_str()),
            TokenKind::Ident(name) => format!("identifier `{name}`"),
            TokenKind::Int(_) => "integer literal".to_string(),
            TokenKind::Str(_) => "string literal".to_string(),
            TokenKind::Error => "invalid token".to_string(),
            TokenKind::Eof => "end of file".to_string(),
            other => format!("`{}`", other.symbol()),
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::LBrace => "{",
            TokenKind::RBrace => "}",
            TokenKind::LBracket => "[",
            TokenKind::RBracket => "]",
            TokenKind::Comma => ",",
            TokenKind::Dot => ".",
            TokenKind::Colon => ":",
            TokenKind::Semi => ";",
            TokenKind::Question => "?",
            TokenKind::Underscore => "_",
            TokenKind::Pipe => "|",
            TokenKind::Arrow => "->",
            TokenKind::FatArrow => "=>",
            TokenKind::Eq => "=",
            TokenKind::EqEq => "==",
            TokenKind::BangEq => "!=",
            TokenKind::Bang => "!",
            TokenKind::Lt => "<",
            TokenKind::Le => "<=",
            TokenKind::Gt => ">",
            TokenKind::Ge => ">=",
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            TokenKind::Percent => "%",
            TokenKind::AmpAmp => "&&",
            TokenKind::PipePipe => "||",
            _ => "",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Keyword {
    // Declarations
    Module,
    Use,
    Type,
    Record,
    Choice,
    Effect,
    Handler,
    Implements,
    State,
    Fn,

    // Contract block
    Where,
    Uses,
    Ensures,
    Old,
    Unchanged,

    // Expressions and statements
    Let,
    If,
    Else,
    Match,
    Return,
    True,
    False,

    // Tests
    Test,
    With,
    Assert,
}

impl Keyword {
    pub fn from_ident(text: &str) -> Option<Keyword> {
        let kw = match text {
            "module" => Keyword::Module,
            "use" => Keyword::Use,
            "type" => Keyword::Type,
            "record" => Keyword::Record,
            "choice" => Keyword::Choice,
            "effect" => Keyword::Effect,
            "handler" => Keyword::Handler,
            "implements" => Keyword::Implements,
            "state" => Keyword::State,
            "fn" => Keyword::Fn,
            "where" => Keyword::Where,
            "uses" => Keyword::Uses,
            "ensures" => Keyword::Ensures,
            "old" => Keyword::Old,
            "unchanged" => Keyword::Unchanged,
            "let" => Keyword::Let,
            "if" => Keyword::If,
            "else" => Keyword::Else,
            "match" => Keyword::Match,
            "return" => Keyword::Return,
            "true" => Keyword::True,
            "false" => Keyword::False,
            "test" => Keyword::Test,
            "with" => Keyword::With,
            "assert" => Keyword::Assert,
            _ => return None,
        };
        Some(kw)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Keyword::Module => "module",
            Keyword::Use => "use",
            Keyword::Type => "type",
            Keyword::Record => "record",
            Keyword::Choice => "choice",
            Keyword::Effect => "effect",
            Keyword::Handler => "handler",
            Keyword::Implements => "implements",
            Keyword::State => "state",
            Keyword::Fn => "fn",
            Keyword::Where => "where",
            Keyword::Uses => "uses",
            Keyword::Ensures => "ensures",
            Keyword::Old => "old",
            Keyword::Unchanged => "unchanged",
            Keyword::Let => "let",
            Keyword::If => "if",
            Keyword::Else => "else",
            Keyword::Match => "match",
            Keyword::Return => "return",
            Keyword::True => "true",
            Keyword::False => "false",
            Keyword::Test => "test",
            Keyword::With => "with",
            Keyword::Assert => "assert",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Keyword, TokenKind};

    #[test]
    fn keyword_round_trips() {
        for text in [
            "module",
            "use",
            "type",
            "record",
            "choice",
            "effect",
            "handler",
            "implements",
            "state",
            "fn",
            "where",
            "uses",
            "ensures",
            "old",
            "unchanged",
            "let",
            "if",
            "else",
            "match",
            "return",
            "true",
            "false",
            "test",
            "with",
            "assert",
        ] {
            let kw = Keyword::from_ident(text).expect("should be a keyword");
            assert_eq!(kw.as_str(), text);
        }
    }

    #[test]
    fn ordinary_words_are_not_keywords() {
        for text in [
            "transfer", "Ledger", "balance", "value", "result", "mut", "class",
        ] {
            assert!(
                Keyword::from_ident(text).is_none(),
                "{text} became a keyword"
            );
        }
    }

    #[test]
    fn descriptions_are_readable() {
        assert_eq!(TokenKind::Arrow.describe(), "`->`");
        assert_eq!(TokenKind::Ident("x".into()).describe(), "identifier `x`");
        assert_eq!(
            TokenKind::Keyword(Keyword::Ensures).describe(),
            "keyword `ensures`"
        );
    }
}
