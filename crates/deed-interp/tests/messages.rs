//! Every message the interpreter can produce, read.
//!
//! `crates/deed-driver/tests/codes.rs` matches on a code's name, so one tested
//! shape satisfies it for a code with several messages behind it. The
//! interpreter had ten codes and forty-eight messages, and eight of the
//! forty-eight had ever been rendered by a test. Three more had a note read
//! while the message it hangs off was not: `DEED6008`'s in `properties.rs`,
//! `DEED6007`'s and `DEED6001`'s in `running.rs`. How the forty-eight divide
//! between the ten codes is written down once, on the constants in
//! `crates/deed-interp/src/codes.rs`.
//!
//! A runtime message is worse to leave unread than a checker message. It fires
//! on a program that already type checked, so the reader has been told
//! everything is fine and then it is not, and there is no fix to offer because
//! the program is running. All the message has is its words.
//!
//! # Three kinds of message, and why the note is not the same for all of them
//!
//! Reading them turned up that they are not one family. Most of the shapes
//! that used to share one helper are shapes `deed check` refuses, so they are
//! reached only when the interpreter is handed a program the checker would
//! have turned down. The message said the language permits them and the
//! interpreter has not got round to it, which is the opposite of true, and it
//! said "yet", which promised work that is not coming.
//!
//! Four are not that. Two really are the interpreter's own gap, and `deed
//! check` accepts both. One is neither: a call into a module whose code was never
//! handed over, which is a gap in what this library was given, as its own
//! `codes.rs` has said all along. And one, `sys.files` with no directory
//! behind it, is an ordinary runtime fact about a program that is right.
//!
//! # What is not here
//!
//! Messages already rendered somewhere else are read there and not again, so
//! that breaking one string fails one test. Those are `unchanged` outside a
//! contract, the call into a module that was not handed over and the handler
//! missing an initial value, all in `running.rs`; the assertion note showing
//! both sides, also in `running.rs`; the precondition and postcondition
//! messages, in `running.rs`; the refinement message and the `assert refuses`
//! message, in `deed-driver/tests/guards.rs`; the missing handler, in
//! `running.rs`; `sys.files` with no directory, in
//! `deed-driver/tests/capabilities.rs`; the property with too few cases, in
//! `properties.rs`; and the row a run did not keep, in
//! `deed-driver/tests/rows_at_runtime.rs`.
//!
//! Two messages are here in prose only, because nothing can reach them:
//! `this effect operation`, which needs an effect operation whose definition
//! has no parent, and `a closure the interpreter lost track of`, which needs
//! an index into a table that is only ever appended to. Both are invariants
//! rather than dead code, and are argued about at the bottom of this file.
//!
//! # Wording and code are two claims
//!
//! [`Reported::under`] appears on four of these and not on the rest, which is
//! deliberate. Swapping the code constant at each of the eighteen emission
//! sites in turn found five whose wording was read and whose code was held by
//! nothing at all, so those five say which code they arrive under as well as
//! what they say: four here and the handler missing an initial value, which is
//! read in `running.rs` and says so there. Putting it on all of them instead
//! would mean one swap fails twenty-five tests, which is noise rather than an
//! answer.
//!
//! Most of what is below runs the interpreter over a file the type checker
//! would refuse. That is deliberate and it is the only way to see these at
//! all: `Program::add` takes a parse tree and a set of resolutions, and
//! nothing makes it ask whether anybody checked them.

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_interp::{DeclaredRows, Guards, Program, codes, run_main, run_tests};
use deed_lexer::tokenize;
use deed_parser::parse;
use deed_resolve::{Universe, resolve};
use std::path::Path;

/// One runtime failure, as a reader meets it and as a tool reads it.
struct Reported {
    code: &'static str,
    text: String,
    underlined: String,
}

impl Reported {
    /// What the reader sees, anywhere in the rendering.
    fn says(&self, needle: &str) -> &Self {
        assert!(
            self.text.contains(needle),
            "expected `{needle}` in:\n{}",
            self.text
        );
        self
    }

    fn never_says(&self, needle: &str) -> &Self {
        assert!(
            !self.text.contains(needle),
            "did not expect `{needle}` in:\n{}",
            self.text
        );
        self
    }

