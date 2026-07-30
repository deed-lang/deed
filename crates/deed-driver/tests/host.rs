//! What a program asks its host for.
//!
//! A WebAssembly module cannot open a file, write a line or read a clock. It
//! declares what it wants and whoever runs it decides, which is the same
//! shape the language's capabilities already have and most of why this
//! backend targets WASM rather than an object file.
//!
//! So a compiled Deed program's import section is its capability
//! requirements, written down where a host can read them before running
//! anything. That is worth checking rather than assuming.

use deed_codegen::{Trap, Value, call, compile};
use deed_diagnostics::SourceMap;
use deed_driver::check_all;

fn module_for(source: &str) -> deed_codegen::Module {
    let mut sources = SourceMap::new();
    let id = sources.add("host.deed".to_string(), source.to_string());
    let mut all = check_all(&sources, &[id]);
    let one = all.pop().expect("one file in, one result out");
    assert!(
        !one.has_errors(),
        "this program should check: {:?}",
        one.diagnostics
    );

    let lowered = deed_mir::lower(&one.module, &one.resolutions, &one.types).expect("this lowers");
    compile(&lowered).expect("this compiles")
}

const WRITING: &str = "module a\n\n\
fn main(sys: System) -> Int\n\
  uses\n\
    Io.write,\n\
{\n\
    Io.write(sys.console, \"hi\")\n\
    0\n\
}\n";

#[test]
fn what_a_program_cannot_do_by_itself_it_asks_for_by_name() {
    let module = module_for(WRITING);

    let asked: Vec<String> = module
        .imports
        .iter()
        .map(|import| format!("{}.{}", import.module, import.name))
        .collect();

    // Writing a line, and narrowing `System` down to the console that is
    // written to. Both are the host's, and neither is something the module
    // could do with the memory it has.
    assert!(
        asked.contains(&"deed:io.write".to_string()),
        "a program that writes should ask for it: {asked:?}"
    );
    assert!(
        asked.contains(&"deed:sys.console".to_string()),
        "narrowing a capability is the host's too: {asked:?}"
    );
}

/// A program that does nothing outside itself asks for nothing.
///
/// The half that makes the test above worth having. An import section that
/// listed everything the backend knows how to emit would say nothing about
/// the program.
#[test]
fn a_program_that_touches_nothing_asks_for_nothing() {
    let module = module_for("module a\n\nfn answer() -> Int { 2 + 2 }\n");
    assert!(
        module.imports.is_empty(),
        "arithmetic needs no host: {:?}",
        module.imports
    );
}

/// The capability is the argument, not just the row.
///
/// `Io.write` takes the console it writes to, and the compiled call keeps
/// it. A module that imported `write` and called it with a string alone
/// would have turned a capability into a permission bit, which is the thing
/// `design/04-capabilities.md` says this language does not do.
#[test]
fn a_host_call_still_carries_the_capability_it_acts_on() {
    let module = module_for(WRITING);
    let write = module
        .imports
        .iter()
        .find(|import| import.name == "write")
        .expect("write is imported");

    let signature = &module.types[write.type_index as usize];
    assert_eq!(
        signature.params.len(),
        2,
        "write takes the console and the text"
    );
}

/// Function indices count imports first, so the program's own functions
/// move. Exporting one at the wrong number is a module that calls the host
/// when it meant to call itself.
#[test]
fn the_functions_the_module_defines_are_numbered_after_what_it_imports() {
    let module = module_for(WRITING);
    let shift = module.imports.len() as u32;
    assert!(shift > 0, "this program imports something");

    for (name, index) in &module.exports {
        assert!(
            *index >= shift,
            "`{name}` is exported at {index}, which is inside the import range"
        );
    }
}

/// Running one without a host says so rather than guessing.
///
/// The runner in `deed-codegen` is a test oracle. Deciding what `Io.write`
/// does would be a program taking authority nobody granted it, so it stops
/// and names what it was asked for. That is also the most useful thing it
/// could say about a module on its way to a real embedder.
#[test]
fn running_a_program_that_needs_a_host_says_which_operation_it_wanted() {
    let module = module_for(WRITING);
    let stopped = call(&module, "main", &[Value::I64(0)]).expect_err("this needs a host");

    let Trap::NeedsAHost(what) = stopped else {
        panic!("it should say what it wanted, not {stopped}");
    };
    assert!(what.starts_with("deed:"), "{what}");
}
