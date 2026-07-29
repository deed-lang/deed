//! Every message the type checker can produce, read.
//!
//! `checking.rs` proves the checker refuses and accepts the right programs, but
//! most of those tests hold on diagnostic codes rather than on the words a
//! reader sees. A code is not a sentence. This file renders the diagnostics,
//! asserts the sentences, and pins the code at each site whose wording appears
//! here so that swapping constants is caught too.

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_lexer::tokenize;
use deed_parser::parse;
use deed_resolve::{Universe, resolve};
use deed_typeck::{Checked, World, check, codes, surface};

/// The other modules a test source can see.
#[derive(Default)]
struct Deps {
    universe: Universe,
    world: World,
}

/// One diagnostic from the type checker, as a reader meets it.
struct Reported {
    code: &'static str,
    text: String,
}

impl Reported {
    fn says(&self, needle: &str) -> &Self {
        assert!(
            self.text.contains(needle),
            "expected `{needle}` in:\n{}",
            self.text
        );
        self
    }

    fn under(&self, code: &str) -> &Self {
        assert_eq!(self.code, code, "in:\n{}", self.text);
        self
    }
}

fn message(src: &str) -> Reported {
    message_in(src, &Deps::default())
}

fn message_with(src: &str, modules: &[&str]) -> Reported {
    message_in(src, &universe_of(modules))
}

fn message_in(src: &str, deps: &Deps) -> Reported {
    let (sources, checked) = check_source_in(src, deps);
    assert_eq!(
        checked.diagnostics.len(),
        1,
        "expected exactly one diagnostic:\n{}",
        rendered(&sources, &checked.diagnostics)
    );
    let d = &checked.diagnostics[0];
    Reported {
        code: d.code,
        text: render_human(&sources, d),
    }
}

fn check_source_in(src: &str, deps: &Deps) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);

    let lexed = tokenize(file, sources.file(file).text());
    assert!(!lexed.has_errors(), "test source should lex cleanly");
    let parsed = parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "test source should parse cleanly");
    let resolved = resolve(file, &parsed.module, &deps.universe);
    assert!(
        !resolved.has_errors(),
        "test source should resolve cleanly: {:?}",
        resolved
            .diagnostics
            .iter()
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    );

    let checked = check(file, &parsed.module, &resolved.resolutions, &deps.world);
    (sources, checked)
}

fn universe_of(modules: &[&str]) -> Deps {
    let mut deps = Deps::default();
    let mut sources = SourceMap::new();
    for (index, source) in modules.iter().enumerate() {
        let file = sources.add(format!("dep{index}.deed"), *source);
        let lexed = tokenize(file, sources.file(file).text());
        let parsed = parse(file, &lexed.tokens);
        deps.universe.add(&parsed.module);
    }

    let mut sources = SourceMap::new();
    let mut surfaces = Vec::new();
    for (index, source) in modules.iter().enumerate() {
        let file = sources.add(format!("dep{index}.deed"), *source);
        let lexed = tokenize(file, sources.file(file).text());
        let parsed = parse(file, &lexed.tokens);
        let resolved = resolve(file, &parsed.module, &deps.universe);
        if let Some(name) = &parsed.module.name {
            surfaces.push((
                name.to_string_path(),
                surface(file, &parsed.module, &resolved.resolutions),
            ));
        }
    }
    deps.world = World::of(surfaces);
    deps
}

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

const FIRST: &str = "module a\n\n\
     fn first<T>(items: List<T>) -> Result<T, String> {\n\
     \x20   at(items, 0)\n\
     }\n\n";

const COUNTER: &str = "module a\n\n\
     effect Counter {\n    fn set(to: Int) -> ()\n    fn value() -> Int\n}\n\n";

const RESULT_PRELUDE: &str = "module a\n\n\
     choice Failure {\n\
     \x20   TooBig { limit: Int },\n\
     \x20   Empty,\n\
     }\n\n\
     fn small(n: Int) -> Result<Int, Failure> {\n\
     \x20   if n > 10 {\n\
     \x20       return err(TooBig { limit: 10 })\n\
     \x20   }\n\
     \x20   ok(n)\n\
     }\n";