    /// What the caret is drawn under.
    ///
    /// The rendering says the same thing, but only as a row of arrows under a
    /// line of source, so reading it out of the text would pin the renderer's
    /// layout rather than the span.
    fn underlines(&self, expected: &str) -> &Self {
        assert_eq!(self.underlined, expected, "in:\n{}", self.text);
        self
    }

    /// Which code it arrived under.
    fn under(&self, code: &str) -> &Self {
        assert_eq!(self.code, code, "in:\n{}", self.text);
        self
    }
}

/// The source the primary caret is drawn over, in the file the failure names.
fn underlined(sources: &SourceMap, failure: &Diagnostic) -> String {
    let span = failure.primary.span;
    sources.file(failure.file).text()[span.start as usize..span.end as usize].to_string()
}

/// The one failure the single test in `src` produces.
///
/// Nothing here asserts that the source lexes, parses or resolves cleanly, the
/// way `running.rs` does. Several of these messages are only reachable through
/// a file that did not, and refusing to run one would put those messages out of
/// reach of any test at all.
fn message(src: &str) -> Reported {
    message_in(src, &Universe::new())
}

fn message_in(src: &str, universe: &Universe) -> Reported {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, universe);

    let mut program = Program::new();
    program.add(
        file,
        &parsed.module,
        &resolved.resolutions,
        Guards::new(),
        DeclaredRows::new(),
    );

    let mut outcomes = run_tests(&program, file);
    assert_eq!(outcomes.len(), 1, "expected exactly one test");
    let failure = outcomes
        .remove(0)
        .failure
        .expect("the test should have failed");
    Reported {
        code: failure.code,
        underlined: underlined(&sources, &failure),
        text: render_human(&sources, &failure),
    }
}

/// The failure `main` produces, run with `root` behind `sys.files`.
///
/// A `test` block has no `System` to hand out, so everything about a
/// capability has to go through an entry point.
fn message_from_main(src: &str, root: &Path) -> Reported {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, &Universe::new());

    let mut program = Program::new();
    program.add(
        file,
        &parsed.module,
        &resolved.resolutions,
        Guards::new(),
        DeclaredRows::new(),
    );

    let run = run_main(&program, file, root, &[]).expect("there should be a main");
    let failure = run.result.expect_err("`main` should have failed");
    Reported {
        code: failure.code,
        underlined: underlined(&sources, &failure),
        text: render_human(&sources, &failure),
    }
}

/// A directory that is really there, for the filesystem shapes.
///
/// None of them get as far as touching it: every one is refused for the shape
/// of its arguments before a path is resolved.
fn somewhere() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn wrap(body: &str) -> String {
    format!("module a\n\ntest \"t\" {{\n{body}\n}}\n")
}

/// An effect with two operations, for the handler shapes.
const EFFECT: &str = "\
effect E {
    fn one() -> Int
    fn two() -> Int
}
";

// -- DEED6001, an assertion ------------------------------------------------

/// The headline, which is what a reader sees when the condition is not a
/// comparison and there are no two sides to show.
#[test]
fn an_assertion_that_is_not_true_says_so() {
    message(&wrap("  assert false")).says("this assertion is not true");
}

// -- DEED6005, no handler --------------------------------------------------

/// Unreachable from a file that checked: `DEED4029` refuses a handler that
/// leaves an operation out. Kept, and addressed to the compiler, for the same
/// reason `DEED6010` is.
#[test]
fn a_handler_that_leaves_an_operation_out_is_named_with_the_operation() {
    let src = format!(
        "module a\n\n{EFFECT}\n\
         handler H implements E {{\n\
         \x20 fn one() -> Int {{ 1 }}\n\
         }}\n\n\
         fn f() -> Int\n\
         \x20 uses E.two,\n\
         {{\n\
         \x20 E.two()\n\
         }}\n\n\
         test \"t\" {{\n\
         \x20 with H {{\n\
         \x20   assert f() == 1\n\
         \x20 }}\n\
         }}\n"
    );
    message(&src)
        .under(codes::NO_HANDLER)
        .says("the handler `H` does not implement `two`")
        .says("hole in the type checker");
}

