//! When the two engines stop, they stop for the same reason.
//!
//! `agreement.rs` compares answers, which only covers programs that have
//! one. A program that breaks its own contract has no answer, and the
//! interesting thing about it is the sentence it stops with. A compiled
//! program that only said "the program stopped" would be worse to debug than
//! an interpreted one for no reason anybody chose, so the code and the
//! message are written into memory before the trap and read back out after.
//!
//! What this checks is that the two agree on which one it was.

use deed_codegen::{Trap, Value, call, compile};
use deed_diagnostics::SourceMap;
use deed_driver::check_all;
use deed_interp::{Program as Interpreted, run_tests};

/// Checks a source, or says why it would not.
fn checked(source: &str) -> deed_driver::Checked {
    let mut sources = SourceMap::new();
    let id = sources.add("failing.deed".to_string(), source.to_string());
    let mut all = check_all(&sources, &[id]);
    let one = all.pop().expect("one file in, one result out");
    assert!(
        !one.has_errors(),
        "this program should check: {:?}",
        one.diagnostics
    );
    one
}

/// Compiles a source and calls a function in it, expecting it to stop.
fn stopped(source: &str, name: &str, args: &[Value]) -> Trap {
    let one = checked(source);
    let program = deed_mir::lower(&one.module, &one.resolutions, &one.types).expect("this lowers");
    let module = compile(&program).expect("this compiles");
    call(&module, name, args).expect_err("this should have stopped")
}

/// A `where` clause the checker could not settle, broken by a caller.
const PRECONDITION: &str = "module a\n\n\
fn halve(n: Int) -> Int\n\
  where n > 0,\n\
{ n / 2 }\n\n\
fn answer(n: Int) -> Int { halve(n) }\n";

#[test]
fn a_broken_precondition_says_which_clause_and_whose_fault_it_is() {
    let trap = stopped(PRECONDITION, "answer", &[Value::I64(-4)]);
    let Trap::Failed { code, message } = trap else {
        panic!("a broken precondition should say what it was, not {trap}");
    };

    assert_eq!(code, deed_interp::codes::PRECONDITION_FAILED);
    assert!(
        message.contains("halve") && message.contains("requires"),
        "the sentence should name the callee and its clause: {message}"
    );
}

/// The compiled program keeps the interpreter's vocabulary rather than
/// inventing a second one.
///
/// `deed-mir` spells these out instead of depending on `deed-interp`, since
/// a backend that needed the interpreter to build could not replace it. The
/// copy is what this pins.
#[test]
fn the_codes_the_backend_uses_are_the_interpreters_codes() {
    assert_eq!(
        deed_mir::codes::ASSERTION_FAILED,
        deed_interp::codes::ASSERTION_FAILED
    );
    assert_eq!(
        deed_mir::codes::PRECONDITION_FAILED,
        deed_interp::codes::PRECONDITION_FAILED
    );
    assert_eq!(
        deed_mir::codes::NOT_RUNNABLE,
        deed_interp::codes::NOT_RUNNABLE
    );
}

/// The same program, run by the interpreter, files a diagnostic with the
/// same code.
///
/// Two engines agreeing that a program is wrong is worth less than two
/// engines agreeing on why, and this is the second one.
#[test]
fn the_interpreter_stops_on_the_same_code() {
    let source = format!("{PRECONDITION}\ntest \"it stops\" {{\n    assert answer(-4) == 0\n}}\n");
    let one = checked(&source);

    let mut interpreted = Interpreted::new();
    interpreted.add(
        one.file,
        &one.module,
        &one.resolutions,
        one.guards(),
        one.rows(),
    );
    let outcomes = run_tests(&interpreted, one.file);

    let codes: Vec<String> = outcomes
        .iter()
        .filter_map(|outcome| outcome.failure.as_ref())
        .map(|failure| failure.code.to_string())
        .collect();

    assert!(
        codes
            .iter()
            .any(|code| code == deed_interp::codes::PRECONDITION_FAILED),
        "the interpreter should have filed a precondition failure, not {codes:?}"
    );
}

/// Both engines stop with the same sentence, not just the same code.
///
/// A compiled program that said something different would force a reader
/// to learn two dialects of the same failure, which is worse than one.
/// The message comes from the same place in both paths, so they should
/// agree word for word.
#[test]
fn the_interpreter_and_compiler_stop_with_the_same_message() {
    let trap = stopped(PRECONDITION, "answer", &[Value::I64(-4)]);
    let Trap::Failed {
        message: compiled_message,
        ..
    } = trap
    else {
        panic!("the compiled engine should have stopped with a message, not {trap}");
    };

    let source = format!("{PRECONDITION}\ntest \"it stops\" {{\n    assert answer(-4) == 0\n}}\n");
    let one = checked(&source);

    let mut interpreted = Interpreted::new();
    interpreted.add(
        one.file,
        &one.module,
        &one.resolutions,
        one.guards(),
        one.rows(),
    );
    let outcomes = run_tests(&interpreted, one.file);

    let interp_message = outcomes
        .iter()
        .find_map(|outcome| {
            let diag = outcome.failure.as_ref()?;
            (diag.code == deed_interp::codes::PRECONDITION_FAILED).then(|| diag.message.clone())
        })
        .expect("the interpreter should have filed a precondition failure");

    assert_eq!(
        compiled_message, interp_message,
        "compiled and interpreted engines should stop with the same sentence"
    );
}

/// A contract the checker settled leaves nothing to fail, so nothing is
/// written and the program does not stop.
///
/// The other half of `a_proven_precondition_compiles_to_nothing` in
/// `agreement.rs`: that one counts instructions, this one runs the thing.
#[test]
fn a_proven_precondition_does_not_stop_the_program() {
    let source = "module a\n\n\
fn halve(n: Int) -> Int\n\
  where n > 0,\n\
{ n / 2 }\n\n\
fn answer() -> Int { halve(8) }\n";

    let one = checked(source);
    let program = deed_mir::lower(&one.module, &one.resolutions, &one.types).expect("this lowers");
    let module = compile(&program).expect("this compiles");

    assert_eq!(
        call(&module, "answer", &[]).expect("this should not have stopped"),
        Some(Value::I64(4))
    );
}
