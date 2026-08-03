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
    per_operation();
    println!();
    per_argument();
    println!();
    per_push();
    println!();
    per_table();
    println!();
    per_map();
    println!();
    per_map_compiled();
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

// -- what a handler operation costs ------------------------------------------

/// A handler operation against an equivalent plain call.
///
/// An operation is a call dispatched through Deed's effect system: the
/// runtime finds the nearest enclosing `with` block for the matching effect,
/// then calls the handler's body. The overhead over a plain call is that
/// search plus one extra call level. The two data rows measure a stateless
/// handler (no state fields at all) and a stateful one that reads a field,
/// since state access could add a lookup on top.
///
/// Both are compared against the same plain-call baseline rather than
/// against each other, so the two `added` numbers say the same thing: what
/// an operation costs over a call that does the same work.
fn per_operation() {
    println!("{TURNS} turns, a handler operation against a plain call");
    println!("                              total      per turn   added");
    println!("-----------------------------------------------------------");

    struct OpRow {
        name: &'static str,
        body: &'static str,
        claim: &'static str,
        /// The expression handed to `with`, or `""` for a plain call.
        install: &'static str,
        /// Which row this is one thing more than.
        over: Option<usize>,
    }

    let rows = [
        OpRow {
            name: "a call taking nothing",
            body: "sum + nothing()",
            claim: "got == turns",
            install: "",
            over: None,
        },
        OpRow {
            name: "  + a stateless operation",
            body: "sum + Nop.value()",
            claim: "got == 0",
            install: "NopHandler",
            over: Some(0),
        },
        OpRow {
            name: "  + reads state instead",
            body: "sum + Reading.value()",
            claim: "got == turns",
            install: "ReaderOne { one: 1 }",
            over: Some(0),
        },
    ];

    let mut taken: Vec<Duration> = Vec::new();
    for row in &rows {
        let elapsed = time(
            &[(
                "bench.deed",
                operation_source(row.body, row.claim, row.install),
            )],
            0,
        );
        let added = match row.over {
            Some(i) => nanos(elapsed.saturating_sub(taken[i]) / TURNS as u32),
            None => String::new(),
        };
        println!(
            "{:<29} {:<10} {:<10} {added}",
            row.name,
            millis(elapsed),
            nanos(elapsed / TURNS as u32),
        );
        taken.push(elapsed);
    }
}

