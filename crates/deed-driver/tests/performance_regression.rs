//! A performance ratchet for the two measurements that used to live only in
//! examples: the edit loop's recheck cost and the `logs.deed` runtime slope.
//!
//! These are timing tests, so they do not run in the normal CI job. The
//! dedicated workflow runs just this file, in release mode and with one test
//! thread, and records each run's numbers as an artifact.

use std::{
    env, fs,
    time::{Duration, Instant},
};

use deed_diagnostics::SourceMap;
use deed_driver::{Checked, Timings, check_all};
use deed_interp::{Program, run_tests};

const EDIT_SIZES: [usize; 4] = [8, 32, 128, 512];
const LOG_BLOCKS: [usize; 4] = [20, 40, 80, 160];
const ROUNDS: usize = 5;

const STEP_CHANGE_LIMIT: f64 = 2.0;
const SEGMENT_RATIO_LIMIT: f64 = 1.8;

// Recorded from the first ratchet run added in #676. The workflow artifact keeps
// each run's fresh numbers so a deliberate change can update these with a visible
// before-and-after diff.
const EDIT_TOTAL_BASELINE_US_PER_FILE: f64 = 68.5;
const EDIT_PASS_BASELINES_US_PER_FILE: [(&str, f64); 5] = [
    ("lex", 5.6),
    ("parse", 8.0),
    ("resolve", 16.7),
    ("typeck", 24.6),
    ("effects", 4.5),
];
const LOGS_REPORT_BASELINE_US_PER_LINE: f64 = 26.4;

#[test]
#[ignore = "runs in .github/workflows/performance.yml"]
fn existing_bench_slopes_stay_inside_the_ratchet() {
    let edit = measure_edit_loop();
    let logs = measure_logs_report();

    maybe_write_results(&edit, &logs);

    let mut failures = Vec::new();
    let edit_total = edit
        .total
        .series("edit loop recheck", EDIT_TOTAL_BASELINE_US_PER_FILE);
    if let Err(why) = ratchet(&edit_total) {
        failures.push(why);
    }

    for (name, baseline) in EDIT_PASS_BASELINES_US_PER_FILE {
        let series = edit
            .pass(name)
            .series(&format!("edit loop `{name}`"), baseline);
        if let Err(why) = ratchet(&series) {
            failures.push(why);
        }
    }

    let logs_total = logs.total.series(
        "`examples/logs.deed` report",
        LOGS_REPORT_BASELINE_US_PER_LINE,
    );
    if let Err(why) = ratchet(&logs_total) {
        failures.push(why);
    }

    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn a_deliberate_regression_is_caught() {
    let regression = Series {
        name: "deliberate regression".to_string(),
        unit: "us/file",
        baseline_slope: 80.0,
        points: vec![
            (8.0, 1_280.0),
            (32.0, 5_120.0),
            (128.0, 20_480.0),
            (512.0, 81_920.0),
        ],
    };

    let message = ratchet(&regression).expect_err("a 2x slowdown should fail the ratchet");
    assert!(message.contains("2.00x"), "{message}");
}

#[derive(Clone, Debug)]
struct Series {
    name: String,
    unit: &'static str,
    baseline_slope: f64,
    points: Vec<(f64, f64)>,
}

fn ratchet(series: &Series) -> Result<(), String> {
    let slope = least_squares_slope(&series.points);
    let step = slope / series.baseline_slope.max(f64::EPSILON);
    if step >= STEP_CHANGE_LIMIT {
        return Err(format!(
            "{} got {:.2} {}, {:.2}x its recorded {:.2} {} baseline",
            series.name, slope, series.unit, step, series.baseline_slope, series.unit
        ));
    }

    let segment_ratio = adjacent_segment_ratio(&series.points);
    if segment_ratio > SEGMENT_RATIO_LIMIT {
        return Err(format!(
            "{} stopped looking linear: adjacent slopes varied by {:.2}x",
            series.name, segment_ratio
        ));
    }

    Ok(())
}

fn least_squares_slope(points: &[(f64, f64)]) -> f64 {
    let count = points.len() as f64;
    let mean_x = points.iter().map(|(x, _)| *x).sum::<f64>() / count;
    let mean_y = points.iter().map(|(_, y)| *y).sum::<f64>() / count;
    let numerator = points
        .iter()
        .map(|(x, y)| (x - mean_x) * (y - mean_y))
        .sum::<f64>();
    let denominator = points
        .iter()
        .map(|(x, _)| (x - mean_x) * (x - mean_x))
        .sum::<f64>()
        .max(f64::EPSILON);
    numerator / denominator
}

fn adjacent_segment_ratio(points: &[(f64, f64)]) -> f64 {
    let slopes: Vec<_> = points
        .windows(2)
        .map(|pair| {
            let (x0, y0) = pair[0];
            let (x1, y1) = pair[1];
            (y1 - y0) / (x1 - x0).max(f64::EPSILON)
        })
        .collect();
    let fastest = slopes
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min)
        .max(f64::EPSILON);
    let slowest = slopes.iter().copied().fold(0.0, f64::max);
    slowest / fastest
}

