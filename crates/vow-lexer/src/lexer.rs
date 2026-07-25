//! The lexer.
//!
//! Two properties matter more than anything else here.
//!
//! **It does not stop at the first problem.** One stray character should not
//! hide the twelve real problems after it. Every hidden problem is another
//! round trip, and round trips are the multiplier in the cost model in
//! `design/00-motivation.md`.
//!
//! **Every problem is a [`Diagnostic`], never a string.** The lexer is the
//! first thing that can produce output, so it is where P7 either gets
//! established or quietly gets skipped.

use vow_diagnostics::{Applicability, Diagnostic, FileId, Span};

use crate::codes;
use crate::token::{Keyword, Token, TokenKind};

/// The result of lexing one file.
///
/// Tokens and diagnostics are returned together rather than as a `Result`,
/// because a file with errors still produces a usable token stream and the
/// parser should get the chance to say something about it.
pub struct Lexed {
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Lexed {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Turns source text into tokens. Always succeeds, possibly with diagnostics.
pub fn tokenize(file: FileId, src: &str) -> Lexed {
    Lexer {
        src,
        file,
        // A byte order mark is not part of the program. Plenty of editors write
        // one and none of them mention it, so rejecting it would be a first
        // impression of "unexpected character" on a file the user did not type
        // a single wrong thing into.
        pos: if src.starts_with('\u{FEFF}') {
            '\u{FEFF}'.len_utf8()
        } else {
            0
        },
        tokens: Vec::new(),
        diagnostics: Vec::new(),
    }
    .run()
}

struct Lexer<'a> {
    src: &'a str,
    file: FileId,
    pos: usize,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Lexer<'a> {
    fn run(mut self) -> Lexed {
        loop {
            self.skip_trivia();
            let start = self.pos;
            let Some(c) = self.bump() else { break };

            let kind = match c {
                '(' => Some(TokenKind::LParen),
                ')' => Some(TokenKind::RParen),
                '{' => Some(TokenKind::LBrace),
                '}' => Some(TokenKind::RBrace),
                '[' => Some(TokenKind::LBracket),
                ']' => Some(TokenKind::RBracket),
                ',' => Some(TokenKind::Comma),
                '.' => Some(TokenKind::Dot),
                ':' => Some(TokenKind::Colon),
                ';' => Some(TokenKind::Semi),
                '?' => Some(TokenKind::Question),
                '+' => Some(TokenKind::Plus),
                '*' => Some(TokenKind::Star),
                '%' => Some(TokenKind::Percent),
                '/' => Some(TokenKind::Slash),

                '-' if self.eat('>') => Some(TokenKind::Arrow),
                '-' => Some(TokenKind::Minus),

                '=' if self.eat('>') => Some(TokenKind::FatArrow),
                '=' if self.eat('=') => Some(TokenKind::EqEq),
                '=' => Some(TokenKind::Eq),

                '!' if self.eat('=') => Some(TokenKind::BangEq),
                '!' => Some(TokenKind::Bang),

                '<' if self.eat('=') => Some(TokenKind::Le),
                '<' => Some(TokenKind::Lt),

                '>' if self.eat('=') => Some(TokenKind::Ge),
                '>' => Some(TokenKind::Gt),

                '|' if self.eat('|') => Some(TokenKind::PipePipe),
                '|' => Some(TokenKind::Pipe),

                '&' if self.eat('&') => Some(TokenKind::AmpAmp),
                '&' => {
                    let span = Span::new(start as u32, self.pos as u32);
                    self.emit(
                        Diagnostic::error(
                            codes::UNKNOWN_CHARACTER,
                            self.file,
                            span,
                            "`&` is not an operator in Vow",
                        )
                        .with_primary_label("expected `&&`")
                        .with_note("Vow has no bitwise operators, and `&&` is logical and")
                        .with_fix(
                            "use `&&`",
                            span,
                            "&&",
                            Applicability::MaybeIncorrect,
                        ),
                    );
                    Some(TokenKind::Error)
                }

                '"' => Some(self.string(start)),

                c if c.is_ascii_digit() => Some(self.number(start)),
                c if is_ident_start(c) => Some(self.ident(start)),

                other => {
                    self.unknown_character(start, other);
                    Some(TokenKind::Error)
                }
            };

            if let Some(kind) = kind {
                let span = Span::new(start as u32, self.pos as u32);
                self.tokens.push(Token::new(kind, span));
            }
        }

        let eof = Span::at(self.pos as u32);
        self.tokens.push(Token::new(TokenKind::Eof, eof));

        Lexed {
            tokens: self.tokens,
            diagnostics: self.diagnostics,
        }
    }

    // -- character helpers -------------------------------------------------

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek_second(&self) -> Option<char> {
        self.src[self.pos..].chars().nth(1)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.pos += expected.len_utf8();
            true
        } else {
            false
        }
    }

