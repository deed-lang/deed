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
//!
//! The two tests added for issue #629 prove the enforcement is structural,
//! not just claimed. [`Host`] links a module against a specific offer list and
//! refuses anything not on it. A module without an import has no index in its
//! own index space to call it through, so a host that offers it cannot help.

use std::path::Path;

use deed_codegen::{Host, Trap, Value, call, compile};
use deed_diagnostics::{SourceMap, render_human};
use deed_driver::check_all;
use deed_interp::{Program, run_main};

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

/// A component whose row does not mention writing cannot write, even when the
/// host offers it.
///
/// This is enforcement by absence: the operation's function index does not
/// exist in the module's index space, so there is no index to call it
/// through. A host that offers write cannot hand that capability to a module
/// that never imported it.
#[test]
fn what_the_row_does_not_name_is_not_reachable() {
    // A component whose row does not mention writing: pure arithmetic.
    let module = module_for("module a\n\nfn answer() -> Int { 2 + 2 }\n");

    // The host offers write. The module has not imported it.
    let mut host = Host::new();
    host.offer("deed:io", "write", |_| {
        unreachable!("write is not in this module's import section")
    });

    // Linking succeeds: the module has no imports, so all of them (vacuously)
    // are satisfied. Offering write changes nothing about what the module can
    // call.
    let linked = host.link(&module).expect("a pure module links to any host");

    // Write is not in the import section. There is no function index for it
    // inside this module's index space.
    assert!(
        !module.imports.iter().any(|i| i.name == "write"),
        "a program that does not use Io.write should not import it"
    );

    // The module runs correctly. The host's write was never dispatched,
    // because there was no import to dispatch through.
    let result = linked
        .call("answer", &[])
        .expect("arithmetic runs under any host");
    assert_eq!(result, Some(Value::I64(4)));
}

/// A component whose row mentions writing is refused at link time when the
/// host does not offer it.
///
/// Refused before a single instruction runs: the import section declares what
/// the module needs, and a host that cannot satisfy every entry refuses the
/// module rather than waiting for the missing call.
#[test]
fn a_component_asking_for_what_the_host_does_not_offer_is_refused_at_load() {
    // A component whose row mentions writing.
    let module = module_for(WRITING);

    // A host that offers nothing.
    let host = Host::new();

    // The module needs write (and the console narrowing). The host has
    // neither, so it refuses the module at link time.
    let err = host
        .link(&module)
        .expect_err("a writing component is refused by a host that cannot write");

    // The error names the specific import that was not satisfied.
    assert!(
        err.module.starts_with("deed:"),
        "the unsatisfied import is from a deed namespace: {}",
        err.module
    );
}

/// The interpreter is not a host, and now says so.
///
/// A user effect that reaches `main` is the same thing `Io.write` is: an entry
/// in the world a host answers. The interpreter has no host, so it cannot run
/// this program, but it used to report that as "no handler is installed" with
/// a note saying to wrap the call in a `with` block. That is one of two
/// readings and it is the one that changes what the program is: a `with` block
/// here would discharge the effect and take the import out of the world.
#[test]
fn an_effect_that_reaches_main_is_reported_as_the_host_s_rather_than_as_a_missing_handler() {
    const ESCAPES: &str = "module a\n\n\
effect Log {\n\
    fn note(line: String) -> ()\n\
}\n\n\
fn main() -> Int\n\
  uses\n\
    Log.note,\n\
{\n\
    Log.note(\"hi\")\n\
    0\n\
}\n";

    let mut sources = SourceMap::new();
    let id = sources.add("host.deed".to_string(), ESCAPES.to_string());
    let mut all = check_all(&sources, &[id]);
    let one = all.pop().expect("one file in, one result out");
    assert!(
        !one.has_errors(),
        "this program is well formed and has a world: {:?}",
        one.diagnostics
    );
    assert!(
        deed_driver::wit_world_for(&one).contains("import deed:log.note;"),
        "the effect is an import, which is the whole reason this is not a mistake"
    );

    let mut program = Program::new();
    program.add(
        one.file,
        &one.module,
        &one.resolutions,
        one.guards(),
        one.rows(),
        one.operators(),
    );
    let run = run_main(&program, one.file, Path::new("."), &[]).expect("there is a `main`");
    let failure = run.result.expect_err("no host means it cannot run");

    assert_eq!(failure.code, "DEED6005");
    let text = render_human(&sources, &failure);
    assert!(
        text.contains("is in `main`'s row"),
        "it should say where the effect got to: {text}"
    );
    assert!(
        text.contains("import deed:log.note"),
        "it should name the import a host would answer: {text}"
    );
    assert!(
        text.contains("`deed run` is not a host"),
        "it should say why this run in particular could not answer: {text}"
    );
}