/// Also unreachable from a file that checked: every `Io` operation declares
/// which capability it takes, so handing it another is a type error.
#[test]
fn an_io_operation_names_the_capability_it_was_handed() {
    message_from_main(
        "module a\n\n\
         fn main(sys: System) -> ()\n\
         \x20 uses Io.write,\n\
         {\n\
         \x20 Io.write(sys.clock, \"hi\")\n\
         }\n",
        somewhere(),
    )
    .under(codes::NO_HANDLER)
    .says("`Io.write` cannot be performed with a `Clock`");
}

// -- DEED6006, the shapes the checker refuses ------------------------------

/// The note every message through the shared helper carries.
///
/// It used to say the opposite, that this was a gap in the interpreter rather
/// than something the language forbids, and it is read here once rather than
/// on every message that carries it.
#[test]
fn the_shared_note_points_at_the_check_rather_than_at_the_interpreter() {
    message(&wrap("  assert 1 + true == 2"))
        .says(
            "nothing that passes `deed check` reaches this, so either this file was not checked or the check has a hole",
        )
        // The "yet" that went with the old note. Nothing is coming: the answer
        // to every one of these is a diagnostic from an earlier pass.
        .never_says("yet");
}

#[test]
fn a_unary_operator_names_itself_and_the_value() {
    // Its binary neighbour has always named both sides. This one said "this
    // operator on this value", which is what the caret already said.
    message(&wrap("  assert !\"hi\" == \"hi\"")).says("the interpreter cannot run `!` on a String");
}

#[test]
fn a_binary_operator_names_both_sides() {
    message(&wrap("  assert 1 + true == 2"))
        .says("the interpreter cannot run `+` on an Int and a Bool");
}

#[test]
fn an_operator_two_strings_do_not_have_is_told_apart_from_the_rest() {
    message(&wrap("  assert \"a\" - \"b\" == \"\""))
        .says("the interpreter cannot run `-` on two Strings");
}

#[test]
fn a_question_mark_on_something_that_is_not_a_result_says_what_it_was() {
    message(&wrap("  let n = 5\n  let m = n?\n  assert m == 5"))
        .says("the interpreter cannot run `?` on an Int, which is not a Result");
}

#[test]
fn a_for_over_something_that_is_not_a_list_says_what_it_was() {
    message(&wrap("  let s = for n in 5 { n }\n  assert s == 5"))
        .says("the interpreter cannot run a `for` over an Int");
}

#[test]
fn a_for_condition_that_is_not_a_bool_says_what_it_was() {
    message(&wrap(
        "  let s = for n in [1, 2] with acc = 0 while acc { acc + n }\n  assert s == 3",
    ))
    .says("the interpreter cannot run a `for` condition that is an Int");
}

#[test]
fn a_condition_that_is_not_a_bool_says_so() {
    message(&wrap("  if 1 { }\n  assert true"))
        .says("the interpreter cannot run a condition that is not a Bool");
}

#[test]
fn old_outside_a_contract_is_refused() {
    // The sibling of `unchanged` outside a contract, which `running.rs` reads.
    // Both read what entering a call captured, and entering captures for the
    // sake of the `ensures` clauses.
    message("module a\n\nfn f(n: Int) -> Int { old(n) }\n\ntest \"t\" {\n  assert f(1) == 1\n}\n")
        .says("the interpreter cannot run `old` outside a contract");
}

#[test]
fn an_effect_reference_that_is_not_an_effect_is_refused() {
    message(
        "module a\n\nrecord R { n: Int }\n\n\
         fn f() -> Bool\n\
         \x20 ensures ok => unchanged(R),\n\
         { true }\n\n\
         test \"t\" {\n  assert f()\n}\n",
    )
    .says("the interpreter cannot run this effect reference");
}

#[test]
fn code_that_did_not_parse_is_refused_rather_than_guessed_at() {
    message(&wrap("  assert 1 * == 2"))
        .says("the interpreter cannot run code that did not compile");
}

#[test]
fn a_name_the_resolver_could_not_find_is_refused() {
    message(&wrap("  assert nonesuch == 1")).says("the interpreter cannot run an unresolved name");
}

#[test]
fn a_name_that_is_not_a_value_says_which_name() {
    message(
        "module a\n\nrecord Point { x: Int }\n\n\
         test \"t\" {\n  let p = Point\n  assert true\n}\n",
    )
    .says("the interpreter cannot run `Point`, which has no value here");
}

