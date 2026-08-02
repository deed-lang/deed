//! The corpus's own test blocks, run through the compiled backend.
//!
//! `deed test` runs the test blocks through the interpreter.
//! `deed test --compiled` runs the same blocks through the compiled backend.
//! This file is the ratchet for that second path: it records which files have
//! test blocks the compiled backend can run today, and keeps that list from
//! shrinking silently.
//!
//! A test block is lowered into a body function (every assertion except
//! `assert refuses`) and one probe function per `assert refuses` statement.
//! A file whose test blocks the backend can compile appears in `expected`.
//! A file whose test blocks the backend cannot compile yet is silently skipped
//! by `lower_with_tests`, so it never appears.
//!
//! The floor is a count of blocks that ran, not just a count of files:
//! a file might compile but have its test blocks skipped individually.

use std::path::{Path, PathBuf};

use deed_codegen::{Trap, call, compile};
use deed_diagnostics::SourceMap;
use deed_driver::{check_all, shipped_modules, shipped_source};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Every `.deed` file the repository carries, corpus and library both.
fn sources() -> Vec<(String, String)> {
    let mut found = Vec::new();
    for directory in ["examples", "std"] {
        let base = root().join(directory);
        collect(&base, &mut found);
    }
    found.sort();
    assert!(
        found.len() > 20,
        "the corpus should be more than a handful of files, found {}",
        found.len()
    );
    found
}

fn collect(at: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(at) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "deed") {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a file name")
                .to_string();
            out.push((
                name,
                std::fs::read_to_string(&path).expect("a readable file"),
            ));
        }
    }
}

/// Whether a diagnostic code from the compiled backend counts as a contract
/// failure, which is what `assert refuses` expects.
fn is_contract_failure(code: &str) -> bool {
    // DEED6002 = PRECONDITION_FAILED
    // DEED6003 = POSTCONDITION_FAILED  (not yet compiled, included for when it is)
    // DEED6004 = REFINEMENT_FAILED     (not yet compiled, included for when it is)
    matches!(code, "DEED6002" | "DEED6003" | "DEED6004")
}

/// What happened to one file's test blocks.
struct Outcome {
    name: String,
    /// How many blocks ran and passed.
    passed: usize,
    /// Names and failure reasons of blocks that ran and failed.
    failed: Vec<(String, String)>,
}

fn walked() -> Vec<Outcome> {
    let mut outcomes = Vec::new();

    for (name, text) in sources() {
        let mut map = SourceMap::new();
        let mut ids = vec![map.add(name.clone(), text)];

        for module in shipped_modules() {
            if let Some(source) = shipped_source(module) {
                ids.push(map.add(format!("<shipped>/{module}.deed"), source.to_string()));
            }
        }

        let checks = check_all(&map, &ids);
        let subject = &checks[0];
        if subject.has_errors() {
            continue;
        }

        // lower_with_tests silently skips blocks the backend cannot lower.
        let lowered =
            match deed_mir::lower_with_tests(&subject.module, &subject.resolutions, &subject.types)
            {
                Ok(p) => p,
                Err(_) => continue,
            };

        if lowered.tests.is_empty() {
            continue;
        }

        let compiled = match compile(&lowered) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let mut passed = 0usize;
        let mut failed = Vec::new();

        for test in &lowered.tests {
            let label = test.name.clone();
            let mut ok = true;

            // The body must not trap.
            if let Err(trap) = call(&compiled, &test.body, &[]) {
                failed.push((label.clone(), format!("body trapped: {trap}")));
                ok = false;
            }

            // Each `assert refuses` probe must get a contract-failure trap.
            if ok {
                for probe in &test.refuses {
                    match call(&compiled, probe, &[]) {
                        Err(Trap::Failed { code, .. }) if is_contract_failure(&code) => {}
                        Ok(_) => {
                            failed.push((
                                label.clone(),
                                "an `assert refuses` expression did not fail".to_string(),
                            ));
                            ok = false;
                            break;
                        }
                        Err(other) => {
                            failed.push((
                                label.clone(),
                                format!("an `assert refuses` probe trapped unexpectedly: {other}"),
                            ));
                            ok = false;
                            break;
                        }
                    }
                }
            }

            if ok {
                passed += 1;
            }
        }

        outcomes.push(Outcome {
            name,
            passed,
            failed,
        });
    }

    outcomes
}

/// Every test block the compiled backend can run must pass.
///
/// A block that the backend cannot lower is silently skipped. A block that
/// the backend lowers but that then fails is a bug.
#[test]
fn compiled_test_blocks_all_pass() {
    let outcomes = walked();
    assert!(
        !outcomes.is_empty(),
        "no test blocks ran in the compiled backend at all; \
         either the corpus has no compilable test blocks or the walker stopped looking"
    );
    let all_failed: Vec<String> = outcomes
        .iter()
        .flat_map(|o| {
            o.failed
                .iter()
                .map(|(block, reason)| format!("`{}` in `{}`: {}", block, o.name, reason))
        })
        .collect();
    assert!(
        all_failed.is_empty(),
        "test blocks failed in the compiled backend:\n{}",
        all_failed.join("\n")
    );
}

/// Which corpus files have test blocks the compiled backend can run today.
///
/// A floor rather than a target, and named rather than counted: a list says
/// which file stopped running, and a number only says that one did. What
/// it catches is a change that quietly stops running blocks that used to.
///
/// The other direction regresses just as quietly, so this checks it too.
/// Every file whose test blocks the backend runs today must be in this list.
/// A file that starts passing and is not added here passes silently, and
/// six months later nobody remembers whether that was on purpose. Growing
/// this list is the last commit of whatever PR made the file pass.
#[test]
fn compiled_tests_run_in_the_expected_files() {
    let outcomes = walked();
    let mut ran: Vec<&str> = outcomes
        .iter()
        .filter(|o| o.passed > 0 || !o.failed.is_empty())
        .map(|o| o.name.as_str())
        .collect();
    ran.sort_unstable();

    let mut expected = vec!["diverge.deed", "proven.deed"];
    expected.sort_unstable();

    for name in &expected {
        assert!(
            ran.contains(name),
            "`{name}` used to run compiled test blocks and does not any more; \
             the compiled backend now runs blocks in {ran:?}"
        );
    }

    assert_eq!(
        ran, expected,
        "the compiled backend now runs blocks in a file this list does not name; \
         add it to `expected` in the same change that made it run, so the next \
         reader does not have to rediscover which files the backend handles"
    );
}

/// How many test blocks ran in the compiled backend.
///
/// A floor on the count of blocks that actually executed, not just compiled.
/// Keeps a regression where blocks are skipped silently from going unnoticed.
#[test]
fn at_least_one_compiled_test_block_ran() {
    let total: usize = walked().iter().map(|o| o.passed + o.failed.len()).sum();
    assert!(
        total > 0,
        "no test blocks ran in the compiled backend; \
         either the corpus lost its compilable test blocks or the ratchet \
         stopped counting correctly"
    );
}
