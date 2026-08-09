//! What the compiler works out about a callee's parameters.
//!
//! `design/decisions/2026-08-09-what-a-callee-does-with-its-argument.md` names
//! one missing fact and fixes the order: compute it, be able to print it, and
//! only then let anything act on it. These tests are the first half. Nothing
//! reads the answer yet, so a wrong one here costs nothing today and would
//! cost silent data corruption the moment something does — which is why the
//! cases below include the ones the analysis must refuse.

use deed_diagnostics::SourceMap;
use deed_mir::FuncId;
use deed_mir::reuse::{ParamUse, Summaries};

fn summarise(src: &str) -> (deed_mir::Program, Summaries) {
    let mut sources = SourceMap::new();
    let checked = deed_driver::check_text(&mut sources, "a.deed", src);
    assert!(
        !checked.has_errors(),
        "the fixture should check: {:?}",
        checked
            .diagnostics
            .iter()
            .map(|d| d.code)
            .collect::<Vec<_>>()
    );
    let program = deed_mir::lower_with_tests_alongside(
        &checked.module,
        &checked.resolutions,
        &checked.types,
        &[],
    )
    .expect("the fixture should lower");
    let summaries = deed_mir::reuse::summarise(&program);
    (program, summaries)
}

fn use_of(
    program: &deed_mir::Program,
    summaries: &Summaries,
    name: &str,
    param: usize,
) -> ParamUse {
    let at = program
        .functions
        .iter()
        .position(|f| f.name == name || f.name.ends_with(&format!("::{name}")))
        .unwrap_or_else(|| {
            panic!(
                "no function named `{name}`; there are {:?}",
                program
                    .functions
                    .iter()
                    .map(|f| f.name.as_str())
                    .collect::<Vec<_>>()
            )
        });
    summaries.get(FuncId(at)).param(param)
}

/// The function the decision record is about.
///
/// `push` hands back a list whose elements are the argument's, so the result
/// shares. Nothing keeps it past the call, which is the half a caller needs:
/// a caller holding the only reference is handing over something the callee
/// will be finished with.
#[test]
fn a_function_that_pushes_onto_its_argument_returns_it_and_keeps_nothing() {
    let src = "module a\n\n\
        fn added(list: List<Int>, n: Int) -> List<Int> { push(list, n) }\n\n\
        test \"t\" { assert length(added([1], 2)) == 2 }\n";
    let (program, summaries) = summarise(src);

    let list = use_of(&program, &summaries, "added", 0);
    assert!(list.shared_with_result, "the answer holds the argument");
    assert!(!list.retained, "nothing keeps it past the call");
    assert!(!list.released());
}

/// A number is copied into the call, so the question does not arise.
#[test]
fn a_parameter_that_is_not_boxed_has_no_storage_to_share() {
    let src = "module a\n\n\
        fn added(list: List<Int>, n: Int) -> List<Int> { push(list, n) }\n\n\
        test \"t\" { assert length(added([1], 2)) == 2 }\n";
    let (program, summaries) = summarise(src);
    assert!(use_of(&program, &summaries, "added", 1).released());
}

/// Reading a length gives back a number, and a number cannot be the list.
#[test]
fn a_function_that_only_measures_its_argument_releases_it() {
    let src = "module a\n\n\
        fn counted(list: List<Int>) -> Int { length(list) }\n\n\
        test \"t\" { assert counted([1, 2]) == 2 }\n";
    let (program, summaries) = summarise(src);
    assert!(use_of(&program, &summaries, "counted", 0).released());
}

/// Text comes back built character by character, never sharing a byte.
#[test]
fn joining_text_hands_back_something_new() {
    let src = "module a\n\n\
        fn joined(parts: List<String>, sep: String) -> String { join(parts, sep) }\n\n\
        test \"t\" { assert joined([\"a\", \"b\"], \",\") == \"a,b\" }\n";
    let (program, summaries) = summarise(src);
    assert!(use_of(&program, &summaries, "joined", 0).released());
    assert!(use_of(&program, &summaries, "joined", 1).released());
}

