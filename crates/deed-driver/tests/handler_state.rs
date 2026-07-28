//! Handler state, and the one value that could carry a read of it out of the
//! handler that owns it.
//!
//! Handler state is the only mutable thing in Deed. That is what lets an empty
//! effect row mean a function cannot change anything, and it is the claim
//! `design/03-effects.md` and `design/04-capabilities.md` both lean on. A
//! handler's lifetime is the `with` block that installed it, and everything
//! that reads its state runs inside one of its operations.
//!
//! Everything except a closure. A closure is a value, it captures the frame
//! rather than the handler, and it leaves through a function type. Written
//! inside a handler operation it could name the state around it, and the
//! interpreter answered such a read out of whichever handler was innermost
//! when the call landed. Called under a different handler that happened to use
//! the same state name, that is a wrong number and nothing said so: `deed
//! check` exited 0, the run exited 0, and the closure quietly answered with
//! the other handler's state.
//!
//! Two answers were possible and one of them is refused here.
//!
//! Capturing the handler would make the closure keep it alive past its `with`
//! block, and the closure's type would not say so. `Fn() -> Int` says the
//! value takes nothing, hands back an `Int` and performs nothing, and
//! `design/03-effects.md` argues that a row left off a function type cannot
//! mean "any row" because a signature is complete. A value that is also a live
//! window onto one particular handler's state carries an input and a lifetime
//! through a signature that mentions neither, and there is no notation for
//! either. So the closure is refused where it is written, which is `DEED4030`,
//! and the reader writes the snapshot the rest of the language already takes.
//!
//! The refusal is lexical: a closure written inside a handler operation may
//! not name that handler's state, wherever the closure ends up. Asking instead
//! whether a particular closure escapes is escape analysis, and not having to
//! answer that question is why a closure's effects are charged to whoever
//! wrote it in the first place.
//!
//! The rest of this file is the neighbours, because this is the kind of change
//! that fixes one shape and breaks another.

use std::path::PathBuf;

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_driver::{Checked, check_text};
use deed_interp::{Program, run_main, run_tests};
use deed_typeck::codes::CLOSURE_OVER_STATE;

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

fn check(src: &str) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.deed", src);
    (sources, checked)
}

fn errors(checked: &Checked) -> Vec<Diagnostic> {
    checked
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .cloned()
        .collect()
}

/// What a source came back with, from the outside.
///
/// A refusal and an answer are two ways for the same program to end up
/// somewhere, and the claim this file is about is over both of them at once,
/// so they are one value rather than two helpers.
#[derive(Debug, PartialEq, Eq)]
enum Answer {
    /// `deed check` turned the file down, with these codes.
    Refused(Vec<String>),
    /// `main` ran and wrote this.
    Wrote(String),
    /// `main` ran and failed.
    Failed(String),
}

/// Checks the source, and runs `main` if it checked.
///
/// The number comes back through the console rather than out of a `Value`,
/// because the console is what a person running the program would see and this
/// is about what the program answers.
fn answer(src: &str) -> Answer {
    let (sources, checked) = check(src);
    let failures = errors(&checked);
    if !failures.is_empty() {
        return Answer::Refused(failures.iter().map(|d| d.code.to_string()).collect());
    }

    let mut program = Program::new();
    program.add(
        checked.file,
        &checked.module,
        &checked.resolutions,
        checked.guards(),
        checked.rows(),
    );
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let run = run_main(&program, checked.file, &root, &[]).expect("a `main` to run");
    match run.result {
        Ok(_) => Answer::Wrote(run.output.join("\n")),
        Err(failure) => Answer::Failed(rendered(&sources, &[*failure])),
    }
}

/// Checks the source and runs every `test` block in it, all of which pass.
fn every_test_passes(src: &str) {
    let (sources, checked) = check(src);
    let failures = errors(&checked);
    assert!(
        failures.is_empty(),
        "this should have checked cleanly:\n{}",
        rendered(&sources, &failures)
    );

    let mut program = Program::new();
    program.add(
        checked.file,
        &checked.module,
        &checked.resolutions,
        checked.guards(),
        checked.rows(),
    );

    let outcomes = run_tests(&program, checked.file);
    // Without this every assertion below is satisfied by a file whose tests
    // nobody ran, which is what a `test` block renamed out of existence looks
    // like from here.
    assert!(!outcomes.is_empty(), "no test ran");
    for outcome in outcomes {
        if let Some(failure) = outcome.failure {
            panic!(
                "`{}` should have passed:\n{}",
                outcome.name,
                rendered(&sources, &[failure])
            );
        }
    }
}

// -- the reproduction ------------------------------------------------------

/// Two handlers, one state name, a closure written under one and called under
/// the other.
///
/// `A` holds 7 and `B` holds 1, and `cross` asks `A` for a closure and hands
/// it to `B`. Nothing here is unusual: both handlers are whole, both effects
/// are declared, `cross` names both operations in its row, and both `with`
/// blocks are open when the call happens.
const CROSSED: &str = "\
module a

