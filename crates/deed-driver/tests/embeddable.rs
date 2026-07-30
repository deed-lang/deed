//! `check_all` reachable from named strings alone, no filesystem involved.
//!
//! Part of issue #585. A page has no disk, so whatever a wasm build ends up
//! exposing has to be this function, not a copy of it. This test plays the
//! part of that third caller: it builds a [`SourceMap`] out of string literals
//! and the shipped modules' embedded text, then calls the exact function the
//! CLI (`deed-cli`) and the language server (`deed-lsp`'s `check_workspace`)
//! already call. No `std::fs` appears anywhere in this file.

use deed_diagnostics::SourceMap;
use deed_driver::{check_all, shipped_modules, shipped_source};

#[test]
fn a_program_and_every_shipped_module_check_from_strings_alone() {
    let mut sources = SourceMap::new();
    let mut ids = Vec::new();

    // The "page" hands over one file by name and content, nothing else.
    ids.push(sources.add(
        "main.deed".to_string(),
        "module main\n\nfn main() -> Int {\n    1 + 1\n}\n".to_string(),
    ));

    // The shipped library ships as text embedded in the binary
    // (`include_str!`, see shipped.rs), never as a path to open.
    for module in shipped_modules() {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("{module}.deed"), text.to_string()));
    }

    let checked = check_all(&sources, &ids);
    assert_eq!(
        checked.len(),
        ids.len(),
        "one Checked per named source, nothing dropped and nothing added"
    );
    for entry in &checked {
        assert!(
            !entry.has_errors(),
            "{} should check cleanly:\n{}",
            sources.file(entry.file).name(),
            entry
                .diagnostics
                .iter()
                .map(|d| deed_diagnostics::render_human(&sources, d))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
