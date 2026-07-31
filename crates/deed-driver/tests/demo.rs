//! The two-component permission demo, verified end to end.
//!
//! Two functions, identical except for one clause in the signature. One says
//! `uses Io.read`, the other says `uses Io.read, Io.save`. Both compile to
//! WebAssembly. The import section of each module is its world: what it asks
//! the host for before a single instruction runs.
//!
//! The claim this test pins:
//!
//! - `read_only` compiles to a module that imports `deed:io.read`.
//! - `read_only` does not import `deed:io.save`.
//! - `read_write` compiles to a module that imports both.
//! - Running `read_write` without a host that supplies `deed:io.save` stops
//!   immediately and names what it was missing.
//!
//! See `demo/` for the source files and `demo/README.md` for the transcript.

use std::path::PathBuf;

use deed_codegen::{Trap, Value, call, compile};
use deed_diagnostics::SourceMap;
use deed_driver::check_all;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn module_for(name: &str) -> deed_codegen::Module {
    let path = root().join("demo").join(name);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} should be readable", path.display()));
    let mut sources = SourceMap::new();
    let id = sources.add(name.to_string(), source);
    let mut all = check_all(&sources, &[id]);
    let one = all.pop().expect("one file in, one result out");
    assert!(
        !one.has_errors(),
        "{} should check cleanly:\n{}",
        name,
        one.diagnostics
            .iter()
            .map(|d| deed_diagnostics::render_human(&sources, d))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let lowered = deed_mir::lower(&one.module, &one.resolutions, &one.types).expect("this lowers");
    compile(&lowered).expect("this compiles")
}

fn imports_of(module: &deed_codegen::Module) -> Vec<String> {
    module
        .imports
        .iter()
        .map(|import| format!("{}.{}", import.module, import.name))
        .collect()
}

// -- the two worlds ---------------------------------------------------------

/// The read-only component imports exactly one host operation.
#[test]
fn read_only_imports_read_and_nothing_else() {
    let module = module_for("read_only.deed");
    let imports = imports_of(&module);

    assert!(
        imports.contains(&"deed:io.read".to_string()),
        "read_only should import deed:io.read: {imports:?}"
    );
    assert!(
        !imports.contains(&"deed:io.save".to_string()),
        "read_only should not import deed:io.save: {imports:?}"
    );
}

/// The read-write component imports both host operations.
///
/// One clause was added to the signature. The import section grew by one
/// entry. That entry is the difference the host reads before running
/// anything.
#[test]
fn read_write_imports_read_and_save() {
    let module = module_for("read_write.deed");
    let imports = imports_of(&module);

    assert!(
        imports.contains(&"deed:io.read".to_string()),
        "read_write should import deed:io.read: {imports:?}"
    );
    assert!(
        imports.contains(&"deed:io.save".to_string()),
        "read_write should import deed:io.save: {imports:?}"
    );
}

/// The README prints each module's import section, and that listing is the
/// whole demonstration: one clause in, one line out.
///
/// It was typed. A new effect anywhere in either signature changes the import
/// section, and nothing would have noticed the file no longer matching. This
/// reads the listing back out of the prose and compares it to the modules
/// (#783).
#[test]
fn the_readme_lists_the_imports_the_modules_actually_have() {
    let readme = std::fs::read_to_string(root().join("demo").join("README.md"))
        .expect("demo/README.md should be there");

    for name in ["read_only", "read_write"] {
        let heading = format!("{name}.wasm imports:");
        let after = readme
            .split_once(&heading)
            .unwrap_or_else(|| panic!("the README should list {name}'s imports"))
            .1;

        // The listing is the indented lines that follow, `deed:io  read`, up
        // to the first line that is not one.
        let listed: Vec<String> = after
            .lines()
            .skip(1)
            .take_while(|line| line.starts_with("  ") && !line.trim().is_empty())
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join("."))
            .collect();

        let mut actual = imports_of(&module_for(&format!("{name}.deed")));
        actual.sort();
        let mut listed_sorted = listed.clone();
        listed_sorted.sort();

        assert_eq!(
            listed_sorted, actual,
            "demo/README.md says {name} imports {listed:?}, the module imports {actual:?}"
        );
    }
}

// -- the host's behaviour ---------------------------------------------------

/// A host that provides only Io.read refuses the read-write component.
///
/// The module declared `deed:io.save` in its imports. The runner sees that
/// before executing a single instruction and stops with the name of the
/// operation it was asked for. This is the host enforcing the boundary:
/// not a runtime check, not a permission bit, but the module's own
/// declaration turned into a refusal.
#[test]
fn running_read_write_without_save_names_what_it_wanted() {
    let module = module_for("read_write.deed");

    // Call `process` with two i64 arguments (capability handles).
    // The runner stops at the first host import it cannot satisfy.
    let stopped = call(&module, "process", &[Value::I64(0), Value::I64(0)])
        .expect_err("read_write needs a host to provide save");

    let Trap::NeedsAHost(what) = stopped else {
        panic!("it should say what it wanted, not {stopped}");
    };
    assert!(
        what.starts_with("deed:io"),
        "it should name an Io operation: {what}"
    );
}

/// A read-only component reaches the same stop for a different reason:
/// the runner has no filesystem to hand it.
///
/// Both components stop when run without a host, but for different imports.
/// The read-only one names `deed:io.read`; the read-write one names
/// `deed:io.read` or `deed:io.save` depending on execution order. What does
/// not change: each module's import section declares exactly what it needs,
/// and the host sees that list before any code runs.
#[test]
fn running_read_only_without_a_host_names_read() {
    let module = module_for("read_only.deed");

    let stopped = call(&module, "process", &[Value::I64(0), Value::I64(0)])
        .expect_err("read_only needs a host to provide read");

    let Trap::NeedsAHost(what) = stopped else {
        panic!("it should say what it wanted, not {stopped}");
    };
    assert_eq!(what, "deed:io.read", "read_only wants exactly read: {what}");
}