/// The helpers that write text write all of it.
///
/// One case each, because they are one arm of one table and a single fixture
/// would leave the rest of the arm saying whatever it liked. Concatenation is
/// the one that looks most like sharing and is not: the answer is as long as
/// both arguments and holds neither.
#[test]
fn every_helper_that_builds_text_builds_all_of_it() {
    let src = "module a\n\n\
        fn shouted(word: String) -> String { word + \"!\" }\n\n\
        fn tidied(word: String) -> String { trim(word) }\n\n\
        fn loud(word: String) -> String { upper(word) }\n\n\
        fn quiet(word: String) -> String { lower(word) }\n\n\
        fn pieces(line: String) -> List<String> { split(line, \",\") }\n\n\
        test \"t\" {\n\
        \x20 assert shouted(\"a\") == \"a!\"\n\
        \x20 assert tidied(\" a \") == \"a\"\n\
        \x20 assert loud(\"a\") == \"A\"\n\
        \x20 assert quiet(\"A\") == \"a\"\n\
        \x20 assert length(pieces(\"a,b\")) == 2\n\
        }\n";
    let (program, summaries) = summarise(src);
    for name in ["shouted", "tidied", "loud", "quiet", "pieces"] {
        let word = use_of(&program, &summaries, name, 0);
        assert!(
            word.released(),
            "`{name}` hands back text it wrote itself: {word:?}"
        );
    }
}

/// A record holding the argument shares through the field it was written to.
#[test]
fn a_record_built_around_the_argument_shares_with_it() {
    let src = "module a\n\n\
        record Holder { items: List<Int>, size: Int }\n\n\
        fn held(list: List<Int>) -> Holder { Holder { items: list, size: length(list) } }\n\n\
        test \"t\" { assert held([1]).size == 1 }\n";
    let (program, summaries) = summarise(src);
    let held = use_of(&program, &summaries, "held", 0);
    assert!(held.shared_with_result);
    assert!(!held.retained);
}

/// The answer is a number, so whatever the walk found on the way out does not
/// leave with it.
#[test]
fn a_function_returning_a_number_shares_nothing_however_it_got_there() {
    let src = "module a\n\n\
        record Holder { items: List<Int>, size: Int }\n\n\
        fn measured(list: List<Int>) -> Int { Holder { items: list, size: 1 }.size }\n\n\
        test \"t\" { assert measured([1]) == 1 }\n";
    let (program, summaries) = summarise(src);
    assert!(use_of(&program, &summaries, "measured", 0).released());
}

/// A handler's `state` is the one thing in the language that outlives the call
/// that wrote it, so a parameter written into it is retained.
#[test]
fn a_parameter_written_into_handler_state_is_retained() {
    let src = "module a\n\n\
        effect Store {\n\
        \x20 fn keep(items: List<Int>) -> ()\n\
        \x20 fn size() -> Int\n\
        }\n\n\
        handler Kept implements Store {\n\
        \x20 state held: List<Int>\n\n\
        \x20 fn keep(items) -> () { held = items }\n\n\
        \x20 fn size() -> Int { length(held) }\n\
        }\n\n\
        fn kept(list: List<Int>) -> Int uses Store.keep, Store.size {\n\
        \x20 Store.keep(list)\n\
        \x20 Store.size()\n\
        }\n\n\
        test \"t\" { with Kept { held: [] } { assert kept([1, 2]) == 2 } }\n";
    let (program, summaries) = summarise(src);

    let keep = program
        .functions
        .iter()
        .position(|f| f.name.contains("keep"))
        .expect("the handler operation is a function");
    let items = summaries.get(FuncId(keep));
    assert!(
        items.params.iter().any(|p| p.retained),
        "writing a parameter into state keeps it: {items:?}"
    );

    // And the caller inherits it, because performing an operation is answered
    // by a handler this call site cannot name.
    let outer = use_of(&program, &summaries, "kept", 0);
    assert!(
        outer.retained,
        "an operation may keep what it is handed: {outer:?}"
    );
}

