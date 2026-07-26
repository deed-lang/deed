//! Nothing in a clean file has an unknown type.
//!
//! `Unknown` agrees with everything. An expression that has one is an
//! expression nothing done with it gets checked against, so a file that passes
//! `vow check` while containing one is a file the compiler only pretended to
//! read.
//!
//! Three holes in this language have been exactly that shape. A type name in
//! expression position had no type, so `Io.write(Console, "hi")` conjured
//! authority. A function parameter could be written without one, so a closure
//! could carry any effect through it. A handler operation's parameters never
//! got theirs from the effect, so the code holding the state was the least
//! checked in the language. All three were found by accident, one at a time.
//!
//! This is the invariant that would have caught all of them at once.

use vow_diagnostics::{SourceMap, render_human};
use vow_driver::{Checked, check_all, check_text};

/// Every span in `checked` whose expression the checker never worked out.
fn unknowns(sources: &SourceMap, checked: &Checked) -> Vec<String> {
    let text = sources.file(checked.file).text();
    let mut found: Vec<(u32, String)> = checked
        .types
        .unknowns()
        .map(|span| {
            let start = span.start as usize;
            let end = (span.end as usize).min(text.len());
            let snippet = text.get(start..end).unwrap_or("<bad span>");
            (span.start, format!("{}: `{snippet}`", span.start))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, line)| line).collect()
}

fn expect_all_known(name: &str, source: &str) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, name, source.to_string());

    assert!(
        !checked.has_errors(),
        "{name} should check cleanly:\n{}",
        checked
            .diagnostics
            .iter()
            .map(|d| render_human(&sources, d))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let unknown = unknowns(&sources, &checked);
    assert!(
        unknown.is_empty(),
        "{name} checks cleanly but {} expression(s) have no type, \
         so nothing done with them was checked:\n  {}",
        unknown.len(),
        unknown.join("\n  ")
    );
}

// -- the examples ----------------------------------------------------------

#[test]
fn every_example_is_fully_typed() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let mut paths: Vec<_> = std::fs::read_dir(root)
        .expect("examples should be there")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "vow"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no examples found");

    // The ones that import each other have to be checked together, or the
    // imported names have nothing behind them and are unknown for a reason
    // that is nothing to do with this.
    let mut sources = SourceMap::new();
    let ids: Vec<_> = paths
        .iter()
        .map(|path| {
            let text = std::fs::read_to_string(path).expect("an example should be readable");
            let name = format!("examples/{}", path.file_name().unwrap().to_string_lossy());
            sources.add(name, text)
        })
        .collect();

    for checked in check_all(&sources, &ids) {
        assert!(
            !checked.has_errors(),
            "the examples should check cleanly:\n{}",
            checked
                .diagnostics
                .iter()
                .map(|d| render_human(&sources, d))
                .collect::<Vec<_>>()
                .join("\n")
        );

        let unknown = unknowns(&sources, &checked);
        assert!(
            unknown.is_empty(),
            "{} checks cleanly but {} expression(s) have no type:\n  {}",
            sources.file(checked.file).name(),
            unknown.len(),
            unknown.join("\n  ")
        );
    }
}

// -- one per construct the language has ------------------------------------

#[test]
fn literals_and_arithmetic() {
    expect_all_known(
        "arith.vow",
        "module a\n\n\
         fn f(n: Int, s: String, b: Bool) -> Int {\n\
         \x20 let joined = s + \"x\"\n\
         \x20 let flag = b && length(joined) > 0\n\
         \x20 if flag {\n    n * 2 + 1\n  } else {\n    -n\n  }\n\
         }\n",
    );
}

#[test]
fn records_and_fields() {
    expect_all_known(
        "records.vow",
        "module a\n\n\
         record Point { x: Int, y: Int }\n\n\
         fn f(p: Point) -> Int { p.x + p.y }\n\n\
         fn make(x: Int) -> Point { Point { x, y: 0 } }\n",
    );
}

#[test]
fn choices_and_matches() {
    expect_all_known(
        "choices.vow",
        "module a\n\n\
         choice Tone { Plain, Loud { level: Int } }\n\n\
         fn f(t: Tone) -> Int {\n\
         \x20 match t {\n\
         \x20   Plain => 0,\n\
         \x20   Loud { level } => level,\n\
         \x20 }\n\
         }\n",
    );
}

#[test]
fn results_and_the_question_mark() {
    expect_all_known(
        "results.vow",
        "module a\n\n\
         fn might(n: Int) -> Result<Int, String> {\n\
         \x20 if n > 0 {\n    ok(n)\n  } else {\n    err(\"no\")\n  }\n\
         }\n\n\
         fn twice(n: Int) -> Result<Int, String> {\n\
         \x20 let one = might(n)?\n\
         \x20 ok(one + one)\n\
         }\n",
    );
}