#[test]
fn type_declarations_and_type_positions_are_read() {
    message("module a\n\ntype NonEmpty<T> = List<T> where length(value) > 0\n")
        .under(codes::REFINEMENT_TYPE_PARAM)
        .says("`NonEmpty` has a predicate, so it cannot take `T`")
        .says("a refinement takes no type parameters")
        .says("the predicate it carries")
        .says("parameters on one are the same substitution a `record` does")
        .says("has nothing it can say");

    message("module a\n\nfn empty<T>() -> List<T> { [] }\n")
        .under(codes::UNDETERMINED_TYPE_PARAM)
        .says("nothing at a call site says what `T` is")
        .says("appears in no parameter's type")
        .says("a return type is what a call produces");

    message("module a\n\nfn odd<T>(n: Int) -> Int { n }\n")
        .under(codes::UNDETERMINED_TYPE_PARAM)
        .says("nothing at a call site says what `T` is")
        .says("nothing to match");

    message("module a\n\nfn f() -> Result<Int> { ok(1) }\n")
        .under(codes::NOT_GENERIC)
        .says("`Result` takes exactly two type arguments, and 1 was given")
        .says("wrong number of type arguments")
        .says("`Result<Value, Error>`");

    message("module a\n\nfn f() -> List<Int, Int> { [] }\n")
        .under(codes::NOT_GENERIC)
        .says("`List` takes exactly one type argument, and 2 were given")
        .says("`List<Element>`");

    message("module a\n\nfn thing() -> Int { 0 }\n\nfn f(x: thing) -> Int { 0 }\n")
        .under(codes::NOT_A_TYPE)
        .says("`thing` is a function, not a type")
        .says("not a type");

    message("module a\n\nfn f(x: Int<String>) -> Int { 0 }\n")
        .under(codes::NOT_GENERIC)
        .says("`Int` does not take type arguments")
        .says("unexpected type arguments")
        .says("an alias and an effect may not");

    message("module a\n\nrecord R { n: Int }\n\nfn f(x: R<Int>) -> Int { 0 }\n")
        .under(codes::NOT_GENERIC)
        .says("`R` takes no type arguments, and 1 was given")
        .says("a type argument is written out rather than left to be worked out");

    message_with(
        "module a\n\nuse other.{thing}\n\nfn f(x: thing) -> Int { 0 }\n",
        &["module other\n\nfn thing() -> Int { 0 }\n"],
    )
    .under(codes::NOT_A_TYPE)
    .says("`thing` is a function, not a type")
    .says("declared in `other`");

    message("module a\n\ntype Loop = Loop\n\nfn f(x: Loop) -> Int { 0 }\n")
        .under(codes::TYPE_ALIAS_CYCLE)
        .says("the type alias `Loop` expands to itself")
        .says("cycle starts here");
}

#[test]
fn calls_refinements_and_ordering_are_read() {
    message("module a\n\nfn take(n: Int) -> Int { n }\n\nfn f() -> Int { take(true) }\n")
        .under(codes::TYPE_MISMATCH)
        .says("expected `Int`, found `Bool`")
        .says("the parameter it is passed to");

    message(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn take(n: Positive) -> Int { 0 }\n\n\
         fn f() -> Int { take(0) }\n",
    )
    .under(codes::VIOLATED_REFINEMENT)
    .says("this value does not satisfy `Positive`")
    .says("violates the refinement")
    .says("the predicate it has to satisfy");

    message(
        "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn take(n: Positive) -> Int { 0 }\n\n\
         fn f(n: Int) -> Int { take(n) }\n",
    )
    .under(codes::UNPROVEN_REFINEMENT)
    .says("cannot prove this satisfies `Positive`, so it becomes a runtime check")
    .says("checked at runtime")
    .says("this one is Guarded");

    message(
        "module a\n\n\
         fn halve(n: Int) -> Int\n\
         \x20 where\n\
         \x20   n >= 0,\n\
         {\n\
         \x20 n\n\
         }\n\n\
         fn caller() -> Int { halve(0 - 5) }\n",
    )
    .under(codes::BROKEN_PRECONDITION)
    .says("this call does not satisfy what `halve` requires")
    .says("the precondition does not hold here")
    .says("the clause it has to satisfy")
    .says("a precondition failure is a mistake in the caller");

    message(&format!(
        "{RESULT_PRELUDE}\n\
         fn outer() -> Int {{\n\
         \x20 small(1)?\n\
         }}\n"
    ))
    .under(codes::TRY_NEEDS_RESULT_RETURN)
    .says("`?` can only be used in a function returning a `Result`, and this one returns `Int`")
    .says("nowhere to propagate the error to")
    .says("the declared return type");

    message("module a\n\nfn f() -> Result<Int, Int> {\n  let n = 1?\n  ok(n)\n}\n")
        .under(codes::NOT_A_RESULT)
        .says("`?` needs a `Result`, and this is `Int`")
        .says("not a Result")
        .says("unwraps the success case");

    message("module a\n\nrecord Point { x: Int }\n\nfn f(a: Point, b: Point) -> Bool { a < b }\n")
        .under(codes::NOT_ORDERED)
        .says("`<` needs an order, and there is none on `Point`")
        .says("cannot be compared with `<`")
        .says("`Int` and `String` are ordered");
}