#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    total: Duration,
    timings: Timings,
}

impl Sample {
    fn micros(self) -> f64 {
        self.total.as_secs_f64() * 1_000_000.0
    }

    fn series(self, x: f64) -> (f64, f64) {
        (x, self.micros())
    }
}

#[derive(Clone, Debug)]
struct SampleSet {
    unit: &'static str,
    points: Vec<(f64, Sample)>,
}

impl SampleSet {
    fn series(&self, name: &str, baseline: f64) -> Series {
        Series {
            name: name.to_string(),
            unit: self.unit,
            baseline_slope: baseline,
            points: self
                .points
                .iter()
                .map(|(x, sample)| sample.series(*x))
                .collect(),
        }
    }
}

#[derive(Clone, Debug)]
struct EditLoopMetrics {
    total: SampleSet,
    lex: SampleSet,
    parse: SampleSet,
    resolve: SampleSet,
    typeck: SampleSet,
    effects: SampleSet,
}

impl EditLoopMetrics {
    fn pass(&self, name: &str) -> &SampleSet {
        match name {
            "lex" => &self.lex,
            "parse" => &self.parse,
            "resolve" => &self.resolve,
            "typeck" => &self.typeck,
            "effects" => &self.effects,
            other => panic!("unknown pass `{other}`"),
        }
    }
}

#[derive(Clone, Debug)]
struct RuntimeMetrics {
    total: SampleSet,
}

fn measure_edit_loop() -> EditLoopMetrics {
    verify_workspace();

    let mut total = Vec::new();
    let mut lex = Vec::new();
    let mut parse = Vec::new();
    let mut resolve = Vec::new();
    let mut typeck = Vec::new();
    let mut effects = Vec::new();

    for size in EDIT_SIZES {
        let workspace = generated_workspace(size);
        let mut edited = workspace.clone();
        edited[size - 1].push_str("\nfn typing() -> Int { 0 }\n");

        recheck_sample(&edited);
        let sample = fastest_sample(ROUNDS, || recheck_sample(&edited));

        total.push((size as f64, sample));
        lex.push((
            size as f64,
            Sample {
                total: sample.timings.lex,
                timings: Timings::default(),
            },
        ));
        parse.push((
            size as f64,
            Sample {
                total: sample.timings.parse,
                timings: Timings::default(),
            },
        ));
        resolve.push((
            size as f64,
            Sample {
                total: sample.timings.resolve,
                timings: Timings::default(),
            },
        ));
        typeck.push((
            size as f64,
            Sample {
                total: sample.timings.typeck,
                timings: Timings::default(),
            },
        ));
        effects.push((
            size as f64,
            Sample {
                total: sample.timings.effects,
                timings: Timings::default(),
            },
        ));
    }

    EditLoopMetrics {
        total: SampleSet {
            unit: "us/file",
            points: total,
        },
        lex: SampleSet {
            unit: "us/file",
            points: lex,
        },
        parse: SampleSet {
            unit: "us/file",
            points: parse,
        },
        resolve: SampleSet {
            unit: "us/file",
            points: resolve,
        },
        typeck: SampleSet {
            unit: "us/file",
            points: typeck,
        },
        effects: SampleSet {
            unit: "us/file",
            points: effects,
        },
    }
}

