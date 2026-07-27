//! Every `.deed` file in the repository is in canonical form.
//!
//! Without this, P4 goes back to being a preference. A formatter nothing runs
//! is a formatter that drifts from the code it is supposed to describe, and
//! the drift is invisible until someone runs it and gets a thousand line diff.

use std::path::{Path, PathBuf};

use deed_diagnostics::SourceMap;
use deed_fmt::format;

#[test]
fn every_deed_file_in_the_repository_is_formatted() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();

    assert!(!files.is_empty(), "no `.deed` files found under {root:?}");

    let mut unformatted = Vec::new();
    for path in &files {
        let source = std::fs::read_to_string(path).unwrap();
        let mut sources = SourceMap::new();
        let file = sources.add(path.display().to_string(), source.clone());

        match format(file, &source) {
            Ok(formatted) if formatted == source => {}
            Ok(_) => unformatted.push(path.display().to_string()),
            Err(_) => panic!("{} does not parse", path.display()),
        }
    }

    assert!(
        unformatted.is_empty(),
        "not in canonical form, run `deed fmt`:\n  {}",
        unformatted.join("\n  ")
    );
}

/// One canonical form, reached from somewhere other than itself.
///
/// The test above says the corpus is already canonical, which means `format`
/// runs over it as the identity function. That is worth having and it proves
/// nothing about formatting, because the interesting input to a formatter is
/// code that is laid out badly, and nothing here had ever handed it any.
///
/// The properties in `format.rs` have the same shape of hole from the other
/// side. They run over a list of snippets written by hand, which is the set
/// somebody thought of rather than the set that turns up, and that file says
/// so about itself in its first paragraph.
///
/// So these are the real programs, with their indentation thrown away and
/// trailing spaces added, and the claim is the one P4 actually makes: whatever
/// the layout was, formatting lands on the same file.
#[test]
fn any_layout_of_a_real_program_formats_back_to_the_canonical_one() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no `.deed` files found under {root:?}");

    for path in &files {
        let canonical = std::fs::read_to_string(path).unwrap();
        let mangled = mangle(&canonical);

        // Without this the whole test passes on a mangle that does nothing,
        // which is what it would become if the corpus were ever flat.
        assert_ne!(
            mangled,
            canonical,
            "{} came back unchanged, so nothing was tested",
            path.display()
        );

        let mut sources = SourceMap::new();
        let file = sources.add(path.display().to_string(), mangled.clone());
        match format(file, &mangled) {
            Ok(formatted) => assert_eq!(
                formatted,
                canonical,
                "{} did not come back to its canonical form",
                path.display()
            ),
            Err(diagnostics) => panic!(
                "{} stopped parsing when its layout changed: {}",
                path.display(),
                diagnostics
                    .iter()
                    .map(|d| d.message.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// A block's closing brace sits under the line that opened it.
///
/// Nothing ever said so, and a call that broke over several lines did not do
/// it. The arguments were rendered at the indent of the line that could not
/// hold them and then moved in by putting spaces in front, which moves the
/// first line and nothing after it, so `map(xs, |n: Int| { ... })` came out
/// with the closure's body and its closing brace a level short of where they
/// belong. `examples/using_list.deed` had two of those checked in.
///
/// Neither test above could see it. Both ask whether formatting lands on the
/// file that is there, and the file that was there was the wrong one. This is
/// the first thing here that asks what the output looks like rather than what
/// it is equal to.
#[test]
fn a_closing_brace_sits_under_the_line_that_opened_it() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no `.deed` files found under {root:?}");

    for path in &files {
        let source = std::fs::read_to_string(path).unwrap();
        let mut opened: Vec<(usize, usize)> = Vec::new();

        for (number, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            let mut leading = trimmed.starts_with('}');

            for brace in braces(line) {
                if brace == '{' {
                    opened.push((number + 1, indent));
                    leading = false;
                    continue;
                }
                let (at, was) = opened.pop().unwrap_or_else(|| {
                    panic!(
                        "{}:{} closes a block nothing opened",
                        path.display(),
                        number + 1
                    )
                });
                if leading {
                    assert_eq!(
                        indent,
                        was,
                        "{}:{} closes the block opened on line {at}, so it belongs at {was} \
                         spaces and is at {indent}",
                        path.display(),
                        number + 1
                    );
                }
                leading = false;
            }
        }

        assert!(
            opened.is_empty(),
            "{} leaves a block open at line {}",
            path.display(),
            opened[0].0
        );
    }
}

/// The braces on a line, with the ones inside text and comments left out.
fn braces(line: &str) -> Vec<char> {
    let mut found = Vec::new();
    let mut chars = line.chars().peekable();
    let mut in_string = false;

    while let Some(c) = chars.next() {
        match c {
            '\\' if in_string => {
                chars.next();
            }
            '"' => in_string = !in_string,
            '/' if !in_string && chars.peek() == Some(&'/') => break,
            '{' | '}' if !in_string => found.push(c),
            _ => {}
        }
    }
    found
}

/// The same program, laid out by somebody who was not paying attention.
///
/// Indentation and trailing space only. A line break is not free in this
/// language, since it is what ends a statement, so joining lines would be
/// writing a different program rather than the same one badly.
fn mangle(source: &str) -> String {
    let mut out = String::new();
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // A run of blank lines is one paragraph break however long it is,
            // so doubling them is the same program with more air in it.
            out.push('\n');
        } else {
            // Varying so that it is never accidentally the right indent.
            for _ in 0..(index % 5) * 3 {
                out.push(' ');
            }
            out.push_str(trimmed);
            out.push_str("  ");
        }
        out.push('\n');
    }
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            // Build output and version control are not source.
            if name == "target" || name == ".git" {
                continue;
            }
            collect(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "deed") {
            out.push(path);
        }
    }
}
