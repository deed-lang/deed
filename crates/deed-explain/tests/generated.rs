//! The two tests that keep `generated/pages.rs` inside this package.
//!
//! This used to be a build script. It read every `codes.rs` in the workspace
//! and the whole test corpus, which works exactly as long as the workspace is
//! there. A published `deed-explain` carries its own directory and nothing
//! else, so the same build script would have found an empty tree, generated
//! zero pages, and **compiled**: `deed explain DEED4025` would print nothing
//! at all, for every code, on every machine that installed the compiler from
//! crates.io. Measured with `cargo package -p deed-explain --list`, which
//! lists `build.rs` and `src/lib.rs` and no workspace.
//!
//! So the pages are generated, committed, and shipped as source. The reading
//! of the tree happens where the tree exists, and the generator itself lives
//! in `crates/deed-driver/tests/explain_pages.rs`, because deciding that an
//! example produces its code takes a compiler and this package depends on
//! nothing.

use std::fs;
use std::path::PathBuf;

/// The pages have to travel inside the crate.
///
/// This is the bug that started it, stated as a rule: whatever `src/lib.rs`
/// pulls the pages out of has to be a file this package carries. `OUT_DIR`
/// does not qualify, because filling it needed a workspace that a published
/// crate does not have, and neither does anything above the package root.
#[test]
fn the_pages_come_from_a_file_this_package_carries() {
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lib = fs::read_to_string(package.join("src").join("lib.rs")).expect("src/lib.rs");

    let inside = carried_path(&lib).unwrap_or_else(|why| panic!("{why}"));
    assert!(
        package.join(inside).is_file(),
        "the pages are included from a file that is not there"
    );
}

/// Where `include!` reads the pages from, relative to the package root, if
/// that is somewhere inside it.
fn carried_path(lib: &str) -> Result<String, String> {
    let argument = lib
        .split_once("include!(")
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(argument, _)| argument.trim())
        .ok_or("src/lib.rs does not include the generated pages")?;

    let path = argument
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .ok_or_else(|| {
            format!(
                "the pages are included as `{argument}`, which is not a path this package holds"
            )
        })?;

    // Resolved against `src/`, where the including file sits.
    let mut parts: Vec<&str> = vec!["src"];
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!(
                        "the pages are included from `{path}`, which leaves the package"
                    ));
                }
            }
            other => parts.push(other),
        }
    }
    Ok(parts.join("/"))
}

#[test]
fn the_rule_rejects_the_shape_this_crate_used_to_have() {
    let out_dir = carried_path("include!(concat!(env!(\"OUT_DIR\"), \"/pages.rs\"));").unwrap_err();
    assert!(
        out_dir.contains("not a path this package holds"),
        "{out_dir}"
    );

    let above = carried_path("include!(\"../../pages.rs\");").unwrap_err();
    assert!(above.contains("leaves the package"), "{above}");

    assert_eq!(
        carried_path("include!(\"../generated/pages.rs\");"),
        Ok("generated/pages.rs".to_string())
    );
}
