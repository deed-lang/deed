//! The generator behind `crates/deed-explain/generated/pages.rs`, and the two
//! tests that keep that file honest.
//!
//! This used to be a build script. It read every `codes.rs` in the workspace
//! and the whole test corpus, which works exactly as long as the workspace is
//! there. A published `deed-explain` carries its own directory and nothing
//! else, so the same build script would have found an empty tree, generated
//! zero pages, and **compiled**: `deed explain DEED4025` would print nothing
//! at all, for every code, on every machine that installed the compiler from
//! crates.io. Measured with `cargo package -p deed-explain --list`, which
//! lists `build.rs` and `src/lib.rs` and no workspace.
//!
//! So the pages are generated here, committed, and shipped as source. The
//! reading of the tree happens where the tree exists: in a test.
//!
//! - `the_generated_pages_are_the_ones_this_tree_would_produce` fails when the
//!   committed file has drifted from `codes.rs`.
//! - `regenerate_the_pages` (`--ignored`) writes it.
//!
//! Nothing here is invented. The reasoning is the `///` lines already above
//! each `pub const` in a `codes.rs`; the example is lifted from the test that
//! `crates/deed-driver/tests/codes.rs` already requires to exist.

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root should be two directories up")
        .to_path_buf()
}

fn generated() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("generated")
        .join("pages.rs")
}

#[test]
fn the_generated_pages_are_the_ones_this_tree_would_produce() {
    let committed = fs::read_to_string(generated()).expect("generated/pages.rs should be there");
    assert_eq!(
        committed.replace("\r\n", "\n"),
        render(&root()),
        "generated/pages.rs has drifted from the `codes.rs` files it is made of. \
         Run: cargo test -p deed-explain --test generated -- --ignored"
    );
}

#[test]
#[ignore = "writes a file; run it on purpose"]
fn regenerate_the_pages() {
    fs::write(generated(), render(&root())).expect("should write generated/pages.rs");
}

/// The pages have to travel inside the crate.
///
/// This is the bug that started it, stated as a rule: whatever `src/lib.rs`
/// pulls the pages out of has to be a file this package carries. `OUT_DIR`
/// does not qualify, because filling it needed a workspace that a published
/// crate does not have, and neither does anything above the package root.
#[test]
fn the_pages_come_from_a_file_this_package_carries() {
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lib = fs::read_to_string(package.join("src").join("lib.rs")).expect("src/lib.rs");

    let inside = carried_path(&lib).unwrap_or_else(|why| panic!("{why}"));
    assert!(
        package.join(inside).is_file(),
        "the pages are included from a file that is not there"
    );
}

/// Where `include!` reads the pages from, relative to the package root, if
/// that is somewhere inside it.
fn carried_path(lib: &str) -> Result<String, String> {
    let argument = lib
        .split_once("include!(")
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(argument, _)| argument.trim())
        .ok_or("src/lib.rs does not include the generated pages")?;

    let path = argument
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .ok_or_else(|| {
            format!(
                "the pages are included as `{argument}`, which is not a path this package holds"
            )
        })?;

    // Resolved against `src/`, where the including file sits.
    let mut parts: Vec<&str> = vec!["src"];
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!(
                        "the pages are included from `{path}`, which leaves the package"
                    ));
                }
            }
            other => parts.push(other),
        }
    }
    Ok(parts.join("/"))
}

#[test]
fn the_rule_rejects_the_shape_this_crate_used_to_have() {
    let out_dir = carried_path("include!(concat!(env!(\"OUT_DIR\"), \"/pages.rs\"));").unwrap_err();
    assert!(
        out_dir.contains("not a path this package holds"),
        "{out_dir}"
    );

    let above = carried_path("include!(\"../../pages.rs\");").unwrap_err();
    assert!(above.contains("leaves the package"), "{above}");

    assert_eq!(
        carried_path("include!(\"../generated/pages.rs\");"),
        Ok("generated/pages.rs".to_string())
    );
}

/// The whole of the generated file, as text.
fn render(root: &Path) -> String {
    let codes = declared_codes(root);
    assert!(
        codes.len() > 50,
        "only {} codes were found, so something did not read the tree",
        codes.len()
    );
    let test_text = test_corpus(root);

    let mut output = String::new();
    output
        .push_str("// @generated by `cargo test -p deed-explain --test generated -- --ignored`.\n");
    output.push_str("// The reasoning comes from the doc comments above each `pub const` in a\n");
    output.push_str("// `codes.rs`; the example comes from a test that already had to exist.\n");
    output.push_str("// Edit those, not this.\n\n");
    output.push_str("pub static PAGES: &[Page] = &[\n");

    for (name, code, doc) in &codes {
        let (example, example_source) = find_example(&test_text, name, code);
        output.push_str("    Page {\n");
        output.push_str(&format!("        code: {code:?},\n"));
        output.push_str(&format!("        name: {name:?},\n"));
        output.push_str(&format!("        text: {doc:?},\n"));
        match example {
            Some(ex) => {
                output.push_str(&format!("        example: Some({ex:?}),\n"));
                output.push_str(&format!(
                    "        example_source: Some({:?}),\n",
                    example_source.unwrap_or_default()
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
    output
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
                let content = rest.strip_prefix(' ').unwrap_or(rest);
                doc_lines.push(content.to_string());
            } else if let Some(rest) = trimmed.strip_prefix("pub const ") {
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
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    // Sorted, or the pages depend on what order the filesystem hands them back
    // and the freshness check above becomes a coin toss.
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_test_files(&path, root, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && path.file_name().is_some_and(|n| n != "codes.rs")
        {
            if let Ok(text) = fs::read_to_string(&path) {
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
/// literal in that function that looks like deed source. "Looks like deed
/// source" means it contains a newline and at least one deed keyword.
fn find_example(
    corpus: &[(String, String)],
    name: &str,
    code: &str,
) -> (Option<String>, Option<String>) {
    for (path, text) in corpus {
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
    let test_positions: Vec<usize> = text.match_indices("#[test]").map(|(i, _)| i).collect();

    for &start in &test_positions {
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
    // A `"..."` literal that spans a newline and carries a deed keyword.
    // Walked character by character rather than with a regex, because this
    // workspace has no dependencies.
    let deed_markers = [
        "fn ", "type ", "record ", "module ", "handler ", "let ", "for ", "effect ", "choice ",
    ];

    let bytes = body.as_bytes();
    let len = body.len();
    let mut i = 0;

    while i < len {
        // Line comments, skipped so a string inside one is not found.
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
        // placeholders and doubled braces are Rust talking to `format!`, and a
        // page that prints them shows something no compiler would accept. The
        // literal is skipped rather than repaired.
        let is_template = ends_with_format_macro(&body[..i]);

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
                    'x' => {
                        i += 1;
                        if i + 1 < len {
                            i += 1;
                        }
                        buf.push(' ');
                    }
                    // A backslash before a newline is Rust joining two source
                    // lines: neither the newline nor the indentation after it
                    // is in the string.
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
                    break;
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
