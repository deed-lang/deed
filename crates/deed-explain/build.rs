//! Build script for deed-explain.
//!
//! Reads every `codes.rs` in the workspace, extracts each code's identifier,
//! constant name and doc-comment reasoning, then searches the test corpus for
//! the smallest deed snippet that triggers it.  The result is written to
//! `${OUT_DIR}/pages.rs` and included by `src/lib.rs`.
//!
//! Nothing here is invented.  The reasoning comes from the `///` lines
//! already above each `pub const` in `codes.rs`; the example comes from the
//! test that the ratchet in `deed-driver/tests/codes.rs` already requires to
//! exist.  A code with no doc-comment lines produces an entry whose `text`
//! field is empty, and the ratchet in `deed-driver/tests/explain.rs` treats
//! that the same way as a code with no test: a build failure.

use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=../");

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir.join("..").join("..");

    let codes = declared_codes(&root);
    let test_text = test_corpus(&root);

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set by Cargo");
    let out_path = PathBuf::from(out_dir).join("pages.rs");

    let mut output = String::new();
    output.push_str("/// One generated page per diagnostic code.\n");
    output.push_str("#[derive(Debug)]\n");
    output.push_str("pub struct Page {\n");
    output.push_str("    /// The code string, e.g. `\"DEED4025\"`.\n");
    output.push_str("    pub code: &'static str,\n");
    output.push_str("    /// The constant name, e.g. `\"BROKEN_PRECONDITION\"`.\n");
    output.push_str("    pub name: &'static str,\n");
    output.push_str("    /// The reasoning: doc-comment lines from `codes.rs`.\n");
    output.push_str("    pub text: &'static str,\n");
    output.push_str("    /// A deed snippet that triggers this code, if one could be\n");
    output.push_str("    /// extracted automatically from an existing test.\n");
    output.push_str("    pub example: Option<&'static str>,\n");
    output.push_str("    /// The test file the example came from.\n");
    output.push_str("    pub example_source: Option<&'static str>,\n");
    output.push_str("}\n\n");

    output.push_str("pub static PAGES: &[Page] = &[\n");

    for (name, code, doc) in &codes {
        let (example, example_source) = find_example(&test_text, name, code);
        output.push_str("    Page {\n");
        output.push_str(&format!("        code: {:?},\n", code));
        output.push_str(&format!("        name: {:?},\n", name));
        output.push_str(&format!("        text: {:?},\n", doc));
        match example {
            Some(ex) => {
                output.push_str(&format!("        example: Some({:?}),\n", ex));
                output.push_str(&format!(
                    "        example_source: Some({:?}),\n",
                    example_source.unwrap_or_else(String::new)
                ));
            }
            None => {
                output.push_str("        example: None,\n");
                output.push_str("        example_source: None,\n");
            }
        }
        output.push_str("    },\n");
    }

    output.push_str("];\n");

    fs::write(&out_path, output).expect("should write pages.rs");
}

/// All `pub const NAME: &str = "DEEDnnnn"` entries across the workspace,
/// together with the `///` doc-comment lines that immediately precede them.
fn declared_codes(root: &Path) -> Vec<(String, String, String)> {
    let mut codes = Vec::new();

    let mut crate_dirs: Vec<PathBuf> = fs::read_dir(root.join("crates"))
        .expect("crates/ should exist")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    crate_dirs.sort();

    for krate in crate_dirs {
        let path = krate.join("src").join("codes.rs");
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };

        let mut doc_lines: Vec<String> = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();

            if let Some(rest) = trimmed.strip_prefix("///") {
                // Doc-comment line: strip the leading space if any.
                let content = rest.strip_prefix(' ').unwrap_or(rest);
                doc_lines.push(content.to_string());
            } else if let Some(rest) = trimmed.strip_prefix("pub const ") {
                // Constant declaration.
                if let Some((name, rest)) = rest.split_once(':') {
                    if let Some(start) = rest.find("\"DEED") {
                        if let Some(end) = rest[start + 1..].find('"') {
                            let code = rest[start + 1..start + 1 + end].to_string();
                            let doc = doc_lines.join("\n");
                            codes.push((name.trim().to_string(), code, doc));
                        }
                    }
                }
                doc_lines.clear();
            } else if !trimmed.is_empty() && !trimmed.starts_with("//") {
                // Non-comment, non-empty, non-constant line resets accumulator.
                doc_lines.clear();
            }
        }
    }

    codes
}

/// All test source text in the workspace, with its path, so examples can be
/// traced back to the file they came from.
fn test_corpus(root: &Path) -> Vec<(String, String)> {
    let mut corpus = Vec::new();
    let crates_dir = root.join("crates");

    if let Ok(entries) = fs::read_dir(&crates_dir) {
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for krate in dirs {
            collect_test_files(&krate.join("tests"), root, &mut corpus);
        }
    }
    corpus
}

fn collect_test_files(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_test_files(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && path.file_name().is_some_and(|n| n != "codes.rs")
        {
            if let Ok(text) = fs::read_to_string(&path) {
                // Record a workspace-relative path for display.
                let rel = path
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| path.to_string_lossy().into_owned());
                out.push((rel, text));
            }
        }
    }
}

