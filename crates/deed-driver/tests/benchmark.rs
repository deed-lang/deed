//! The benchmark, held against itself.
//!
//! A scorer nobody has tried to fool says yes to everything. These require the
//! references to score full marks, and require two deliberately wrong answers
//! to score less: one that does not check, and one that checks and gets the
//! answer wrong. Without the second, the whole thing could be a compile check
//! wearing a benchmark's name.

#[path = "../examples/agent_bench.rs"]
#[allow(dead_code)]
mod bench;

use std::fs;

/// Every task carries the three files the harness and a reader both need.
#[test]
fn every_task_has_a_prompt_its_own_tests_and_an_answer() {
    let tasks = bench::tasks();
    assert!(
        tasks.len() >= 5,
        "a benchmark of {} tasks is a sample, not a measurement",
        tasks.len()
    );

    for task in &tasks {
        for path in [
            bench::prompt(task),
            bench::checks(task),
            bench::reference(task),
        ] {
            assert!(path.is_file(), "{task} is missing {}", path.display());
        }
    }
}

/// The tests belong to the task, and they have to actually reach the answer.
///
/// A `checks.deed` that imported nothing would pass against any answer at all,
/// including an empty file, which is this repository's oldest way of passing
/// for free.
#[test]
fn every_task_tests_the_module_the_prompt_asks_for() {
    for task in bench::tasks() {
        let checks = fs::read_to_string(bench::checks(&task)).expect("the checks are readable");
        assert!(
            checks.contains(&format!("use bench/{task}.")),
            "{task}'s checks never import `bench/{task}`"
        );
        assert!(
            checks.contains("test \""),
            "{task}'s checks declare no tests"
        );

        let prompt = fs::read_to_string(bench::prompt(&task)).expect("the prompt is readable");
        assert!(
            prompt.contains(&format!("bench/{task}")),
            "{task}'s prompt never names the module it wants"
        );
    }
}

/// The reference is one answer that works, so it should score full marks.
///
/// This is the harness testing itself: a scorer that cannot pass a known good
/// answer is measuring something other than what it says.
#[test]
fn every_reference_checks_and_passes_the_tests_it_is_given() {
    for task in bench::tasks() {
        let source =
            fs::read_to_string(bench::reference(&task)).expect("the reference is readable");
        let score = bench::score(&task, &source);

        assert!(score.checks, "the reference for {task} does not check");
        assert!(
            score.tests_run > 0,
            "{task} ran no tests, so passing them says nothing"
        );
        assert!(
            score.tests_pass,
            "the reference for {task} passed {}/{} of its own tests",
            score.tests_passed, score.tests_run
        );
    }
}

/// An answer that does not compile scores nothing, and says so rather than
/// being reported as zero tests out of zero.
#[test]
fn an_answer_that_does_not_check_is_not_a_pass() {
    for task in bench::tasks() {
        let score = bench::score(
            &task,
            &format!("module bench/{task}\n\nfn nothing() -> Int {{\n    missing\n}}\n"),
        );

        assert!(!score.checks, "{task} accepted an answer naming nothing");
        assert!(
            !score.tests_pass,
            "{task} passed an answer that does not check"
        );
    }
}

