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

use deed_codegen::{Host, Trap, Value, call, compile};
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

/// The README also prints this file's test names, and that listing was typed.
///
/// It said `running 5 tests` and named one that no longer exists, which is how
/// a transcript nobody reads back goes stale. The import listing above has been
/// held since #783; this is the other listing on the same page.
#[test]
fn the_readme_lists_the_tests_this_file_declares() {
    let readme = std::fs::read_to_string(root().join("demo").join("README.md"))
        .expect("demo/README.md should be there");
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("demo.rs"),
    )
    .expect("this file should be readable");

    // Spelled in two pieces so this line is not itself one of the markers it
    // counts.
    let marker = concat!("#[", "test]");
    let mut declared: Vec<String> = source
        .split(marker)
        .skip(1)
        .filter_map(|after| after.split_once("fn "))
        .filter_map(|(_, rest)| rest.split_once('('))
        .map(|(name, _)| name.trim().to_string())
        .collect();
    assert!(declared.len() > 1, "the source should declare tests");

    let after = readme
        .split_once("$ cargo test -p deed-driver --test demo\n")
        .expect("the README should show the transcript")
        .1;
    let mut listed: Vec<String> = after
        .lines()
        .skip(1)
        .take_while(|line| line.starts_with("test "))
        .filter_map(|line| line.split_whitespace().nth(1).map(str::to_string))
        .collect();

    assert!(
        after.starts_with(&format!("running {} tests\n", declared.len())),
        "the README says `{}`, and this file declares {} tests",
        after.lines().next().unwrap_or(""),
        declared.len()
    );

    declared.sort();
    listed.sort();
    assert_eq!(
        listed, declared,
        "demo/README.md names {listed:?}, this file declares {declared:?}"
    );
}

// -- the host's behaviour ---------------------------------------------------

/// A host that answers `Io.read` and not `Io.save` refuses the read-write
/// component, and says which operation it could not meet.
///
/// This is the sentence `demo/README.md` is built on, and until #964 nothing
/// demonstrated it. What was here called itself
/// `running_read_write_without_save_names_what_it_wanted`, ran the module with
/// no host at all, and asserted only that the answer began `deed:io`. Measured:
/// it said `deed:io.read`, because `process` reads before it saves. The test
/// named for `save` never mentioned `save`, and the operation it did name is
/// the one both modules share — the opposite of the difference the demo is
/// about.
///
/// `Host::link` is the mechanism the prose describes: it reads the whole
/// import section and refuses before an instruction runs, rather than stopping
/// at whichever import execution reaches first. A host that stopped on the
/// first one would already have read the file.
#[test]
fn a_host_without_save_refuses_read_write_and_names_save() {
    let mut host = Host::new();
    host.offer("deed:io", "read", |_| Ok(Some(Value::I64(0))));

    let refused = host
        .link(&module_for("read_write.deed"))
        .expect_err("a host that cannot save should refuse the module that saves");

    assert_eq!(refused.module, "deed:io");
    assert_eq!(
        refused.name, "save",
        "the refusal should name the clause that is the difference"
    );
}

/// And the same host runs the read-only component without restriction.
///
/// Half of the claim is the refusal; this is the other half. A host that
/// refused both would be a host with no filesystem, which demonstrates
/// nothing about the clause.
#[test]
fn the_same_host_links_read_only_without_restriction() {
    let mut host = Host::new();
    host.offer("deed:io", "read", |_| Ok(Some(Value::I64(0))));

    assert!(
        host.link(&module_for("read_only.deed")).is_ok(),
        "read_only asks for nothing this host withheld"
    );
}

/// With no host at all, a module stops at the first import it reaches.
///
/// Kept because it is the weaker guarantee and it is worth being able to tell
/// the two apart: this one is a trap during execution, and the two above are a
/// refusal before it. Both modules name `deed:io.read` here, which is why this
/// shape cannot demonstrate what the clause bought.
#[test]
fn running_without_a_host_stops_at_the_first_import_either_way() {
    for name in ["read_only.deed", "read_write.deed"] {
        let stopped = call(
            &module_for(name),
            "process",
            &[Value::I64(0), Value::I64(0)],
        )
        .expect_err("neither runs without a host");

        let Trap::NeedsAHost(what) = stopped else {
            panic!("it should say what it wanted, not {stopped}");
        };
        assert_eq!(what, "deed:io.read", "{name} reaches read first");
    }
}
