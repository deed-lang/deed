//! The counts and lists the documents state, checked against what is there.
//!
//! Five sentences in the design documents were wrong at once when this was
//! written, and every one of them had been right at the commit that wrote it:
//! the prelude grew, `Io` grew from six operations to ten, `examples/` grew
//! from eleven files to eighteen, a construct that used to be fiction started
//! parsing, and a walk the README describes in the present tense stopped
//! existing. Prose does not fail a build, so nothing noticed.
//!
//! What is checkable here is narrow on purpose: a list of names the compiler
//! also holds, or a number the repository can count. An argument is not
//! checkable and is not attempted. The point is that the claims which go stale
//! are almost always the countable ones, because those are the claims a later
//! change quietly falsifies without touching the sentence.

use std::path::{Path, PathBuf};

use deed_diagnostics::SourceMap;
use deed_interp::{Program, PropertyConfig, run_properties, run_tests};
use deed_resolve::{IO_OPERATIONS, PRELUDE};

/// The repository root, from this crate's manifest.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(relative: &str) -> String {
    let path = root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} should be there", path.display()))
}

/// The text between two markers, so a check names the paragraph it is about.
fn between<'a>(text: &'a str, from: &str, to: &str) -> &'a str {
    let start = text
        .find(from)
        .unwrap_or_else(|| panic!("the sentence starting {from:?} has moved or been rewritten"));
    let rest = &text[start..];
    let end = rest
        .find(to)
        .unwrap_or_else(|| panic!("the sentence starting {from:?} no longer reaches {to:?}"));
    &rest[..end]
}

/// Every `` `name` `` in a stretch of prose, in the order it is written.
fn backticked(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('`') else { break };
        found.push(rest[..close].to_string());
        rest = &rest[close + 1..];
    }
    found
}

/// How the documents write a small number, because they write them as words.
///
/// A table rather than a crate, and it stops where the documents stop. A count
/// that outgrows it should be read as a sign that whatever is being counted is
/// no longer the kind of thing to state in a sentence.
fn spelled(n: usize) -> &'static str {
    const WORDS: [&str; 21] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    WORDS
        .get(n)
        .unwrap_or_else(|| panic!("{n} is past what these documents spell out"))
}

fn examples() -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(root().join("examples"))
        .expect("examples/ should be there")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "deed"))
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no .deed files found under examples/");
    found
}

/// The one paragraph that writes out the whole prelude, in order.
///
/// It carries the names, the built-in effect, its operations and the second
/// effect, so one comparison covers every list a reader could be misled by.
#[test]
fn the_prelude_the_syntax_document_lists_is_the_prelude_there_is() {
    let syntax = read("design/02-syntax.md");
    let paragraph = between(&syntax, "**The prelude is ", "Everything else");

    let mut expected: Vec<String> = PRELUDE.iter().map(|name| name.to_string()).collect();
    expected.push("Io".to_string());
    expected.extend(IO_OPERATIONS.iter().map(|name| name.to_string()));
    expected.push("Diverge".to_string());

    assert_eq!(backticked(paragraph), expected);
}

#[test]
fn the_number_of_prelude_names_is_the_number_it_says() {
    let syntax = read("design/02-syntax.md");
    let claim = format!(
        "**The prelude is {} names and two effects:**",
        spelled(PRELUDE.len())
    );
    assert!(
        syntax.contains(&claim),
        "the prelude has {} names, so the sentence should read {claim:?}",
        PRELUDE.len()
    );
}

/// The capability document lists the operations a second time, which is a
/// second place to go stale and did.
#[test]
fn the_operations_the_capability_document_lists_are_the_ones_there_are() {
    let capabilities = read("design/04-capabilities.md");
    let sentence = between(
        &capabilities,
        "What exists is one built-in\neffect,",
        "and a `System` carrying",
    );

    let mut expected = vec!["Io".to_string()];
    expected.extend(IO_OPERATIONS.iter().map(|name| name.to_string()));
    assert_eq!(backticked(sentence), expected);
}

