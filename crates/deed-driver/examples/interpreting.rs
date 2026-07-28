//! Where a run goes, before anything is compiled.
//!
//! The roadmap asks what shape code generation should take, a bytecode machine
//! or native code. There is a question in front of that one: what a run spends
//! its time on today. Both shapes remove the same thing, which is walking the
//! syntax, and neither of them changes what a value is or what a call has to
//! set up before a body runs. If the walk is not where the time goes then the
//! shape question has nothing to decide between, and a compiler measured
//! against this interpreter would be measuring something else.
//!
//! ```text
//! cargo run -p deed-driver --example interpreting --release
//! ```
//!
//! Four things are printed. What one turn of a walk costs, with one more
//! thing in it each time, so the difference between two rows is what the added
//! thing costs. What one call costs by how many arguments it takes, which is
//! separate because the first table's call rows call different functions and
//! so cannot say what an argument costs on its own. What one `push` costs onto
//! a list of a given length, which is separate because a list is copied rather
//! than extended and the copy was the thing everyone expected to be expensive.
//! And a real program from `examples/`, whole and then a stage at a time,
//! which is the only one of the four that anybody would run.
//!
//! No dependency and no framework, for the same reason as
//! `examples/edit_loop.rs`: a question this coarse does not need one.

use std::time::{Duration, Instant};

use deed_diagnostics::SourceMap;
use deed_driver::check_all;
use deed_interp::{Program, run_tests};

/// How many turns the per-turn table walks. Big enough that the fixed cost of
/// starting a run is lost in it.
const TURNS: usize = 100_000;

/// How many pushes the `push` table does, and how long the list it pushes onto
/// is. The number of pushes is the same in every row so that only the length
/// changes.
const PUSHES: usize = 50_000;
const LENGTHS: [usize; 5] = [0, 16, 64, 256, 1024];

/// How many blocks of [`SAMPLE`] the real program reads.
const BLOCKS: [usize; 4] = [20, 40, 80, 160];

/// How many times to repeat each measurement, so one unlucky run does not
/// decide the answer.
const ROUNDS: usize = 5;

fn main() {
    per_turn();
    println!();
    per_argument();
    println!();
    per_push();
    println!();
    real_program();
    println!();
    notes();
}

// -- one turn of a walk ------------------------------------------------------

/// One thing at a time, against the row it is one thing more than.
///
/// Every row runs the same number of turns, so `added` is what the one extra
/// thing costs, once. The rows are not a ladder: a field read and a call are
/// both measured against the same row, because each of them replaces the same
/// single operand rather than sitting on top of the other.
struct Row {
    name: &'static str,
    /// The loop body, or `None` for the row that does no turns at all.
    body: Option<&'static str>,
    claim: &'static str,
    /// Which row this is one thing more than.
    over: Option<usize>,
}

const ROWS: [Row; 8] = [
    Row {
        name: "setup, no turns",
        body: None,
        claim: "got == turns",
        over: None,
    },
    Row {
        name: "a turn",
        body: Some("sum"),
        claim: "got == 0",
        over: Some(0),
    },
    Row {
        name: "  + an operator",
        body: Some("sum + n"),
        claim: "got == turns",
        over: Some(1),
    },
    Row {
        name: "  + a field read",
        body: Some("sum + here.n"),
        claim: "got == turns",
        over: Some(2),
    },
    Row {
        name: "  + a call taking none",
        body: Some("sum + nothing()"),
        claim: "got == turns",
        over: Some(2),
    },
    Row {
        name: "  + a call",
        body: Some("sum + itself(n)"),
        claim: "got == turns",
        over: Some(2),
    },
    Row {
        name: "  + another argument",
        body: Some("sum + first(n, n)"),
        claim: "got == turns",
        over: Some(5),
    },
    Row {
        name: "  + a contract on it",
        body: Some("sum + guarded(n)"),
        claim: "got == turns",
        over: Some(5),
    },
];

fn per_turn() {
    println!("{TURNS} turns");
    println!("                      total      per turn   added");
    println!("-----------------------------------------------------");

    let mut taken: Vec<Duration> = Vec::new();
    for row in &ROWS {
        let elapsed = time(&[("bench.deed", walk_source(row.body, row.claim))], 0);
        let added = match row.over {
            Some(index) => nanos(elapsed.saturating_sub(taken[index]) / TURNS as u32),
            None => String::new(),
        };

        println!(
            "{:<21} {:<10} {:<10} {added}",
            row.name,
            millis(elapsed),
            nanos(elapsed / TURNS as u32),
        );
        taken.push(elapsed);
    }
}