#[test]
fn lists() {
    // An empty literal is a `List<unknown>`, which is a type, not a hole. The
    // distinction matters here more than anywhere: if `[]` were unknown
    // outright then nothing done with any list would be checked.
    expect_all_known(
        "lists.vow",
        "module a\n\n\
         fn f(items: List<String>) -> Int {\n\
         \x20 let more = push(items, \"x\")\n\
         \x20 let empty: List<String> = []\n\
         \x20 match at(more, 0) {\n\
         \x20   ok(first) => length(first) + length(empty),\n\
         \x20   err(why) => length(why),\n\
         \x20 }\n\
         }\n",
    );
}

#[test]
fn closures() {
    expect_all_known(
        "closures.vow",
        "module a\n\n\
         fn f(a: Int, b: Int) -> Int {\n\
         \x20 let add = |x: Int, y: Int| { x + y }\n\
         \x20 add(a, b)\n\
         }\n",
    );
}

#[test]
fn function_values() {
    // A function type is the one place a value's type is written out as a
    // shape rather than a name, so it is the natural place for a hole. Every
    // way one can be made or used is here.
    expect_all_known(
        "function_values.vow",
        "module a\n\n\
         fn apply(f: Fn(Int) -> Int, n: Int) -> Int { f(n) }\n\n\
         fn double(n: Int) -> Int { n + n }\n\n\
         fn adder() -> Fn(Int) -> Int { |x: Int| x + 1 }\n\n\
         fn f(n: Int) -> Int {\n\
         \x20 let step: Fn(Int) -> Int = |x: Int| x - 1\n\
         \x20 apply(double, n) + apply(step, n) + apply(adder(), n)\n\
         }\n",
    );
}

#[test]
fn effects_and_handlers() {
    expect_all_known(
        "effects.vow",
        "module a\n\n\
         effect Counter {\n    fn value() -> Int\n    fn bump(by: Int) -> ()\n}\n\n\
         handler InMemory implements Counter {\n\
         \x20 state count: Int\n\n\
         \x20 fn value() -> Int { count }\n\n\
         \x20 fn bump(by) -> () {\n    count = count + by\n  }\n\
         }\n\n\
         fn twice() -> Int\n\
         \x20 uses\n\
         \x20   Counter.bump,\n\
         \x20   Counter.value,\n\
         {\n\
         \x20 Counter.bump(1)\n\
         \x20 Counter.bump(1)\n\
         \x20 Counter.value()\n\
         }\n\n\
         test \"it counts\" {\n\
         \x20 with InMemory { count: 0 } {\n\
         \x20   assert twice() == 2\n\
         \x20 }\n\
         }\n",
    );
}

#[test]
fn contracts() {
    expect_all_known(
        "contracts.vow",
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn f(n: Int) -> Positive\n\
         \x20 where\n\
         \x20   n > 0,\n\
         \x20 ensures\n\
         \x20   ok  => result > 0,\n\
         {\n\
         \x20 n\n\
         }\n",
    );
}

#[test]
fn capabilities() {
    expect_all_known(
        "capabilities.vow",
        "module a\n\n\
         fn greet(out: Console) -> ()\n\
         \x20 uses\n\
         \x20   Io.write,\n\
         {\n\
         \x20 Io.write(out, \"hello\")\n\
         }\n\n\
         fn main(sys: System) -> ()\n\
         \x20 uses\n\
         \x20   Io.write,\n\
         {\n\
         \x20 greet(sys.console)\n\
         }\n",
    );
}

#[test]
fn a_contract_talking_about_an_effect() {
    expect_all_known(
        "observing.vow",
        "module a\n\n\
         effect Ledger {\n    fn balance() -> Int\n}\n\n\
         fn f() -> Int\n\
         \x20 ensures\n\
         \x20   ok  => Ledger.balance() == old(Ledger.balance()),\n\
         \x20   err => unchanged(Ledger),\n\
         {\n\
         \x20 0\n\
         }\n",
    );
}

// -- and the case where an unknown is the right answer ---------------------

#[test]
fn a_file_that_does_not_check_may_have_them() {
    // The invariant is about clean files. Once something has been reported,
    // `Unknown` is how the checker stops a single mistake turning into ten,
    // and insisting otherwise would be insisting on the cascade.
    let mut sources = SourceMap::new();
    let checked = check_text(
        &mut sources,
        "broken.vow",
        "module a\n\nfn f() -> Int { missing() }\n".to_string(),
    );
    assert!(checked.has_errors());
}