fn verify_workspace() {
    let mut sources = SourceMap::new();
    let texts = generated_workspace(2);
    let ids: Vec<_> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| sources.add(format!("m{index}.deed"), text.clone()))
        .collect();

    for checked in check_all(&sources, &ids) {
        if let Some(diagnostic) = checked.diagnostics.first() {
            panic!(
                "the generated workspace should be clean, and it is not:\n{}",
                deed_diagnostics::render_human(&sources, diagnostic)
            );
        }
    }
}

fn generated_workspace(size: usize) -> Vec<String> {
    (0..size).map(generated_module).collect()
}

fn generated_module(index: usize) -> String {
    let mut text = format!("module m{index}\n\n");

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

fn recheck_sample(texts: &[String]) -> Sample {
    let mut sources = SourceMap::new();
    let ids: Vec<_> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| sources.add(format!("m{index}.deed"), text.clone()))
        .collect();

    let start = Instant::now();
    let checks = check_all(&sources, &ids);
    let total = start.elapsed();
    ensure_clean(&sources, &checks, "the edit-loop sample");

    let mut timings = Timings::default();
    for checked in &checks {
        timings.lex += checked.timings.lex;
        timings.parse += checked.timings.parse;
        timings.resolve += checked.timings.resolve;
        timings.typeck += checked.timings.typeck;
        timings.effects += checked.timings.effects;
    }

    std::hint::black_box(checks.len());
    Sample { total, timings }
}

fn fastest_sample(rounds: usize, mut run: impl FnMut() -> Sample) -> Sample {
    let mut best: Option<Sample> = None;
    for _ in 0..rounds {
        let candidate = run();
        if best.is_none_or(|current| candidate.total < current.total) {
            best = Some(candidate);
        }
    }
    best.expect("at least one timing round")
}

fn measure_logs_report() -> RuntimeMetrics {
    let logs = read_example("logs.deed");

    let points = LOG_BLOCKS
        .into_iter()
        .map(|blocks| {
            let driver = logs_driver_source(blocks);

            // Asked for rather than listed. This used to name `std/table` by
            // hand, and the day `logs.deed` imported a second shipped module
            // the benchmark stopped compiling while every other test passed.
            let mut files = vec![
                ("bench.deed".to_string(), driver.clone()),
                ("logs.deed".to_string(), logs.clone()),
            ];
            for module in deed_driver::shipped_for([logs.as_str(), driver.as_str()]) {
                let text =
                    deed_driver::shipped_source(module).expect("a module that ships has a source");
                files.push((format!("<shipped>/{module}.deed"), text.to_string()));
            }

            let files: Vec<(&str, String)> = files
                .iter()
                .map(|(name, text)| (name.as_str(), text.clone()))
                .collect();

            runtime_sample(&files, 0);
            let sample = fastest_sample(ROUNDS, || runtime_sample(&files, 0));
            ((blocks * LOG_SAMPLE.len()) as f64, sample)
        })
        .collect();

    RuntimeMetrics {
        total: SampleSet {
            unit: "us/line",
            points,
        },
    }
}