#[test]
fn a_field_on_something_with_no_fields_says_what_it_was() {
    // The span is the receiver rather than the field name. Nothing is wrong
    // with the name: a value of the right shape would have had it, and what
    // the reader has to look at is the thing that turned out not to be that
    // shape.
    message(&wrap("  let n = 1\n  assert n.x == 1"))
        .says("the interpreter cannot run field access on an Int")
        .underlines("n");
}

#[test]
fn a_field_a_value_does_not_have_says_which_field() {
    message(
        "module a\n\nrecord Point { x: Int }\n\n\
         test \"t\" {\n  let p = Point { x: 1 }\n  assert p.y == 1\n}\n",
    )
    .says("the interpreter cannot run `y`, which the value does not have");
}

#[test]
fn a_call_that_is_not_a_call_says_only_that() {
    // One message, eleven emission sites: a callee that is not a name, a
    // declaration with no body, a prelude function handed the wrong shapes,
    // and so on. They are left sharing a sentence deliberately. What tells
    // them apart is which arm of the interpreter's call machinery gave up,
    // and a reader cannot act on that: the note already says the file was not
    // checked or the check has a hole, and the caret is already on the call.
    // Eleven sentences would name parts of the interpreter rather than parts
    // of the program, which is the mistake this change is undoing everywhere
    // else in this file.
    message(&wrap("  assert length(1) == 1")).says("the interpreter cannot run this call");
}

#[test]
fn a_closure_called_with_the_wrong_number_of_arguments_says_so() {
    message(&wrap("  let f = |x: Int| { x }\n  assert f(1, 2) == 1"))
        .says("the interpreter cannot run a closure called with the wrong arity");
}

#[test]
fn a_literal_that_names_something_that_is_not_a_shape_is_refused() {
    message(&format!(
        "module a\n\n{EFFECT}\ntest \"t\" {{\n  let x = E {{ a: 1 }}\n  assert true\n}}\n"
    ))
    .says("the interpreter cannot run this literal");
}

#[test]
fn installing_something_that_is_not_a_handler_is_refused() {
    message(&format!(
        "module a\n\n{EFFECT}\nrecord R {{ n: Int }}\n\n\
         test \"t\" {{\n  with R {{ n: 1 }} {{\n    assert true\n  }}\n}}\n"
    ))
    .says("the interpreter cannot run this handler");
}

#[test]
fn a_handler_implementing_something_that_is_not_an_effect_is_refused() {
    message(
        "module a\n\nrecord R { n: Int }\n\n\
         handler H implements R {\n\
         \x20 fn one() -> Int { 1 }\n\
         }\n\n\
         test \"t\" {\n  with H {\n    assert true\n  }\n}\n",
    )
    .says("the interpreter cannot run this handler's effect");
}

#[test]
fn an_assignment_to_a_name_the_resolver_could_not_find_is_refused() {
    message(&wrap("  nonesuch = 2\n  assert true"))
        .says("the interpreter cannot run this assignment");
}

#[test]
fn an_assignment_outside_a_handler_is_refused() {
    // Handler state is the one thing in the language that can change, so an
    // assignment anywhere else has nothing it could be assigning to.
    message(&wrap("  let x = 1\n  x = 2\n  assert true"))
        .says("the interpreter cannot run assignment from outside a handler");
}

#[test]
fn a_match_that_ran_out_of_arms_says_what_it_was_given() {
    message(
        "module a\n\nchoice T { A, B }\n\n\
         fn f(t: T) -> Int {\n\
         \x20 match t {\n\
         \x20   A => 1,\n\
         \x20 }\n\
         }\n\n\
         test \"t\" {\n  assert f(B) == 1\n}\n",
    )
    .under(codes::NOT_RUNNABLE)
    .says("no arm of this match accepted B")
    // The note as well, because it is the part that says whose bug this is,
    // and it is not the note the rest of `DEED6006` carries: exhaustiveness
    // is checked, so a match that runs out of arms is a hole in that check
    // rather than a file nobody checked.
    .says("the type checker believes this match is exhaustive");
}

// -- DEED6006, the built-in effect -----------------------------------------

#[test]
fn a_system_capability_has_no_field_but_its_three() {
    message_from_main(
        "module a\n\nfn main(sys: System) -> () {\n  let x = sys.nonesuch\n  ()\n}\n",
        somewhere(),
    )
    .says("the interpreter cannot run `System.nonesuch`, which does not exist");
}

