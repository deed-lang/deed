//! What the backend computes, against what the interpreter computes.
//!
//! A compiler and an interpreter for the same language are two answers to
//! one question, and the only useful thing to do with two answers is compare
//! them. This is the ratchet that stops the backend drifting: every program
//! below is run twice, once by `deed-interp` through its own `test` blocks
//! and once by compiling it to WebAssembly and calling the function, and the
//! two have to land on the same number.
//!
//! Each program carries its expected answer in two places on purpose. The
//! `test` block inside the Deed source is what the interpreter checks, and
//! the number in the table here is what the compiled module is checked
//! against. One place would let a wrong shared answer pass twice.

use deed_codegen::{Value, call, compile};
use deed_diagnostics::SourceMap;
use deed_driver::check_all;
use deed_interp::{Program as Interpreted, run_tests};

/// A program, the function to call in it, and what it should come back with.
struct Agreed {
    name: &'static str,
    source: &'static str,
    call: &'static str,
    expect: i64,
}

fn programs() -> Vec<Agreed> {
    vec![
        Agreed {
            name: "arithmetic",
            source: "module a\n\nfn answer() -> Int { 2 + 3 * 4 }\n\ntest \"it adds and multiplies\" {\n    assert answer() == 14\n}\n",
            call: "answer",
            expect: 14,
        },
        Agreed {
            name: "precedence and subtraction",
            source: "module a\n\nfn answer() -> Int { 10 - 2 - 3 }\n\ntest \"subtraction goes left to right\" {\n    assert answer() == 5\n}\n",
            call: "answer",
            expect: 5,
        },
        Agreed {
            name: "division and remainder",
            source: "module a\n\nfn answer() -> Int { 17 / 5 + 17 % 5 }\n\ntest \"it divides and takes a remainder\" {\n    assert answer() == 5\n}\n",
            call: "answer",
            expect: 5,
        },
        Agreed {
            name: "a branch",
            source: "module a\n\nfn answer() -> Int {\n    if 3 > 2 {\n        10\n    } else {\n        20\n    }\n}\n\ntest \"it takes the branch that holds\" {\n    assert answer() == 10\n}\n",
            call: "answer",
            expect: 10,
        },
        Agreed {
            name: "the other branch",
            source: "module a\n\nfn answer() -> Int {\n    if 1 > 2 {\n        10\n    } else {\n        20\n    }\n}\n\ntest \"it takes the other one\" {\n    assert answer() == 20\n}\n",
            call: "answer",
            expect: 20,
        },
        Agreed {
            name: "a call",
            source: "module a\n\nfn double(n: Int) -> Int { n + n }\n\nfn answer() -> Int { double(21) }\n\ntest \"it calls what it declared\" {\n    assert answer() == 42\n}\n",
            call: "answer",
            expect: 42,
        },
        Agreed {
            name: "a call taking two arguments",
            source: "module a\n\nfn combine(a: Int, b: Int) -> Int { a * 10 + b }\n\nfn answer() -> Int { combine(4, 2) }\n\ntest \"arguments arrive in order\" {\n    assert answer() == 42\n}\n",
            call: "answer",
            expect: 42,
        },
        Agreed {
            name: "a let binding",
            source: "module a\n\nfn answer() -> Int {\n    let a = 6\n    let b = 7\n    a * b\n}\n\ntest \"a name holds what it was bound to\" {\n    assert answer() == 42\n}\n",
            call: "answer",
            expect: 42,
        },
        Agreed {
            name: "booleans and comparison",
            source: "module a\n\nfn both(a: Bool, b: Bool) -> Bool { a && b }\n\nfn answer() -> Int {\n    if both(1 < 2, 3 >= 3) {\n        1\n    } else {\n        0\n    }\n}\n\ntest \"it compares and combines\" {\n    assert answer() == 1\n}\n",
            call: "answer",
            expect: 1,
        },
        Agreed {
            name: "negation and not",
            source: "module a\n\nfn answer() -> Int {\n    if !(0 - 5 > 0) {\n        0 - 7\n    } else {\n        7\n    }\n}\n\ntest \"a negative number is less than zero\" {\n    assert answer() == 0 - 7\n}\n",
            call: "answer",
            expect: -7,
        },
        Agreed {
            name: "one function calling another calling another",
            source: "module a\n\nfn one() -> Int { 1 }\n\nfn two() -> Int { one() + one() }\n\nfn answer() -> Int { two() + two() }\n\ntest \"calls compose\" {\n    assert answer() == 4\n}\n",
            call: "answer",
            expect: 4,
        },
        // A call written above the declaration it names. Deed has no
        // forward-declaration rule, so lowering has to place every signature
        // before it lowers any body.
        Agreed {
            name: "a call to something declared later",
            source: "module a\n\nfn answer() -> Int { later() }\n\nfn later() -> Int { 99 }\n\ntest \"order in the file does not matter\" {\n    assert answer() == 99\n}\n",
            call: "answer",
            expect: 99,
        },
    ]
}