fn runtime_sample(files: &[(&str, String)], entry: usize) -> Sample {
    let mut sources = SourceMap::new();
    let ids: Vec<_> = files
        .iter()
        .map(|(name, text)| sources.add(*name, text.clone()))
        .collect();

    let checks = check_all(&sources, &ids);
    ensure_clean(&sources, &checks, "the runtime sample");

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
    let start = Instant::now();
    let outcomes = run_tests(&program, file);
    let total = start.elapsed();

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

    Sample {
        total,
        timings: Timings::default(),
    }
}

fn ensure_clean(sources: &SourceMap, checks: &[Checked], context: &str) {
    for checked in checks {
        if let Some(diagnostic) = checked.diagnostics.iter().find(|d| d.is_error()) {
            panic!(
                "{context} should check cleanly, and it does not:\n{}",
                deed_diagnostics::render_human(sources, diagnostic)
            );
        }
    }
}

fn logs_driver_source(blocks: usize) -> String {
    let sample = LOG_SAMPLE.join("\\n");
    format!(
        "module bench

use examples/logs.{{report}}

fn lines(blocks: Int) -> List<String> {{
    split(join(repeat(\"{sample}\", blocks), \"\\n\"), \"\\n\")
}}

test \"reporting\" {{
    let ls = lines({blocks})
    let got = length(report(ls))
    assert got > 0
}}
"
    )
}

const LOG_SAMPLE: [&str; 12] = [
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

fn read_example(name: &str) -> String {
    let path = format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|why| panic!("{path} should be readable: {why}"))
}

fn maybe_write_results(edit: &EditLoopMetrics, logs: &RuntimeMetrics) {
    let Ok(path) = env::var("DEED_PERF_RESULTS") else {
        return;
    };

    let json = format!(
        concat!(
            "{{\n",
            "  \"edit_loop\": {{\n",
            "    \"total_us_per_file\": {},\n",
            "    \"total_segment_ratio\": {},\n",
            "    \"passes_us_per_file\": {{\n",
            "      \"lex\": {},\n",
            "      \"parse\": {},\n",
            "      \"resolve\": {},\n",
            "      \"typeck\": {},\n",
            "      \"effects\": {}\n",
            "    }}\n",
            "  }},\n",
            "  \"logs_report\": {{\n",
            "    \"us_per_line\": {},\n",
            "    \"segment_ratio\": {}\n",
            "  }}\n",
            "}}\n"
        ),
        least_squares_slope(
            &edit
                .total
                .series("edit loop recheck", EDIT_TOTAL_BASELINE_US_PER_FILE)
                .points
        ),
        adjacent_segment_ratio(
            &edit
                .total
                .series("edit loop recheck", EDIT_TOTAL_BASELINE_US_PER_FILE)
                .points
        ),
        least_squares_slope(
            &edit
                .lex
                .series("edit loop `lex`", EDIT_PASS_BASELINES_US_PER_FILE[0].1)
                .points
        ),
        least_squares_slope(
            &edit
                .parse
                .series("edit loop `parse`", EDIT_PASS_BASELINES_US_PER_FILE[1].1)
                .points
        ),
        least_squares_slope(
            &edit
                .resolve
                .series("edit loop `resolve`", EDIT_PASS_BASELINES_US_PER_FILE[2].1)
                .points
        ),
        least_squares_slope(
            &edit
                .typeck
                .series("edit loop `typeck`", EDIT_PASS_BASELINES_US_PER_FILE[3].1)
                .points
        ),
        least_squares_slope(
            &edit
                .effects
                .series("edit loop `effects`", EDIT_PASS_BASELINES_US_PER_FILE[4].1)
                .points
        ),
        least_squares_slope(
            &logs
                .total
                .series(
                    "`examples/logs.deed` report",
                    LOGS_REPORT_BASELINE_US_PER_LINE
                )
                .points
        ),
        adjacent_segment_ratio(
            &logs
                .total
                .series(
                    "`examples/logs.deed` report",
                    LOGS_REPORT_BASELINE_US_PER_LINE
                )
                .points
        ),
    );

    fs::write(path, json).expect("the workflow should be able to write its perf artifact");
}