#[test]
fn an_io_operation_with_no_capability_at_all_is_refused() {
    message_from_main(
        "module a\n\n\
         fn main(sys: System) -> ()\n\
         \x20 uses Io.write,\n\
         {\n\
         \x20 Io.write(1, \"hi\")\n\
         }\n",
        somewhere(),
    )
    .says("the interpreter cannot run an `Io` operation with no capability");
}

#[test]
fn a_filesystem_operation_with_no_name_is_refused() {
    message_from_main(
        "module a\n\n\
         fn main(sys: System) -> ()\n\
         \x20 uses Io.read,\n\
         {\n\
         \x20 let r = Io.read(sys.files)\n\
         \x20 ()\n\
         }\n",
        somewhere(),
    )
    .says("the interpreter cannot run a filesystem operation with no name");
}

#[test]
fn a_filesystem_name_that_is_not_a_string_says_what_it_was() {
    message_from_main(
        "module a\n\n\
         fn main(sys: System) -> ()\n\
         \x20 uses Io.read,\n\
         {\n\
         \x20 let r = Io.read(sys.files, 1)\n\
         \x20 ()\n\
         }\n",
        somewhere(),
    )
    .says("the interpreter cannot run a filesystem name that is an Int");
}

#[test]
fn a_save_with_nothing_to_save_is_refused() {
    message_from_main(
        "module a\n\n\
         fn main(sys: System) -> ()\n\
         \x20 uses Io.save,\n\
         {\n\
         \x20 let r = Io.save(sys.files, \"x.txt\")\n\
         \x20 ()\n\
         }\n",
        somewhere(),
    )
    .says("the interpreter cannot run a save with nothing to save");
}

#[test]
fn contents_that_are_not_a_string_say_what_they_were() {
    message_from_main(
        "module a\n\n\
         fn main(sys: System) -> ()\n\
         \x20 uses Io.save,\n\
         {\n\
         \x20 let r = Io.save(sys.files, \"x.txt\", 1)\n\
         \x20 ()\n\
         }\n",
        somewhere(),
    )
    .says("the interpreter cannot run a file's contents that are an Int");
}

// -- DEED6006, the interpreter's own gap -----------------------------------

/// A closure written inside a handler operation reads handler state through
/// whichever handler is innermost when the closure is called, not the one it
/// was written in. Called after the operation returned, that is no handler at
/// all.
///
/// `deed check` accepts this program, which is what makes it the interpreter's
/// gap rather than the checker's, and why it does not carry the note the
/// shapes above carry.
#[test]
fn a_closure_that_outlives_its_handler_operation_says_the_gap_is_here() {
    message(
        "module a\n\n\
         effect Give {\n\
         \x20 fn getter() -> Fn() -> Int\n\
         }\n\n\
         handler A implements Give {\n\
         \x20 state n: Int\n\n\
         \x20 fn getter() -> Fn() -> Int { || { n } }\n\
         }\n\n\
         fn escape() -> Int\n\
         \x20 uses Give.getter,\n\
         {\n\
         \x20 let f = Give.getter()\n\
         \x20 f()\n\
         }\n\n\
         test \"t\" {\n\
         \x20 with A { n: 7 } {\n\
         \x20   assert escape() == 7\n\
         \x20 }\n\
         }\n",
    )
    .under(codes::NOT_RUNNABLE)
    .says("the interpreter cannot run handler state from outside a handler yet")
    .says(
        "this is a gap in the interpreter rather than something the language forbids; please report it",
    );
}

/// The same gap, one step along: the closure is called inside another
/// handler's operation, so it looks in that handler's state and does not find
/// the name.
///
/// When the two handlers happen to use the same state name there is no message
/// at all and the closure quietly reads the other handler's number. That is a
/// bug in the interpreter rather than a wording problem, and it is reported
/// rather than fixed here.
#[test]
fn a_closure_reaching_into_another_handlers_state_says_the_gap_is_here() {
    message(
        "module a\n\n\
         effect Give {\n\
         \x20 fn getter() -> Fn() -> Int\n\
         }\n\n\
         effect Take {\n\
         \x20 fn use_it(f: Fn() -> Int) -> Int\n\
         }\n\n\
         handler A implements Give {\n\
         \x20 state n: Int\n\n\
         \x20 fn getter() -> Fn() -> Int { || { n } }\n\
         }\n\n\
         handler B implements Take {\n\
         \x20 state m: Int\n\n\
         \x20 fn use_it(f) -> Int { f() }\n\
         }\n\n\
         fn cross() -> Int\n\
         \x20 uses Give.getter, Take.use_it,\n\
         {\n\
         \x20 let f = Give.getter()\n\
         \x20 Take.use_it(f)\n\
         }\n\n\
         test \"t\" {\n\
         \x20 with A { n: 7 } {\n\
         \x20   with B { m: 1 } {\n\
         \x20     assert cross() == 7\n\
         \x20   }\n\
         \x20 }\n\
         }\n",
    )
    .says("the interpreter cannot run handler state that was never initialised yet");
}