#[test]
fn handlers_and_assignment_are_read() {
    message(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: Int\n\n\
         \x20 fn set(to) -> () {{\n    count = to\n  }}\n\n\
         \x20 fn value() -> Int {{\n    count\n  }}\n\n\
         \x20 fn nonsense() -> Int {{\n    1\n  }}\n\
         }}\n"
    ))
    .under(codes::OPERATION_MISMATCH)
    .says("`Counter` does not declare an operation called `nonsense`")
    .says("not part of the effect")
    .says("the effect this handler implements");

    message(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: Int\n\n\
         \x20 fn set(to, extra) -> () {{\n    count = 1\n  }}\n\n\
         \x20 fn value() -> Int {{\n    count\n  }}\n\
         }}\n"
    ))
    .under(codes::OPERATION_MISMATCH)
    .says("`Counter.set` takes 1 argument, and this takes 2")
    .says("does not match the effect")
    .says("shape has to line up");

    message(&format!(
        "{COUNTER}\
         handler InMemory implements Counter {{\n\
         \x20 state count: Int\n\n\
         \x20 fn set(to) -> () {{\n    count = to\n  }}\n\
         }}\n"
    ))
    .under(codes::HANDLER_MISSING_OPERATION)
    .says("`InMemory` does not implement `value`")
    .says("one operation still to write")
    .says("`Counter` declares them")
    .says("a `with` block discharges the effect");

    message("module a\n\nfn f(n: Int) -> Int {\n  n = 1\n  n\n}\n")
        .under(codes::NOT_ASSIGNABLE)
        .says("`n` is a parameter, not handler state")
        .says("cannot be assigned to")
        .says("declared here")
        .says("handler state is the only mutable thing in Deed");

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
         \x20 state n: Int\n\n\
         \x20 fn use_it(f) -> Int { f() }\n\
         }\n",
    )
    .under(codes::CLOSURE_OVER_STATE)
    .says("`n` is handler state, and this closure can outlive the handler")
    .says("read inside a closure")
    .says("the handler state it names")
    .says("read the state into a local and let the closure carry that number")
    .says("a handler lives as long as the `with` block that installed it");

    message(
        "module a\n\n\
         effect Give {\n\
         \x20 fn bump() -> Fn() -> ()\n\
         }\n\n\
         handler A implements Give {\n\
         \x20 state n: Int\n\n\
         \x20 fn bump() -> Fn() -> () {\n\
         \x20   || { n = 1 }\n\
         \x20 }\n\
         }\n",
    )
    .under(codes::CLOSURE_OVER_STATE)
    .says("`n` is handler state, and this closure can outlive the handler")
    .says("assigned to inside a closure");
}

#[test]
fn values_fields_and_literals_are_read() {
    message(&format!(
        "{FIRST}\
         fn apply(f: Fn(List<Int>) -> Result<Int, String>) -> Int {{ 0 }}\n\n\
         fn f() -> Int {{ apply(first) }}\n"
    ))
    .under(codes::GENERIC_AS_VALUE)
    .says("`first` is generic, so naming it does not say what it is")
    .says("nothing here says what `T` is")
    .says("call it, or write a closure that calls it at the type you want");

    message_with(
        "module a\n\n\
         use lib.{first}\n\n\
         fn apply(f: Fn(List<Int>) -> Result<Int, String>) -> Int { 0 }\n\n\
         fn f() -> Int { apply(first) }\n",
        &["module lib\n\nfn first<T>(items: List<T>) -> Result<T, String> { at(items, 0) }\n"],
    )
    .under(codes::GENERIC_AS_VALUE)
    .says("`first` is generic, so naming it does not say what it is")
    .says("nothing here says what `T` is");

    message("module a\n\nfn f() -> Int { Int }\n")
        .under(codes::NOT_A_VALUE)
        .says("`Int` is a type, not a value")
        .says("`Int` cannot be used here");

    message("module a\n\nfn f() -> Int { System }\n")
        .under(codes::NOT_A_VALUE)
        .says("`System` is a type, not a value")
        .says("a `System` cannot be constructed, only received");

    message("module a\n\nfn f(sys: System) -> Int { sys.power }\n")
        .under(codes::NO_SUCH_FIELD)
        .says("`System` carries no `power`")
        .says("no such capability")
        .says("it carries `console`, `clock` and `files`");

    message("module a\n\nrecord R { alpha: Int }\n\nfn f(r: R) -> Int { r.beta }\n")
        .under(codes::NO_SUCH_FIELD)
        .says("`R` has no field `beta`")
        .says("no such field")
        .says("it has `alpha`");

    message("module a\n\nfn f(xs: List<Int>) -> Int { xs.length() }\n")
        .under(codes::NO_SUCH_FIELD)
        .says("`List<Int>` has no field `length`")
        .says("there are no methods")
        .says("`length(x)` rather than `x.length()`");

    message(
        "module a\n\nfn g() -> Int { 0 }\n\nfn f() -> Int {\n    let a = g { x: 1 }\n    0\n}\n",
    )
    .under(codes::NOT_A_CONSTRUCTOR)
    .says("`Fn() -> Int` is not a record or a variant")
    .says("cannot be built with a literal");

    message("module a\n\nrecord R { a: Int }\n\nfn f() -> R { R { a: 1, z: 2 } }\n")
        .under(codes::UNKNOWN_FIELD)
        .says("`R` has no field `z`")
        .says("no such field")
        .says("it has `a`");

    message("module a\n\nrecord R { a: Int, b: Int, c: Int }\n\nfn f() -> R { R { a: 1 } }\n")
        .under(codes::MISSING_FIELDS)
        .says("`R` is missing `b` and `c`")
        .says("incomplete literal")
        .says("every field has to be given");
}

