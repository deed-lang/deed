//! Parsing a `deed.manifest` file.
//!
//! A manifest answers exactly one question: where does code that is not in
//! the current source tree live? It lists the roots of external components,
//! one per line. Nothing else. It cannot change what a program means; it can
//! only tell the compiler where to look for modules it could not find through
//! normal path derivation.
//!
//! # Format
//!
//! ```text
//! # Comments start with a hash sign.
//! # Blank lines are ignored.
//!
//! component ../other-project
//! component /absolute/path/to/lib
//! ```
//!
//! Each `component` line names a directory that is an additional root for
//! module resolution. Paths are resolved relative to the directory that
//! contains the manifest file.
//!
//! No other directives exist. An unrecognized line is `DEED7001`. A
//! `component` with no path is `DEED7002`. Both are errors reported through
//! the same diagnostic infrastructure as every other compiler error.
//!
//! # What the manifest cannot do
//!
//! A manifest cannot change the meaning of a module. It cannot override a
//! module the named files already imply a root for, remap names, select
//! features, or configure any compiler behaviour. Every component root it
//! declares is searched only after the roots derived from the named files
//! have been asked and have not answered.
//!
//! That constraint is structural. The format has one directive. The directive
//! adds a root. There is no other directive, so there is no other effect.

use std::path::PathBuf;

use deed_diagnostics::{Diagnostic, FileId, Span};

use crate::codes;

/// A component root declared by a `deed.manifest` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentRoot {
    pub path: PathBuf,
}

/// The result of parsing one `deed.manifest` file.
#[derive(Debug)]
pub struct Manifest {
    /// Component roots in declaration order.
    pub components: Vec<ComponentRoot>,
    /// Parse errors. A manifest with errors still returns whatever roots
    /// could be read, so an error on one line does not hide the others.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses the text of a `deed.manifest` file.
///
/// `file` is the [`FileId`] the caller registered in a [`SourceMap`] before
/// calling this function; it is used only to anchor the diagnostics.
///
/// [`SourceMap`]: deed_diagnostics::SourceMap
pub fn parse_manifest(file: FileId, text: &str) -> Manifest {
    let mut components = Vec::new();
    let mut diagnostics = Vec::new();

    let mut offset: u32 = 0;
    for line in text.lines() {
        let line_start = offset;
        // Advance past this line's bytes plus the newline that follows it.
        // `.lines()` strips the newline, so we add 1 when the byte that
        // follows the content is `\n` (or `\r\n`).
        let raw_len = if text[offset as usize..].starts_with(line)
            && text[offset as usize + line.len()..].starts_with('\n')
        {
            line.len() as u32 + 1
        } else if text[offset as usize..].starts_with(line)
            && text[offset as usize + line.len()..].starts_with("\r\n")
        {
            line.len() as u32 + 2
        } else {
            // Last line with no trailing newline.
            line.len() as u32
        };
        offset += raw_len;

        let trimmed = line.trim();

        // Blank lines and comment lines are silently accepted.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("component") {
            // The directive is valid. Check that a path follows.
            let path_str = rest.trim();
            if path_str.is_empty() {
                let span = directive_span(line, line_start, "component");
                diagnostics.push(
                    Diagnostic::error(
                        codes::MISSING_COMPONENT_PATH,
                        file,
                        span,
                        "`component` needs a path",
                    )
                    .with_primary_label("path missing here")
                    .with_note(
                        "write the directory that is the root of the external component, \
                         relative to this manifest file",
                    ),
                );
            } else {
                components.push(ComponentRoot {
                    path: PathBuf::from(path_str),
                });
            }
            continue;
        }

        // Anything else is unknown.
        let span = line_span(line, line_start);
        diagnostics.push(
            Diagnostic::error(
                codes::UNKNOWN_DIRECTIVE,
                file,
                span,
                format!(
                    "unknown manifest directive `{}`",
                    trimmed.split_whitespace().next().unwrap_or(trimmed)
                ),
            )
            .with_primary_label("not a recognized directive")
            .with_note("the only directive in a `deed.manifest` is `component <path>`"),
        );
    }

    Manifest {
        components,
        diagnostics,
    }
}

/// Span covering the entire trimmed content of a line.
fn line_span(line: &str, line_start: u32) -> Span {
    let leading = (line.len() - line.trim_start().len()) as u32;
    let trimmed_len = line.trim().len() as u32;
    Span::new(line_start + leading, line_start + leading + trimmed_len)
}