    fn emit(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    // -- trivia ------------------------------------------------------------

    fn skip_trivia(&mut self) {
        loop {
            match (self.peek(), self.peek_second()) {
                (Some(c), _) if c.is_whitespace() => {
                    self.bump();
                }
                (Some('/'), Some('/')) => {
                    while let Some(c) = self.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.bump();
                    }
                }
                (Some('/'), Some('*')) => self.block_comment(),
                _ => return,
            }
        }
    }

    /// Block comments nest, so a commented out region containing a comment
    /// still ends where you expect it to.
    fn block_comment(&mut self) {
        let start = self.pos;
        self.bump();
        self.bump();
        let mut depth = 1usize;

        while depth > 0 {
            match (self.peek(), self.peek_second()) {
                (Some('/'), Some('*')) => {
                    self.bump();
                    self.bump();
                    depth += 1;
                }
                (Some('*'), Some('/')) => {
                    self.bump();
                    self.bump();
                    depth -= 1;
                }
                (Some(_), _) => {
                    self.bump();
                }
                (None, _) => {
                    let open = Span::new(start as u32, (start + 2) as u32);
                    let plural = if depth == 1 { "" } else { "s" };
                    self.emit(
                        Diagnostic::error(
                            codes::UNTERMINATED_BLOCK_COMMENT,
                            self.file,
                            open,
                            "unterminated block comment",
                        )
                        .with_primary_label("this comment is never closed")
                        .with_note(format!(
                            "block comments nest, so {depth} `*/`{plural} are still needed"
                        ))
                        .with_fix(
                            "close the comment",
                            Span::at(self.pos as u32),
                            "*/".repeat(depth),
                            Applicability::MachineApplicable,
                        ),
                    );
                    return;
                }
            }
        }
    }

    // -- identifiers -------------------------------------------------------

    fn ident(&mut self, start: usize) -> TokenKind {
        while let Some(c) = self.peek() {
            if is_ident_continue(c) {
                self.bump();
            } else {
                break;
            }
        }

        let text = &self.src[start..self.pos];
        if text == "_" {
            return TokenKind::Underscore;
        }
        match Keyword::from_ident(text) {
            Some(kw) => TokenKind::Keyword(kw),
            None => TokenKind::Ident(text.to_string()),
        }
    }

    // -- numbers -----------------------------------------------------------