#[test]
fn lists_discards_and_calls_are_read() {
    message(
        "module a\n\nfn f(text: String) -> Int {\n  for c in text with n = 0 {\n    n + 1\n  }\n}\n",
    )
    .under(codes::NOT_A_LIST)
    .says("`for` walks a list, and this is `String`")
    .says("not a list")
    .says("there is one thing to walk in this language");

    message("module a\n\nfn f(items: List<Int>) -> () {\n  for item in items while true {\n    ()\n  }\n}\n")
        .under(codes::WHILE_WITHOUT_ACCUMULATOR)
        .says("a `while` on a `for` needs a `with`")
        .says("nothing here changes between turns")
        .says("this walk has no accumulator")
        .says("what the walk has worked out so far");

    message("module a\n\nfn f(n: Int) -> Int { length(n) }\n")
        .under(codes::NOT_A_LIST)
        .says("`length` needs something with a length, and this is `Int`")
        .says("nothing to measure")
        .says("`length` measures a `String` or a `List`");

    message("module a\n\nfn f() -> List<Int> { push(1, 2) }\n")
        .under(codes::NOT_A_LIST)
        .says("`push` needs a list, and this is `Int`")
        .says("not a list");

    message("module a\n\nfn f() -> Int { length() }\n")
        .under(codes::WRONG_ARITY)
        .says("`length` takes 1 argument, but 0 were given")
        .says("wrong number of arguments");

    message("module a\n\nfn f() -> Result<Int, Int> { ok(1, 2) }\n")
        .under(codes::WRONG_ARITY)
        .says("`ok` takes one argument, but 2 were given")
        .says("wrong number of arguments");

    message_with(
        "module a\n\n\
         use other.{Sink}\n\n\
         fn f() -> Int\n\
         \x20 uses\n\
         \x20   Sink.count,\n\
         {\n\
         \x20 Sink.count(1)\n\
         }\n",
        &["module other\n\neffect Sink {\n    fn count() -> Int\n}\n"],
    )
    .under(codes::WRONG_ARITY)
    .says("`count` takes 0 arguments, but 1 was given")
    .says("wrong number of arguments");

    message("module a\n\nfn f() -> Int {\n  let add = |a: Int, b: Int| { a + b }\n  add(1)\n}\n")
        .under(codes::WRONG_ARITY)
        .says("this takes 2 arguments, but 1 were given")
        .says("wrong number of arguments");

    message("module a\n\nfn take(n: Int, m: Int) -> Int { n }\n\nfn f() -> Int { take(1) }\n")
        .under(codes::WRONG_ARITY)
        .says("`take` takes 2 arguments, but 1 was given")
        .says("declared here");

    message("module a\n\nfn f(n: Int) -> Int { n(1) }\n")
        .under(codes::NOT_CALLABLE)
        .says("`Int` is not a function")
        .says("not callable");

    message(
        "module a\n\n\
         fn twice(n: Int) -> Int { n + n }\n\n\
         fn f(n: Int) -> Int {\n\
         \x20 twice(n)\n\
         \x20 n\n\
         }\n",
    )
    .under(codes::DISCARDED_VALUE)
    .says("this produces `Int` and nothing reads it")
    .says("the value goes nowhere")
    .says("write `let _ = ...`");

    message(
        "module a\n\n\
         fn f(n: Int) -> Int {\n\
         \x20 let a = 1\n\
         \x20 - 2\n\
         \x20 a + n\n\
         }\n",
    )
    .under(codes::DISCARDED_VALUE)
    .says("this produces `Int` and nothing reads it")
    .says("write `let _ = ...`");

    message(
        "module a\n\n\
         fn boom(n: Int) -> Result<Int, String> { ok(n) }\n\n\
         fn f(n: Int) -> Int {\n\
         \x20 boom(n)\n\
         \x20 n\n\
         }\n",
    )
    .under(codes::DISCARDED_VALUE)
    .says("this produces `Result<Int, String>` and nothing reads it")
    .says("the failure case goes with it");
}

