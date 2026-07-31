//! Every Deed snippet in TUTORIAL.md is checked, and the final one runs.

use std::path::{Path, PathBuf};

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_driver::check_text;
use deed_interp::{Program, run_main};
use deed_typeck::Tier;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Expectation {
    Checks,
    Fails,
}

#[derive(Clone, Debug)]
struct Snippet {
    name: String,
    expectation: Expectation,
    source: String,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn tutorial_path() -> PathBuf {
    root().join("TUTORIAL.md")
}

fn tutorial() -> String {
    std::fs::read_to_string(tutorial_path()).expect("TUTORIAL.md should be there")
}

fn snippets() -> Vec<Snippet> {
    let text = tutorial();
    let mut found = Vec::new();
    let mut lines = text.lines().enumerate();

    while let Some((line_no, line)) = lines.next() {
        let Some(info) = line.strip_prefix("```") else {
            continue;
        };
        let mut words = info.split_whitespace();
        if words.next() != Some("deed") {
            continue;
        }

        let name = words.next().unwrap_or_else(|| {
            panic!("deed fence on line {} should name the snippet", line_no + 1)
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
        assert!(closed, "snippet `{name}` is missing a closing fence");

        found.push(Snippet {
            name: name.to_string(),
            expectation,
            source,
        });
    }

    assert!(!found.is_empty(), "TUTORIAL.md declares no deed snippets");
    found
}

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

fn checked(name: &str) -> deed_driver::Checked {
    let snippet = snippets()
        .into_iter()
        .find(|snippet| snippet.name == name)
        .unwrap_or_else(|| panic!("no snippet named `{name}` in TUTORIAL.md"));

    let mut sources = SourceMap::new();
    let checked = check_text(
        &mut sources,
        &format!("tutorial/{name}.deed"),
        &snippet.source,
    );
    assert!(
        !checked.has_errors(),
        "snippet `{name}` should check cleanly:\n{}",
        rendered(&sources, &checked.diagnostics)
    );
    checked
}

#[test]
fn every_tutorial_snippet_checks_or_fails_as_marked() {
    for snippet in snippets() {
        let mut sources = SourceMap::new();
        let checked = check_text(
            &mut sources,
            &format!("tutorial/{}.deed", snippet.name),
            &snippet.source,
        );

        match snippet.expectation {
            Expectation::Checks => assert!(
                !checked.has_errors(),
                "snippet `{}` should check cleanly:\n{}",
                snippet.name,
                rendered(&sources, &checked.diagnostics)
            ),
            Expectation::Fails => assert!(
                checked.has_errors(),
                "snippet `{}` is marked `fails` but checked cleanly",
                snippet.name
            ),
        }
    }
}

#[test]
fn the_final_tutorial_program_runs() {
    let checked = checked("final-program");

    let mut program = Program::new();
    program.add(
        checked.file,
        &checked.module,
        &checked.resolutions,
        checked.guards(),
        checked.rows(),
    );

    let run = run_main(&program, checked.file, Path::new(""), &[]).expect("there should be a main");
    assert!(run.result.is_ok(), "final program should run successfully");
    assert_eq!(run.output, vec!["server on 8080".to_string()]);
}

#[test]
fn the_final_tutorial_program_shows_all_three_tiers() {
    let checked = checked("final-program");

    let proven = checked
        .obligations
        .iter()
        .filter(|obligation| obligation.tier == Tier::Proven)
        .count();
    let tested = checked
        .obligations
        .iter()
        .filter(|obligation| obligation.tier == Tier::Tested)
        .count();
    let guarded = checked
        .obligations
        .iter()
        .filter(|obligation| obligation.tier == Tier::Guarded)
        .count();

    assert!(proven > 0, "the final snippet should include a proven obligation");
    assert!(tested > 0, "the final snippet should include a tested obligation");
    assert!(guarded > 0, "the final snippet should include a guarded obligation");
}