    /// Integers only. There are no float literals, which means `40.try` is
    /// unambiguously `40`, `.`, `try`, and the lexer never has to guess.
    fn number(&mut self, start: usize) -> TokenKind {
        let (radix, digits_start) = match (self.src.as_bytes()[start], self.peek()) {
            (b'0', Some('x' | 'X')) => {
                self.bump();
                (16u32, self.pos)
            }
            (b'0', Some('b' | 'B')) => {
                self.bump();
                (2u32, self.pos)
            }
            (b'0', Some('o' | 'O')) => {
                self.bump();
                (8u32, self.pos)
            }
            _ => (10u32, start),
        };

        // Consume greedily, including characters that are not valid digits, so
        // that `0xZZ` and `123abc` are reported as one bad literal rather than
        // dribbling out as several unrelated tokens.
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' {
                self.bump();
            } else {
                break;
            }
        }

        let raw = &self.src[digits_start..self.pos];
        let digits: String = raw.chars().filter(|&c| c != '_').collect();
        let literal_span = Span::new(start as u32, self.pos as u32);

        if digits.is_empty() {
            let prefix = &self.src[start..digits_start];
            self.emit(
                Diagnostic::error(
                    codes::MALFORMED_NUMBER,
                    self.file,
                    literal_span,
                    format!("numeric literal has no digits after `{prefix}`"),
                )
                .with_primary_label("expected at least one digit"),
            );
            return TokenKind::Error;
        }

        if let Some(bad_offset) = self.first_invalid_digit(digits_start, radix) {
            let bad_char = self.src[bad_offset..].chars().next().unwrap_or('?');
            let span = Span::new(bad_offset as u32, (bad_offset + bad_char.len_utf8()) as u32);
            self.emit(
                Diagnostic::error(
                    codes::MALFORMED_NUMBER,
                    self.file,
                    span,
                    format!("`{bad_char}` is not a valid digit in base {radix}"),
                )
                .with_primary_label("invalid digit")
                .with_secondary(literal_span, "in this literal")
                .with_note(match radix {
                    2 => "binary literals accept `0` and `1`".to_string(),
                    8 => "octal literals accept `0` through `7`".to_string(),
                    16 => "hexadecimal literals accept `0` through `9` and `a` through `f`"
                        .to_string(),
                    _ => "Vow has no literal suffixes, so `100u8` should be written `100`"
                        .to_string(),
                }),
            );
            return TokenKind::Error;
        }

        match i64::from_str_radix(&digits, radix) {
            Ok(value) => TokenKind::Int(value),
            Err(_) => {
                self.emit(
                    Diagnostic::error(
                        codes::INTEGER_OUT_OF_RANGE,
                        self.file,
                        literal_span,
                        "integer literal does not fit in `Int`",
                    )
                    .with_primary_label("too large")
                    .with_note(format!("`Int` holds values up to {}", i64::MAX)),
                );
                TokenKind::Error
            }
        }
    }

    /// Byte offset of the first character in the digit run that is not a valid
    /// digit for `radix`.
    fn first_invalid_digit(&self, digits_start: usize, radix: u32) -> Option<usize> {
        self.src[digits_start..self.pos]
            .char_indices()
            .find(|&(_, c)| c != '_' && !c.is_digit(radix))
            .map(|(offset, _)| digits_start + offset)
    }

    // -- strings -----------------------------------------------------------

    fn string(&mut self, start: usize) -> TokenKind {
        let mut value = String::new();

        loop {
            let Some(c) = self.peek() else {
                self.unterminated_string(start, "end of file");
                return TokenKind::Error;
            };
            if c == '\n' {
                // The newline is left for the trivia skipper, so the next line
                // still lexes normally instead of being swallowed.
                self.unterminated_string(start, "end of line");
                return TokenKind::Error;
            }

            self.bump();
            match c {
                '"' => return TokenKind::Str(value),
                '\\' => {
                    if let Some(decoded) = self.escape() {
                        value.push(decoded);
                    }
                }
                other => value.push(other),
            }
        }
    }

    fn unterminated_string(&mut self, start: usize, reached: &str) {
        let span = Span::new(start as u32, self.pos as u32);
        self.emit(
            Diagnostic::error(
                codes::UNTERMINATED_STRING,
                self.file,
                span,
                format!("string literal reaches {reached} before its closing quote"),
            )
            .with_primary_label("this string is never closed")
            .with_note("string literals cannot span multiple lines")
            .with_fix(
                "add a closing quote",
                Span::at(self.pos as u32),
                "\"",
                Applicability::MachineApplicable,
            ),
        );
    }

    /// Decodes one escape sequence. The backslash has already been consumed.
    ///
    /// Returns `None` only when the sequence was malformed and a diagnostic was
    /// emitted, in which case nothing is appended to the literal.
    fn escape(&mut self) -> Option<char> {
        let backslash = self.pos - 1;
        let c = self.peek()?;

        // Do not consume a newline or quote here. Letting them fall through
        // means the caller reports "unterminated" rather than "bad escape",
        // which is nearly always the real problem.
        if c == '\n' {
            return None;
        }
        self.bump();

        match c {
            'n' => Some('\n'),
            't' => Some('\t'),
            'r' => Some('\r'),
            '0' => Some('\0'),
            '\\' => Some('\\'),
            '"' => Some('"'),
            'u' => self.unicode_escape(backslash),
            other => {
                let span = Span::new(backslash as u32, self.pos as u32);
                self.emit(
                    Diagnostic::error(
                        codes::UNKNOWN_ESCAPE,
                        self.file,
                        span,
                        format!("unknown escape sequence `\\{other}`"),
                    )
                    .with_primary_label("not a recognised escape")
                    .with_note(
                        "Vow defines `\\n`, `\\t`, `\\r`, `\\0`, `\\\\`, `\\\"` and `\\u{...}`",
                    )
                    .with_fix(
                        format!("write a literal backslash as `\\\\{other}`"),
                        span,
                        format!("\\\\{other}"),
                        Applicability::MaybeIncorrect,
                    ),
                );
                None
            }
        }
    }

    fn unicode_escape(&mut self, backslash: usize) -> Option<char> {
        if !self.eat('{') {
            let span = Span::new(backslash as u32, self.pos as u32);
            self.emit(
                Diagnostic::error(
                    codes::UNKNOWN_ESCAPE,
                    self.file,
                    span,
                    "expected `{` after `\\u`",
                )
                .with_primary_label("incomplete unicode escape")
                .with_note("unicode escapes are written `\\u{1F600}`"),
            );
            return None;
        }

        let digits_start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_hexdigit() {
                self.bump();
            } else {
                break;
            }
        }
        let digits = &self.src[digits_start..self.pos];

        if !self.eat('}') {
            let span = Span::new(backslash as u32, self.pos as u32);
            self.emit(
                Diagnostic::error(
                    codes::UNKNOWN_ESCAPE,
                    self.file,
                    span,
                    "unicode escape is missing its closing `}`",
                )
                .with_primary_label("expected `}` here")
                .with_fix(
                    "close the escape",
                    Span::at(self.pos as u32),
                    "}",
                    Applicability::MachineApplicable,
                ),
            );
            return None;
        }

        let span = Span::new(backslash as u32, self.pos as u32);
        let value = u32::from_str_radix(digits, 16).ok();
        match value.and_then(char::from_u32) {
            Some(c) => Some(c),
            None => {
                self.emit(
                    Diagnostic::error(
                        codes::UNKNOWN_ESCAPE,
                        self.file,
                        span,
                        format!("`{digits}` is not a unicode scalar value"),
                    )
                    .with_primary_label("invalid unicode escape")
                    .with_note(
                        "valid values are 0 to 10FFFF, excluding the surrogate range D800 to DFFF",
                    ),
                );
                None
            }
        }
    }

    // -- unknown characters ------------------------------------------------

    /// Characters that cannot start a token.
    ///
    /// The suggestions here look like a small thing and are not. A curly quote
    /// pasted in from a document is a real and frequent failure, and a fix a
    /// tool can apply without asking turns it into a non-event.
    fn unknown_character(&mut self, start: usize, c: char) {
        let span = Span::new(start as u32, self.pos as u32);
        let replacement = match c {
            '\u{201C}' | '\u{201D}' => Some("\""),
            '\u{2018}' | '\u{2019}' => Some("'"),
            '\u{00A0}' | '\u{2007}' | '\u{202F}' => Some(" "),
            '\u{FF1B}' => Some(";"),
            '\u{FF0C}' => Some(","),
            '\u{2013}' | '\u{2014}' => Some("-"),
            _ => None,
        };

        let mut diagnostic = Diagnostic::error(
            codes::UNKNOWN_CHARACTER,
            self.file,
            span,
            format!("unexpected character `{}`", c.escape_debug()),
        )
        .with_primary_label("not valid at the start of a token");

        if let Some(replacement) = replacement {
            let shown = if replacement == " " {
                "a plain space".to_string()
            } else {
                format!("`{replacement}`")
            };
            diagnostic = diagnostic
                .with_note("this looks like a character pasted in from formatted text")
                .with_fix(
                    format!("replace it with {shown}"),
                    span,
                    replacement,
                    Applicability::MachineApplicable,
                );
        }

        self.emit(diagnostic);
    }
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}