#[test]
fn the_odd_operation_is_odd_among_however_many_there_are() {
    let capabilities = read("design/04-capabilities.md");
    let claim = format!("the odd operation of the {}", spelled(IO_OPERATIONS.len()));
    assert!(
        capabilities.contains(&claim),
        "there are {} operations, so the sentence should read {claim:?}",
        IO_OPERATIONS.len()
    );
}

#[test]
fn the_examples_the_readme_names_are_the_examples_there_are() {
    let readme = read("README.md");
    let named: Vec<String> = {
        let mut found: Vec<String> = Vec::new();
        let mut rest = readme.as_str();
        while let Some(at) = rest.find("examples/") {
            rest = &rest[at + "examples/".len()..];
            let end = rest
                .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
                .unwrap_or(rest.len());
            let name = &rest[..end];
            if name.ends_with(".deed") && !found.iter().any(|seen| seen == name) {
                found.push(name.to_string());
            }
        }
        found.sort();
        found
    };

    assert_eq!(
        named,
        examples(),
        "the README should name every example and no others"
    );
}

#[test]
fn the_files_the_timing_paragraph_counted_are_the_files_there_are() {
    let principles = read("design/01-principles.md");
    let claim = format!(
        "checking the {} files in `examples/`",
        spelled(examples().len())
    );
    assert!(
        principles.contains(&claim),
        "there are {} examples, so the sentence should read {claim:?}",
        examples().len()
    );
}

/// The one place the README writes out what a corpus file contains. It is the
/// claim the file makes about itself, so it should say all of it.
#[test]
fn the_library_the_readme_lists_is_the_library_that_is_written() {
    let readme = read("README.md");
    let sentence = between(
        &readme,
        "list library written in Deed:",
        "none of them known",
    );

    let written: Vec<String> = read("examples/list.deed")
        .lines()
        .filter_map(|line| line.strip_prefix("fn "))
        .map(|rest| {
            rest.chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect()
        })
        .collect();

    assert!(!written.is_empty(), "examples/list.deed declares nothing");
    assert_eq!(backticked(sentence), written);
}

/// The paragraph whose whole job is to say which parts of the document are
/// fiction. It said four and listed three, because generic application was
/// deleted from the list when it started parsing and the number was not.
#[test]
fn the_constructs_that_do_not_parse_are_counted_where_they_are_listed() {
    let syntax = read("design/02-syntax.md");
    let after = syntax
        .split_once("constructs appear in the illustrations and do not parse at\nall")
        .expect("the paragraph naming what does not parse has moved")
        .1;
    let listed = after
        .lines()
        .skip_while(|line| !line.starts_with("- "))
        .take_while(|line| line.starts_with("- "))
        .count();

    let claim = format!("{} constructs appear in the illustrations", spelled(listed));
    let claim = claim[..1].to_uppercase() + &claim[1..];
    assert!(
        syntax.contains(&claim),
        "{listed} are listed, so the sentence should read {claim:?}"
    );
}

/// The design documents are read as a set, so a missing one would make every
/// check above pass by having nothing to disagree with.
#[test]
fn the_documents_these_read_are_all_there() {
    for name in [
        "design/00-motivation.md",
        "design/01-principles.md",
        "design/02-syntax.md",
        "design/03-effects.md",
        "design/04-capabilities.md",
        "README.md",
    ] {
        let path: &Path = &root().join(name);
        assert!(path.is_file(), "{name} should be there");
    }
}

/// Every crate under `crates/`, by directory name.
fn crates() -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(root().join("crates"))
        .expect("crates/ should be there")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.join("Cargo.toml").is_file())
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no crates found under crates/");
    found
}

