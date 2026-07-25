//! How check time scales with the size of the input.
//!
//! Not a wall clock budget. A test that fails when the machine is busy is a
//! test people learn to rerun until it passes, and CI is always busy. What is
//! worth guarding is the shape of the curve: accidental quadratic behaviour
//! shows up as a ratio and does not care how fast the machine is.
//!
//! The mistakes this is here to catch are already in the codebase in miniature.
//! `member_of` walks every definition, `imported_function` walks a module's
//! functions, `handled_effect` walks every definition again. All of them are
//! fine at four functions, and none of them are fine at four thousand.

use std::time::{Duration, Instant};

use vow_diagnostics::SourceMap;
use vow_driver::check_text;

/// A module with `count` functions that call each other, so resolution and the
/// effects pass have real work rather than a list of independent leaves.
///
/// The source has to check cleanly. Measuring a file full of errors measures
/// the error path, which has its own cost, and mixing the two would hide
/// whichever one regressed.
fn generated(count: usize) -> String {
    let mut source = String::from(
        "module bench\n\n\
         effect Log {\n    fn write(line: Int) -> ()\n}\n\n\
         record Pair {\n    left: Int,\n    right: Int,\n}\n\n\
         choice Side {\n    Left,\n    Right,\n}\n\n\
         fn pick(side: Side, pair: Pair) -> Int {\n\
         \x20   match side {\n\
         \x20       Left => pair.left,\n\
         \x20       Right => pair.right,\n\
         \x20   }\n\
         }\n",
    );

    for index in 0..count {
        let previous = index.saturating_sub(1);
        source.push_str(&format!(
            "\nfn f{index}(n: Int) -> Int\n\
             \x20 where\n\
             \x20   n > 0,\n\
             \x20 uses\n\
             \x20   Log.write,\n\
             \x20 ensures\n\
             \x20   ok  => result > 0,\n\
             {{\n\
             \x20   Log.write(n)\n\
             \x20   let pair = Pair {{ left: n, right: f{previous}_pure(n) }}\n\
             \x20   pick(Left, pair)\n\
             }}\n\n\
             fn f{index}_pure(n: Int) -> Int\n\
             \x20 ensures\n\
             \x20   ok  => result >= n,\n\
             {{\n\
             \x20   n + {index}\n\
             }}\n"
        ));
    }

    source
}

fn check_time(source: &str) -> Duration {
    let mut sources = SourceMap::new();
    let start = Instant::now();
    let checked = check_text(&mut sources, "bench.vow", source.to_string());
    let elapsed = start.elapsed();
    // Touch the result so nothing can be optimised away.
    assert!(checked.diagnostics.len() < usize::MAX);
    elapsed
}

/// The median of several runs, since one run on a shared machine is noise.
fn median_check_time(source: &str) -> Duration {
    let mut samples: Vec<Duration> = (0..5).map(|_| check_time(source)).collect();
    samples.sort();
    samples[samples.len() / 2]
}

#[test]
fn checking_scales_close_to_linearly() {
    let small = generated(40);
    let large = generated(400);

    // Warm up, so the first measurement does not pay for whatever the
    // allocator and the caches were doing beforehand.
    check_time(&small);
    check_time(&large);

    let small_time = median_check_time(&small);
    let large_time = median_check_time(&large);

    // Ten times the input. Linear would be 10x, quadratic would be 100x. The
    // bound is 30x, which leaves room for a slow or loaded machine and still
    // fails long before anything quadratic gets comfortable.
    let ratio = large_time.as_secs_f64() / small_time.as_secs_f64().max(f64::EPSILON);
    assert!(
        ratio < 30.0,
        "checking 10x the input took {ratio:.1}x the time \
         ({:.2}ms vs {:.2}ms), which is the shape of an accidental quadratic",
        large_time.as_secs_f64() * 1000.0,
        small_time.as_secs_f64() * 1000.0
    );
}

#[test]
fn the_generated_source_is_actually_checked() {
    // Otherwise the measurement above could be timing a parse failure, which
    // would be fast, stable and meaningless. It also has to check cleanly, or
    // the numbers would be about the error path instead.
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "bench.vow", generated(3));
    assert!(
        checked.diagnostics.is_empty(),
        "{}",
        checked
            .diagnostics
            .iter()
            .map(|d| vow_diagnostics::render_human(&sources, d))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        checked.obligations.len() > 3,
        "the generated source should carry contract obligations"
    );
}

#[test]
fn checking_a_file_full_of_errors_scales_too() {
    // The case that matters most and is easiest to forget. A file with many
    // errors is the normal state of a file being edited, which is exactly when
    // P9's latency claim is about something.
    //
    // The first version of this test found a real quadratic: every unresolved
    // name ran an edit distance against every name in scope, so ten times the
    // input cost fifty seven times the time.
    let small = broken(40);
    let large = broken(400);

    check_time(&small);
    check_time(&large);

    let small_time = median_check_time(&small);
    let large_time = median_check_time(&large);

    let ratio = large_time.as_secs_f64() / small_time.as_secs_f64().max(f64::EPSILON);
    assert!(
        ratio < 30.0,
        "checking 10x the broken input took {ratio:.1}x the time \
         ({:.2}ms vs {:.2}ms)",
        large_time.as_secs_f64() * 1000.0,
        small_time.as_secs_f64() * 1000.0
    );
}

/// The same shape, with every call naming something that does not exist.
fn broken(count: usize) -> String {
    let mut source = String::from("module bench\n");
    for index in 0..count {
        source.push_str(&format!(
            "\nfn declared{index}(n: Int) -> Int {{ n }}\n\n\
             fn caller{index}(n: Int) -> Int {{ declrae{index}(n) }}\n"
        ));
    }
    source
}

#[test]
fn every_pass_is_timed() {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "bench.vow", generated(40));

    assert!(
        checked.timings.total() > Duration::ZERO,
        "nothing was measured"
    );
    for (name, elapsed) in checked.timings.passes() {
        assert!(
            elapsed <= checked.timings.total(),
            "`{name}` reported more time than the whole check"
        );
    }
}
