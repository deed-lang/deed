//! The modules that ship inside the compiler.
//!
//! Two things can go wrong here and neither of them is loud. A module can be
//! added under `std/` and left out of the table, in which case it is a file in
//! this repository that no program can reach and the mistake looks like
//! nothing at all. And a shipped module's own tests can stop being run,
//! because a program that imports one gets it as context and context does not
//! run its tests, which is right for the program and would leave the library
//! itself checked by nobody. That second one is asked module by module rather
//! than in total, because a total says yes for as long as any one of them
//! still has a test in it, and it is asked as "wrote one and ran it" rather
//! than "ran what it wrote", because a module that wrote none satisfies the
//! second reading and `std/list` did.

use std::path::{Path, PathBuf};

use deed_ast::Item;
use deed_diagnostics::{SourceMap, Span};
use deed_driver::{check_all, shipped_modules, shipped_source};
use deed_interp::{Program, run_tests};
use deed_resolve::{DefId, DefKind};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Every `.deed` file under `std/`, by the module name its path implies.
fn files_under_std() -> Vec<String> {
    let base = root().join("std");
    let mut found = Vec::new();
    collect(&base, &base, &mut found);
    found.sort();
    assert!(!found.is_empty(), "no `.deed` files found under std/");
    found
}

fn collect(base: &Path, at: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(at) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect(base, &path, out);
        } else if path.extension().is_some_and(|ext| ext == "deed") {
            let relative = path.strip_prefix(base).expect("under the base");
            let mut name = String::from("std");
            for segment in relative.with_extension("").components() {
                name.push('/');
                name.push_str(&segment.as_os_str().to_string_lossy());
            }
            out.push(name);
        }
    }
}

#[test]
fn every_file_under_std_is_one_that_ships() {
    let mut shipping: Vec<String> = shipped_modules().map(str::to_string).collect();
    shipping.sort();

    assert_eq!(
        files_under_std(),
        shipping,
        "a module under std/ that the compiler does not carry is a file no program can reach"
    );
}

#[test]
fn what_ships_is_what_the_file_says() {
    // `include_str!` makes this true at build time. It is here because the
    // table is the one place a name and a path are written next to each other,
    // and a copied line with one of them changed would ship the wrong module
    // under the right name.
    let mut compared = 0;
    for module in shipped_modules() {
        let text = shipped_source(module).expect("a module that ships has a source");
        let declared = format!("module {module}");
        assert!(
            text.lines().any(|line| line.trim_end() == declared),
            "`{module}` ships text whose own module line is not `{declared}`"
        );
        compared += 1;
    }

    // An empty table satisfies the loop above. The test next door already
    // pins the table against a directory walk that is guarded, but this one
    // should not need the one next door to be about something.
    assert!(compared > 0, "nothing ships, so nothing was compared");
}

#[test]
fn the_shipped_modules_pass_their_own_tests() {
    let mut sources = SourceMap::new();
    let mut ids = Vec::new();
    for module in shipped_modules() {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("<shipped>/{module}.deed"), text.to_string()));
    }

    let checks = check_all(&sources, &ids);
    let mut program = Program::new();
    for checked in &checks {
        assert!(
            !checked.has_errors(),
            "{} should check cleanly:\n{}",
            sources.file(checked.file).name(),
            checked
                .diagnostics
                .iter()
                .map(|d| deed_diagnostics::render_human(&sources, d))
                .collect::<Vec<_>>()
                .join("\n")
        );
        program.add(
            checked.file,
            &checked.module,
            &checked.resolutions,
            checked.guards(),
            checked.rows(),
        );
    }

    let mut passed = 0;
    for checked in &checks {
        // What the module wrote down, off the parse tree, against what running
        // it produced. A total over all of them cannot tell the difference
        // between a module's tests running and another module's tests being
        // enough to keep the total above zero, which is exactly the state this
        // file exists to notice: `std/table` arrived carrying seven tests and
        // a `passed > 0` would have said the same thing whether they ran or
        // not.
        let declared = checked
            .module
            .items
            .iter()
            .filter(|item| matches!(item, Item::Test(_)))
            .count();

        let mut ran = 0;
        for outcome in run_tests(&program, checked.file) {
            assert!(
                outcome.failure.is_none(),
                "`{}` should pass in `{}`",
                outcome.name,
                sources.file(checked.file).name()
            );
            ran += 1;
        }

        assert_eq!(
            ran,
            declared,
            "{} writes {declared} test(s) and {ran} of them ran",
            sources.file(checked.file).name()
        );

        // And that it wrote one at all. Asking whether a module's tests ran is
        // free for a module with none, which is not a hypothetical: `std/list`
        // shipped without a test of its own and what stood behind it was
        // `examples/using_list.deed`, a program about using the library. That
        // file is the corpus rather than the library, and a program that
        // imports `std/list` from outside this repository does not get it.
        assert!(
            declared > 0,
            "{} ships without a test of its own, so nothing here answers for it",
            sources.file(checked.file).name()
        );
        passed += ran;
    }

    assert!(
        passed > 0,
        "the shipped modules ran no tests, so this proves nothing about them"
    );
}

