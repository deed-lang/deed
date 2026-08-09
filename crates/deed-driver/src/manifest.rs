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
//! Each `module` line names bytes rather than a directory: where they are and
//! what they hash to. What the module is called is not here, because the file
//! says that on its own `module` line, so a location and a hash are the whole
//! of what a dependency is.
//!
//! No other directives exist. An unrecognized line is `DEED7001`. A
//! `component` with no path is `DEED7002`, and the three ways of writing a
//! `module` line wrong are `DEED7003` to `DEED7005`. All are errors reported
//! through the same diagnostic infrastructure as every other compiler error.
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

/// A module declared by where its bytes are and what they hash to.
///
/// The name is not here on purpose. A fetched module is named by its own
/// `module` line, the same way every other module is, so the URL says where
/// the bytes were and the hash says what they are and neither says what the
/// module is called. Two projects that fetch the same bytes from two
/// locations get one module and one cache entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedModule {
    /// Where to look when the cache does not already hold these bytes.
    pub url: String,
    /// The SHA-256 the bytes must have, as 64 lowercase hexadecimal digits.
    pub hash: String,
    /// Where the directive was written, for a diagnostic about the fetch.
    pub span: Span,
}

/// The result of parsing one `deed.manifest` file.
#[derive(Debug)]
pub struct Manifest {
    /// Component roots in declaration order.
    pub components: Vec<ComponentRoot>,
    /// Fetched modules in declaration order.
    pub modules: Vec<FetchedModule>,
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
    let mut modules = Vec::new();
    let mut diagnostics = Vec::new();

    // The same rule `design/06-grammar.md` gives source: a byte order mark is
    // not part of the file. Windows editors write one by default, and without
    // this the first directive is refused as unrecognized by a message that
    // lists it as one of the two that are recognized.
    let mut offset: u32 = 0;
    let text = match text.strip_prefix('\u{FEFF}') {
        Some(rest) => {
            offset = '\u{FEFF}'.len_utf8() as u32;
            rest
        }
        None => text,
    };

    for raw_line in text.split_inclusive('\n') {
        let line_start = offset;
        offset += raw_line.len() as u32;
        let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        let line = line.strip_suffix('\r').unwrap_or(line);

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

        if let Some(rest) = trimmed.strip_prefix("module")
            && rest.chars().next().is_none_or(char::is_whitespace)
        {
            parse_module(file, line, line_start, rest, &mut modules, &mut diagnostics);
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
            .with_note(
                "a `deed.manifest` has `component <path>` and `module <url> sha256:<digest>`",
            ),
        );
    }

    Manifest {
        components,
        modules,
        diagnostics,
    }
}

/// Reads `module <url> sha256:<digest>`.
///
/// The hash is not optional and there is no second algorithm. A dependency
/// without one is a dependency whose bytes are whatever the other end felt
/// like today, and a build that accepts those is not a build anybody can
/// repeat.
fn parse_module(
    file: FileId,
    line: &str,
    line_start: u32,
    rest: &str,
    modules: &mut Vec<FetchedModule>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let span = line_span(line, line_start);
    let mut parts = rest.split_whitespace();
    let Some(url) = parts.next() else {
        diagnostics.push(
            Diagnostic::error(
                codes::MISSING_MODULE_SOURCE,
                file,
                directive_span(line, line_start, "module"),
                "`module` needs a location and a hash",
            )
            .with_primary_label("nothing to fetch")
            .with_note("write `module <url> sha256:<digest>`"),
        );
        return;
    };

    let Some(hash) = parts.next() else {
        diagnostics.push(
            Diagnostic::error(
                codes::MISSING_MODULE_HASH,
                file,
                span,
                format!("`{url}` was given no hash"),
            )
            .with_primary_label("no hash")
            .with_note(
                "a location says where bytes were and a hash says what they are; without \
                 the second a build cannot be repeated, so it is not optional",
            ),
        );
        return;
    };

    if parts.next().is_some() {
        diagnostics.push(
            Diagnostic::error(
                codes::UNKNOWN_DIRECTIVE,
                file,
                span,
                "`module` takes a location and a hash, and nothing else",
            )
            .with_primary_label("too much on this line")
            .with_note("one module a line, so a diff shows which one changed"),
        );
        return;
    }

    let Some(digest) = hash.strip_prefix("sha256:") else {
        diagnostics.push(
            Diagnostic::error(
                codes::BAD_MODULE_HASH,
                file,
                span,
                format!("`{hash}` does not name an algorithm this understands"),
            )
            .with_primary_label("expected `sha256:`")
            .with_note(
                "SHA-256 is the only one, and it is spelled out so that a second one could \
                 be told from it rather than guessed at by length",
            ),
        );
        return;
    };

    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        diagnostics.push(
            Diagnostic::error(
                codes::BAD_MODULE_HASH,
                file,
                span,
                format!("`{digest}` is not a SHA-256 digest"),
            )
            .with_primary_label("not sixty-four hexadecimal digits")
            .with_note("a digest that is nearly right is a digest nothing will ever match"),
        );
        return;
    }

    if digest.bytes().any(|b| b.is_ascii_uppercase()) {
        diagnostics.push(
            Diagnostic::error(
                codes::BAD_MODULE_HASH,
                file,
                span,
                format!("`{digest}` is not written in lowercase"),
            )
            .with_primary_label("uppercase digits")
            .with_note(
                "the cache is keyed by these characters, so two spellings of one digest \
                 would be two entries for one set of bytes",
            ),
        );
        return;
    }

    modules.push(FetchedModule {
        url: url.to_string(),
        hash: digest.to_string(),
        span,
    });
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