#[test]
fn matches_and_patterns_are_read() {
    message(
        "module a\n\nchoice E { A, B, C }\n\nfn f(e: E) -> Int {\n  match e {\n    A => 1,\n    _ => 2,\n  }\n}\n",
    )
    .under(codes::CATCH_ALL_ON_CHOICE)
    .says("this arm matches every variant of `E`")
    .says("catches everything")
    .says("in this match")
    .says("adding one to `E` should break every match that has to care");

    message_with(
        "module a\n\n\
         use other.{E, A}\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   A => 1,\n\
         \x20   _ => 2,\n\
         \x20 }\n\
         }\n",
        &["module other\n\nchoice E { A, B }\n"],
    )
    .under(codes::CATCH_ALL_ON_CHOICE)
    .says("this arm matches every variant of `E` from `other`")
    .says("as true across a module boundary as inside one");

    message(&format!(
        "{RESULT_PRELUDE}\n\
         fn handled(n: Int) -> Int {{\n\
         \x20 match small(n) {{\n\
         \x20   ok(value) => value,\n\
         \x20   _ => 0,\n\
         \x20 }}\n\
         }}\n"
    ))
    .under(codes::CATCH_ALL_ON_CHOICE)
    .says("this arm matches both cases of the `Result`")
    .says("cannot be handled by accident");

    message(
        "module a\n\nchoice E { A, B, C }\n\nfn f(e: E) -> Int {\n  match e {\n    A => 1,\n  }\n}\n",
    )
    .under(codes::NON_EXHAUSTIVE_MATCH)
    .says("this match does not cover `B` and `C`")
    .says("not exhaustive")
    .says("every variant of `E` needs an arm");

    message_with(
        "module a\n\n\
         use other.{E, A}\n\n\
         fn f(e: E) -> Int {\n\
         \x20 match e {\n\
         \x20   A => 1,\n\
         \x20 }\n\
         }\n",
        &["module other\n\nchoice E { A, B }\n"],
    )
    .under(codes::NON_EXHAUSTIVE_MATCH)
    .says("this match does not cover `B`")
    .says("every variant of `E` from `other` needs an arm");

    message(&format!(
        "{RESULT_PRELUDE}\n\
         fn handled(n: Int) -> Int {{\n\
         \x20 match small(n) {{\n\
         \x20   ok(value) => value,\n\
         \x20 }}\n\
         }}\n"
    ))
    .under(codes::NON_EXHAUSTIVE_MATCH)
    .says("this match does not cover `err`")
    .says("a `Result` has two cases and both need an arm");

    message(
        "module a\n\nfn f(n: Int) -> Int {\n  match n {\n    ok(x) => x,\n    _ => 0,\n  }\n}\n",
    )
    .under(codes::PATTERN_MISMATCH)
    .says("`ok(...)` matches a `Result`, and this is `Int`")
    .says("cannot match");

    message(&format!(
        "{RESULT_PRELUDE}\n\
         fn handled(f: Failure) -> Int {{\n\
         \x20 match f {{\n\
         \x20   TooBig(limit) => limit,\n\
         \x20   Empty => 0,\n\
         \x20 }}\n\
         }}\n"
    ))
    .under(codes::PATTERN_MISMATCH)
    .says("only `ok` and `err` carry a value in a pattern")
    .says("not a pattern that can match")
    .says("`Variant { field }`");

    message(&format!(
        "{RESULT_PRELUDE}\n\
         fn handled(n: Int) -> Int {{\n\
         \x20 match small(n) {{\n\
         \x20   ok(a, b) => a,\n\
         \x20   err(e) => 0,\n\
         \x20 }}\n\
         }}\n"
    ))
    .under(codes::PATTERN_MISMATCH)
    .says("this pattern binds 2 values, and it should bind one")
    .says("wrong number of bindings");
}