/// The one that matters: an answer that compiles and is wrong has to fail.
///
/// Every function the task asks for is here with the right signature and a
/// body that gives back something else, so nothing but actually running the
/// tests can tell it apart from the reference.
#[test]
fn an_answer_that_compiles_and_is_wrong_is_not_a_pass() {
    let wrong = [
        (
            "twice",
            "module bench/twice\n\nfn twice(n: Int) -> Int {\n    n\n}\n",
        ),
        (
            "total",
            "module bench/total\n\nfn total(numbers: List<Int>) -> Int {\n    0\n}\n\n\
             fn largest(numbers: List<Int>, fallback: Int) -> Int {\n    fallback\n}\n",
        ),
        (
            "grade",
            "module bench/grade\n\nchoice Grade {\n    Low,\n    Middling,\n    High,\n}\n\n\
             fn grade(score: Int) -> Grade {\n    Low\n}\n\n\
             fn describe(mark: Grade) -> String {\n    match mark {\n        Low => \"low\",\n        \
             Middling => \"middling\",\n        High => \"high\",\n    }\n}\n",
        ),
        (
            "split_evenly",
            "module bench/split_evenly\n\n\
             fn split_evenly(amount: Int, people: Int) -> Result<Int, String> {\n    ok(0)\n}\n",
        ),
        (
            "audit",
            "module bench/audit\n\neffect Audit {\n    fn note(entry: String) -> ()\n    \
             fn so_far() -> String\n}\n\n\
             handler Collected implements Audit {\n    state entries: List<String>\n\n    \
             fn note(entry) -> () {\n        entries = entries\n    }\n\n    \
             fn so_far() -> String {\n        join(entries, \", \")\n    }\n}\n\n\
             fn record_all(entries: List<String>) -> ()\n  uses\n    Audit.note,\n{\n    \
             for entry in entries {\n        Audit.note(entry)\n    }\n}\n\n\
             fn collected(entries: List<String>) -> String {\n    \
             with Collected { entries: [] } {\n        record_all(entries)\n        \
             Audit.so_far()\n    }\n}\n",
        ),
        (
            "stock",
            "module bench/stock\n\ntype InStock = Int where value > 0\n\n\
             fn take_one(count: InStock) -> Int {\n    count\n}\n\n\
             fn restock(count: Int, delivered: Int) -> InStock\n  where\n    count > 0,\n    \
             delivered >= 0,\n    count < 1000000000,\n    delivered < 1000000000,\n{\n    \
             count + delivered\n}\n",
        ),
    ];

    // Every task is covered, so adding one without a wrong answer for it
    // fails here rather than quietly narrowing what this test holds.
    let mut covered: Vec<&str> = wrong.iter().map(|(task, _)| *task).collect();
    covered.sort_unstable();
    let mut all = bench::tasks();
    all.sort();
    assert_eq!(covered, all, "a task has no wrong answer to check against");

    for (task, source) in wrong {
        let score = bench::score(task, source);
        assert!(
            score.checks,
            "the wrong answer for {task} does not even check, so it tests nothing"
        );
        assert!(
            !score.tests_pass,
            "{task} passed an answer that compiles and gives the wrong result"
        );
    }
}

/// A prompt that contains the answer measures nothing.
///
/// Bodies rather than signatures: a prompt is supposed to name the signatures,
/// and it is the lines inside the braces that would give it away.
#[test]
fn no_prompt_hands_over_the_body_of_its_own_answer() {
    for task in bench::tasks() {
        let prompt = fs::read_to_string(bench::prompt(&task)).expect("the prompt is readable");
        let reference =
            fs::read_to_string(bench::reference(&task)).expect("the reference is readable");

        for line in reference.lines() {
            let line = line.trim();
            // Signatures, declarations and punctuation are meant to be shared.
            let structural = line.is_empty()
                || line.len() < 12
                || line.starts_with("module ")
                || line.starts_with("fn ")
                || line.starts_with("type ")
                || line.starts_with("choice ")
                || line.starts_with("record ")
                || line.starts_with("effect ")
                || line.starts_with("handler ")
                || line.starts_with("state ")
                || line.starts_with("uses")
                || line.starts_with("//");
            if structural {
                continue;
            }

            assert!(
                !prompt.contains(line),
                "{task}'s prompt gives away a line of the answer: {line:?}"
            );
        }
    }
}

/// The task set covers the language rather than one corner of it.
///
/// Not a count: a benchmark of six arithmetic tasks would pass a count and say
/// nothing about whether anything else can be written here.
#[test]
fn the_tasks_between_them_reach_the_features_the_language_is_about() {
    let mut seen = String::new();
    for task in bench::tasks() {
        seen.push_str(&fs::read_to_string(bench::reference(&task)).expect("readable"));
    }

    for (feature, spelling) in [
        ("a contract", "ensures"),
        ("a precondition", "where"),
        ("a refinement", "type InStock"),
        ("a list walk", "for "),
        ("a choice and a match", "match "),
        ("a record", "record "),
        ("an effect", "effect "),
        ("a handler", "handler "),
        ("an installed handler", "with "),
        ("Result", "err("),
    ] {
        assert!(
            seen.contains(spelling),
            "no task reaches {feature}, so nothing here measures whether it can be written"
        );
    }
}