/// A call through a value has no name to look up.
#[test]
fn a_call_through_a_value_keeps_what_it_is_given() {
    let src = "module a\n\n\
        fn through(list: List<Int>, step: Fn(List<Int>) -> Int) -> Int { step(list) }\n\n\
        test \"t\" { assert through([1], |xs: List<Int>| length(xs)) == 1 }\n";
    let (program, summaries) = summarise(src);
    let list = use_of(&program, &summaries, "through", 0);
    assert!(
        list.retained,
        "nothing here says what the value does with it: {list:?}"
    );
}

/// A summary is read through a call, so a caller of a releasing function
/// releases too, and a caller of a retaining one does not.
#[test]
fn a_caller_reads_its_callees_answer() {
    let src = "module a\n\n\
        fn counted(list: List<Int>) -> Int { length(list) }\n\n\
        fn twice_counted(list: List<Int>) -> Int { counted(list) + counted(list) }\n\n\
        test \"t\" { assert twice_counted([1]) == 2 }\n";
    let (program, summaries) = summarise(src);
    assert!(use_of(&program, &summaries, "twice_counted", 0).released());
}

/// Recursion converges rather than being refused for being recursive.
#[test]
fn a_function_that_calls_itself_still_gets_an_answer() {
    let src = "module a\n\n\
        fn drained(list: List<Int>, seen: Int) -> Int uses Diverge {\n\
        \x20 if seen >= length(list) { seen } else { drained(list, seen + 1) }\n\
        }\n\n\
        test \"t\" { assert drained([1, 2], 0) == 2 }\n";
    let (program, summaries) = summarise(src);
    assert!(use_of(&program, &summaries, "drained", 0).released());
}

/// Handing a list to the host is handing it somewhere this program cannot see.
#[test]
fn what_goes_to_the_host_is_retained() {
    let src = "module a\n\n\
        fn wrote(sys: System, line: String) -> () uses Io.write { Io.write(sys.console, line) }\n";
    let (program, summaries) = summarise(src);
    let line = use_of(&program, &summaries, "wrote", 1);
    assert!(line.retained, "{line:?}");
}

/// The printed form is what a person tuning this reads, and the decision
/// record asks for it by name because the cheap answer is otherwise silent.
#[test]
fn the_summary_can_be_printed() {
    let src = "module a\n\n\
        fn added(list: List<Int>, n: Int) -> List<Int> { push(list, n) }\n\n\
        fn counted(list: List<Int>) -> Int { length(list) }\n\n\
        test \"t\" { assert length(added([1], 2)) == counted([1, 2]) }\n";
    let (program, summaries) = summarise(src);
    let printed = summaries.print(&program);

    assert!(printed.contains("returns"), "{printed}");
    assert!(printed.contains("releases"), "{printed}");
    for word in ["added", "counted"] {
        assert!(printed.contains(word), "{printed}");
    }
}

/// The helpers that hand back a number, a boolean or a fresh `Result` hand
/// back none of what they were given.
///
/// `to_int` is the one worth a case of its own: it answers with a `Result`,
/// which is an aggregate and so is boxed, so unlike `length` its answer is a
/// thing that could in principle hold the argument. It does not.
#[test]
fn reading_a_value_out_of_text_hands_back_none_of_it() {
    let src = "module a\n\n\
        fn parsed(text: String) -> Result<Int, String> { to_int(text) }\n\n\
        fn same(left: String, right: String) -> Bool { left == right }\n\n\
        test \"t\" {\n\
        \x20 assert parsed(\"1\") == ok(1)\n\
        \x20 assert same(\"a\", \"a\")\n\
        }\n";
    let (program, summaries) = summarise(src);
    assert!(use_of(&program, &summaries, "parsed", 0).released());
    assert!(use_of(&program, &summaries, "same", 0).released());
    assert!(use_of(&program, &summaries, "same", 1).released());
}

