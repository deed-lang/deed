//! Every Deed snippet in `how-to/` checks, and the indexes point at the pages.

use std::path::{Path, PathBuf};

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_driver::check_all;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Expectation {
    Checks,
    Fails,
}

#[derive(Clone, Debug)]
struct Snippet {
    page: String,
    name: String,
    expectation: Expectation,
    source: String,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn how_to_dir() -> PathBuf {
    root().join("how-to")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|_| panic!("{} should be there", path.display()))
}

fn pages() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(how_to_dir())
        .expect("how-to/ should be there")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect();
    found.sort();
    assert!(!found.is_empty(), "how-to/ has no markdown pages");
    found
}

fn snippets_in(page: &Path) -> Vec<Snippet> {
    let text = read(page);
    let page_name = page
        .file_name()
        .and_then(|name| name.to_str())
        .expect("a how-to page should have a file name")
        .to_string();
    let mut found = Vec::new();
    let mut lines = text.lines().enumerate();
    let mut last_nonempty = "";

    while let Some((line_no, line)) = lines.next() {
        let Some(info) = line.strip_prefix("```") else {
            if !line.trim().is_empty() {
                last_nonempty = line;
            }
            continue;
        };
        let mut words = info.split_whitespace();
        if words.next() != Some("deed") {
            if !line.trim().is_empty() {
                last_nonempty = line;
            }
            continue;
        }

        assert!(
            last_nonempty.contains("deed-lang.github.io"),
            "{page_name}:{line_no}: every how-to snippet should have a playground link immediately above it"
        );

        let name = words.next().unwrap_or_else(|| {
            panic!(
                "{page_name}: deed fence on line {} should name the snippet",
                line_no + 1
            )
        });
        let expectation = if words.any(|word| word == "fails") {
            Expectation::Fails
        } else {
            Expectation::Checks
        };

        let mut source = String::new();
        let mut closed = false;
        for (_, body) in lines.by_ref() {
            if body == "```" {
                closed = true;
                break;
            }
            source.push_str(body);
            source.push('\n');
        }
        assert!(
            closed,
            "{page_name}: snippet `{name}` is missing a closing fence"
        );

        found.push(Snippet {
            page: page_name.clone(),
            name: name.to_string(),
            expectation,
            source,
        });

        last_nonempty = "```";
    }

    found
}

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

fn files_of(snippet: &Snippet) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut current_name = None::<String>;
    let mut current = String::new();
    let mut saw_markers = false;

    for line in snippet.source.lines() {
        if let Some(path) = line.strip_prefix("// file: ") {
            saw_markers = true;
            if let Some(name) = current_name.replace(path.trim().to_string()) {
                found.push((name, std::mem::take(&mut current)));
            }
            continue;
        }

        current.push_str(line);
        current.push('\n');
    }

    if saw_markers {
        let name = current_name.unwrap_or_else(|| {
            panic!(
                "{}:{} starts a multi-file snippet but names no first file",
                snippet.page, snippet.name
            )
        });
        found.push((name, current));
    } else {
        found.push((
            format!("how-to/{}.deed", snippet.name),
            snippet.source.clone(),
        ));
    }

    assert!(
        found.iter().all(|(_, source)| !source.trim().is_empty()),
        "{}:{} contains an empty virtual file",
        snippet.page,
        snippet.name
    );
    found
}

#[test]
fn every_how_to_page_has_a_checked_deed_snippet() {
    for page in pages() {
        let name = page
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a how-to page should have a file name");
        if name == "README.md" {
            continue;
        }
        assert!(
            !snippets_in(&page).is_empty(),
            "{name} should contain at least one deed snippet"
        );
    }
}

#[test]
fn every_how_to_snippet_checks_or_fails_as_marked() {
    for page in pages() {
        for snippet in snippets_in(&page) {
            let mut sources = SourceMap::new();
            let files = files_of(&snippet);
            let subject = files.len();
            let mut ids = Vec::new();
            for (name, source) in files {
                ids.push(sources.add(name, source));
            }
            for module in deed_driver::shipped_modules() {
                let text = deed_driver::shipped_source(module)
                    .expect("a shipped module should have a source");
                ids.push(sources.add(format!("<shipped>/{module}.deed"), text.to_string()));
            }

            let checks = check_all(&sources, &ids);
            let subject_checks = &checks[..subject];

            match snippet.expectation {
                Expectation::Checks => {
                    for checked in subject_checks {
                        assert!(
                            !checked.has_errors(),
                            "{}:{} should check cleanly:\n{}",
                            snippet.page,
                            snippet.name,
                            rendered(&sources, &checked.diagnostics)
                        );
                    }
                }
                Expectation::Fails => {
                    assert!(
                        subject_checks.iter().any(|checked| checked.has_errors()),
                        "{}:{} is marked `fails` but checked cleanly",
                        snippet.page,
                        snippet.name
                    );
                }
            }
        }
    }
}

#[test]
fn the_how_to_index_links_to_every_page() {
    let index = read(&how_to_dir().join("README.md"));
    for page in pages() {
        let name = page
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a how-to page should have a file name");
        if name == "README.md" {
            continue;
        }
        assert!(
            index.contains(&format!("({name})")),
            "how-to/README.md should link to {name}"
        );
    }
}

#[test]
fn the_readme_links_to_the_how_to_index() {
    let readme = read(&root().join("README.md"));
    assert!(
        readme.contains("(how-to/README.md)"),
        "README.md should link to how-to/README.md"
    );
}
