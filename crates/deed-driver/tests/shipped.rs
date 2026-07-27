//! The modules that ship inside the compiler.
//!
//! Two things can go wrong here and neither of them is loud. A module can be
//! added under `std/` and left out of the table, in which case it is a file in
//! this repository that no program can reach and the mistake looks like
//! nothing at all. And a shipped module's own tests can stop being run,
//! because a program that imports one gets it as context and context does not
//! run its tests, which is right for the program and would leave the library
//! itself checked by nobody.

use std::path::{Path, PathBuf};

use deed_diagnostics::SourceMap;
use deed_driver::{check_all, shipped_modules, shipped_source};
use deed_interp::{Program, run_tests};

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
    for module in shipped_modules() {
        let text = shipped_source(module).expect("a module that ships has a source");
        let declared = format!("module {module}");
        assert!(
            text.lines().any(|line| line.trim_end() == declared),
            "`{module}` ships text whose own module line is not `{declared}`"
        );
    }
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
        for outcome in run_tests(&program, checked.file) {
            assert!(
                outcome.failure.is_none(),
                "`{}` should pass in `{}`",
                outcome.name,
                sources.file(checked.file).name()
            );
            passed += 1;
        }
    }

    assert!(
        passed > 0,
        "the shipped modules ran no tests, so this proves nothing about them"
    );
}