fn walk_source(body: Option<&str>, claim: &str) -> String {
    let work = match body {
        Some(body) => format!("let got = for n in xs with sum = 0 {{ {body} }}"),
        None => "let got = length(xs)".to_string(),
    };

    format!(
        "module bench

record Holding {{ n: Int }}

fn itself(n: Int) -> Int {{ n }}

fn nothing() -> Int {{ 1 }}

fn first(a: Int, b: Int) -> Int {{ a }}

fn takes0() -> Int {{ 1 }}

fn takes1(a: Int) -> Int {{ 1 }}

fn takes2(a: Int, b: Int) -> Int {{ 1 }}

fn takes3(a: Int, b: Int, c: Int) -> Int {{ 1 }}

fn takes4(a: Int, b: Int, c: Int, d: Int) -> Int {{ 1 }}

fn guarded(n: Int) -> Int
  where
    n > 0,
  ensures
    ok  => result == n,
{{
    n
}}

test \"walking\" {{
    let turns = {TURNS}
    let xs = repeat(1, turns)
    let here = Holding {{ n: 1 }}
    {work}
    assert {claim}
}}
"
    )
}

// -- what an argument costs --------------------------------------------------

/// A call by how many arguments it takes, with the callee held still.
///
/// The table above cannot answer this. Its two call rows call different
/// functions: `nothing()` returns a literal and `itself(n)` returns its
/// parameter, so the step from no arguments to one adds a name read in the
/// callee as well as the argument, and the step from one to two does not add
/// one. That made the first argument look several times more expensive than
/// the second, which is a fact about the two bodies rather than about calls.
///
/// Here every callee is `-> Int { 1 }` and only the arity changes, so each row
/// adds exactly one argument: a name read at the call site, a slot in the
/// argument list and a binding in the callee. Four of them, because one
/// difference is a difference and four is a slope.
///
/// The first two rows are the same trick on a name. `sum + 1` and `sum + n`
/// differ by one name read and nothing else, which is the number the question
/// about giving names slots turns on: a name is two hash lookups today, and
/// what that is worth cannot be argued from the cost of a call that also
/// allocates an argument list and a frame.
const ARGUMENTS: [Row; 7] = [
    Row {
        name: "an operator on a literal",
        body: Some("sum + 1"),
        claim: "got == turns",
        over: None,
    },
    Row {
        name: "  + a name instead",
        body: Some("sum + n"),
        claim: "got == turns",
        over: Some(0),
    },
    Row {
        name: "a call taking nothing",
        body: Some("sum + takes0()"),
        claim: "got == turns",
        over: Some(0),
    },
    Row {
        name: "  + one argument",
        body: Some("sum + takes1(n)"),
        claim: "got == turns",
        over: Some(2),
    },
    Row {
        name: "  + a second",
        body: Some("sum + takes2(n, n)"),
        claim: "got == turns",
        over: Some(3),
    },
    Row {
        name: "  + a third",
        body: Some("sum + takes3(n, n, n)"),
        claim: "got == turns",
        over: Some(4),
    },
    Row {
        name: "  + a fourth",
        body: Some("sum + takes4(n, n, n, n)"),
        claim: "got == turns",
        over: Some(5),
    },
];

fn per_argument() {
    println!("{TURNS} turns, one call in the body, by how many arguments it takes");
    println!("                        total      per turn   added");
    println!("-------------------------------------------------------");

    let mut taken: Vec<Duration> = Vec::new();
    for row in &ARGUMENTS {
        let elapsed = time(&[("bench.deed", walk_source(row.body, row.claim))], 0);
        let added = match row.over {
            Some(index) => nanos(elapsed.saturating_sub(taken[index]) / TURNS as u32),
            None => String::new(),
        };

        println!(
            "{:<23} {:<10} {:<10} {added}",
            row.name,
            millis(elapsed),
            nanos(elapsed / TURNS as u32),
        );
        taken.push(elapsed);
    }

    // One difference between two rows is worth less than the slope across
    // four of them. At this size a single step is inside the noise, and the
    // question here is what an argument costs rather than what the third one
    // in particular did on this run.
    let none = taken[2];
    let four = taken[6];
    println!();
    println!(
        "an argument, averaged over the four: {}",
        nanos(four.saturating_sub(none) / (4 * TURNS as u32))
    );
    println!(
        "a name read, from the first two rows: {}",
        nanos(taken[1].saturating_sub(taken[0]) / TURNS as u32)
    );
}

