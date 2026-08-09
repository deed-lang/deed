//! What something else wrote, scored against the tasks it was given.
//!
//! Every `.deed` file in this repository was written by one person, and that
//! is the largest open question about the language. `benchmarks/README.md` has
//! the argument; this is the tool.
//!
//! ```text
//! cargo run -p deed-driver --example agent_bench --release -- <answers-directory>
//! ```
//!
//! The directory holds one `<task>.deed` per task in `benchmarks/tasks/`. With
//! no directory given, the references are scored, which is the tool checking
//! itself rather than a useful measurement.
//!
//! No dependency, no framework and no network. Producing the answers is
//! somebody else's job; `deed mcp` is the intended way to let a model produce
//! them against the real compiler.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use deed_diagnostics::SourceMap;
use deed_driver::{check_all, shipped_for, shipped_source};
use deed_interp::{Program, run_tests};
use deed_typeck::Tier;

/// Where the tasks live, relative to this crate.
const TASKS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/tasks");

fn main() {
    let answers = std::env::args().nth(1).map(PathBuf::from);

    let tasks = tasks();
    assert!(!tasks.is_empty(), "no tasks found under {TASKS}");

    println!(
        "{:<16} {:>7} {:>7} {:>7} {:>8}  why not proven",
        "task", "checks", "tests", "proven", "guarded"
    );

    let mut answered = 0;
    let mut checked = 0;
    let mut passed = 0;

    for task in &tasks {
        let source = match &answers {
            Some(directory) => std::fs::read_to_string(directory.join(format!("{task}.deed"))).ok(),
            None => std::fs::read_to_string(reference(task)).ok(),
        };

        let Some(source) = source else {
            println!("{task:<16} {:>7} {:>7} {:>7} {:>8}  -", "-", "-", "-", "-");
            continue;
        };
        answered += 1;

        let score = score(task, &source);
        if score.checks {
            checked += 1;
        }
        if score.tests_pass {
            passed += 1;
        }

        println!(
            "{task:<16} {:>7} {:>7} {:>7} {:>8}  {}",
            yes(score.checks),
            measured(
                score.checks,
                format!("{}/{}", score.tests_passed, score.tests_run)
            ),
            measured(score.checks, score.proven.to_string()),
            measured(score.checks, score.guarded.to_string()),
            match score.reasons.is_empty() {
                true => "-".to_string(),
                false => score
                    .reasons
                    .iter()
                    .map(|(reason, count)| format!("{reason} ({count})"))
                    .collect::<Vec<_>>()
                    .join("; "),
            }
        );
    }

    println!();
    println!(
        "{answered}/{} answered, {checked} check, {passed} pass their tests",
        tasks.len()
    );
}

fn yes(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// A zero that was measured, or a dash for one that was not.
fn measured(checks: bool, value: String) -> String {
    if checks { value } else { "-".to_string() }
}

/// What one answer scored.
pub struct Score {
    pub checks: bool,
    pub tests_run: usize,
    pub tests_passed: usize,
    pub tests_pass: bool,
    pub proven: usize,
    pub guarded: usize,
    /// Why the guarded ones were not proven, counted. This is the number the
    /// benchmark exists for: it says whether an author could not express the
    /// contract or whether the checker could not settle it.
    pub reasons: BTreeMap<String, usize>,
}

/// Checks an answer together with the task's own tests, and runs them.
///
/// The tests are added as a second module rather than appended to the answer,
/// so an answer cannot pass by rewriting them.
pub fn score(task: &str, answer: &str) -> Score {
    let mut sources = SourceMap::new();
    let checks_text = std::fs::read_to_string(checks(task)).expect("every task has its checks");

    let module = sources.add(format!("bench/{task}.deed"), answer.to_string());
    let tests = sources.add(format!("bench/{task}_checks.deed"), checks_text.clone());
    let mut ids = vec![module, tests];

    for shipped in shipped_for([answer, checks_text.as_str()]) {
        let text = shipped_source(shipped).expect("a module that ships has a source");
        ids.push(sources.add(format!("{shipped}.deed"), text.to_string()));
    }

    let checked = check_all(&sources, &ids);
    let checks = !checked.iter().any(|one| one.has_errors());

    // Nothing further is measured, which is what the README promises. A file
    // the compiler rejected still carries obligations, and reporting their
    // tiers scores a program that does not exist: four blind runs came back
    // "0 check" with a proven obligation beside it.
    if !checks {
        return Score {
            checks,
            tests_run: 0,
            tests_passed: 0,
            tests_pass: false,
            proven: 0,
            guarded: 0,
            reasons: BTreeMap::new(),
        };
    }

    let mut proven = 0;
    let mut guarded = 0;
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    // Only the answer's own obligations. The tests belong to the task, so
    // counting theirs would score the task rather than the answer.
    for obligation in &checked[0].obligations {
        match obligation.tier {
            Tier::Proven => proven += 1,
            Tier::Guarded => {
                guarded += 1;
                let reason = match obligation.reason {
                    Some(reason) => reason.text().to_string(),
                    None => "no reason given".to_string(),
                };
                *reasons.entry(reason).or_default() += 1;
            }
            _ => {}
        }
    }

    let mut program = Program::new();
    for one in &checked {
        program.add(
            one.file,
            &one.module,
            &one.resolutions,
            one.guards(),
            one.rows(),
            one.operators(),
        );
    }

    let outcomes = run_tests(&program, tests);
    let tests_run = outcomes.len();
    let tests_passed = outcomes
        .iter()
        .filter(|outcome| outcome.failure.is_none())
        .count();

    Score {
        checks,
        tests_run,
        tests_passed,
        tests_pass: tests_run > 0 && tests_run == tests_passed,
        proven,
        guarded,
        reasons,
    }
}

/// Every task directory name, sorted.
pub fn tasks() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(TASKS)
        .expect("the tasks directory is part of the repository")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

pub fn reference(task: &str) -> PathBuf {
    Path::new(TASKS).join(task).join("reference.deed")
}

pub fn checks(task: &str) -> PathBuf {
    Path::new(TASKS).join(task).join("checks.deed")
}

pub fn prompt(task: &str) -> PathBuf {
    Path::new(TASKS).join(task).join("prompt.md")
}
