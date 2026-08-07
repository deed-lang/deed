//! Every prelude name and every `Io` operation, checked against the backend
//! by name rather than remembered from what got noticed.
//!
//! Part of #621. `design/05-backend.md` used to name `split` and `at`
//! specifically as missing, which is a list assembled from what somebody
//! happened to trip over. This enumerates the same way #205 and #206
//! enumerated diagnostic codes and shipped modules: read the compiler's own
//! list ([`PRELUDE`], [`IO_OPERATIONS`]), try each one through the real
//! pipeline, and pin the result by name so a change to either list is a
//! change to this file rather than a surprise somebody notices later.
//!
//! One tiny program per name rather than one program calling all of them,
//! so a name that does not even check cannot take an unrelated one down
//! with it, and so the message attached to a refusal is about the name the
//! table says it is about.

use deed_diagnostics::SourceMap;
use deed_driver::check_all;
use deed_resolve::{IO_OPERATIONS, PRELUDE};

/// Whether the backend compiles a one-function program, and what it said if
/// not.
fn tried(source: &str) -> Result<(), String> {
    let mut sources = SourceMap::new();
    let id = sources.add("probe.deed".to_string(), source.to_string());
    let checks = check_all(&sources, &[id]);
    let checked = &checks[0];
    assert!(
        !checked.has_errors(),
        "the probe itself should check cleanly:\n{}",
        checked
            .diagnostics
            .iter()
            .map(|d| deed_diagnostics::render_human(&sources, d))
            .collect::<Vec<_>>()
            .join("\n")
    );

    match deed_mir::lower(&checked.module, &checked.resolutions, &checked.types) {
        Err(why) => Err(why.to_string()),
        Ok(lowered) => match deed_codegen::compile(&lowered) {
            Err(why) => Err(why.to_string()),
            Ok(module) => {
                assert_eq!(
                    &module.encode()[..4],
                    b"\0asm",
                    "a program the backend accepted should encode into a module"
                );
                Ok(())
            }
        },
    }
}

/// One line of source calling the prelude function named, minimal enough to
/// type check and nothing else.
///
/// `Int`, `String`, `Bool`, `Result`, `List`, `System`, `Console`, `Clock`
/// and `Dir` are left out: they are type names rather than functions to
/// call, and whether a value of each can pass through the backend is a
/// question `capabilities.rs`, `generics.rs`, `result.rs` and `lists.rs`
/// already ask about directly, not one this table can ask by calling it.
fn probe_for(name: &str) -> Option<&'static str> {
    Some(match name {
        "ok" => "module probe\n\nfn f() -> Result<Int, String> {\n    ok(1)\n}\n",
        "err" => "module probe\n\nfn f() -> Result<Int, String> {\n    err(\"no\")\n}\n",
        "length" => "module probe\n\nfn f(s: String) -> Int {\n    length(s)\n}\n",
        "at" => "module probe\n\nfn f(xs: List<Int>) -> Result<Int, String> {\n    at(xs, 0)\n}\n",
        "push" => "module probe\n\nfn f(xs: List<Int>) -> List<Int> {\n    push(xs, 1)\n}\n",
        "repeat" => "module probe\n\nfn f() -> List<Int> {\n    repeat(1, 3)\n}\n",
        "split" => "module probe\n\nfn f(s: String) -> List<String> {\n    split(s, \",\")\n}\n",
        "join" => "module probe\n\nfn f(xs: List<String>) -> String {\n    join(xs, \",\")\n}\n",
        "trim" => "module probe\n\nfn f(s: String) -> String {\n    trim(s)\n}\n",
        "upper" => "module probe\n\nfn f(s: String) -> String {\n    upper(s)\n}\n",
        "lower" => "module probe\n\nfn f(s: String) -> String {\n    lower(s)\n}\n",
        "to_string" => "module probe\n\nfn f(n: Int) -> String {\n    to_string(n)\n}\n",
        "to_int" => "module probe\n\nfn f(s: String) -> Result<Int, String> {\n    to_int(s)\n}\n",
        _ => return None,
    })
}

