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

/// Every task asks for a module and nothing else, and the README says what
/// that costs.
///
/// The rule itself is deliberate: the answer is judged by tests it never saw,
/// so it cannot pass by writing tests it already satisfies. The cost is that
/// an answer with no `main` in it gives `deed_run` nothing to do, and a
/// transcript that never calls it is therefore not evidence about it.
///
/// I read one as evidence anyway and filed #833 off it. A task that asks for
/// tests would make that reading right and this paragraph wrong, so the two
/// are held together here.
///
/// `deed_test` is the one that is not costed, and the README has to keep
/// saying so: an answer with no tests still carries a contract, and #828 made
/// the agent surface run the properties a contract generates. Three of the
/// eight `deed_test` calls in `RESULTS.md` came back with one.
#[test]
fn the_readme_says_what_asking_for_no_tests_costs() {
    for task in bench::tasks() {
        let prompt = fs::read_to_string(bench::prompt(&task)).expect("the prompt is readable");
        assert!(
            prompt.contains("no tests"),
            "{task}'s prompt asks for tests, so the README's paragraph about \
             the tools that cannot be reached is now wrong"
        );
    }

    let readme = fs::read_to_string(root().join("benchmarks").join("README.md"))
        .expect("the README is readable");
    assert!(
        readme.contains("deed_run"),
        "the README never says a task cannot reach `deed_run`"
    );
    assert!(
        readme.contains("deed_check"),
        "the README never says what a transcript here is evidence about"
    );
    assert!(
        readme.contains("deed_test") && readme.contains("property"),
        "the README no longer says that a contract gives `deed_test` something \
         to run, which is the half of this that is not a cost"
    );

    // The claim above is about this compiler, not about that run: a surface
    // that stopped generating properties would leave the paragraph standing.
    let reference = fs::read_to_string(bench::reference("twice")).expect("readable");
    let score = bench::score("twice", &reference);
    assert!(
        score.checks && score.tests_run > 0,
        "the contract task no longer produces tests, so the README's paragraph \
         about properties is now wrong"
    );
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

/// "Anything else, and nothing further is measured" is what the README says
/// the `checks` column means, and this is that sentence.
///
/// A rejected file still has obligations, and the checker still puts them in
/// tiers, so the scorer used to report them. Four blind runs came back with
/// `0 check` and a proven obligation printed beside it, which reads as the arm
/// that wrote nothing compilable proving as much as the arm that did.
///
/// The answer below is the one those runs produced, cut down: a refinement the
/// checker settles from the `where` clause above it, in a file the parser
/// rejects. The tier is real and it is about a program that does not exist.
#[test]
fn a_rejected_answer_measures_nothing_beyond_the_first_column() {
    let source = "module bench/stock\n\n\
         type InStock = Int where value > 0\n\n\
         export fn restock(count: Int, delivered: Int) -> InStock\n  \
         where count + delivered > 0\n  count + delivered\n";

    let rejected = bench::score("stock", source);
    assert!(
        !rejected.checks,
        "this answer was chosen because the compiler rejects it"
    );
    assert_eq!(
        (rejected.proven, rejected.guarded),
        (0, 0),
        "an answer the compiler rejected was scored {} proven and {} guarded",
        rejected.proven,
        rejected.guarded
    );
    assert!(
        rejected.reasons.is_empty(),
        "a rejected answer was given a reason for not proving something"
    );

    // The same shape written so it compiles still counts, or the assertion
    // above would hold just as well for a scorer that never counts anything.
    let accepted = bench::score(
        "stock",
        "module bench/stock\n\ntype InStock = Int where value > 0\n\n\
         fn take_one(count: InStock) -> Int {\n    count - 1\n}\n\n\
         fn restock(count: Int, delivered: Int) -> InStock\n  where\n    \
         count + delivered > 0,\n{\n    count + delivered\n}\n",
    );
    assert!(accepted.checks, "the compiling shape stopped compiling");
    assert!(
        accepted.proven + accepted.guarded > 0,
        "the compiling shape carries no obligation, so the comparison is empty"
    );
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

/// The published record is about this benchmark, and says so by naming it.
///
/// `benchmarks/RESULTS.md` is a record rather than a ratchet: reproducing it
/// needs a model, a network and a key, none of which this repository has. What
/// can be held is that it is still describing the thing it claims to describe.
/// A seventh task, or a renamed one, makes the published numbers a measurement
/// of something else, and that should be loud rather than quiet.
#[test]
fn the_published_result_names_every_task_and_no_others() {
    let text = results();

    for task in bench::tasks() {
        assert!(
            text.contains(&format!("| {task} |")),
            "RESULTS.md has no row for {task}, so its tables are about a different task set"
        );
    }

    // The other direction, or a deleted task would leave its numbers behind.
    let named: Vec<String> = text
        .lines()
        .filter_map(|line| line.strip_prefix("| "))
        .filter_map(|line| line.split_once(" |"))
        .map(|(first, _)| first.trim().to_string())
        .filter(|first| first.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .filter(|first| !first.is_empty() && first != "task")
        .collect();
    for name in &named {
        assert!(
            bench::tasks().contains(name),
            "RESULTS.md has a row for {name}, which is not a task"
        );
    }
    assert!(
        !named.is_empty(),
        "no task rows found in RESULTS.md, so the check above holds nothing"
    );
}

/// Every diagnostic the record quotes is one the compiler still has.
///
/// A record that names `DEED2015` after `DEED2015` has been renamed is a page
/// pointing at nothing, and the reader has no way to tell.
#[test]
fn the_published_result_quotes_codes_that_still_exist() {
    let text = results();

    let declared: String = std::fs::read_dir(root().join("crates"))
        .expect("crates/ is there")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("src").join("codes.rs")).ok())
        .collect();
    assert!(
        declared.contains("DEED"),
        "no codes.rs was read, so the check below holds nothing"
    );

    let mut quoted = 0;
    for (index, _) in text.match_indices("DEED") {
        let code: String = text[index..].chars().take(8).collect();
        if code.len() == 8 && code[4..].chars().all(|c| c.is_ascii_digit()) {
            quoted += 1;
            assert!(
                declared.contains(&code),
                "RESULTS.md quotes {code}, which no crate declares"
            );
        }
    }
    assert!(quoted > 0, "RESULTS.md quotes no diagnostic code");
}

/// The tool table adds up to the total the sentence above it spells out.
///
/// Two ways of saying the same number, in one file, is how a number goes
/// stale: somebody corrects the table and leaves the sentence.
#[test]
fn the_published_tool_counts_add_up_to_the_total_it_claims() {
    let text = results();

    let mut total = 0;
    let mut rows = 0;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("| `deed_") else {
            continue;
        };
        let Some((_, count)) = rest.split_once("` | ") else {
            continue;
        };
        let count = count.trim_end_matches(" |").trim();
        total += count
            .parse::<usize>()
            .unwrap_or_else(|_| panic!("a tool row in RESULTS.md counts {count:?}"));
        rows += 1;
    }

    assert!(rows >= 6, "only {rows} tool rows found in RESULTS.md");
    assert!(
        text.to_lowercase()
            .contains(&format!("{} tool calls", spelled(total))),
        "the tool table adds up to {total}, which RESULTS.md never says in words"
    );
}

/// The record is reachable from the directory it belongs to.
#[test]
fn the_readme_points_at_the_published_result() {
    let readme = std::fs::read_to_string(root().join("benchmarks").join("README.md"))
        .expect("the README is readable");
    assert!(
        readme.contains("RESULTS.md"),
        "nothing in the benchmarks README points at the published result"
    );
}

fn results() -> String {
    std::fs::read_to_string(root().join("benchmarks").join("RESULTS.md"))
        .expect("benchmarks/RESULTS.md is readable")
}

fn root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// English for the two- and three-digit numbers a tool table can add up to.
fn spelled(value: usize) -> String {
    const ONES: [&str; 20] = [
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
    ];
    const TENS: [&str; 10] = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];

    match value {
        0..=19 => ONES[value].to_string(),
        20..=99 => match value % 10 {
            0 => TENS[value / 10].to_string(),
            rest => format!("{}-{}", TENS[value / 10], ONES[rest]),
        },
        100..=999 => match value % 100 {
            0 => format!("{} hundred", ONES[value / 100]),
            rest => format!("{} hundred and {}", ONES[value / 100], spelled(rest)),
        },
        _ => panic!("no spelling for {value}"),
    }
}