fn operation_source(body: &str, claim: &str, install: &str) -> String {
    let work = format!("let got = for n in xs with sum = 0 {{ {body} }}");

    // When a handler is needed the `for` runs inside a `with` block, so the
    // handler is installed for every turn. The `let` and the `assert` are
    // both inside the block because a `let` inside a block is not visible
    // outside it.
    let test_inner = if install.is_empty() {
        format!("    {work}\n    assert {claim}")
    } else {
        format!("    with {install} {{\n        {work}\n        assert {claim}\n    }}")
    };

    format!(
        "module bench

fn nothing() -> Int {{ 1 }}

effect Nop {{
    fn value() -> Int
}}

handler NopHandler implements Nop {{
    fn value() -> Int {{ 0 }}
}}

effect Reading {{
    fn value() -> Int
}}

handler ReaderOne implements Reading {{
    state one: Int

    fn value() -> Int {{ one }}
}}

test \"measuring\" {{
    let turns = {TURNS}
    let xs = repeat(1, turns)
{test_inner}
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

// -- what std/table costs -----------------------------------------------------

/// `std/table`'s `or_else` (lookup) and `set` (insert), against a table with a
/// given number of distinct keys.
///
/// #614: `std/table` is a list of entries, so `get`, `set` and `or_else` all
/// walk it, and `examples/logs.deed` counts by key, which is one walk per
/// line per distinct key. Nobody had measured what that costs, only argued
/// it from the shape of the code, and one reading proves nothing: four sizes
/// are what tell a slope from noise (#266 had to redo a measurement it took
/// from a single difference).
///
/// Both probes ask for the worst case a lookup or an insert onto a table of
/// that size can have. `or_else` looks up the *last* key `set` put in, so the
/// walk that answers it crosses every entry before matching one, rather than
/// stopping at the front for a table built by repeated `set`. `set` inserts a
/// key that is not present at all, so `holds` walks the whole list before
/// `push` copies it, which is `set`'s real cost on a key that was not already
/// there: two full walks rather than one.
fn per_table() {
    let table = deed_driver::shipped_source("std/table")
        .expect("a module that ships has a source")
        .to_string();
    let files = |bench: String| {
        vec![
            ("bench.deed", bench),
            ("<shipped>/std/table.deed", table.clone()),
        ]
    };

    println!("{PUSHES} lookups, against a table with this many distinct keys");
    println!("keys       total      per lookup over an empty one");
    println!("-----------------------------------------------------------");

    let mut empty = Duration::ZERO;
    for (index, size) in LENGTHS.into_iter().enumerate() {
        let last = size.saturating_sub(1);
        let source = format!(
            "module bench

use std/table.{{set, or_else}}

test \"lookup\" {{
    let keys = repeat(0, {size})
    let base = for _k at i in keys with entries = [] {{ set(entries, to_string(i), i) }}
    let turns = {PUSHES}
    let ns = repeat(0, turns)
    let got = for _n in ns with sum = 0 {{ sum + or_else(base, \"{last}\", 0) }}
    assert got == turns * {last}
}}
"
        );
        let elapsed = time(&files(source), 0);
        if index == 0 {
            empty = elapsed;
        }

        println!(
            "{size:<10} {:<10} {:<10} {}",
            millis(elapsed),
            nanos(elapsed / PUSHES as u32),
            if index == 0 {
                String::new()
            } else {
                nanos(elapsed.saturating_sub(empty) / PUSHES as u32)
            },
        );
    }

    println!();
    println!("{PUSHES} inserts of a key not already there, into a table this big");
    println!("keys       total      per insert over an empty one");
    println!("-----------------------------------------------------------");

    let mut empty = Duration::ZERO;
    for (index, size) in LENGTHS.into_iter().enumerate() {
        let source = format!(
            "module bench

use std/table.{{set}}

test \"inserting\" {{
    let keys = repeat(0, {size})
    let base = for _k at i in keys with entries = [] {{ set(entries, to_string(i), i) }}
    let turns = {PUSHES}
    let ns = repeat(0, turns)
    let got = for _n in ns with sum = 0 {{ sum + length(set(base, \"new\", 0)) }}
    assert got == turns * {}
}}
",
            size + 1
        );
        let elapsed = time(&files(source), 0);
        if index == 0 {
            empty = elapsed;
        }

        println!(
            "{size:<10} {:<10} {:<10} {}",
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

// -- what std/map costs, vs std/table -----------------------------------------

/// The files a keyed benchmark needs, by which library it uses.
fn table_files(bench: String) -> Vec<(&'static str, String)> {
    let table = deed_driver::shipped_source("std/table")
        .expect("a module that ships has a source")
        .to_string();
    vec![("bench.deed", bench), ("<shipped>/std/table.deed", table)]
}

fn map_files(bench: String) -> Vec<(&'static str, String)> {
    let map = deed_driver::shipped_source("std/map")
        .expect("a module that ships has a source")
        .to_string();
    let list = deed_driver::shipped_source("std/list")
        .expect("a module that ships has a source")
        .to_string();
    vec![
        ("bench.deed", bench),
        ("<shipped>/std/map.deed", map),
        ("<shipped>/std/list.deed", list),
    ]
}

/// Look the last key up, `PUSHES` times, over a structure holding `size` keys.
///
/// Worst-case discipline on both sides: the key asked for is one that is
/// present, so the list walks all of it and the tree walks to a leaf.
fn lookup_sources(size: usize) -> (String, String) {
    let last = size.saturating_sub(1);
    let table = format!(
        "module bench

use std/table.{{set, or_else}}

test \"lookup\" {{
    let keys = repeat(0, {size})
    let base = for _k at i in keys with entries = [] {{ set(entries, to_string(i), i) }}
    let turns = {PUSHES}
    let ns = repeat(0, turns)
    let got = for _n in ns with sum = 0 {{ sum + or_else(base, \"{last}\", 0) }}
    assert got == turns * {last}
}}
"
    );
    let map = format!(
        "module bench

use std/map.{{insert, get, cmp_string, Map, Empty}}

fn lookup(m: Map<String, Int>, key: String) -> Int
  uses Diverge,
{{
    match get(m, key, cmp_string) {{
        ok(v) => v,
        err(_) => 0,
    }}
}}

test \"lookup\" {{
    let keys = repeat(0, {size})
    let base = for _k at i in keys with m = Empty {{ insert(m, to_string(i), i, cmp_string) }}
    let turns = {PUSHES}
    let ns = repeat(0, turns)
    let got = for _n in ns with sum = 0 {{ sum + lookup(base, \"{last}\") }}
    assert got == turns * {last}
}}
"
    );
    (table, map)
}

/// Insert a key that is not already there, `PUSHES` times.
///
/// The map half avoids calling `size` on the result, which would be O(N) and
/// would dominate the O(log N) insert. The body runs the insert and adds one,
/// so `got == turns` confirms nothing was skipped without walking the result.
fn insert_sources(size: usize) -> (String, String) {
    let table = format!(
        "module bench

use std/table.{{set}}

test \"inserting\" {{
    let keys = repeat(0, {size})
    let base = for _k at i in keys with entries = [] {{ set(entries, to_string(i), i) }}
    let turns = {PUSHES}
    let ns = repeat(0, turns)
    let got = for _n in ns with sum = 0 {{ sum + length(set(base, \"new\", 0)) }}
    assert got == turns * {}
}}
",
        size + 1
    );
    let map = format!(
        "module bench

use std/map.{{insert, cmp_string, Empty}}

test \"inserting\" {{
    let keys = repeat(0, {size})
    let base = for _k at i in keys with m = Empty {{ insert(m, to_string(i), i, cmp_string) }}
    let turns = {PUSHES}
    let ns = repeat(0, turns)
    let got = for _n in ns with sum = 0 {{
        let _m = insert(base, \"new\", 0, cmp_string)
        sum + 1
    }}
    assert got == turns
}}
"
    );
    (table, map)
}

/// `std/map`'s `get` (lookup) and `insert`, against a map with a given number
/// of distinct keys, run alongside `std/table` at the same sizes.
///
/// The question from deed-lang/deed#616: does the red-black tree beat the
/// list at the sizes real programs reach? Both halves use the same LENGTHS
/// and the same PUSHES so the numbers sit next to each other rather than in
/// separate tables.
fn per_map() {
    println!("{PUSHES} lookups: std/table vs std/map, by number of keys");
    println!("keys       table      table/key  map        map/key");
    println!("------------------------------------------------------");

    for size in LENGTHS {
        let (table_source, map_source) = lookup_sources(size);
        let t = time(&table_files(table_source), 0);
        let m = time(&map_files(map_source), 0);

        println!(
            "{size:<10} {:<10} {:<10} {:<10} {}",
            millis(t),
            nanos(t / PUSHES as u32),
            millis(m),
            nanos(m / PUSHES as u32),
        );
    }

    println!();
    println!("{PUSHES} inserts of a key not already there: std/table vs std/map");
    println!("keys       table      table/ins  map        map/ins");
    println!("------------------------------------------------------");

    for size in LENGTHS {
        let (table_source, map_source) = insert_sources(size);
        let t = time(&table_files(table_source), 0);
        let m = time(&map_files(map_source), 0);

        println!(
            "{size:<10} {:<10} {:<10} {:<10} {}",
            millis(t),
            nanos(t / PUSHES as u32),
            millis(m),
            nanos(m / PUSHES as u32),
        );
    }
}

/// The same two questions, compiled.
///
/// The open question left by `design/decisions/2026-07-31-tree-vs-table-decision.md`:
/// the crossover it measured is the interpreter's, and the tree's constant
/// factor is mostly the per-call cost the interpreter pays per tree level.
/// Compiled code pays less for a call, so the crossover should move toward
/// smaller N. This is that measurement, over the same programs, so the two
/// sets of numbers are about the same work.
fn per_map_compiled() {
    println!("{PUSHES} lookups, compiled: std/table vs std/map");
    println!("keys       table      table/key  map        map/key");
    println!("------------------------------------------------------");

    for size in LENGTHS {
        let (table_source, map_source) = lookup_sources(size);
        compiled_row(size, &table_files(table_source), &map_files(map_source));
    }

    println!();
    println!("{PUSHES} inserts of a key not already there, compiled");
    println!("keys       table      table/ins  map        map/ins");
    println!("------------------------------------------------------");

    for size in LENGTHS {
        let (table_source, map_source) = insert_sources(size);
        compiled_row(size, &table_files(table_source), &map_files(map_source));
    }
}

fn compiled_row(size: usize, table: &[(&str, String)], map: &[(&str, String)]) {
    let t = time_compiled(table, 0);
    let m = time_compiled(map, 0);
    match (t, m) {
        (Some(t), Some(m)) => println!(
            "{size:<10} {:<10} {:<10} {:<10} {}",
            millis(t),
            nanos(t / PUSHES as u32),
            millis(m),
            nanos(m / PUSHES as u32),
        ),
        _ => println!("{size:<10} the backend refused one of them"),
    }
}

// -- a real program ----------------------------------------------------------
///
/// The pure half of it, which is everything down to `report`: it takes lines
/// and gives back lines, so a benchmark can hand it text without a filesystem
/// being involved in the number.
fn real_program() {
    let logs = read("logs.deed");
    // The modules it counts with ship inside the compiler, so they come from
    // there rather than from a path, the same way they reach a program. Asked
    // for rather than named: naming one by hand is how this stopped compiling
    // the day `logs.deed` imported a second one.
    let shipped: Vec<(String, String)> = deed_driver::shipped_for([logs.as_str()])
        .into_iter()
        .map(|module| {
            let text = deed_driver::shipped_source(module)
                .expect("a module that ships has a source")
                .to_string();
            (format!("<shipped>/{module}.deed"), text)
        })
        .collect();
    let files = |bench: String| {
        let mut files = vec![("bench.deed", bench), ("logs.deed", logs.clone())];
        for (name, text) in &shipped {
            files.push((name.as_str(), text.clone()));
        }
        files
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

/// The same, through the compiled backend.
///
/// `None` when the backend cannot lower or compile the program, which is the
/// honest answer for a row rather than a zero that reads like a fast one.
/// Compiling is outside the measurement for the same reason checking is.
fn time_compiled(files: &[(&str, String)], entry: usize) -> Option<Duration> {
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

    let subject = &checks[entry];
    let alongside: Vec<deed_mir::Alongside<'_>> = checks
        .iter()
        .enumerate()
        .filter(|(at, _)| *at != entry)
        .map(|(_, checked)| deed_mir::Alongside {
            module: &checked.module,
            resolutions: &checked.resolutions,
            types: &checked.types,
        })
        .collect();

    let lowered = deed_mir::lower_with_tests_alongside(
        &subject.module,
        &subject.resolutions,
        &subject.types,
        &alongside,
    )
    .ok()?;
    let compiled = deed_codegen::compile(&lowered).ok()?;
    let test = lowered.tests.first()?;

    let mut best = Duration::MAX;
    for _ in 0..ROUNDS {
        let start = Instant::now();
        let outcome = deed_codegen::call(&compiled, &test.body, &[]);
        let elapsed = start.elapsed();
        assert!(
            outcome.is_ok(),
            "`{}` should have passed in the backend: {:?}",
            test.name,
            outcome.err()
        );
        best = best.min(elapsed);
    }
    Some(best)
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
    println!("The operation table is the second one. A stateless operation and a");
    println!("state-reading one are both compared against the same plain-call");
    println!("baseline, so their `added` numbers say what effect dispatch costs");
    println!("over a call. State access is a field read on top of that, and the");
    println!("field-read row in the first table says what a field read costs, so");
    println!("the two numbers together say whether state adds anything beyond what");
    println!("the first table already shows.");
    println!();
    println!("All ten operations across the six effects in the examples corpus");
    println!("tail-resume: each handler body returns immediately without capturing");
    println!("or discarding the continuation. Koka compiles tail-resuming operations");
    println!("to something close to a virtual call by avoiding the general");
    println!("continuation machinery. Whether the same distinction is worth having");
    println!("in Deed's compiler is what design/decisions/ records.");
    println!();
    println!("The third table says which of those three it is. An argument is");
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