effect Give {
    fn getter() -> Fn() -> Int
}

effect Take {
    fn use_it(f: Fn() -> Int) -> Int
}

handler A implements Give {
    state n: Int

    fn getter() -> Fn() -> Int { || { n } }
}

handler B implements Take {
    state n: Int

    fn use_it(f) -> Int { f() }
}

fn cross() -> Int
  uses
    Give.getter,
    Take.use_it,
{
    let f = Give.getter()
    Take.use_it(f)
}

fn main(sys: System) -> ()
  uses
    Io.write,
{
    with A { n: 7 } {
        with B { n: 1 } {
            Io.write(sys.console, to_string(cross()))
        }
    }
}
";

/// The bug, said as the thing that must not happen.
///
/// `cross` reads `A`'s state and `A` holds 7, so 1 is `B`'s number arriving
/// through a closure that was written in `A`. This used to be what the program
/// wrote: `deed check` said nothing, the run exited 0, and the answer was
/// wrong. It fails on the answer rather than on a missing diagnostic, which is
/// the point: a diagnostic is one way to stop this and the right answer is the
/// other, and the language is entitled to either.
#[test]
fn a_closure_written_in_one_handler_never_answers_out_of_another() {
    assert_ne!(
        answer(CROSSED),
        Answer::Wrote("1".to_string()),
        "1 is `B`'s state, read by a closure written in `A`"
    );
}

/// Which of the two ways out it took, and the words it takes it in.
///
/// The test above says the wrong number is gone. This says what is there
/// instead, so that the two together cannot be satisfied by an answer nobody
/// chose.
#[test]
fn the_closure_is_refused_where_it_is_written() {
    assert_eq!(
        answer(CROSSED),
        Answer::Refused(vec![CLOSURE_OVER_STATE.to_string()])
    );

    let (sources, checked) = check(CROSSED);
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("`n` is handler state, and this closure can outlive the handler"),
        "{text}"
    );
    assert!(text.contains("read inside a closure"), "{text}");
    assert!(text.contains("the handler state it names"), "{text}");
    // Why, and what to write instead. A refusal that does not say either is a
    // reader stuck with a rule.
    assert!(
        text.contains("read the state into a local and let the closure carry that number"),
        "{text}"
    );
    assert!(
        text.contains("a handler lives as long as the `with` block that installed it"),
        "{text}"
    );
}

/// The snapshot the refusal points at, running.
///
/// A rule with no way to write the program is a rule that stops the program
/// being written, so the alternative is a test rather than a sentence in a
/// note. The closure carries a number, which is what its type said it did.
#[test]
fn reading_the_state_into_a_local_first_is_the_way_to_write_it() {
    every_test_passes(
        "\
module a

effect Give {
    fn getter() -> Fn() -> Int
}

effect Take {
    fn use_it(f: Fn() -> Int) -> Int
}

handler A implements Give {
    state n: Int

    fn getter() -> Fn() -> Int {
        let current = n
        || { current }
    }
}

handler B implements Take {
    state n: Int

    fn use_it(f) -> Int { f() }
}

fn cross() -> Int
  uses
    Give.getter,
    Take.use_it,
{
    let f = Give.getter()
    Take.use_it(f)
}

test \"the closure carries A's number\" {
    with A { n: 7 } {
        with B { n: 1 } {
            assert cross() == 7
        }
    }
}
",
    );
}

/// Assigning to it is refused for the same reason and says so at the target.
///
/// Reading a handler's state from a value that outlived it is a wrong answer;
/// writing it is a wrong answer somebody else will read later. Both go through
/// the same rule, and the message points at the name being written rather than
/// at the value.
#[test]
fn a_closure_may_not_assign_to_handler_state_either() {
    let (sources, checked) = check(
        "\
module a

effect Give {
    fn bump() -> Fn() -> ()
}

handler A implements Give {
    state n: Int

    fn bump() -> Fn() -> () {
        || { n = 1 }
    }
}
",
    );
    assert_eq!(
        errors(&checked).iter().map(|d| d.code).collect::<Vec<_>>(),
        vec![CLOSURE_OVER_STATE]
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(text.contains("assigned to inside a closure"), "{text}");
}

// -- the neighbours --------------------------------------------------------

const COUNTER: &str = "\
module a

effect Give {
    fn peek() -> Int
}

handler A implements Give {
    state n: Int

    fn peek() -> Int { n }
}

fn ask() -> Int
  uses
    Give.peek,
{
    Give.peek()
}
";

/// A closure written in a handler operation and called inside the same `with`,
/// in the same operation that wrote it.
///
/// This one worked, and it is refused now. Saying so is the cost of the rule
/// rather than an oversight: the alternative is deciding at each closure
/// whether it escapes, and a closure that is called on the next line is one
/// line away from being returned instead. What it wanted is a local, and a
/// local is what it should have been.
#[test]
fn a_closure_in_a_handler_operation_is_refused_even_when_it_never_leaves() {
    let (_, checked) = check(
        "\
module a

effect Give {
    fn peek() -> Int
}

handler A implements Give {
    state n: Int

    fn peek() -> Int {
        let same = || { n }
        same()
    }
}
",
    );
    assert_eq!(
        errors(&checked).iter().map(|d| d.code).collect::<Vec<_>>(),
        vec![CLOSURE_OVER_STATE]
    );
}

/// A closure written outside any handler and called inside one.
///
/// Nothing changes here and nothing could: handler state is in scope inside
/// the handler that declared it and nowhere else, so a closure written
/// anywhere else has no name to reach it by. The rule is about what a closure
/// can be written over, not about where it is called.
#[test]
fn a_closure_written_outside_a_handler_is_still_called_inside_one() {
    every_test_passes(
        "\
module a

effect Give {
    fn through(f: Fn(Int) -> Int) -> Int
}

handler A implements Give {
    state n: Int

    fn through(f) -> Int { f(n) }
}

fn ask(f: Fn(Int) -> Int) -> Int
  uses
    Give.through,
{
    Give.through(f)
}

test \"the closure sees the handler's number as an argument\" {
    let double = |x: Int| x + x
    with A { n: 7 } {
        assert ask(double) == 14
    }
}
",
    );
}

