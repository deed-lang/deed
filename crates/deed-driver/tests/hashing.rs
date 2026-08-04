//! `hash`, which is the equality walk with a different accumulator.
//!
//! The argument is
//! `design/decisions/2026-08-05-a-hash-is-the-equality-walk.md`. What is held
//! here is the property that makes a hash usable at all — equal values hash
//! equal — and the two refusals that keep it true.
//!
//! The exact numbers are not here. They are in
//! `crates/deed-driver/tests/agreement.rs`, where every entry is checked
//! against both engines, because a hash that two engines computed differently
//! would be a program that passes in one and fails in the other.

use deed_diagnostics::SourceMap;
use deed_driver::{Checked, check_all};
use deed_interp::{Program, run_tests};

fn checked(source: &str) -> (SourceMap, Vec<Checked>, deed_diagnostics::FileId) {
    let mut sources = SourceMap::new();
    let subject = sources.add("probe.deed".to_string(), source.to_string());
    let checks = check_all(&sources, &[subject]);
    (sources, checks, subject)
}

/// The codes a program is refused with.
fn refused(source: &str) -> Vec<String> {
    let (_, checks, _) = checked(source);
    checks
        .iter()
        .flat_map(|one| one.diagnostics.iter())
        .filter(|diagnostic| diagnostic.is_error())
        .map(|diagnostic| diagnostic.code.to_string())
        .collect()
}

/// The names of the `test` blocks that failed, so success is an empty list.
fn interpreted(source: &str) -> Vec<String> {
    let (_, checks, subject) = checked(source);
    for one in &checks {
        assert!(
            !one.has_errors(),
            "this program should check: {:?}",
            one.diagnostics
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
        );
    }

    let mut program = Program::new();
    for one in &checks {
        program.add(
            one.file,
            &one.module,
            &one.resolutions,
            one.guards(),
            one.rows(),
            one.operators(),
        );
    }
    run_tests(&program, subject)
        .into_iter()
        .filter(|outcome| !outcome.passed())
        .map(|outcome| outcome.name.clone())
        .collect()
}

/// The property the whole thing exists for.
///
/// Written as a program rather than as a table of numbers, because what has to
/// hold is a relation between two calls and not a value. Every shape that can
/// hold another one is here, because the walk recurses and a shape that
/// stopped recursing would still pass on the flat ones.
#[test]
fn equal_values_hash_equal() {
    let source = "module a\n\n\
         record Point { x: Int, y: Int }\n\n\
         choice Tone {\n\
         \x20   Plain { at: Point },\n\
         \x20   Loud { at: Point },\n\
         }\n\n\
         test \"numbers, text and booleans\" {\n\
         \x20   assert hash(7) == hash(7)\n\
         \x20   assert hash(\"abc\") == hash(\"abc\")\n\
         \x20   assert hash(true) == hash(true)\n\
         }\n\n\
         test \"a list, and a list of lists\" {\n\
         \x20   assert hash([1, 2, 3]) == hash([1, 2, 3])\n\
         \x20   assert hash([[1], [2]]) == hash([[1], [2]])\n\
         }\n\n\
         test \"a record, however it was written\" {\n\
         \x20   assert hash(Point { x: 1, y: 2 }) == hash(Point { y: 2, x: 1 })\n\
         }\n\n\
         test \"a variant, and what it holds\" {\n\
         \x20   assert hash(Plain { at: Point { x: 1, y: 2 } }) == hash(Plain { at: Point { x: 1, y: 2 } })\n\
         }\n\n\
         test \"a Result\" {\n\
         \x20   assert hash(ok(1)) == hash(ok(1))\n\
         }\n";

    assert_eq!(interpreted(source), Vec::<String>::new());
}

/// The other half, and the half a broken walk would still pass without.
///
/// A hash that absorbed nothing would make every one of these equal, and the
/// test above would not notice. Each pair here is two values equality tells
/// apart, so each is a part of the value the walk has to have read.
#[test]
fn values_that_differ_hash_apart() {
    let source = "module a\n\n\
         record Point { x: Int, y: Int }\n\n\
         choice Tone {\n\
         \x20   Plain { n: Int },\n\
         \x20   Loud { n: Int },\n\
         }\n\n\
         test \"the length, which is what nesting comes down to\" {\n\
         \x20   assert hash([[1], [2]]) != hash([[1, 2]])\n\
         \x20   assert hash([\"a\", \"bc\"]) != hash([\"ab\", \"c\"])\n\
         }\n\n\
         test \"which field a value sits in\" {\n\
         \x20   assert hash(Point { x: 1, y: 2 }) != hash(Point { x: 2, y: 1 })\n\
         }\n\n\
         test \"which variant it is\" {\n\
         \x20   assert hash(Plain { n: 1 }) != hash(Loud { n: 1 })\n\
         }\n\n\
         test \"which side of a Result\" {\n\
         \x20   assert hash(ok(1)) != hash(err(1))\n\
         }\n";

    assert_eq!(interpreted(source), Vec::<String>::new());
}

/// Sequential keys are what a map meets first, and a fold whose low bits did
/// not move under them would put every one of them in the same bucket.
#[test]
fn sequential_keys_do_not_pile_into_one_bucket() {
    let source = "module a\n\n\
         fn bucket(key: Int) -> Int {\n\
         \x20   let h = hash(key)\n\
         \x20   if h < 0 { 0 - h % 8 } else { h % 8 }\n\
         }\n\n\
         test \"eight keys do not land on one\" {\n\
         \x20   let seen = for k in [0, 1, 2, 3, 4, 5, 6, 7] with out = [] {\n\
         \x20       push(out, bucket(k))\n\
         \x20   }\n\
         \x20   assert length(seen) == 8\n\
         \x20   assert at(seen, 0) != at(seen, 1)\n\
         \x20   assert at(seen, 1) != at(seen, 2)\n\
         \x20   assert at(seen, 2) != at(seen, 3)\n\
         }\n";

    assert_eq!(interpreted(source), Vec::<String>::new());
}

// -- what has nothing to read ---------------------------------------------

/// A function value is equal to another when it is the same one, so the only
/// thing to hash would be an address.
#[test]
fn a_function_value_cannot_be_hashed() {
    let source = "module a\n\n\
         fn twice(n: Int) -> Int { n + n }\n\n\
         fn answer() -> Int {\n\
         \x20   let f = twice\n\
         \x20   hash(f)\n\
         }\n";

    assert!(
        refused(source).contains(&"DEED4032".to_string()),
        "{:?}",
        refused(source)
    );
}

/// A capability holds nothing a program is allowed to look at, which is what
/// makes being handed one mean something.
#[test]
fn a_capability_cannot_be_hashed() {
    let source = "module a\n\n\
         fn answer(out: Console) -> Int {\n\
         \x20   hash(out)\n\
         }\n";

    assert!(
        refused(source).contains(&"DEED4032".to_string()),
        "{:?}",
        refused(source)
    );
}

/// And the refusal is about what it was given rather than about the name, so
/// everything else still goes through.
#[test]
fn everything_else_is_still_hashable() {
    let source = "module a\n\n\
         record Point { x: Int, y: Int }\n\n\
         fn answer() -> Int {\n\
         \x20   hash(1) + hash(\"a\") + hash(true) + hash([1]) + hash(Point { x: 1, y: 2 })\n\
         }\n";

    assert_eq!(refused(source), Vec::<String>::new());
}

/// One argument, because a hash is about one value.
#[test]
fn hash_takes_one_thing() {
    let source = "module a\n\nfn answer() -> Int { hash(1, 2) }\n";
    assert!(!refused(source).is_empty());
}
