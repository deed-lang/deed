//! Source files and the mapping from byte offsets back to line and column.
//!
//! Everything downstream of the lexer works in byte offsets. Line and column
//! are computed only when a diagnostic is rendered, so the common path never
//! pays for them.

use crate::span::Span;

/// Handle to a file inside a [`SourceMap`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct FileId(u32);

impl FileId {
    /// The file's position in the [`SourceMap`] that handed it out.
    pub fn index(&self) -> u32 {
        self.0
    }
}

/// A one based line and column pair, suitable for showing to a person.
///
/// The column counts characters rather than bytes, so an underline lines up
/// under non-ASCII text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Location {
    pub line: u32,
    pub column: u32,
}

/// A single source file, with a precomputed index of where each line starts.
pub struct SourceFile {
    name: String,
    text: String,
    line_starts: Vec<u32>,
}

impl SourceFile {
    fn new(name: String, text: String) -> Self {
        let mut line_starts = vec![0u32];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset as u32 + 1);
            }
        }
        Self {
            name,
            text,
            line_starts,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }

    /// Resolves a byte offset to a one based line and column.
    ///
    /// Offsets past the end of the file clamp to the last position, so a
    /// diagnostic about unexpected end of input still renders somewhere sane.
    pub fn location(&self, offset: u32) -> Location {
        let offset = offset.min(self.text.len() as u32);
        let line_index = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(next) => next - 1,
        };
        let line_start = self.line_starts[line_index] as usize;
        let column = self.text[line_start..offset as usize].chars().count() as u32 + 1;
        Location {
            line: line_index as u32 + 1,
            column,
        }
    }

    /// The text of a one based line, without its trailing newline.
    pub fn line_text(&self, line: u32) -> &str {
        if line == 0 || line > self.line_count() {
            return "";
        }
        let start = self.line_starts[line as usize - 1] as usize;
        let end = self
            .line_starts
            .get(line as usize)
            .map(|&next| next as usize)
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }

    pub fn slice(&self, span: Span) -> &str {
        let end = (span.end as usize).min(self.text.len());
        let start = (span.start as usize).min(end);
        &self.text[start..end]
    }
}

/// Owns every file the compiler has seen.
#[derive(Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, name: impl Into<String>, text: impl Into<String>) -> FileId {
        let id = FileId(self.files.len() as u32);
        self.files.push(SourceFile::new(name.into(), text.into()));
        id
    }

    /// # Panics
    ///
    /// Panics if the id came from a different `SourceMap`, which is a bug
    /// rather than a recoverable condition.
    pub fn file(&self, id: FileId) -> &SourceFile {
        &self.files[id.0 as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::SourceMap;

    fn map(text: &str) -> (SourceMap, super::FileId) {
        let mut map = SourceMap::new();
        let id = map.add("test.vow", text);
        (map, id)
    }

    #[test]
    fn locations_are_one_based() {
        let (map, id) = map("abc\ndef\n");
        let file = map.file(id);
        assert_eq!(file.location(0), super::Location { line: 1, column: 1 });
        assert_eq!(file.location(2), super::Location { line: 1, column: 3 });
        assert_eq!(file.location(4), super::Location { line: 2, column: 1 });
    }

    #[test]
    fn newline_belongs_to_the_line_it_ends() {
        let (map, id) = map("ab\ncd");
        let file = map.file(id);
        assert_eq!(file.location(2), super::Location { line: 1, column: 3 });
        assert_eq!(file.location(3), super::Location { line: 2, column: 1 });
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let (map, id) = map("çğü x");
        let file = map.file(id);
        let offset = "çğü ".len() as u32;
        assert_eq!(
            file.location(offset),
            super::Location { line: 1, column: 5 }
        );
    }

    #[test]
    fn offsets_past_the_end_clamp() {
        let (map, id) = map("ab");
        let file = map.file(id);
        assert_eq!(file.location(999), super::Location { line: 1, column: 3 });
    }

    #[test]
    fn line_text_drops_the_newline() {
        let (map, id) = map("first\r\nsecond\n");
        let file = map.file(id);
        assert_eq!(file.line_text(1), "first");
        assert_eq!(file.line_text(2), "second");
        assert_eq!(file.line_text(99), "");
    }
}
