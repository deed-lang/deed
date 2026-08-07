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

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use deed_codegen::{Grants, Host, Trap, Value, call, compile};
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

// -- an effect nobody handles is a foreign function -------------------------

const FOREIGN: &str = "module a\n\n\
effect Random from \"wasi:random/random\" {\n\
  fn roll(sides: Int) -> Int\n\
}\n\n\
fn pick(sides: Int) -> Int\n\
  uses Random.roll,\n\
{\n\
    Random.roll(sides)\n\
}\n";

/// An effect the program never handles is asked of the host by name.
///
/// This is the whole of what interop means here, and it is worth saying why
/// it is not the ambient authority `design/04-capabilities.md` worries about:
/// a module cannot reach an import nobody gave it, so a foreign call is a
/// capability the host decided to hand over rather than a hole in the
/// boundary. What used to happen is worse than either: the module declared no
/// import at all and trapped when the export was called. See issue #912.
#[test]
fn an_effect_the_program_never_handles_is_asked_of_the_host() {
    let module = module_for(FOREIGN);
    let asked: Vec<String> = module
        .imports
        .iter()
        .map(|import| format!("{}.{}", import.module, import.name))
        .collect();
    assert_eq!(asked, vec!["wasi:random/random.roll".to_string()]);
}

/// A host is handed the operation's own arguments and nothing else.
///
/// An installed handler is given its state cell first. There is no handler
/// here, so there is no cell, and passing one would mean the host had to
/// know about a thing the program does not have.
#[test]
fn the_host_is_handed_the_operations_own_arguments() {
    let module = module_for(FOREIGN);
    let roll = module
        .imports
        .iter()
        .find(|import| import.name == "roll")
        .expect("roll is imported");
    let signature = &module.types[roll.type_index as usize];
    assert_eq!(signature.params.len(), 1, "roll takes the number of sides");
    assert_eq!(signature.results.len(), 1, "and answers with a number");
}

/// An effect the program does handle is not asked of anybody.
///
/// The other half of the rule, and the one that stops the world from turning
/// into a list of every effect a module mentions. A `with` answers, so the
/// operation never reaches the boundary and the host is not troubled for it.
#[test]
fn an_effect_the_program_handles_is_not_asked_of_the_host() {
    let module = module_for(
        "module a\n\n\
         effect Log {\n\
           fn note(m: String) -> Int\n\
         }\n\n\
         handler Quiet implements Log {\n\
           state seen: Int\n\n\
           fn note(m: String) -> Int {\n\
             seen\n\
           }\n\
         }\n\n\
         fn talks(n: Int) -> Int\n\
           uses Log.note,\n\
         {\n\
           n + Log.note(\"hi\")\n\
         }\n\n\
         fn counted(n: Int) -> Int {\n\
           with Quiet { seen: 0 } { talks(n) }\n\
         }\n",
    );
    assert!(
        !module.imports.iter().any(|import| import.name == "note"),
        "a handled effect should not be a host import: {:?}",
        module.imports
    );
}

/// Which import a performed operation calls, when there is more than one.
///
/// Every test above this one leaves the module with a single import, and a
/// lookup that picks the only entry is right however it is spelled. So the
/// three comparisons that find the import were free to be wrong: matching on
/// the interface alone, on the operation alone, or on either, all answer the
/// same in a module with one. Getting it wrong is not a trap, it is a call to
/// the wrong host function, which is a silently different answer.
///
/// So this program declares three, and two of them share an operation name
/// across two interfaces.
const TWO_INTERFACES: &str = "module a\n\n\
effect Alpha from \"one:iface\" {\n\
  fn ask(n: Int) -> Int\n\
  fn tell(n: Int) -> Int\n\
}\n\n\
effect Beta from \"two:iface\" {\n\
  fn ask(n: Int) -> Int\n\
}\n\n\
fn ask_one(n: Int) -> Int\n  uses Alpha.ask,\n{\n  Alpha.ask(n)\n}\n\n\
fn tell_one(n: Int) -> Int\n  uses Alpha.tell,\n{\n  Alpha.tell(n)\n}\n\n\
fn ask_two(n: Int) -> Int\n  uses Beta.ask,\n{\n  Beta.ask(n)\n}\n";