/// Span covering one keyword at the start of the trimmed content of a line.
fn directive_span(line: &str, line_start: u32, keyword: &str) -> Span {
    let leading = (line.len() - line.trim_start().len()) as u32;
    Span::new(
        line_start + leading,
        line_start + leading + keyword.len() as u32,
    )
}

#[cfg(test)]
mod tests {
    use deed_diagnostics::SourceMap;

    use super::*;

    fn parse_text(text: &str) -> Manifest {
        let mut sources = SourceMap::new();
        let file = sources.add("deed.manifest", text.to_string());
        parse_manifest(file, text)
    }

    #[test]
    fn empty_manifest_is_valid() {
        let m = parse_text("");
        assert!(m.components.is_empty());
        assert!(m.diagnostics.is_empty());
    }

    #[test]
    fn blank_lines_and_comments_are_ignored() {
        let m = parse_text("\n# just a comment\n\n");
        assert!(m.components.is_empty());
        assert!(m.diagnostics.is_empty());
    }

    #[test]
    fn single_component_is_parsed() {
        let m = parse_text("component ../other\n");
        assert_eq!(m.components.len(), 1);
        assert_eq!(m.components[0].path, PathBuf::from("../other"));
        assert!(m.diagnostics.is_empty());
    }

    #[test]
    fn multiple_components_are_parsed() {
        let m = parse_text("component ../a\ncomponent ../b\n");
        assert_eq!(m.components.len(), 2);
        assert_eq!(m.components[0].path, PathBuf::from("../a"));
        assert_eq!(m.components[1].path, PathBuf::from("../b"));
        assert!(m.diagnostics.is_empty());
    }

    #[test]
    fn component_with_absolute_path() {
        let m = parse_text("component /abs/path\n");
        assert_eq!(m.components.len(), 1);
        assert_eq!(m.components[0].path, PathBuf::from("/abs/path"));
    }

    #[test]
    fn comments_and_components_mixed() {
        let text = "# first root\ncomponent ../a\n\n# second root\ncomponent ../b\n";
        let m = parse_text(text);
        assert_eq!(m.components.len(), 2);
        assert!(m.diagnostics.is_empty());
    }

    #[test]
    fn component_without_path_is_deed7002() {
        let m = parse_text("component\n");
        assert!(m.components.is_empty());
        assert_eq!(m.diagnostics.len(), 1);
        assert_eq!(m.diagnostics[0].code, codes::MISSING_COMPONENT_PATH);
    }

    #[test]
    fn component_with_only_whitespace_is_deed7002() {
        let m = parse_text("component   \n");
        assert!(m.components.is_empty());
        assert_eq!(m.diagnostics.len(), 1);
        assert_eq!(m.diagnostics[0].code, codes::MISSING_COMPONENT_PATH);
    }

    #[test]
    fn unknown_directive_is_deed7001() {
        let m = parse_text("target release\n");
        assert!(m.components.is_empty());
        assert_eq!(m.diagnostics.len(), 1);
        assert_eq!(m.diagnostics[0].code, codes::UNKNOWN_DIRECTIVE);
    }

    #[test]
    fn errors_on_one_line_do_not_suppress_valid_components() {
        let text = "component ../a\nbuild debug\ncomponent ../b\n";
        let m = parse_text(text);
        assert_eq!(m.components.len(), 2);
        assert_eq!(m.diagnostics.len(), 1);
        assert_eq!(m.diagnostics[0].code, codes::UNKNOWN_DIRECTIVE);
    }

    #[test]
    fn span_of_unknown_directive_covers_the_directive_word() {
        let text = "target release\n";
        let m = parse_text(text);
        assert_eq!(m.diagnostics.len(), 1);
        let span = m.diagnostics[0].primary.span;
        // "target release" starts at offset 0; the whole line is 14 chars.
        assert_eq!(span.start, 0);
        assert_eq!(span.end, 14);
    }

    #[test]
    fn span_of_missing_path_covers_the_component_keyword() {
        let text = "component\n";
        let m = parse_text(text);
        assert_eq!(m.diagnostics.len(), 1);
        let span = m.diagnostics[0].primary.span;
        // "component" starts at offset 0, length 9.
        assert_eq!(span.start, 0);
        assert_eq!(span.end, 9);
    }

    #[test]
    fn leading_whitespace_does_not_confuse_the_parser() {
        let m = parse_text("  component ../x\n");
        assert_eq!(m.components.len(), 1);
        assert_eq!(m.components[0].path, PathBuf::from("../x"));
        assert!(m.diagnostics.is_empty());
    }
}