/// The example and its source path for a given code, if one can be extracted
/// from the test corpus.
///
/// Strategy: find the test function that references the code (by constant
/// name or by the `"DEEDnnnn"` string), then look for the first string
/// literal in that function that looks like deed source.  "Looks like deed
/// source" means it contains a newline and at least one deed keyword.
fn find_example(
    corpus: &[(String, String)],
    name: &str,
    code: &str,
) -> (Option<String>, Option<String>) {
    for (path, text) in corpus {
        // Quick check before the heavier per-function scan.
        if !text.contains(name) && !text.contains(code) {
            continue;
        }

        if let Some(snippet) = extract_from_file(text, name, code) {
            return (Some(snippet), Some(path.clone()));
        }
    }
    (None, None)
}

/// Find the first deed snippet in `text` that is in a function referencing
/// `name` or `code`.
fn extract_from_file(text: &str, name: &str, code: &str) -> Option<String> {
    // Split into test functions.  Each starts at a `#[test]` marker.
    let test_positions: Vec<usize> = text.match_indices("#[test]").map(|(i, _)| i).collect();

    for &start in &test_positions {
        // The function body ends at the next `#[test]` or end of file.
        let end = test_positions
            .iter()
            .find(|&&p| p > start)
            .copied()
            .unwrap_or(text.len());
        let body = &text[start..end];

        if !body.contains(name) && !body.contains(code) {
            continue;
        }

        if let Some(snippet) = first_deed_snippet(body) {
            return Some(snippet);
        }
    }
    None
}

/// Attempt to extract the first deed-looking string literal from a block of
/// Rust source.
fn first_deed_snippet(body: &str) -> Option<String> {
    // We are looking for a `"..."` string literal that:
    //  - spans at least two characters and contains a newline
    //  - contains at least one deed keyword (`fn `, `type `, `record `,
    //    `module `, `handler `, `let `, `for `, `effect `)
    //
    // We walk character by character rather than using a regex so there
    // are no extra dependencies.

    let deed_markers = [
        "fn ", "type ", "record ", "module ", "handler ", "let ", "for ", "effect ", "choice ",
    ];

    let bytes = body.as_bytes();
    let len = body.len();
    let mut i = 0;

    while i < len {
        // Skip line comments so we do not find strings inside them.
        if bytes[i] == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if bytes[i] != b'"' {
            i += 1;
            continue;
        }

        // #780: a `format!` argument is a template, not a program. Its
        // placeholders and its doubled braces are Rust telling `format!` what
        // to do, and a page that prints them is showing something no compiler
        // would accept. The literal is skipped rather than repaired, and the
        // search carries on to the next one.
        let is_template = ends_with_format_macro(&body[..i]);

        // We are at the opening `"`.
        i += 1;
        let mut buf = String::new();
        let mut escaped = false;

        while i < len {
            let ch = bytes[i] as char;
            if escaped {
                match ch {
                    'n' => buf.push('\n'),
                    't' => buf.push('\t'),
                    'r' => buf.push('\r'),
                    '\\' => buf.push('\\'),
                    '"' => buf.push('"'),
                    // `\x20` and similar hex escapes: skip to end of escape.
                    'x' => {
                        i += 1;
                        if i + 1 < len {
                            i += 1; // consume two hex digits
                        }
                        buf.push(' '); // approximate
                    }
                    // A backslash before a newline is Rust joining two source
                    // lines into one string line: the newline and the
                    // indentation that follows it are not in the string. This
                    // arm used to fall through to the one below and put the
                    // backslash in the page.
                    '\n' => {
                        i += 1;
                        while i < len && (bytes[i] as char).is_whitespace() {
                            i += 1;
                        }
                        escaped = false;
                        continue;
                    }
                    '\r' => {
                        i += 1;
                        while i < len && (bytes[i] as char).is_whitespace() {
                            i += 1;
                        }
                        escaped = false;
                        continue;
                    }
                    _ => {
                        buf.push('\\');
                        buf.push(ch);
                    }
                }
                escaped = false;
                i += 1;
                continue;
            }
            match ch {
                '\\' => {
                    escaped = true;
                    i += 1;
                }
                '"' => {
                    i += 1;
                    break; // end of string
                }
                _ => {
                    buf.push(ch);
                    i += 1;
                }
            }
        }

        if is_template {
            continue;
        }

        // Check whether this looks like deed source.
        if buf.contains('\n') && deed_markers.iter().any(|m| buf.contains(m)) {
            return Some(buf);
        }
    }

    None
}

/// Whether the text immediately before a string literal opens a `format!`-like
/// macro call, which makes that literal a template rather than a program.
fn ends_with_format_macro(before: &str) -> bool {
    let head = before.trim_end();
    let Some(head) = head.strip_suffix('(') else {
        return false;
    };
    let head = head.trim_end();
    // `format!`, `write!`, `writeln!`, `panic!`, `assert!` and the rest all
    // end the same way, and every one of them takes a template first.
    head.ends_with('!')
}