/// Whether anything under one crate's `src/` contains any of `needles`.
fn crate_mentions(name: &str, needles: &[&str]) -> bool {
    fn walk(at: &Path, needles: &[&str]) -> bool {
        let Ok(entries) = std::fs::read_dir(at) else {
            return false;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                if walk(&path, needles) {
                    return true;
                }
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && let Ok(text) = std::fs::read_to_string(&path)
                && needles.iter().any(|needle| text.contains(needle))
            {
                return true;
            }
        }
        false
    }

    walk(&root().join("crates").join(name).join("src"), needles)
}

/// The two counts the answer about `Result` and `List` rests on.
///
/// The whole argument for leaving them built in is that the type is named in
/// one crate and the syntax over it is named in most of them, so if that stops
/// being true the answer is wrong and nothing else would say so. This is the
/// one claim in these documents where a refactor moving a match arm is exactly
/// the event that should make somebody read the paragraph again.
#[test]
fn what_holds_result_in_the_language_is_counted_where_it_is_claimed() {
    let all = crates();

    let types: Vec<&String> = all
        .iter()
        .filter(|name| crate_mentions(name, &["Ty::Result", "Ty::List"]))
        .collect();
    assert_eq!(
        types.len(),
        1,
        "the paragraph says the type is named in one crate, and it is named in {types:?}"
    );

    // `?` and the outcome an `ensures` clause is keyed by. Both are syntax
    // over `Result` rather than the type, which is the point being made.
    let syntax: Vec<&String> = all
        .iter()
        .filter(|name| crate_mentions(name, &["Expr::Try", "Outcome::"]))
        .collect();

    let syntax_doc = read("design/02-syntax.md");
    let claim = format!(
        "those are in {} of the {} crates",
        spelled(syntax.len()),
        spelled(all.len())
    );
    assert!(
        syntax_doc.contains(&claim),
        "{} crates name `?` or an outcome ({syntax:?}) out of {}, so the sentence should read {claim:?}",
        syntax.len(),
        all.len()
    );
}

/// The number in the README's own transcript of `deed test examples/`.
///
/// The transcript is the first thing anybody sees and it is the claim that is
/// easiest to leave behind, because every change that adds a test to the
/// corpus falsifies it and none of them has any reason to look. It said 84
/// for the eighteen tests it took to notice.
#[test]
fn the_readme_reports_the_number_of_tests_the_corpus_runs() {
    let mut sources = SourceMap::new();
    let mut ids = Vec::new();
    for name in examples() {
        let path = root().join("examples").join(&name);
        let text = std::fs::read_to_string(&path).expect("an example should be readable");
        ids.push(sources.add(format!("examples/{name}"), text));
    }

    // Together, because they import each other, and every file runs only its
    // own tests, which is what `deed test examples/` does.
    let checks = deed_driver::check_all(&sources, &ids);
    let mut program = Program::new();
    for checked in &checks {
        assert!(
            !checked.has_errors(),
            "{} should check cleanly",
            sources.file(checked.file).name()
        );
        program.add(
            checked.file,
            &checked.module,
            &checked.resolutions,
            checked.guards(),
            checked.rows(),
        );
    }

    let mut passed = 0;
    for checked in &checks {
        for outcome in run_tests(&program, checked.file) {
            assert!(
                outcome.failure.is_none(),
                "`{}` should pass in `{}`",
                outcome.name,
                sources.file(checked.file).name()
            );
            passed += 1;
        }
        // A property is a line in that transcript too, so it is one of the
        // numbers being claimed.
        for property in run_properties(
            &program,
            checked.file,
            &checked.module,
            &checked.resolutions,
            PropertyConfig::default(),
        ) {
            assert!(
                property.failure.is_none(),
                "`{}` should hold in `{}`",
                property.function,
                sources.file(checked.file).name()
            );
            passed += 1;
        }
    }

    assert!(passed > 0, "the corpus ran no tests at all");
    let claim = format!("{passed} passed, 0 failed");
    assert!(
        read("README.md").contains(&claim),
        "the corpus runs {passed} tests, so the transcript should read {claim:?}"
    );
}