/// An element read out of a list lives inside the list, and the index does not.
#[test]
fn an_element_comes_from_the_list_rather_than_the_index() {
    let src = "module a\n\n\
        fn picked(words: List<String>, index: Int) -> Result<String, String> { at(words, index) }\n\n\
        test \"t\" { assert picked([\"a\"], 0) == ok(\"a\") }\n";
    let (program, summaries) = summarise(src);

    let words = use_of(&program, &summaries, "picked", 0);
    assert!(
        words.shared_with_result,
        "the answer holds a string that lives in the list: {words:?}"
    );
    assert!(!words.retained);
    assert!(
        use_of(&program, &summaries, "picked", 1).released(),
        "the index is a number"
    );
}

/// An answer travels back up a chain of calls, however deep.
///
/// Three deep and written outermost first, so a single pass over the functions
/// in declaration order cannot reach it: the outer one is looked at before the
/// one it depends on has an answer. Anything that stopped iterating early
/// would leave `outer` looking like it releases, which is the wrong half.
#[test]
fn an_answer_travels_back_up_a_chain_of_calls() {
    let src = "module a\n\n\
        fn outer(line: String, sys: System) -> () uses Io.write { middle(line, sys) }\n\n\
        fn middle(line: String, sys: System) -> () uses Io.write { inner(line, sys) }\n\n\
        fn inner(line: String, sys: System) -> () uses Io.write { Io.write(sys.console, line) }\n";
    let (program, summaries) = summarise(src);

    for name in ["outer", "middle", "inner"] {
        let line = use_of(&program, &summaries, name, 0);
        assert!(
            line.retained,
            "`{name}` hands it towards the host: {line:?}"
        );
    }
}

/// How much of the shipped library the analysis can answer.
///
/// A floor rather than the exact number: the point is that the answer is
/// mostly not the safe default, because an analysis that refused everything
/// would pass every test above and buy nothing. Measured on 2026-08-09 at
/// 329 of 389 boxed parameters not retained.
#[test]
fn the_analysis_answers_most_of_the_library_rather_than_defaulting() {
    let names: Vec<&str> = deed_driver::shipped_modules().collect();
    let mut sources = SourceMap::new();
    let mut files = Vec::new();
    for name in &names {
        let text = deed_driver::shipped_source(name).expect("a shipped module");
        files.push(sources.add(format!("{name}.deed"), text.to_string()));
    }
    let checks = deed_driver::check_all(&sources, &files);

    let mut boxed = 0;
    let mut answered = 0;

    for (at, check) in checks.iter().enumerate() {
        let alongside: Vec<deed_mir::lower::Alongside> = checks
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != at)
            .map(|(_, c)| deed_mir::lower::Alongside {
                module: &c.module,
                resolutions: &c.resolutions,
                types: &c.types,
            })
            .collect();
        let program = deed_mir::lower_with_tests_alongside(
            &check.module,
            &check.resolutions,
            &check.types,
            &alongside,
        )
        .expect("the shipped library lowers");
        let summaries = deed_mir::reuse::summarise(&program);

        for (index, function) in program.functions.iter().enumerate() {
            for (param, ty) in function.params.iter().enumerate() {
                if !ty.is_boxed() {
                    continue;
                }
                boxed += 1;
                if !summaries.get(FuncId(index)).param(param).retained {
                    answered += 1;
                }
            }
        }
    }

    assert!(
        boxed > 200,
        "only {boxed} boxed parameters, so this measured almost nothing"
    );
    assert!(
        answered * 4 >= boxed * 3,
        "only {answered} of {boxed} boxed parameters are not retained, \
         which is close enough to the safe default to buy nothing"
    );
}
