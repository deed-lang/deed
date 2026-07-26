//! Just enough JSON to talk to an editor.
//!
//! Written by hand for the same reason [`vow_diagnostics::render_json`] is:
//! this compiler has no dependencies while it is small enough not to need any,
//! and the language server protocol is a handful of object shapes rather than
//! an open-ended data format. If what arrives here grows past this, reach for
//! a real parser rather than making this cleverer.
//!
//! An object keeps its fields in a `Vec` rather than a map. Messages have a
//! few fields each, a linear scan beats hashing at that size, and the order
//! surviving a round trip makes the tests readable.

use std::fmt::Write as _;

#[derive(Clone, PartialEq, Debug)]
pub enum Json {
    Null,
    Bool(bool),
    /// JSON has one number type. The protocol only ever sends integers small
    /// enough to be exact here, and treating them as one type means nothing
    /// has to guess which kind arrived.
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn string(value: impl Into<String>) -> Self {
        Json::String(value.into())
    }

    pub fn number(value: i64) -> Self {
        Json::Number(value as f64)
    }

    pub fn object(fields: Vec<(&str, Json)>) -> Self {
        Json::Object(
            fields
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect(),
        )
    }

    /// The value of a field, or `None` for anything that is not an object with
    /// that field.
    pub fn get(&self, name: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    /// The value at a path of field names, for reaching into the nested
    /// objects the protocol is made of.
    pub fn at(&self, path: &[&str]) -> Option<&Json> {
        let mut current = self;
        for name in path {
            current = current.get(name)?;
        }
        Some(current)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Number(value) => Some(*value as i64),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    pub fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(true) => out.push_str("true"),
            Json::Bool(false) => out.push_str("false"),
            Json::Number(value) => {
                // Integers are written as integers. An editor that reads an id
                // back as `1.0` is within its rights to call it a different
                // request from `1`.
                if value.fract() == 0.0 && value.is_finite() {
                    let _ = write!(out, "{}", *value as i64);
                } else {
                    let _ = write!(out, "{value}");
                }
            }
            Json::String(value) => write_string(out, value),
            Json::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Json::Object(fields) => {
                out.push('{');
                for (index, (name, value)) in fields.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write_string(out, name);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }
}

fn write_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// What was wrong with a message, in enough detail to put in a log.
///
/// Not a diagnostic. A diagnostic is about the program being compiled, and
/// this is about the editor talking to the compiler, which is a different
/// audience with a different idea of what is actionable.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParseError {
    pub message: String,
    pub offset: usize,
}

pub fn parse(text: &str) -> Result<Json, ParseError> {
    let mut parser = Parser {
        chars: text.chars().collect(),
        pos: 0,
    };
    parser.skip_space();
    let value = parser.value()?;
    parser.skip_space();
    if parser.pos != parser.chars.len() {
        return Err(parser.error("trailing text after the value"));
    }
    Ok(value)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn error(&self, message: &str) -> ParseError {
        ParseError {
            message: message.to_string(),
            offset: self.pos,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), ParseError> {
        if self.peek() == Some(expected) {
            self.pos += 1;
            return Ok(());
        }
        Err(self.error(&format!("expected `{expected}`")))
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, ParseError> {
        for expected in word.chars() {
            if self.bump() != Some(expected) {
                return Err(self.error(&format!("expected `{word}`")));
            }
        }
        Ok(value)
    }

    fn value(&mut self) -> Result<Json, ParseError> {
        match self.peek() {
            None => Err(self.error("expected a value")),
            Some('n') => self.literal("null", Json::Null),
            Some('t') => self.literal("true", Json::Bool(true)),
            Some('f') => self.literal("false", Json::Bool(false)),
            Some('"') => {
                let text = self.string()?;
                Ok(Json::String(text))
            }
            Some('[') => self.array(),
            Some('{') => self.object(),
            Some(c) if c == '-' || c.is_ascii_digit() => self.number(),
            Some(c) => Err(self.error(&format!("unexpected character `{c}`"))),
        }
    }

    fn array(&mut self) -> Result<Json, ParseError> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_space();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_space();
            items.push(self.value()?);
            self.skip_space();
            match self.bump() {
                Some(',') => {}
                Some(']') => return Ok(Json::Array(items)),
                _ => return Err(self.error("expected `,` or `]`")),
            }
        }
    }

    fn object(&mut self) -> Result<Json, ParseError> {
        self.expect('{')?;
        let mut fields = Vec::new();
        self.skip_space();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_space();
            let name = self.string()?;
            self.skip_space();
            self.expect(':')?;
            self.skip_space();
            let value = self.value()?;
            fields.push((name, value));
            self.skip_space();
            match self.bump() {
                Some(',') => {}
                Some('}') => return Ok(Json::Object(fields)),
                _ => return Err(self.error("expected `,` or `}`")),
            }
        }
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some('+' | '-')) {
                self.pos += 1;
            }
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        match text.parse::<f64>() {
            Ok(value) => Ok(Json::Number(value)),
            Err(_) => Err(self.error("not a number")),
        }
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return Err(self.error("unterminated string")),
                Some('"') => return Ok(out),
                Some('\\') => out.push(self.escape()?),
                Some(c) => out.push(c),
            }
        }
    }

    fn escape(&mut self) -> Result<char, ParseError> {
        match self.bump() {
            Some('"') => Ok('"'),
            Some('\\') => Ok('\\'),
            Some('/') => Ok('/'),
            Some('b') => Ok('\u{8}'),
            Some('f') => Ok('\u{c}'),
            Some('n') => Ok('\n'),
            Some('r') => Ok('\r'),
            Some('t') => Ok('\t'),
            Some('u') => self.unicode_escape(),
            _ => Err(self.error("unknown escape sequence")),
        }
    }

    /// `\uXXXX`, and the surrogate pair that follows it when there is one.
    ///
    /// An editor sending an emoji in a document is ordinary, and a server that
    /// mangled one would corrupt the file it was asked to help with.
    fn unicode_escape(&mut self) -> Result<char, ParseError> {
        let high = self.hex4()?;
        if !(0xD800..0xDC00).contains(&high) {
            return char::from_u32(high).ok_or_else(|| self.error("not a character"));
        }

        if self.bump() != Some('\\') || self.bump() != Some('u') {
            return Err(self.error("a high surrogate needs a low one after it"));
        }
        let low = self.hex4()?;
        if !(0xDC00..0xE000).contains(&low) {
            return Err(self.error("expected a low surrogate"));
        }
        let combined = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
        char::from_u32(combined).ok_or_else(|| self.error("not a character"))
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let mut value = 0u32;
        for _ in 0..4 {
            let Some(digit) = self.bump().and_then(|c| c.to_digit(16)) else {
                return Err(self.error("expected four hex digits"));
            };
            value = value * 16 + digit;
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{Json, parse};

    #[test]
    fn round_trips_the_shapes_the_protocol_uses() {
        for text in [
            "null",
            "true",
            "false",
            "0",
            "-1",
            "12345",
            "\"\"",
            "\"hello\"",
            "[]",
            "[1,2,3]",
            "{}",
            "{\"a\":1,\"b\":[true,null]}",
        ] {
            let value = parse(text).expect(text);
            assert_eq!(value.to_text(), text, "round trip of {text}");
        }
    }

    #[test]
    fn whitespace_between_anything_is_fine() {
        let value = parse("  {\n \"a\" : [ 1 , 2 ]\t}  ").unwrap();
        assert_eq!(value.to_text(), "{\"a\":[1,2]}");
    }

    #[test]
    fn escapes_survive_both_directions() {
        let value = parse("\"a\\\"b\\\\c\\nd\\u0041\"").unwrap();
        assert_eq!(value.as_str().unwrap(), "a\"b\\c\ndA");
        assert_eq!(value.to_text(), "\"a\\\"b\\\\c\\ndA\"");
    }

    #[test]
    fn a_surrogate_pair_becomes_one_character() {
        // An emoji in a document is ordinary, and mangling one would corrupt
        // the file the server was asked to help with.
        let value = parse("\"\\ud83d\\ude00\"").unwrap();
        assert_eq!(value.as_str().unwrap(), "\u{1F600}");
    }

    #[test]
    fn a_lone_high_surrogate_is_refused_rather_than_guessed_at() {
        assert!(parse("\"\\ud83d\"").is_err());
    }

    #[test]
    fn reaching_into_nested_objects() {
        let value = parse("{\"params\":{\"textDocument\":{\"uri\":\"file:///a.vow\"}}}").unwrap();
        assert_eq!(
            value
                .at(&["params", "textDocument", "uri"])
                .and_then(Json::as_str),
            Some("file:///a.vow")
        );
        assert!(value.at(&["params", "missing"]).is_none());
    }

    #[test]
    fn broken_input_says_where_it_gave_up() {
        for text in ["", "{", "{\"a\"}", "[1,]", "tru", "\"unterminated"] {
            assert!(parse(text).is_err(), "should have refused `{text}`");
        }
    }

    #[test]
    fn trailing_text_is_not_ignored() {
        // Two values in one message means the framing went wrong, and carrying
        // on with the first would hide it.
        assert!(parse("{} {}").is_err());
    }
}
