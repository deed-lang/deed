//! What the edit loop costs, before anything is cached in it.
//!
//! P9 is about the edit loop and what it says is measured is a full check of a
//! small program. Nothing had measured it. The language server rechecks the
//! whole workspace on every request: every keystroke, every hover, every
//! completion. Whether that is fine is a question with a number for an answer,
//! and this prints the number.
//!
//! ```text
//! cargo run -p deed-driver --example edit_loop --release
//! ```
//!
//! Two things are printed. The wall clock, which says whether anybody would
//! notice, and the share of it spent on files that did not change, which is
//! what a cache would remove. If the second number is small then a cache is
//! not worth writing, and that is a real possible answer rather than a failed
//! experiment.
//!
//! No dependency and no framework. Criterion is the obvious tool and it is an
//! obvious violation of the rule this repository has about dependencies, and a
//! question this coarse does not need one.

use std::time::{Duration, Instant};

use deed_diagnostics::SourceMap;

/// How many workspaces to measure, and how big each one is.
const SIZES: [usize; 5] = [1, 8, 32, 128, 512];

/// How many times to repeat each measurement, so one unlucky run does not
/// decide the answer.
const ROUNDS: usize = 5;

fn main() {
    // A benchmark that is quietly measuring the error path is a benchmark
    // measuring the wrong thing, and the generated modules are easy to break
    // without noticing.
    verify();

    println!("files    cold      recheck   per file   unchanged");
    println!("-----------------------------------------------------");

    for size in SIZES {
        let workspace = workspace(size);

        let cold = fastest(ROUNDS, || {
            check(&workspace);
        });

        // What an editor does on a keystroke: one file differs, every file is
        // checked again because nothing remembers anything.
        let mut edited = workspace.clone();
        edited[size - 1].push_str("\nfn typing() -> Int { 0 }\n");
        let recheck = fastest(ROUNDS, || {
            check(&edited);
        });

        // The same measurement with only the changed file in it, which is the
        // most a perfect cache could ever save.
        let alone = fastest(ROUNDS, || {
            check(&edited[size - 1..]);
        });

        let per_file = recheck / size as u32;
        let unchanged = recheck.saturating_sub(alone);
        let share = if recheck.as_nanos() == 0 {
            0.0
        } else {
            unchanged.as_nanos() as f64 / recheck.as_nanos() as f64 * 100.0
        };

        println!(
            "{size:<8} {:<9} {:<9} {:<10} {:.0}%",
            millis(cold),
            millis(recheck),
            micros(per_file),
            share,
        );
    }

    println!();
    println!("cold      first check of the whole workspace");
    println!("recheck   the same, after one file changed by one line");
    println!("per file  recheck divided by how many files there are");
    println!("unchanged the share of a recheck spent on files that did not change,");
    println!("          which is the most any cache could take off it");
}

/// Checks that the generated workspace has nothing wrong with it.
///
/// Two modules is enough: it exercises a declaration, a use of it, and the
/// boundary between them, which is everything the generator produces.
fn verify() {
    let mut sources = SourceMap::new();
    let texts = workspace(2);
    let ids: Vec<_> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| sources.add(format!("m{index}.deed"), text.clone()))
        .collect();

    for checked in deed_driver::check_all(&sources, &ids) {
        if let Some(diagnostic) = checked.diagnostics.first() {
            panic!(
                "the generated workspace should be clean, and it is not:\n{}",
                deed_diagnostics::render_human(&sources, diagnostic)
            );
        }
    }
}

/// Runs the whole pipeline the way the language server does: every file
/// together, so a `use` has something to point at.
fn check(texts: &[String]) {
    let mut sources = SourceMap::new();
    let ids: Vec<_> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| sources.add(format!("m{index}.deed"), text.clone()))
        .collect();
    let checks = deed_driver::check_all(&sources, &ids);
    // Touched so the optimiser cannot decide none of this happened.
    std::hint::black_box(checks.len());
}

/// The fastest of several runs.
///
/// Fastest rather than the mean, because everything that makes a run slower is
/// noise from the rest of the machine and there is no such thing as noise that
/// makes one faster.
fn fastest(rounds: usize, mut run: impl FnMut()) -> Duration {
    let mut best = Duration::MAX;
    for _ in 0..rounds {
        let start = Instant::now();
        run();
        best = best.min(start.elapsed());
    }
    best
}

/// A workspace of `size` modules, each importing the one before it.
///
/// Generated rather than written, so the shape of the curve is visible rather
/// than one number from one repository that happens to have fourteen examples
/// in it. Each module is about the size of a small real one: a record, a
/// choice, an effect, and a handful of functions that use them.
fn workspace(size: usize) -> Vec<String> {
    (0..size).map(module).collect()
}

fn module(index: usize) -> String {
    let mut text = format!("module m{index}\n\n");

    // Every module but the first reads the one before it, so the surface pass
    // and the checker both have a boundary to cross. A chain rather than a
    // star, because a star makes the first module's surface the only one that
    // matters.
    if index > 0 {
        let previous = index - 1;
        text.push_str(&format!(
            "use m{previous}.{{Row{previous}, total{previous}}}\n\n"
        ));
    }

    text.push_str(&format!(
        "record Row{index} {{\n\
         \x20   id: Int,\n\
         \x20   label: String,\n\
         }}\n\n\
         choice Tone{index} {{\n\
         \x20   Plain,\n\
         \x20   Loud {{ times: Int }},\n\
         }}\n\n\
         effect Log{index} {{\n\
         \x20   fn note(message: String) -> ()\n\
         }}\n\n\
         fn label{index}(row: Row{index}) -> String {{ row.label }}\n\n\
         fn total{index}(rows: List<Row{index}>) -> Int {{\n\
         \x20   for row in rows with sum = 0 {{\n\
         \x20       sum + row.id\n\
         \x20   }}\n\
         }}\n\n\
         fn loudness{index}(tone: Tone{index}) -> Int {{\n\
         \x20   match tone {{\n\
         \x20       Plain => 0,\n\
         \x20       Loud {{ times }} => times,\n\
         \x20   }}\n\
         }}\n\n\
         fn announce{index}(row: Row{index}) -> ()\n\
         \x20 uses\n\
         \x20   Log{index}.note,\n\
         {{\n\
         \x20   Log{index}.note(row.label)\n\
         }}\n"
    ));

    if index > 0 {
        let previous = index - 1;
        text.push_str(&format!(
            "\nfn carried{index}(rows: List<Row{previous}>) -> Int {{ total{previous}(rows) }}\n"
        ));
    }

    text
}

fn millis(duration: Duration) -> String {
    format!("{:.1}ms", duration.as_secs_f64() * 1000.0)
}

fn micros(duration: Duration) -> String {
    format!("{:.0}us", duration.as_secs_f64() * 1_000_000.0)
}