#[test]
fn a_performed_operation_calls_the_import_it_named_and_not_a_neighbour() {
    let module = module_for(TWO_INTERFACES);

    let mut host = Host::new();
    host.offer("one:iface", "ask", |_| Ok(Some(Value::I64(100))))
        .offer("one:iface", "tell", |_| Ok(Some(Value::I64(200))))
        .offer("two:iface", "ask", |_| Ok(Some(Value::I64(300))));
    let linked = host.link(&module).expect("the host offers all three");

    for (function, expected) in [("ask_one", 100), ("tell_one", 200), ("ask_two", 300)] {
        let answer = linked
            .call(function, &[Value::I64(0)])
            .unwrap_or_else(|trap| panic!("`{function}` should reach its host: {trap:?}"))
            .expect("it answers with a number");
        assert_eq!(
            answer.as_i64(),
            expected,
            "`{function}` reached the wrong host function"
        );
    }
}

/// A host that does not offer one of them refuses the module before it runs.
///
/// The same property `deed:io` already has, on an interface the program named
/// itself. Whoever runs a component decides what it may reach, and deciding
/// happens at load rather than at the first call.
#[test]
fn a_host_missing_one_interface_refuses_the_module() {
    let module = module_for(TWO_INTERFACES);

    let mut host = Host::new();
    host.offer("one:iface", "ask", |_| Ok(Some(Value::I64(100))))
        .offer("one:iface", "tell", |_| Ok(Some(Value::I64(200))));
    let refused = host
        .link(&module)
        .expect_err("the host offers nothing from `two:iface`");
    assert_eq!(refused.module, "two:iface");
    assert_eq!(refused.name, "ask");
}

// -- a host that actually answers -------------------------------------------

/// A compiled program writes a line, and the line arrives.
///
/// Every test above this one is about the shape of the boundary: which
/// imports a module declares, and which of them a host will answer. None of
/// them crosses it. This one does, and until it passed, `deed run --compiled`
/// on the corpus's `hello.deed` stopped with "`deed:sys.console` is the
/// host's to answer, and this is not one": a compiled Deed program could not
/// write "hello, world", and the interpreted one could.
///
/// What was missing was not the wiring. A host implementation used to be
/// handed the call's arguments and nothing else, and a string argument is an
/// address into the module's memory, so there was no host that could be
/// written for any operation carrying anything but a number.
#[test]
fn a_program_writes_a_line_to_the_console_its_host_granted() {
    let module = module_for(WRITING);

    let written = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&written);
    let granted = Grants::none()
        .console(move |line| sink.borrow_mut().push(line.to_string()))
        .into_host();

    let linked = granted
        .host
        .link(&module)
        .expect("a host with a console answers everything this program asks for");
    linked
        .call("main", &[granted.system])
        .expect("it runs under a host that grants what it asked for");

    assert_eq!(*written.borrow(), vec!["hi".to_string()]);
}

/// The same program, and a host that grants no console, is refused before it
/// runs.
///
/// The half that makes the test above mean anything. A host that answered
/// whatever it was asked would pass the first test too, and the row would be
/// a comment. Here the module is turned down at link time, naming the import
/// nobody answered, which is what a real engine does with a component whose
/// world the host cannot satisfy.
#[test]
fn the_same_program_is_refused_by_a_host_that_grants_no_console() {
    let module = module_for(WRITING);

    let granted = Grants::none().clock().into_host();
    let refused = granted
        .host
        .link(&module)
        .expect_err("a clock is not a console");

    // Whichever of the two the import section lists first. Both of them are
    // the console: one narrows `System` down to it, the other writes to it.
    assert_eq!(refused.module, "deed:io");
    assert_eq!(refused.name, "write");
    assert_eq!(
        refused.to_string(),
        "the host does not offer `deed:io.write`"
    );
}