/// One line of source performing the `Io` operation named, taking `sys:
/// System` and narrowing only as far as the operation needs, the way
/// `examples/hello.deed` and `examples/journal.deed` do.
fn probe_for_io(operation: &str) -> &'static str {
    match operation {
        "write" => {
            "module probe\n\nfn f(sys: System) -> ()\n  uses\n    Io.write,\n{\n    Io.write(sys.console, \"x\")\n}\n"
        }
        "line" => {
            "module probe\n\nfn f(sys: System) -> Result<String, String>\n  uses\n    Io.line,\n{\n    Io.line(sys.console)\n}\n"
        }
        "now" => {
            "module probe\n\nfn f(sys: System) -> Int\n  uses\n    Io.now,\n{\n    Io.now(sys.clock)\n}\n"
        }
        "epoch" => {
            "module probe\n\nfn f(sys: System) -> Int\n  uses\n    Io.epoch,\n{\n    Io.epoch(sys.clock)\n}\n"
        }
        "open" => {
            "module probe\n\nfn f(sys: System) -> Result<Dir, String>\n  uses\n    Io.open,\n{\n    Io.open(sys.files, \"x\")\n}\n"
        }
        "read" => {
            "module probe\n\nfn f(sys: System) -> Result<String, String>\n  uses\n    Io.read,\n{\n    Io.read(sys.files, \"x\")\n}\n"
        }
        "save" => {
            "module probe\n\nfn f(sys: System) -> Result<(), String>\n  uses\n    Io.save,\n{\n    Io.save(sys.files, \"x\", \"y\")\n}\n"
        }
        "remove" => {
            "module probe\n\nfn f(sys: System) -> Result<(), String>\n  uses\n    Io.remove,\n{\n    Io.remove(sys.files, \"x\")\n}\n"
        }
        "make" => {
            "module probe\n\nfn f(sys: System) -> Result<Dir, String>\n  uses\n    Io.make,\n{\n    Io.make(sys.files, \"x\")\n}\n"
        }
        "list" => {
            "module probe\n\nfn f(sys: System) -> Result<List<String>, String>\n  uses\n    Io.list,\n{\n    Io.list(sys.files)\n}\n"
        }
        "args" => {
            "module probe\n\nfn f(sys: System) -> List<String>\n  uses\n    Io.args,\n{\n    Io.args(sys)\n}\n"
        }
        "reach" => {
            "module probe\n\nfn f(sys: System) -> Result<Net, String>\n  uses\n    Io.reach,\n{\n    Io.reach(sys.net, \"x\")\n}\n"
        }
        "fetch" => {
            "module probe\n\nfn f(sys: System) -> Result<String, String>\n  uses\n    Io.fetch,\n{\n    Io.fetch(sys.net, \"x\")\n}\n"
        }
        "send" => {
            "module probe\n\nfn f(sys: System) -> Result<String, String>\n  uses\n    Io.send,\n{\n    Io.send(sys.net, \"x\", \"y\")\n}\n"
        }
        other => panic!("no probe written for `Io.{other}`, add one"),
    }
}

/// Every prelude name the table above has a probe for, and whether the
/// backend compiles it, pinned by name.
///
/// All thirteen do, as of #877. The list stays because the failure it rules
/// out has not gone anywhere: a name that quietly stops compiling is a
/// regression, and a name added to the prelude without a backend answer
/// should be visible here rather than found by whoever runs `deed build`
/// first.
#[test]
fn every_callable_prelude_name_is_named_by_whether_the_backend_compiles_it() {
    let compiles = [
        "ok",
        "err",
        "length",
        "at",
        "push",
        "repeat",
        "split",
        "join",
        "trim",
        "upper",
        "lower",
        "to_string",
        "to_int",
    ];
    let does_not_yet: [&str; 0] = [];

    let mut checked = 0;
    for name in PRELUDE {
        let Some(source) = probe_for(name) else {
            continue;
        };
        checked += 1;
        let outcome = tried(source);
        if compiles.contains(name) {
            assert!(
                outcome.is_ok(),
                "`{name}` used to compile and no longer does: {outcome:?}"
            );
        } else if does_not_yet.contains(name) {
            assert!(
                outcome.is_err(),
                "`{name}` now compiles; move it out of `does_not_yet` and into `compiles` \
                 in the same change that made it compile"
            );
        } else {
            panic!("`{name}` has a probe but is in neither list above, name it in one");
        }
    }

    // Every name with a probe was in one list or the other; this catches a
    // probe added without adding the name to either.
    assert_eq!(
        checked,
        compiles.len() + does_not_yet.len(),
        "a probe exists for a name that is not in `compiles` or `does_not_yet`"
    );
}

/// Every `Io` operation, and whether the backend compiles it, pinned by
/// name.
///
/// All fourteen do: capabilities reach the backend as host imports (#569), and
/// a call through one compiles the same way any other direct call does. That
/// contradicts `compile.rs`'s own doc comment, which still says capabilities
/// are refused by name; fixed alongside this test, since a stale comment a
/// reader would trust over the code is worse than no comment.
///
/// The three network operations are in here for a reason worth stating: the
/// interpreter speaks `http` and nothing else, and a compiled program does not
/// go through the interpreter at all. `Io.fetch` becomes `deed:io.fetch` in
/// the world the module asks its host for, and what the host does with it is
/// the host's business. So the backend compiling them is not the same claim as
/// the interpreter performing them, and this test makes the first one.
#[test]
fn every_io_operation_is_named_by_whether_the_backend_compiles_it() {
    for operation in IO_OPERATIONS {
        let outcome = tried(probe_for_io(operation));
        assert!(
            outcome.is_ok(),
            "`Io.{operation}` used to compile and no longer does: {outcome:?}"
        );
    }
}