// -- DEED6006, a gap in what the runner was given --------------------------

/// The note, which is the whole reason this one is not `not_runnable`.
///
/// The wording is read in `running.rs`, by the test that arranges the
/// condition. This reads what it says about whose problem it is, which is what
/// `codes.rs` has claimed about this arm since it was written.
#[test]
fn a_module_that_was_not_handed_over_blames_neither_the_language_nor_the_check() {
    let mut universe = Universe::new();
    let mut sources = SourceMap::new();
    let dep = sources.add(
        "dep.deed",
        "module b\n\nfn twice(n: Int) -> Int { n + n }\n",
    );
    let lexed = tokenize(dep, sources.file(dep).text());
    universe.add(&parse(dep, &lexed.tokens).module);

    message_in(
        "module a\n\nuse b.{twice}\n\ntest \"t\" {\n  assert twice(21) == 42\n}\n",
        &universe,
    )
    .says(
        "every module a program calls into has to be in the `Program`, and this one was resolved without being added",
    );
}

// -- DEED6007 and DEED6009 -------------------------------------------------

/// The headline. `running.rs` reads the label and the note beneath it, which
/// say `overflow` and that `Int` does not wrap; neither of them says what the
/// diagnostic is about.
#[test]
fn arithmetic_with_no_answer_says_that_in_its_headline() {
    message(
        "module a\n\nfn divide(a: Int, b: Int) -> Int { a / b }\n\n\
         test \"t\" {\n  assert divide(1, 0) == 0\n}\n",
    )
    .says("this arithmetic has no answer");
}

#[test]
fn a_run_that_went_too_deep_says_how_deep_it_was_willing_to_go() {
    message(
        "module a\n\n\
         fn forever(n: Int) -> Int\n\
         \x20 uses\n\
         \x20   Diverge,\n\
         {\n\
         \x20 forever(n + 1)\n\
         }\n\n\
         test \"t\" {\n  assert forever(0) == 0\n}\n",
    )
    .says("this call went more than 128 deep");
}

// -- what nothing can reach ------------------------------------------------

/// Two arms are unreachable, and both are kept.
///
/// `this effect operation` needs an effect operation whose definition has no
/// parent. The resolver writes the parent in at all three places an operation
/// gets a definition: the declaration, the first mention through an import,
/// and the built-in `Io` operations it seeds a universe with. So there is no
/// operation without one.
///
/// `a closure the interpreter lost track of` needs a closure value holding an
/// index that is not in the table. The table is only ever appended to, never
/// truncated and never cleared, and the only thing that produces an index is
/// the push that made the entry.
///
/// Neither is dead code and neither is deleted. An invariant check that has
/// never fired is an invariant nothing has violated, which is the case for
/// keeping it rather than against: it costs one arm of a match that is being
/// written anyway, and the alternative to refusing is carrying on with a value
/// that is not what it claims. `DEED6010` is the same argument made out loud,
/// and it is the reason the effect checker's five holes were findable at all.
///
/// This test is here so that the reasoning fails loudly rather than going
/// stale. If somebody removes one of the two arms, the paragraph above is what
/// needs rewriting.
#[test]
fn the_two_arms_nothing_can_reach_are_still_there() {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("interp.rs"),
    )
    .expect("the interpreter's source should be readable");

    for arm in [
        "\"this effect operation\"",
        "\"a closure the interpreter lost track of\"",
    ] {
        assert!(
            source.contains(arm),
            "{arm} is gone, so the argument for keeping it in \
             crates/deed-interp/tests/messages.rs needs revisiting"
        );
    }
}
