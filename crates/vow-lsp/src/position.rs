//! Between the offsets the compiler uses and the positions an editor sends.
//!
//! Everything downstream of the lexer works in byte offsets. The language
//! server protocol works in zero based lines and **UTF-16 code units**, which
//! is neither bytes nor characters. For ASCII all three agree, which is why
//! this is the kind of thing that works until somebody writes a comment in
//! Turkish and then silently points at the wrong column.
//!
//! `vow_diagnostics::Location` is not reused here on purpose: its column
//! counts characters and is one based, because it exists to underline text for
//! a person. Converting between two nearly-identical wrong answers is worse
//! than doing this once, honestly, in the one place that needs it.

/// A zero based line and UTF-16 column, as the protocol spells it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

impl Position {
    pub fn new(line: u32, character: u32) -> Self {
        Self { line, character }
    }
}

/// The line starts of one document, so a conversion is a binary search rather
/// than a walk from the top of the file.
pub struct Lines {
    starts: Vec<u32>,
    length: u32,
}

impl Lines {
    pub fn of(text: &str) -> Self {
        let mut starts = vec![0u32];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(offset as u32 + 1);
            }
        }
        Self {
            starts,
            length: text.len() as u32,
        }
    }

    /// The byte offset a position names.
    ///
    /// Out of range positions clamp rather than failing. An editor can send a
    /// position for a document state the server has not caught up with yet,
    /// and refusing to answer is worse than answering about the nearest place
    /// that exists.
    pub fn offset(&self, text: &str, position: Position) -> u32 {
        let line = position.line as usize;
        if line >= self.starts.len() {
            return self.length;
        }
        let start = self.starts[line] as usize;
        let end = self
            .starts
            .get(line + 1)
            .map(|&next| next as usize)
            .unwrap_or(text.len());

        let mut units = 0u32;
        for (offset, ch) in text[start..end].char_indices() {
            if units >= position.character {
                return (start + offset) as u32;
            }
            units += ch.len_utf16() as u32;
        }
        end as u32
    }

    /// The position a byte offset lands on.
    ///
    /// An offset inside a character rounds down to its start, because there is
    /// no position between two halves of one and the alternative is a panic.
    pub fn position(&self, text: &str, offset: u32) -> Position {
        let offset = offset.min(self.length) as usize;
        let line = match self.starts.binary_search(&(offset as u32)) {
            Ok(exact) => exact,
            Err(next) => next - 1,
        };
        let start = self.starts[line] as usize;
        let mut end = offset;
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }
        let character = text[start..end].chars().map(char::len_utf16).sum::<usize>() as u32;
        Position {
            line: line as u32,
            character,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Lines, Position};

    fn round_trip(text: &str, offset: u32) {
        let lines = Lines::of(text);
        let position = lines.position(text, offset);
        assert_eq!(
            lines.offset(text, position),
            offset,
            "offset {offset} in {text:?} via {position:?}"
        );
    }

    #[test]
    fn ascii_agrees_with_itself_everywhere() {
        let text = "module a\n\nfn f() -> Int { 0 }\n";
        for offset in 0..=text.len() as u32 {
            round_trip(text, offset);
        }
    }

    #[test]
    fn lines_and_characters_are_zero_based() {
        let text = "abc\ndef\n";
        let lines = Lines::of(text);
        assert_eq!(lines.position(text, 0), Position::new(0, 0));
        assert_eq!(lines.position(text, 2), Position::new(0, 2));
        assert_eq!(lines.position(text, 4), Position::new(1, 0));
    }

    #[test]
    fn a_character_outside_ascii_is_one_unit_and_several_bytes() {
        // `ü` is two bytes and one UTF-16 unit. A server that counted bytes
        // would underline one column too far right for every line with a
        // Turkish word in it, which is most of the comments in this project.
        let text = "let gün = 1";
        let lines = Lines::of(text);
        let offset = text.find('=').unwrap() as u32;
        assert_eq!(lines.position(text, offset), Position::new(0, 8));
        assert_eq!(lines.offset(text, Position::new(0, 8)), offset);
    }

    #[test]
    fn an_astral_character_is_two_units() {
        // The case that makes UTF-16 different from counting characters as
        // well as from counting bytes.
        let text = "a\u{1F600}b";
        let lines = Lines::of(text);
        let b = text.find('b').unwrap() as u32;
        assert_eq!(lines.position(text, b), Position::new(0, 3));
        assert_eq!(lines.offset(text, Position::new(0, 3)), b);
    }

    #[test]
    fn every_offset_round_trips_through_a_document_with_mixed_widths() {
        let text = "// gün\nfn f() -> Int {\n    \u{1F600}\n}\n";
        for offset in 0..=text.len() as u32 {
            if text.is_char_boundary(offset as usize) {
                round_trip(text, offset);
            }
        }
    }

    #[test]
    fn positions_past_the_end_clamp_rather_than_failing() {
        let text = "ab\n";
        let lines = Lines::of(text);
        assert_eq!(lines.offset(text, Position::new(99, 0)), 3);
        assert_eq!(lines.offset(text, Position::new(0, 99)), 3);
        assert_eq!(lines.position(text, 99), Position::new(1, 0));
    }

    #[test]
    fn an_offset_inside_a_character_rounds_down() {
        let text = "ü";
        let lines = Lines::of(text);
        assert_eq!(lines.position(text, 1), Position::new(0, 0));
    }
}