/// What the host is handed is the handle it gave out, not a number the
/// program chose.
///
/// A compiled capability is an opaque number, and the module's memory is
/// full of numbers. Nothing in a checked Deed program can pass one where a
/// capability belongs, so this reaches past the source language and asks the
/// host directly: a number it never handed out is refused rather than
/// treated as the console.
#[test]
fn a_capability_the_host_never_handed_out_is_not_one() {
    let module = module_for(WRITING);

    let granted = Grants::none().console(|_| {}).into_host();
    let linked = granted.host.link(&module).expect("the console is granted");

    let stopped = linked
        .call("main", &[Value::I64(4242)])
        .expect_err("4242 is not the root this host granted");
    let Trap::Refused(why) = stopped else {
        panic!("it should be refused rather than answered: {stopped:?}");
    };
    assert!(
        why.contains("`System`"),
        "it should say what it wanted: {why}"
    );
}

// -- the directory a host grants --------------------------------------------

const READING: &str = "module a\n\n\
fn main(sys: System) -> Int\n\
  uses\n\
    Io.read,\n\
    Io.write,\n\
{\n\
    match Io.read(sys.files, \"note.txt\") {\n\
        ok(text) => { Io.write(sys.console, text) },\n\
        err(why) => { Io.write(sys.console, why) },\n\
    }\n\
    0\n\
}\n";

/// A scratch directory that cleans up after itself.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let at = std::env::temp_dir().join(format!("deed-host-{name}"));
        std::fs::remove_dir_all(&at).ok();
        std::fs::create_dir_all(&at).expect("a scratch directory");
        Self(at)
    }

    fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.0.join(name), contents).expect("writing the fixture");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// A compiled program reads a file out of the directory it was granted.
///
/// The answer is a `Result<String, String>`, which is a shape nobody writes
/// down: the compiler synthesizes the layout, so a host answering one has to
/// build the same thing in the module's own memory. Getting the two variants
/// the wrong way round would be an answer that is inverted rather than
/// missing, which is why `deed_mir` owns the order and this asks it.
#[test]
fn a_program_reads_a_file_out_of_the_directory_its_host_granted() {
    let scratch = Scratch::new("read");
    scratch.write("note.txt", "what the file said");

    let module = module_for(READING);
    let written = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&written);
    let granted = Grants::none()
        .console(move |line| sink.borrow_mut().push(line.to_string()))
        .files(scratch.0.clone())
        .into_host();

    granted
        .host
        .link(&module)
        .expect("the host grants both of the things this asks for")
        .call("main", &[granted.system])
        .expect("it runs");

    assert_eq!(*written.borrow(), vec!["what the file said".to_string()]);
}

/// And it cannot read one outside that directory.
///
/// The rule is `deed_rt::sandbox`'s, which is the one the interpreter asks
/// too. A second copy of it inside this host would be a second answer to
/// "what does a `Dir` reach", and the two would drift.
#[test]
fn a_program_cannot_read_past_the_directory_it_was_granted() {
    let scratch = Scratch::new("escape");
    scratch.write("note.txt", "the one it may read");
    std::fs::create_dir_all(scratch.0.join("inner")).expect("a directory inside");

    let module = module_for(READING);
    let written = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&written);
    let granted = Grants::none()
        .console(move |line| sink.borrow_mut().push(line.to_string()))
        .files(scratch.0.join("inner"))
        .into_host();

    granted
        .host
        .link(&module)
        .expect("the host grants both of the things this asks for")
        .call("main", &[granted.system])
        .expect("it runs, and answers with an err");

    let said = written.borrow().join("");
    assert!(
        !said.contains("the one it may read"),
        "the file above the granted directory should be out of reach: {said}"
    );
    assert!(
        said.contains("note.txt"),
        "and the answer should name what was asked for: {said}"
    );
}

/// The same program, and a host that grants no directory, is refused before
/// it runs.
#[test]
fn the_reading_program_is_refused_by_a_host_that_grants_no_directory() {
    let module = module_for(READING);
    let granted = Grants::none().console(|_| {}).into_host();
    let refused = granted
        .host
        .link(&module)
        .expect_err("a console is not a filesystem");
    assert_eq!(refused.module, "deed:io");
    assert_eq!(refused.name, "read");
}
