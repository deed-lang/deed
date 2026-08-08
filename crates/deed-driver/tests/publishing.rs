//! The manifests, held against the one thing `cargo publish` insists on.
//!
//! A published crate cannot depend on a path, so every internal dependency
//! needs a version next to it. A version repeated in twenty manifests is
//! nineteen places to forget at release time, and this repository already
//! keeps the version in exactly one place, so the dependencies are declared
//! once in `[workspace.dependencies]` and inherited.
//!
//! That trade buys a new way to be wrong: the table can drift from the version
//! above it, and cargo would happily publish `deed-lang 0.3.0` asking for
//! `deed-ast 0.2.9`. So the table is compared to `workspace.package.version`
//! here rather than checked by hand at release time.
//!
//! The publish itself is measured, not inferred: `cargo publish --workspace
//! --dry-run` packages and builds all twenty, and CI runs it.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root should be two directories up")
        .to_path_buf()
}

fn manifest() -> String {
    std::fs::read_to_string(root().join("Cargo.toml")).expect("the workspace manifest")
}

/// The value of a `key = "value"` line in a section.
fn field(text: &str, section: &str, key: &str) -> String {
    let body = text
        .split_once(section)
        .map(|(_, rest)| rest.split("\n[").next().unwrap_or(rest))
        .unwrap_or_else(|| panic!("no {section} section"));
    body.lines()
        .find_map(|line| line.trim().strip_prefix(&format!("{key} = ")))
        .map(|value| value.trim().trim_matches('"').to_string())
        .unwrap_or_else(|| panic!("no {key} in {section}"))
}

/// Every entry of `[workspace.dependencies]`, as name, path and version.
fn declared() -> Vec<(String, String, String)> {
    let text = manifest();
    let body = text
        .split_once("[workspace.dependencies]")
        .map(|(_, rest)| rest.split("\n[").next().unwrap_or(rest))
        .expect("no [workspace.dependencies] section");

    let mut found = Vec::new();
    for line in body.lines() {
        let Some((name, rest)) = line.trim().split_once(" = {") else {
            continue;
        };
        let value = |key: &str| {
            rest.split(key)
                .nth(1)
                .and_then(|after| after.split('"').nth(1))
                .unwrap_or_else(|| panic!("`{name}` has no {key}"))
                .to_string()
        };
        found.push((name.to_string(), value("path = "), value("version = ")));
    }
    assert!(
        found.len() > 10,
        "only {} internal dependencies were declared",
        found.len()
    );
    found
}

#[test]
fn every_internal_dependency_asks_for_the_version_this_workspace_is() {
    let version = field(&manifest(), "[workspace.package]", "version");
    for (name, _, asked) in declared() {
        assert_eq!(
            asked, version,
            "`{name}` asks for {asked} and this workspace is {version}"
        );
    }
}

#[test]
fn every_internal_dependency_points_at_the_crate_of_that_name() {
    for (name, path, _) in declared() {
        assert_eq!(path, format!("crates/{name}"), "`{name}` points elsewhere");
        assert!(
            root().join(&path).join("Cargo.toml").is_file(),
            "`{name}` points at `{path}` and there is no crate there"
        );
    }
}

/// No crate keeps its own copy of where a sibling lives.
///
/// A bare `path` is what `cargo publish` refuses, and it is also the shape
/// that used to make the version question have twenty answers.
/// Every `-p name` in a workflow names a package that is here.
///
/// A package can be renamed without anything in the tree noticing, because
/// nothing in the tree spells the name: the workflows do, and they would fail
/// on a tag rather than on the commit that renamed it.
#[test]
fn the_workflows_build_packages_that_exist() {
    let names: Vec<String> = std::fs::read_dir(root().join("crates"))
        .expect("crates/")
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_to_string(entry.path().join("Cargo.toml")).ok())
        .map(|text| field(&text, "[package]", "name"))
        .collect();

    let mut asked = 0;
    for entry in std::fs::read_dir(root().join(".github").join("workflows")).expect("workflows/") {
        let path = entry.expect("an entry").path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (_, after) in text
            .match_indices("-p ")
            .map(|(at, _)| text.split_at(at + 3))
        {
            let name = after
                .split(|c: char| c.is_whitespace())
                .next()
                .unwrap_or_default();
            if !name.starts_with("deed-") {
                continue;
            }
            asked += 1;
            assert!(
                names.iter().any(|package| package == name),
                "{} builds `{name}` and no crate here is called that",
                path.display()
            );
        }
    }

    assert!(asked > 2, "only {asked} workflow package names were read");
}

#[test]
fn no_crate_names_a_sibling_by_path_alone() {
    let names: Vec<String> = declared().into_iter().map(|(name, _, _)| name).collect();
    let mut inherited = 0;

    for entry in std::fs::read_dir(root().join("crates")).expect("crates/") {
        let path = entry.expect("an entry").path().join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            assert!(
                !line.contains("path = \"../"),
                "{} names a sibling by path, which cannot be published",
                path.display()
            );
            if let Some(name) = line.strip_suffix(".workspace = true") {
                if name.starts_with("deed-") {
                    assert!(
                        names.iter().any(|declared| declared == name),
                        "{} inherits `{name}`, which the workspace does not declare",
                        path.display()
                    );
                    inherited += 1;
                }
            }
        }
    }

    assert!(inherited > 20, "only {inherited} dependencies were checked");
}