// -- one push ----------------------------------------------------------------

/// What `push` costs, onto a list of a given length.
///
/// A list is immutable, so `push` hands back a new one, and the new one is a
/// copy. Every row does the same number of pushes and only the length differs,
/// so the row for zero is what the call costs when there is nothing to copy,
/// and everything after it is the copy.
fn per_push() {
    println!("{PUSHES} pushes, onto a list of this length");
    println!("length     total      per push   over an empty one");
    println!("-----------------------------------------------------");

    let mut empty = Duration::ZERO;
    for (index, length) in LENGTHS.into_iter().enumerate() {
        let source = format!(
            "module bench

test \"pushing\" {{
    let turns = {PUSHES}
    let xs = repeat(1, turns)
    let base = repeat(1, {length})
    let got = for n in xs with sum = 0 {{ sum + length(push(base, n)) }}
    assert got == turns * {}
}}
",
            length + 1
        );
        let elapsed = time(&[("bench.deed", source)], 0);
        if index == 0 {
            empty = elapsed;
        }

        println!(
            "{length:<10} {:<10} {:<10} {}",
            millis(elapsed),
            nanos(elapsed / PUSHES as u32),
            if index == 0 {
                String::new()
            } else {
                nanos(elapsed.saturating_sub(empty) / PUSHES as u32)
            },
        );
    }
}

// -- a real program ----------------------------------------------------------

/// `examples/logs.deed`, on a growing number of lines.
///
/// The pure half of it, which is everything down to `report`: it takes lines
/// and gives back lines, so a benchmark can hand it text without a filesystem
/// being involved in the number.
fn real_program() {
    let logs = read("logs.deed");
    // The table it counts with ships inside the compiler, so it comes from
    // there rather than from a path, the same way it reaches a program.
    let table = deed_driver::shipped_source("std/table")
        .expect("a module that ships has a source")
        .to_string();
    let files = |bench: String| {
        vec![
            ("bench.deed", bench),
            ("logs.deed", logs.clone()),
            ("<shipped>/std/table.deed", table.clone()),
        ]
    };

    println!("examples/logs.deed, the whole report");
    println!("lines      total      per line");
    println!("--------------------------------");

    for blocks in BLOCKS {
        let elapsed = time(&files(driver_source(blocks, "length(report(ls))")), 0);
        let count = blocks * SAMPLE.len();
        println!(
            "{count:<10} {:<10} {}",
            millis(elapsed),
            micros(elapsed / count as u32),
        );
    }

    println!();
    println!("the same report, one stage at a time, on {LINES} lines");
    println!("                         total      per line   added");
    println!("--------------------------------------------------------");

    // Each stage is a name `logs.deed` gave something, so the reader can go
    // and look at what was added rather than being told a fraction. The stages
    // are nested rather than disjoint: `added` is what the outer one does that
    // the inner one did not.
    let stages = [
        ("the input alone", "length(ls)"),
        (
            "  + splitting lines",
            "for line in ls with n = 0 { n + length(fields(line)) }",
        ),
        (
            "  + reading a field",
            "for line in ls with n = 0 { n + length(level_of(line)) }",
        ),
        ("  + counting and ranking", "length(report(ls))"),
    ];

    let mut previous = Duration::ZERO;
    for (index, (name, work)) in stages.iter().enumerate() {
        let elapsed = time(&files(driver_source(STAGE_BLOCKS, work)), 0);
        println!(
            "{name:<24} {:<10} {:<10} {}",
            millis(elapsed),
            micros(elapsed / LINES as u32),
            if index == 0 {
                String::new()
            } else {
                micros(elapsed.saturating_sub(previous) / LINES as u32)
            },
        );
        previous = elapsed;
    }
}

/// A block of log lines, repeated to make an input.
///
/// Three levels and four sources, so the table the program keys by has
/// something in it. Real enough to be the shape `logs.deed` was written
/// against, and short enough to read.
const SAMPLE: [&str; 12] = [
    "2026-07-01 ERROR  ledger   refused a write",
    "2026-07-01 WARN   cache    evicted early",
    "2026-07-01 INFO   mailer   sent 3 messages",
    "2026-07-01 INFO   clock    drifted 2ms",
    "2026-07-01 ERROR  cache    lost a key",
    "2026-07-01 INFO   ledger   settled a batch",
    "2026-07-01 WARN   mailer   retried once",
    "2026-07-01 INFO   cache    warmed up",
    "2026-07-01 ERROR  clock    stepped backwards",
    "2026-07-01 WARN   ledger   held a write",
    "2026-07-01 INFO   mailer   queued 9 messages",
    "2026-07-01 INFO   clock    resynced",
];

