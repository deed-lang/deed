//! What monomorphization costs, measured rather than assumed.
//!
//! A generic function is lowered once per set of type arguments it is
//! called with, so the obvious worry is that a program using generics
//! heavily compiles into something much larger than it looks. That is worth
//! a number rather than a shrug, and the number is what decides whether the
//! alternative, a single copy taking everything by pointer, is worth
//! building.
//!
//! Two numbers, and they answer different questions. Copies grow with
//! distinct type arguments, not with call sites, which is the claim that
//! makes monomorphization affordable. And the corpus is where that claim
//! meets a program somebody wrote rather than one written to make a point.

use deed_codegen::compile;
use deed_diagnostics::SourceMap;
use deed_driver::check_all;

/// Compiles a source and hands back how many functions and how many bytes
/// it came out as.
fn measured(source: &str) -> (usize, usize) {
    let mut sources = SourceMap::new();
    let id = sources.add("growth.deed".to_string(), source.to_string());
    let mut all = check_all(&sources, &[id]);
    let one = all.pop().expect("one file in, one result out");
    assert!(
        !one.has_errors(),
        "this program should check: {:?}",
        one.diagnostics
    );

    let lowered = deed_mir::lower(&one.module, &one.resolutions, &one.types).expect("this lowers");
    let module = compile(&lowered).expect("this compiles");
    (lowered.functions.len(), module.encode().len())
}

/// A program calling one generic function `calls` times, all at `Int`.
fn at_one_type(calls: usize) -> String {
    let mut source =
        "module a\n\nfn count_of<T>(items: List<T>) -> Int { length(items) }\n\n".to_string();
    source.push_str("fn answer() -> Int {\n    0");
    for _ in 0..calls {
        source.push_str(" + count_of([1, 2])");
    }
    source.push_str("\n}\n");
    source
}

/// The thing that makes monomorphization affordable.
///
/// If copies grew with call sites this would be unusable, since a helper
/// called in twenty places would be twenty bodies. They grow with distinct
/// type arguments, so a program calling one generic function forty times at
/// one type has the same number of functions as one calling it once, and
/// what is left growing is the call sites themselves, at a fixed price each.
#[test]
fn calling_a_generic_function_more_often_does_not_make_more_copies() {
    let (once, small) = measured(&at_one_type(1));
    let (twenty, medium) = measured(&at_one_type(20));
    let (forty, large) = measured(&at_one_type(40));

    assert_eq!(
        (once, once),
        (twenty, forty),
        "one call, twenty and forty at the same type should all be {once} functions"
    );

    // Twenty more calls cost the same each as the nineteen before them. A
    // copy per call site would make the later ones dearer, and this is the
    // number that would say so.
    let first = (medium - small) / 19;
    let second = (large - medium) / 20;
    assert!(
        second.abs_diff(first) <= 1,
        "the first nineteen calls cost {first} bytes each and the next twenty cost \
         {second} each, which is not a fixed price per call site"
    );
}

/// The other half: distinct type arguments do cost a body each, and that is
/// the price being paid on purpose.
#[test]
fn each_set_of_type_arguments_costs_one_more_body() {
    let (one, _) = measured(
        "module a\n\nfn count_of<T>(items: List<T>) -> Int { length(items) }\n\n\
         fn answer() -> Int { count_of([1]) }\n",
    );
    let (two, _) = measured(
        "module a\n\nfn count_of<T>(items: List<T>) -> Int { length(items) }\n\n\
         fn answer() -> Int { count_of([1]) + count_of([true]) }\n",
    );

    assert_eq!(
        two,
        one + 1,
        "a second element type should be a second body and nothing more"
    );
}

/// What the corpus actually compiles into.
///
/// A ceiling rather than a target, and a loose one: what it catches is a
/// change that makes a module several times larger without anybody
/// noticing. The numbers are small because the programs are, and the point
/// of writing them down is that the next person to change the backend can
/// see whether they moved.
#[test]
fn a_corpus_program_compiles_into_something_the_size_of_a_corpus_program() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("the workspace root is two up from this crate");

    let mut sizes: Vec<(String, usize)> = Vec::new();
    for directory in ["examples", "std"] {
        let entries = match std::fs::read_dir(root.join(directory)) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("deed") {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };

            let mut sources = SourceMap::new();
            let id = sources.add(path.display().to_string(), source);
            let mut all = check_all(&sources, &[id]);
            let Some(one) = all.pop() else { continue };
            if one.has_errors() {
                continue;
            }
            let Ok(lowered) = deed_mir::lower(&one.module, &one.resolutions, &one.types) else {
                continue;
            };
            let Ok(module) = compile(&lowered) else {
                continue;
            };

            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("?")
                .to_string();
            sizes.push((name, module.encode().len()));
        }
    }

    assert!(
        !sizes.is_empty(),
        "nothing in the corpus compiled, so there was nothing to measure"
    );

    // A quarter of a megabyte for a file of a few hundred lines would mean
    // something is being copied that should not be.
    for (name, size) in &sizes {
        assert!(
            *size < 256 * 1024,
            "`{name}` compiled into {size} bytes, which is not the size of a program \
             this small; something is being copied per call rather than per type"
        );
    }
}