/// Every function a shipped module declares is named by one of its own tests.
///
/// A module that carries tests is not the same as a module whose promises are
/// written down. `std/table` carried seven and none of them said `size`; the
/// list library called `prepend` from `reversed` and from nowhere a test
/// could see. Nothing here has a private half, so every `fn` in one of these
/// files is part of what the compiler hands to a program that imports it, and
/// a function nothing tests is a promise nobody made.
///
/// Named rather than reached. A helper called by a function a test calls does
/// run, but what runs it is another claim in the same file, and the point of
/// asking is that each name in the surface says for itself what it answers.
#[test]
fn every_function_a_shipped_module_declares_is_named_by_one_of_its_tests() {
    let mut sources = SourceMap::new();
    let mut ids = Vec::new();
    for module in shipped_modules() {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("<shipped>/{module}.deed"), text.to_string()));
    }

    let mut asked = 0;
    for checked in check_all(&sources, &ids) {
        let tests: Vec<Span> = checked
            .module
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Test(test) => Some(test.span),
                _ => None,
            })
            .collect();

        // Where each name in this file is mentioned, off the resolver rather
        // than off the text, so that a function whose name is a word inside a
        // string or a comment is not counted as tested by it.
        let mentioned: Vec<(Span, DefId)> = checked.resolutions.names().collect();

        for (def, data) in checked.resolutions.defs() {
            if data.kind != DefKind::Function {
                continue;
            }
            let named = mentioned.iter().any(|(span, mention)| {
                *mention == def
                    && tests
                        .iter()
                        .any(|test| span.start >= test.start && span.end <= test.end)
            });
            assert!(
                named,
                "`{}` in {} is not named by any test in that file, so nothing in the library says what it answers",
                data.name,
                sources.file(checked.file).name()
            );
            asked += 1;
        }
    }

    assert!(
        asked > 0,
        "no shipped module declares a function, so this asked nothing"
    );
}

/// How much of the library the compiled backend actually runs.
///
/// Compiling a module is not compiling its generic functions. Those are
/// lowered once per set of type arguments, so a module full of them can build
/// cleanly, be counted in "the backend compiles the corpus", and have no
/// generic body ever put through the backend at all. `std/map` was twenty
/// tests and two of them ran.
///
/// Named per module rather than totalled, for the same reason
/// `crates/deed-driver/tests/corpus_tests.rs` names files: a number that fell
/// says one stopped running, and a list says which. Every one of them runs
/// now, so this is the whole library and it is worth keeping that way.
#[test]
fn the_compiled_backend_runs_the_whole_library() {
    let mut sources = SourceMap::new();
    let mut ids = Vec::new();
    for module in shipped_modules() {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("<shipped>/{module}.deed"), text.to_string()));
    }
    let checks = check_all(&sources, &ids);

    for (at, module) in shipped_modules().enumerate() {
        let subject = &checks[at];
        let alongside: Vec<deed_mir::Alongside<'_>> = checks
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != at)
            .map(|(_, checked)| deed_mir::Alongside {
                module: &checked.module,
                resolutions: &checked.resolutions,
                types: &checked.types,
            })
            .collect();

        let written = subject
            .module
            .items
            .iter()
            .filter(|item| matches!(item, Item::Test(_)))
            .count();

        let lowered = deed_mir::lower_with_tests_alongside(
            &subject.module,
            &subject.resolutions,
            &subject.types,
            &alongside,
        )
        .unwrap_or_else(|refused| panic!("`{module}` should lower: {refused}"));
        assert_eq!(
            lowered.tests.len(),
            written,
            "`{module}` writes {written} tests and the backend lowered {}",
            lowered.tests.len()
        );

        let compiled = deed_codegen::compile(&lowered)
            .unwrap_or_else(|refused| panic!("`{module}` should compile: {refused}"));
        for test in &lowered.tests {
            assert!(
                deed_codegen::call(&compiled, &test.body, &[]).is_ok(),
                "`{}` in `{module}` passes in the interpreter and not in the backend",
                test.name
            );
        }
    }
}