/// Which size the stage table runs at, and how many lines that is.
const STAGE_BLOCKS: usize = 80;
const LINES: usize = STAGE_BLOCKS * SAMPLE.len();

fn driver_source(blocks: usize, work: &str) -> String {
    let sample = SAMPLE.join("\\n");
    format!(
        "module bench

use examples/logs.{{fields, level_of, report}}

// The input, built with `join` and `split` rather than with a fold that
// pushes. A fold would put the cost of building a list into the row that is
// supposed to be measuring the program, and the row above already says what
// that costs.
fn lines(blocks: Int) -> List<String> {{
    split(join(repeat(\"{sample}\", blocks), \"\\n\"), \"\\n\")
}}

test \"reporting\" {{
    let ls = lines({blocks})
    let got = {work}
    assert got > 0
}}
"
    )
}

fn read(name: &str) -> String {
    let path = format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|why| panic!("{path} should be readable: {why}"))
}

// -- running -----------------------------------------------------------------

/// Checks `files` once and then times running the tests in the first one.
///
/// Checking is outside the measurement on purpose: `examples/edit_loop.rs`
/// already says what a check costs, and a number that added the two together
/// would answer neither question.
///
/// A benchmark quietly measuring a failure is a benchmark measuring the wrong
/// thing, so a source that does not check, or a test that does not pass, stops
/// this rather than being averaged into a row.
fn time(files: &[(&str, String)], entry: usize) -> Duration {
    let mut sources = SourceMap::new();
    let ids: Vec<_> = files
        .iter()
        .map(|(name, text)| sources.add(*name, text.clone()))
        .collect();

    let checks = check_all(&sources, &ids);
    for checked in &checks {
        if let Some(diagnostic) = checked.diagnostics.iter().find(|d| d.is_error()) {
            panic!(
                "the benchmark should check cleanly, and it does not:\n{}",
                deed_diagnostics::render_human(&sources, diagnostic)
            );
        }
    }

    let mut program = Program::new();
    for checked in &checks {
        program.add(
            checked.file,
            &checked.module,
            &checked.resolutions,
            checked.guards(),
            checked.rows(),
        );
    }

    let file = checks[entry].file;
    let mut best = Duration::MAX;
    for _ in 0..ROUNDS {
        let start = Instant::now();
        let outcomes = run_tests(&program, file);
        let elapsed = start.elapsed();

        assert!(!outcomes.is_empty(), "the benchmark ran no tests");
        for outcome in &outcomes {
            if let Some(failure) = &outcome.failure {
                panic!(
                    "`{}` should have passed:\n{}",
                    outcome.name,
                    deed_diagnostics::render_human(&sources, failure)
                );
            }
        }

        best = best.min(elapsed);
    }
    best
}

fn notes() {
    println!("A turn is one element of a `for`: the element bound, the accumulator");
    println!("bound, the body walked. `added` is the row minus the one it is one");
    println!("thing more than, so it is what that one thing costs.");
    println!();
    println!("Read the first three tables against each other. A turn, an operator");
    println!("and a field read are the walk, and the walk is what a compiler");
    println!("removes. A call taking nothing is down among them, so what a call");
    println!("costs is its arguments: an argument list, a binding, and a name read.");
    println!();
    println!("The second table says which of those three it is. An argument is");
    println!("around 55ns and the name read inside it is under 15ns, which is about");
    println!("what a field read costs. So a name is not the expensive small thing;");
    println!("the argument list and the binding are, and they are what a slot per");
    println!("name would leave exactly where it is. The first table on its own");
    println!("cannot say this, because its two call rows call different functions.");
    println!();
    println!("Nor is a push the walk: the copy is the per-element part, and the row");
    println!("for an empty list is what it costs before a single element has been");
    println!("copied.");
}

// -- printing ----------------------------------------------------------------

fn millis(duration: Duration) -> String {
    format!("{:.1}ms", duration.as_secs_f64() * 1000.0)
}

fn micros(duration: Duration) -> String {
    format!("{:.1}us", duration.as_secs_f64() * 1_000_000.0)
}

fn nanos(duration: Duration) -> String {
    format!("{}ns", duration.as_nanos())
}