/// Checks a program, and refuses to go on if it does not check.
///
/// A backend is only meant to see programs the checker accepted, so a
/// program that does not check would be testing something this cannot
/// answer.
fn checked(source: &str) -> (SourceMap, deed_driver::Checked) {
    let mut sources = SourceMap::new();
    let id = sources.add("agreement.deed".to_string(), source.to_string());
    let mut all = check_all(&sources, &[id]);
    let one = all.pop().expect("one file in, one result out");
    assert!(
        !one.has_errors(),
        "this program should check: {:?}",
        one.diagnostics
    );
    (sources, one)
}

#[test]
fn the_backend_and_the_interpreter_answer_the_same_thing() {
    let programs = programs();
    assert!(!programs.is_empty(), "nothing to agree about");

    for program in &programs {
        let (_, one) = checked(program.source);

        // The interpreter's half: the file's own `test` block.
        let mut interpreted = Interpreted::new();
        interpreted.add(
            one.file,
            &one.module,
            &one.resolutions,
            one.guards(),
            one.rows(),
        );
        let outcomes = run_tests(&interpreted, one.file);
        assert!(
            !outcomes.is_empty(),
            "`{}` declares no test, so the interpreter is asked nothing",
            program.name
        );
        for outcome in &outcomes {
            assert!(
                outcome.failure.is_none(),
                "`{}` fails under the interpreter: {:?}",
                program.name,
                outcome.failure
            );
        }

        // The backend's half: lower, compile, run.
        let lowered = deed_mir::lower(&one.module, &one.resolutions, &one.types)
            .unwrap_or_else(|why| panic!("`{}` should lower: {why}", program.name));
        let module = compile(&lowered)
            .unwrap_or_else(|why| panic!("`{}` should compile: {why}", program.name));
        let answer = call(&module, program.call, &[])
            .unwrap_or_else(|why| panic!("`{}` should run: {why}", program.name));

        assert_eq!(
            answer.map(Value::as_i64),
            Some(program.expect),
            "`{}` disagrees between the two",
            program.name
        );
    }
}

/// The half of the previous test that is easy to lose: it walks a list, and
/// a list that stopped being built would make every assertion above vacuous.
#[test]
fn the_agreement_covers_more_than_one_program() {
    assert!(
        programs().len() >= 10,
        "the agreement table should carry more than a couple of programs"
    );
}

/// A program the backend cannot compile has to say so, rather than compile
/// into something that runs and answers wrongly.
#[test]
fn what_the_backend_cannot_compile_is_refused_by_name() {
    let (_, one) = checked("module a\n\nfn greet() -> String { \"hi\" }\n");
    let lowered = deed_mir::lower(&one.module, &one.resolutions, &one.types)
        .expect("a string lowers, it is compiling it that does not");
    let refused = compile(&lowered).expect_err("a string is not compiled yet");
    assert_eq!(refused.function, "greet");
    assert!(refused.to_string().contains("a string"), "{refused}");
}

/// The interpreter stays the reference implementation, so a program the
/// backend refuses still runs. That is the whole of why the interpreter is
/// not being replaced, and it is worth a test rather than a sentence.
#[test]
fn a_program_the_backend_refuses_still_runs_under_the_interpreter() {
    let (_, one) = checked(
        "module a\n\nfn greet() -> String { \"hi\" }\n\ntest \"it greets\" {\n    assert greet() == \"hi\"\n}\n",
    );

    let mut interpreted = Interpreted::new();
    interpreted.add(
        one.file,
        &one.module,
        &one.resolutions,
        one.guards(),
        one.rows(),
    );
    let outcomes = run_tests(&interpreted, one.file);
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].failure.is_none());

    let lowered = deed_mir::lower(&one.module, &one.resolutions, &one.types).expect("this lowers");
    assert!(
        compile(&lowered).is_err(),
        "the backend has not got here yet"
    );
}