/// Nested `with` blocks of the same effect.
///
/// The inner handler answers while it is installed and the outer one answers
/// again afterwards, each out of its own state. This is the shape the bug
/// looked like from the outside, so it is worth having it pinned as the thing
/// that was always right.
#[test]
fn nested_with_blocks_of_one_effect_each_answer_out_of_their_own_state() {
    every_test_passes(&format!(
        "{COUNTER}
test \"the innermost handler answers\" {{
    with A {{ n: 7 }} {{
        assert ask() == 7
        with A {{ n: 1 }} {{
            assert ask() == 1
        }}
        assert ask() == 7
    }}
}}
"
    ));
}

/// Nested `with` blocks of different effects.
///
/// Two handlers, both installed, both with a state field called `n`. Each
/// operation reads its own, which is the half of this that was never broken:
/// an operation knows which handler it belongs to. It was only a closure that
/// did not.
#[test]
fn nested_with_blocks_of_different_effects_do_not_read_each_others_state() {
    every_test_passes(
        "\
module a

effect Give {
    fn peek() -> Int
}

effect Take {
    fn grab() -> Int
}

handler A implements Give {
    state n: Int

    fn peek() -> Int { n }
}

handler B implements Take {
    state n: Int

    fn grab() -> Int { n }
}

fn both() -> Int
  uses
    Give.peek,
    Take.grab,
{
    Give.peek() + Take.grab()
}

test \"each operation reads its own handler\" {
    with A { n: 7 } {
        with B { n: 1 } {
            assert both() == 8
            assert Give.peek() == 7
            assert Take.grab() == 1
        }
    }
}
",
    );
}

/// A closure stored in handler state and called later.
///
/// Allowed, and it has to be: the closure is written outside the handler, so
/// it names nothing of the handler's, and storing a value is what state is
/// for. It did not run. Calling a name went to the frame and nothing else,
/// while every other read of a name knew that handler state lives in the
/// handler instance, so the value was stored, found by nothing, and `held()`
/// said the interpreter could not run the call.
#[test]
fn a_closure_kept_in_handler_state_is_called_later() {
    every_test_passes(
        "\
module a

effect Give {
    fn keep(f: Fn() -> Int) -> ()
    fn use_kept() -> Int
}

handler A implements Give {
    state held: Fn() -> Int

    fn keep(f) -> () {
        held = f
    }

    fn use_kept() -> Int { held() }
}

fn store(f: Fn() -> Int) -> ()
  uses
    Give.keep,
{
    Give.keep(f)
}

fn later() -> Int
  uses
    Give.use_kept,
{
    Give.use_kept()
}

test \"the closure that was put away is the one that answers\" {
    with A { held: || 0 } {
        store(|| 7)
        assert later() == 7
    }
}
",
    );
}

/// A closure returned out of a `with` block.
///
/// This is the shape the whole argument is about, and it still works when the
/// closure carries a number instead of a handler. `ask()` runs while the
/// handler is installed, the closure captures what it answered, and calling it
/// afterwards is calling a function over an `Int`. Nothing about it needs a
/// handler to still exist, which is exactly why its type does not mention one.
#[test]
fn a_closure_that_carries_a_number_leaves_the_with_block() {
    every_test_passes(&format!(
        "{COUNTER}
test \"the closure outlives the handler it was made under\" {{
    let escaped = with A {{ n: 7 }} {{
        let here = ask()
        || {{ here + 1 }}
    }}
    assert escaped() == 8
}}
"
    ));
}
