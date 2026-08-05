//! Where a compiled hash map's memory goes.
//!
//! `std/hashmap` stops a few hundred keys in compiled, and the module's own
//! header says why in one sentence: nothing is given back and `set` allocates
//! the whole bucket list every time. That sentence is a claim about which part
//! of the work is the expensive one, and this is the measurement behind it.
//!
//! Bytes rather than time, because bytes are what runs out. A compiled module
//! never reclaims, so what a program allocates in total is what its memory
//! reached, and these numbers are the same on every machine.

use deed_codegen::{Trap, call_measured, compile};
use deed_diagnostics::SourceMap;
use deed_driver::{check_all, shipped_for, shipped_source};

/// Bytes the compiled program allocates running its one test.
fn allocated(source: &str) -> Result<u64, Trap> {
    let mut sources = SourceMap::new();
    let subject = sources.add("probe.deed".to_string(), source.to_string());
    let mut ids = vec![subject];
    for module in shipped_for([source]) {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("{module}.deed"), text.to_string()));
    }

    let checks = check_all(&sources, &ids);
    for one in &checks {
        assert!(
            !one.has_errors(),
            "this should check: {:?}",
            one.diagnostics
        );
    }

    let alongside: Vec<deed_mir::Alongside<'_>> = checks
        .iter()
        .skip(1)
        .map(|checked| deed_mir::Alongside {
            module: &checked.module,
            resolutions: &checked.resolutions,
            types: &checked.types,
        })
        .collect();
    let lowered = deed_mir::lower_with_tests_alongside(
        &checks[0].module,
        &checks[0].resolutions,
        &checks[0].types,
        &alongside,
    )
    .expect("this lowers");
    let module = compile(&lowered).expect("this compiles");
    let test = lowered.tests.first().expect("the program declares a test");
    call_measured(&module, &test.body, &[]).map(|outcome| outcome.allocated)
}

fn probe(body: &str) -> Result<u64, Trap> {
    allocated(&format!(
        "module probe\n\n\
         use std/hashmap.{{empty, set, buckets, range}}\n\n\
         test \"probe\" {{\n\
         \x20   assert length({body}) >= 0\n\
         }}\n"
    ))
}

/// What each part of an insert costs, so the header's claim is a number.
///
/// Printed as well as asserted, because the interesting thing is the shape
/// across the rows and a row on its own says nothing.
#[test]
fn an_insert_costs_about_a_whole_map() {
    let empty = probe("empty(0, 0)").expect("an empty map fits");
    let range = probe("range(buckets())").expect("a range fits");
    let one = probe("set(empty(0, 0), 1, 1)").expect("one insert fits");
    let ten =
        probe("for k in range(10) with m = empty(0, 0) { set(m, k, k) }").expect("ten inserts fit");

    println!("an empty map      {empty:>8}");
    println!("one range         {range:>8}");
    println!("one insert        {one:>8}");
    println!("ten inserts       {ten:>8}");

    let per_insert = (ten - one) / 9;
    println!("per insert        {per_insert:>8}");

    // The whole of the header's claim: what an insert costs is the number of
    // buckets rather than what is in them, so it does not get cheaper as the
    // map fills and it does not get dearer either.
    assert!(
        per_insert > empty / 2,
        "an insert should cost about a whole map: {per_insert} against {empty}"
    );
}

/// The ceiling, as a count rather than as a byte total.
///
/// `std/hashmap`'s header says where it stops. This is what says so, and it
/// fails if reclamation ever arrives and moves it, which is the point: the
/// number is meant to change and to be noticed changing. It has already moved
/// twice, from fifty to two hundred when `range` stopped reading its own
/// accumulator, and from there to three hundred when the rule learned to read
/// a length and `range` stopped carrying a record to work around it.
///
/// The upper end is read as "does not run" rather than as a particular trap.
/// Four hundred keys is over the module's megabyte, and it is also more work
/// than a test's budget of instructions, so which of the two arrives first is
/// not something this is about.
#[test]
fn a_compiled_map_stops_between_three_and_four_hundred_keys() {
    let fits = probe("for k in range(300) with m = empty(0, 0) { set(m, k, k) }");
    assert!(
        fits.is_ok(),
        "three hundred keys should still fit: {fits:?}"
    );

    let over = probe("for k in range(400) with m = empty(0, 0) { set(m, k, k) }");
    assert!(
        over.is_err(),
        "four hundred keys fit now, so something started reclaiming and \
         `std/hashmap`'s header and `design/hash-map-requirements.md` should be reread"
    );
}
