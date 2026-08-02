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

use deed_codegen::{Trap, Value, call, compile};
use deed_diagnostics::SourceMap;
use deed_driver::check_all;
use deed_interp::codes;
use deed_interp::{
    Program as Interpreted, PropertyConfig, Value as InterpretedValue, generate_inputs, run_main,
    run_tests, shrink_inputs,
};

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
            name: "a plain return",
            source: "module a\n\nfn answer() -> Int {\n    return 12\n}\n\ntest \"return ends the function\" {\n    assert answer() == 12\n}\n",
            call: "answer",
            expect: 12,
        },
        Agreed {
            name: "a return inside an if",
            source: "module a\n\nfn absolute(n: Int) -> Int {\n    if n >= 0 {\n        return n\n    }\n    0 - n\n}\n\nfn answer() -> Int { absolute(5) + absolute(2 - 7) }\n\ntest \"one branch can return early\" {\n    assert answer() == 10\n}\n",
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
        Agreed {
            name: "a record",
            source: "module a\n\nrecord Pair {\n    left: Int,\n    right: Int,\n}\n\nfn answer() -> Int {\n    let p = Pair { left: 4, right: 2 }\n    p.left * 10 + p.right\n}\n\ntest \"a record holds both fields\" {\n    assert answer() == 42\n}\n",
            call: "answer",
            expect: 42,
        },
        // A field's place is a property of the type, not of the literal, so
        // writing them out of order still reads back correctly.
        Agreed {
            name: "a record written out of order",
            source: "module a\n\nrecord Pair {\n    left: Int,\n    right: Int,\n}\n\nfn answer() -> Int {\n    let p = Pair { right: 2, left: 4 }\n    p.left * 10 + p.right\n}\n\ntest \"order in the literal does not matter\" {\n    assert answer() == 42\n}\n",
            call: "answer",
            expect: 42,
        },
        // Two addresses live at once, which a shared scratch slot would get
        // wrong by having the inner build overwrite the outer one.
        Agreed {
            name: "a record inside a record",
            source: "module a\n\nrecord Inner {\n    n: Int,\n}\n\nrecord Outer {\n    held: Inner,\n    beside: Int,\n}\n\nfn answer() -> Int {\n    let o = Outer { held: Inner { n: 40 }, beside: 2 }\n    o.held.n + o.beside\n}\n\ntest \"nesting keeps both apart\" {\n    assert answer() == 42\n}\n",
            call: "answer",
            expect: 42,
        },
        Agreed {
            name: "a record passed to a function",
            source: "module a\n\nrecord Pair {\n    left: Int,\n    right: Int,\n}\n\nfn total(p: Pair) -> Int { p.left + p.right }\n\nfn answer() -> Int { total(Pair { left: 40, right: 2 }) }\n\ntest \"a record crosses a call\" {\n    assert answer() == 42\n}\n",
            call: "answer",
            expect: 42,
        },
        Agreed {
            name: "a list and its length",
            source: "module a\n\nfn answer() -> Int { length([1, 2, 3]) }\n\ntest \"a list knows how long it is\" {\n    assert answer() == 3\n}\n",
            call: "answer",
            expect: 3,
        },
        Agreed {
            name: "an empty list",
            source: "module a\n\nfn answer() -> Int { length([]) }\n\ntest \"an empty list has no elements\" {\n    assert answer() == 0\n}\n",
            call: "answer",
            expect: 0,
        },
        Agreed {
            name: "a string length counts unicode scalar values",
            source: "module a\n\nfn answer() -> Int { length(\"e\\u{301}\") }\n\ntest \"a decomposed character counts as two scalar values\" {\n    assert answer() == 2\n}\n",
            call: "answer",
            expect: 2,
        },
        // Strings are bytes in memory, so every one of these is a loop the
        // backend writes rather than an instruction. Both halves of each
        // answer are asked: whether two strings are the same, and what
        // joining them produces.
        Agreed {
            name: "joining two strings",
            source: "module a\n\nfn answer() -> Int {\n    if \"ab\" + \"cde\" == \"abcde\" {\n        1\n    } else {\n        0\n    }\n}\n\ntest \"one string after the other\" {\n    assert answer() == 1\n}\n",
            call: "answer",
            expect: 1,
        },
        Agreed {
            name: "joining an empty string changes nothing",
            source: "module a\n\nfn answer() -> Int { length(\"\" + \"abc\" + \"\") }\n\ntest \"nothing is still nothing\" {\n    assert answer() == 3\n}\n",
            call: "answer",
            expect: 3,
        },
        // The joined string carries a character count rather than a byte
        // count, and the two differ the moment anything is not ASCII.
        Agreed {
            name: "joining counts characters rather than bytes",
            source: "module a\n\nfn answer() -> Int { length(\"é\" + \"x\") }\n\ntest \"two characters, three bytes\" {\n    assert answer() == 2\n}\n",
            call: "answer",
            expect: 2,
        },
        Agreed {
            name: "two strings that differ in one place",
            source: "module a\n\nfn answer() -> Int {\n    if \"abc\" == \"abd\" {\n        1\n    } else {\n        0\n    }\n}\n\ntest \"one byte is enough to tell them apart\" {\n    assert answer() == 0\n}\n",
            call: "answer",
            expect: 0,
        },
        Agreed {
            name: "a string is not its prefix",
            source: "module a\n\nfn answer() -> Int {\n    if \"ab\" != \"abc\" {\n        1\n    } else {\n        0\n    }\n}\n\ntest \"length is asked first\" {\n    assert answer() == 1\n}\n",
            call: "answer",
            expect: 1,
        },
        // The order `design/02-syntax.md` names: by code point, so a number
        // written as text sorts as text.
        Agreed {
            name: "ordering two strings",
            source: "module a\n\nfn earlier(a: String, b: String) -> Bool { a < b }\n\nfn answer() -> Int {\n    if earlier(\"10\", \"9\") && !earlier(\"9\", \"10\") {\n        1\n    } else {\n        0\n    }\n}\n\ntest \"text order is not number order\" {\n    assert answer() == 1\n}\n",
            call: "answer",
            expect: 1,
        },
        Agreed {
            name: "a prefix comes first",
            source: "module a\n\nfn answer() -> Int {\n    if \"ab\" < \"abc\" && \"abc\" > \"ab\" && \"ab\" <= \"ab\" && \"ab\" >= \"ab\" {\n        1\n    } else {\n        0\n    }\n}\n\ntest \"the shorter one comes first when it matches\" {\n    assert answer() == 1\n}\n",
            call: "answer",
            expect: 1,
        },
        // All four in both directions, and on two strings that are the same.
        // Three of them agree wherever one string really does come before the
        // other, so what tells them apart is the pair that is equal and the
        // pair where the answer is no.
        Agreed {
            name: "each ordering operator answers for itself",
            source: "module a\n\nfn answer() -> Int {\n    if \"ab\" < \"abc\" && !(\"abc\" < \"ab\") && !(\"ab\" < \"ab\") &&\n        \"ab\" <= \"abc\" && !(\"abc\" <= \"ab\") && \"ab\" <= \"ab\" &&\n        \"abc\" > \"ab\" && !(\"ab\" > \"abc\") && !(\"ab\" > \"ab\") &&\n        \"abc\" >= \"ab\" && !(\"ab\" >= \"abc\") && \"ab\" >= \"ab\" {\n        1\n    } else {\n        0\n    }\n}\n\ntest \"twelve answers, one for each way round\" {\n    assert answer() == 1\n}\n",
            call: "answer",
            expect: 1,
        },
        // Equality is one operator over every type, so which instruction it
        // is depends on what was compared. A wrong width here answers the
        // wrong question quietly.
        Agreed {
            name: "equality on each width",
            source: "module a\n\nfn answer() -> Int {\n    if 1 == 1 && !(1 == 2) && 1 != 2 && !(1 != 1) &&\n        true == true && !(true == false) && true != false && !(true != true) {\n        1\n    } else {\n        0\n    }\n}\n\ntest \"a number and a boolean are not the same width\" {\n    assert answer() == 1\n}\n",
            call: "answer",
            expect: 1,
        },
        // Two values of a type with no representation. There is nothing on
        // the stack to compare, so the answer is written rather than computed,
        // and it still has to be the right one.
        Agreed {
            name: "equality on something with no representation",
            source: "module a\n\nfn nothing() -> () { () }\n\nfn answer() -> Int {\n    if nothing() == nothing() && !(nothing() != nothing()) {\n        1\n    } else {\n        0\n    }\n}\n\ntest \"two of nothing are the same nothing\" {\n    assert answer() == 1\n}\n",
            call: "answer",
            expect: 1,
        },
        // The prelude's own functions, each of which is a loop the backend
        // writes. What they answer is the interpreter's answer: both engines
        // run every one of these.
        Agreed {
            name: "a number written out",
            source: "module a\n\nfn answer() -> Int {\n    if to_string(0) == \"0\" && to_string(42) == \"42\" && to_string(0 - 7) == \"-7\" {\n        1\n    } else {\n        0\n    }\n}\n\ntest \"a sign and a zero are both digits somebody has to write\" {\n    assert answer() == 1\n}\n",
            call: "answer",
            expect: 1,
        },
        Agreed {
            name: "the smallest number written out",
            source: "module a\n\nfn answer() -> Int { length(to_string(0 - 9223372036854775807 - 1)) }\n\ntest \"the number with no positive counterpart\" {\n    assert to_string(0 - 9223372036854775807 - 1) == \"-9223372036854775808\"\n    assert answer() == 20\n}\n",
            call: "answer",
            expect: 20,
        },
        Agreed {
            name: "the same value a number of times",
            source: "module a\n\nfn answer() -> Int {\n    length(repeat(\"ab\", 3)) + length(repeat(\"ab\", 0)) + length(repeat(\"ab\", 0 - 4))\n}\n\ntest \"a count that went negative is no padding rather than a refusal\" {\n    assert answer() == 3\n}\n",
            call: "answer",
            expect: 3,
        },
        Agreed {
            name: "one more on the end",
            source: "module a\n\nfn answer() -> Int {\n    let grown = push(push([], 4), 5)\n    length(grown)\n}\n\ntest \"pushing onto nothing gives one\" {\n    assert answer() == 2\n}\n",
            call: "answer",
            expect: 2,
        },
        Agreed {
            name: "what push put there is what comes back",
            source: "module a\n\nfn answer() -> Int {\n    let grown = push(push([1], 2), 3)\n    match at(grown, 2) {\n        ok(n) => n,\n        err(why) => 0 - 1,\n    }\n}\n\ntest \"the list that went in is still in front\" {\n    assert answer() == 3\n}\n",
            call: "answer",
            expect: 3,
        },
        // An index nobody promised is there, and the sentence both engines
        // answer with. Two engines writing two different strings here is a
        // difference a program can read.
        Agreed {
            name: "an index that is not there",
            source: "module a\n\nfn answer() -> Int {\n    match at([1, 2], 5) {\n        ok(n) => 0,\n        err(why) => length(why),\n    }\n}\n\ntest \"the message names the index and the length\" {\n    let said = match at([1, 2], 5) {\n        ok(n) => \"\",\n        err(why) => why,\n    }\n    assert said == \"index 5 is outside a list of 2\"\n    assert answer() == 30\n}\n",
            call: "answer",
            expect: 30,
        },
        Agreed {
            name: "an index below the start",
            source: "module a\n\nfn answer() -> Int {\n    match at([1, 2], 0 - 1) {\n        ok(n) => 0,\n        err(why) => length(why),\n    }\n}\n\ntest \"a negative index is outside too\" {\n    let said = match at([1, 2], 0 - 1) {\n        ok(n) => \"\",\n        err(why) => why,\n    }\n    assert said == \"index -1 is outside a list of 2\"\n    assert answer() == 31\n}\n",
            call: "answer",
            expect: 31,
        },
        // A list of booleans is narrower on the stack than in memory, so
        // something has to widen it going in and narrow it coming out.
        Agreed {
            name: "a list of booleans keeps its width",
            source: "module a\n\nfn count(flag: Bool) -> Int {\n    if flag {\n        1\n    } else {\n        0\n    }\n}\n\nfn answer() -> Int {\n    let flags = push([true, false], true)\n    match at(flags, 2) {\n        ok(flag) => count(flag),\n        err(why) => 0 - 1,\n    }\n}\n\ntest \"a boolean survives a list\" {\n    assert answer() == 1\n}\n",
            call: "answer",
            expect: 1,
        },
        // Text taken apart and put back together, which is most of what a
        // program that reads anything does.
        //
        // What `answer` returns is the only thing the compiled side is asked
        // for, so an assertion that sits in the `test` block alone is checked
        // against the interpreter and nothing else. Each of these folds the
        // comparison into the answer instead, and counts how many held.
        Agreed {
            name: "text taken apart and put back together",
            source: "module a\n\nfn same(got: String, want: String) -> Int {\n    if got == want {\n        1\n    } else {\n        0\n    }\n}\n\nfn answer() -> Int {\n    same(join(split(\"a,b,c\", \",\"), \"-\"), \"a-b-c\") +\n        same(join(split(\",a,\", \",\"), \"-\"), \"-a-\") +\n        same(join(split(\"añb\", \"\"), \"-\"), \"a-ñ-b\") +\n        same(join(split(\"a--b--c\", \"--\"), \"|\"), \"a|b|c\") +\n        same(join(split(\"abc\", \",\"), \",\"), \"abc\") +\n        same(join(split(\"\", \",\"), \",\"), \"\")\n}\n\ntest \"a separator at an edge leaves an empty piece\" {\n    assert answer() == 6\n}\n",
            call: "answer",
            expect: 6,
        },
        // How many pieces each of those came apart into, which `join` puts
        // back together whether it is right or not.
        Agreed {
            name: "how many pieces text comes apart into",
            source: "module a\n\nfn answer() -> Int {\n    length(split(\"a,b,c\", \",\")) +\n        length(split(\",a,\", \",\")) +\n        length(split(\"añb\", \"\")) +\n        length(split(\"a--b--c\", \"--\")) +\n        length(split(\"abc\", \",\")) +\n        length(split(\"\", \",\"))\n}\n\ntest \"an empty separator gives characters, not bytes\" {\n    assert length(split(\"añb\", \"\")) == 3\n    assert answer() == 14\n}\n",
            call: "answer",
            expect: 14,
        },
        Agreed {
            name: "the ends taken off",
            source: "module a\n\nfn same(got: String, want: String) -> Int {\n    if got == want {\n        1\n    } else {\n        0\n    }\n}\n\nfn answer() -> Int {\n    same(trim(\"  hi \\t\\n\"), \"hi\") +\n        same(trim(\"   \"), \"\") +\n        same(trim(\"hi\"), \"hi\") +\n        same(trim(\"\"), \"\") +\n        same(trim(\" a b \"), \"a b\") +\n        same(trim(\"\\r\\nx\\r\\n\"), \"x\")\n}\n\ntest \"four characters and not the Unicode table\" {\n    assert answer() == 6\n}\n",
            call: "answer",
            expect: 6,
        },
        // The bytes either side of the two ranges, because a shift applied
        // one place too far is the way this gets written wrong.
        Agreed {
            name: "the twenty-six letters and nothing else",
            source: "module a\n\nfn same(got: String, want: String) -> Int {\n    if got == want {\n        1\n    } else {\n        0\n    }\n}\n\nfn answer() -> Int {\n    same(upper(\"añb1\"), \"AñB1\") +\n        same(lower(\"AñB1\"), \"añb1\") +\n        same(upper(\"az\"), \"AZ\") +\n        same(lower(\"AZ\"), \"az\") +\n        same(upper(\"`{\"), \"`{\") +\n        same(lower(\"@[\"), \"@[\")\n}\n\ntest \"text in a script with no case survives\" {\n    assert answer() == 6\n}\n",
            call: "answer",
            expect: 6,
        },
        Agreed {
            name: "a number written out",
            source: "module a\n\nfn same(got: String, want: String) -> Int {\n    if got == want {\n        1\n    } else {\n        0\n    }\n}\n\nfn answer() -> Int {\n    same(to_string(0), \"0\") +\n        same(to_string(7), \"7\") +\n        same(to_string(0 - 7), \"-7\") +\n        same(to_string(9223372036854775807), \"9223372036854775807\") +\n        same(to_string(0 - 9223372036854775807 - 1), \"-9223372036854775808\")\n}\n\ntest \"the smallest number has no positive to negate\" {\n    assert answer() == 5\n}\n",
            call: "answer",
            expect: 5,
        },
        Agreed {
            name: "text that is not a number",
            source: "module a\n\nfn said(text: String) -> String {\n    match to_int(text) {\n        ok(n) => \"\",\n        err(why) => why,\n    }\n}\n\nfn same(got: String, want: String) -> Int {\n    if got == want {\n        1\n    } else {\n        0\n    }\n}\n\nfn answer() -> Int {\n    same(said(\"4x\"), \"`4x` is not a number\") +\n        same(said(\"\"), \"`` is not a number\") +\n        same(said(\" 1\"), \"` 1` is not a number\") +\n        same(said(\"42\"), \"\")\n}\n\ntest \"the message quotes what it was given\" {\n    assert answer() == 4\n}\n",
            call: "answer",
            expect: 4,
        },
        Agreed {
            name: "the boundaries a number can be read at",
            source: "module a\n\nfn read(text: String) -> Int {\n    match to_int(text) {\n        ok(n) => n,\n        err(why) => 0,\n    }\n}\n\nfn failed(text: String) -> Int {\n    match to_int(text) {\n        ok(n) => 0,\n        err(why) => 1,\n    }\n}\n\nfn is(got: Int, want: Int) -> Int {\n    if got == want {\n        1\n    } else {\n        0\n    }\n}\n\nfn answer() -> Int {\n    failed(\"\") + failed(\"-\") + failed(\" 1\") + failed(\"9223372036854775808\") +\n        is(read(\"-9223372036854775808\"), 0 - 9223372036854775807 - 1) +\n        is(read(\"9223372036854775807\"), 9223372036854775807) +\n        is(read(\"+7\"), 7) +\n        is(read(\"-0\"), 0)\n}\n\ntest \"the edges, from both sides\" {\n    assert answer() == 8\n}\n",
            call: "answer",
            expect: 8,
        },
        // `?`, which is a `match` on a `Result` that nobody wrote. The
        // failure case ends the function, so what these count is how far the
        // body got.
        Agreed {
            name: "an error that ends the function it was met in",
            source: "module a\n\nfn half(n: Int) -> Result<Int, String> {\n    if n == 0 {\n        err(\"zero\")\n    } else {\n        ok(n)\n    }\n}\n\nfn twice(n: Int) -> Result<Int, String> {\n    let one = half(n)?\n    let two = half(one)?\n    ok(one + two)\n}\n\nfn outcome(n: Int) -> Int {\n    match twice(n) {\n        ok(m) => m,\n        err(why) => 0 - length(why),\n    }\n}\n\nfn answer() -> Int { outcome(3) + outcome(0) + outcome(5) }\n\ntest \"the second half never runs when the first failed\" {\n    assert outcome(3) == 6\n    assert outcome(0) == 0 - 4\n    assert answer() == 12\n}\n",
            call: "answer",
            expect: 12,
        },
        // In the middle of an expression rather than at the head of a `let`,
        // which is where it is easiest to lower the wrong part.
        Agreed {
            name: "an error met inside an argument",
            source: "module a\n\nfn word(n: Int) -> Result<String, String> {\n    if n == 1 {\n        ok(\"one\")\n    } else {\n        err(\"not one\")\n    }\n}\n\nfn shout(n: Int) -> Result<String, String> {\n    ok(upper(word(n)?))\n}\n\nfn size(n: Int) -> Int {\n    match shout(n) {\n        ok(text) => length(text),\n        err(why) => 0 - length(why),\n    }\n}\n\nfn answer() -> Int { size(1) + size(2) }\n\ntest \"the message comes out as it went in\" {\n    assert size(1) == 3\n    assert size(2) == 0 - 7\n    assert answer() == 0 - 4\n}\n",
            call: "answer",
            expect: -4,
        },
        // The failure the prelude builds, carried out of a function that only
        // passes it along.
        Agreed {
            name: "an error the prelude wrote",
            source: "module a\n\nfn pick(xs: List<Int>, i: Int) -> Result<Int, String> {\n    ok(at(xs, i)?)\n}\n\nfn got(i: Int) -> Int {\n    match pick([4, 5, 6], i) {\n        ok(n) => n,\n        err(why) => 0 - length(why),\n    }\n}\n\nfn answer() -> Int { got(0) + got(2) + got(9) }\n\ntest \"an index nobody has says which one and how many there were\" {\n    assert got(0) == 4\n    assert got(9) == 0 - 30\n    assert answer() == 0 - 20\n}\n",
            call: "answer",
            expect: -20,
        },
        // A declared function written where a value belongs. A call through
        // a value passes an environment and a call by name does not, so the
        // two cannot be the same function however empty the environment is.
        Agreed {
            name: "a function passed by name",
            source: "module a\n\nfn twice(n: Int) -> Int { n + n }\n\nfn less(n: Int) -> Int { n - 1 }\n\nfn apply(step: Fn(Int) -> Int, n: Int) -> Int { step(n) }\n\nfn answer() -> Int {\n    apply(twice, 5) + apply(less, 5) + apply(twice, apply(less, 3))\n}\n\ntest \"the same name twice costs one wrapper\" {\n    assert apply(twice, 5) == 10\n    assert apply(less, 5) == 4\n    assert answer() == 18\n}\n",
            call: "answer",
            expect: 18,
        },
        // A contract belongs to the function, not to the way it was called,
        // so a name handed over as a value keeps the clause it was declared
        // with.
        Agreed {
            name: "a function passed by name keeps its contract",
            source: "module a\n\nfn halve(n: Int) -> Int\n  where\n    n >= 0,\n{\n    n / 2\n}\n\nfn apply(step: Fn(Int) -> Int, n: Int) -> Int { step(n) }\n\nfn answer() -> Int { apply(halve, 8) }\n\ntest \"the clause travels with the name\" {\n    assert refuses apply(halve, 0 - 2)\n    assert answer() == 4\n}\n",
            call: "answer",
            expect: 4,
        },
        // A closure written where the value belongs, alongside a name, since
        // the two have to be the same kind of thing to a caller.
        Agreed {
            name: "a closure and a name in the same place",
            source: "module a\n\nfn twice(n: Int) -> Int { n + n }\n\nfn apply(step: Fn(Int) -> Int, n: Int) -> Int { step(n) }\n\nfn adder(by: Int) -> Fn(Int) -> Int {\n    |x: Int| x + by\n}\n\nfn answer() -> Int {\n    apply(twice, 3) + apply(adder(10), 3) + apply(|x: Int| x * 3, 3)\n}\n\ntest \"a closure that left the function that wrote it\" {\n    assert apply(adder(10), 3) == 13\n    assert answer() == 28\n}\n",
            call: "answer",
            expect: 28,
        },
        Agreed {
            name: "a match on a choice",
            source: "module a\n\nchoice Tone {\n    Plain,\n    Loud,\n}\n\nfn weight(tone: Tone) -> Int {\n    match tone {\n        Plain => 1,\n        Loud => 10,\n    }\n}\n\nfn answer() -> Int { weight(Loud) }\n\ntest \"each variant answers for itself\" {\n    assert weight(Plain) == 1\n    assert answer() == 10\n}\n",
            call: "answer",
            expect: 10,
        },
        // A variant with fields, bound by the arm that names it.
        Agreed {
            name: "a match that binds a field",
            source: "module a\n\nchoice Shape {\n    Dot,\n    Box { side: Int },\n}\n\nfn area(shape: Shape) -> Int {\n    match shape {\n        Dot => 0,\n        Box { side } => side * side,\n    }\n}\n\nfn answer() -> Int { area(Box { side: 7 }) }\n\ntest \"an arm reads the field it binds\" {\n    assert area(Dot) == 0\n    assert answer() == 49\n}\n",
            call: "answer",
            expect: 49,
        },
        // Alternatives bind nothing, which is what makes them cheap.
        Agreed {
            name: "a match arm naming several variants",
            source: "module a\n\nchoice Step {\n    Up,\n    Down,\n    Stay,\n}\n\nfn moves(step: Step) -> Int {\n    match step {\n        Up | Down => 1,\n        Stay => 0,\n    }\n}\n\nfn answer() -> Int { moves(Up) + moves(Down) + moves(Stay) }\n\ntest \"two variants share one arm\" {\n    assert answer() == 2\n}\n",
            call: "answer",
            expect: 2,
        },
        Agreed {
            name: "a return inside a match arm",
            source: "module a\n\nchoice Step {\n    Keep,\n    Stop,\n}\n\nfn score(step: Step) -> Int {\n    match step {\n        Stop => {\n            return 9\n            0\n        },\n        Keep => 1,\n    }\n}\n\nfn answer() -> Int { score(Stop) + score(Keep) }\n\ntest \"a match arm can return from the function\" {\n    assert answer() == 10\n}\n",
            call: "answer",
            expect: 10,
        },
        Agreed {
            name: "a walk that adds up",
            source: "module a\n\nfn answer() -> Int {\n    for n in [1, 2, 3, 4] with sum = 0 {\n        sum + n\n    }\n}\n\ntest \"a fold reaches the end\" {\n    assert answer() == 10\n}\n",
            call: "answer",
            expect: 10,
        },
        Agreed {
            name: "a return inside a walk",
            source: "module a\n\nfn answer() -> Int {\n    for n in [1, 2, 3, 4] with sum = 0 {\n        if n == 3 {\n            return sum\n        }\n        sum + n\n    }\n}\n\ntest \"a walk can return from its enclosing function\" {\n    assert answer() == 3\n}\n",
            call: "answer",
            expect: 3,
        },
        Agreed {
            name: "a walk over nothing",
            source: "module a\n\nfn answer() -> Int {\n    for n in [] with sum = 7 {\n        sum + n\n    }\n}\n\ntest \"an empty walk answers with what it started from\" {\n    assert answer() == 7\n}\n",
            call: "answer",
            expect: 7,
        },
        Agreed {
            name: "a walk that says where it is",
            source: "module a\n\nfn answer() -> Int {\n    for n at i in [10, 20, 30] with sum = 0 {\n        sum + i\n    }\n}\n\ntest \"the index counts from zero\" {\n    assert answer() == 3\n}\n",
            call: "answer",
            expect: 3,
        },
        // `while` is read before each turn with the accumulator in scope, so
        // this stops after the third element rather than walking all six.
        Agreed {
            name: "a walk that stops early",
            source: "module a\n\nfn answer() -> Int {\n    for n in [1, 2, 3, 4, 5, 6] with sum = 0 while sum < 6 {\n        sum + n\n    }\n}\n\ntest \"a walk can stop before the end\" {\n    assert answer() == 6\n}\n",
            call: "answer",
            expect: 6,
        },
        Agreed {
            name: "a walk over a list of records",
            source: "module a\n\nrecord Item {\n    weight: Int,\n}\n\nfn answer() -> Int {\n    for item in [Item { weight: 3 }, Item { weight: 4 }] with sum = 0 {\n        sum + item.weight\n    }\n}\n\ntest \"a walk reads a field of each element\" {\n    assert answer() == 7\n}\n",
            call: "answer",
            expect: 7,
        },
        // A `where` the caller satisfies. The checker proves it where the
        // call is written, so the compiled form has nothing left to check
        // and still answers the same.
        Agreed {
            name: "a precondition the caller satisfies",
            source: "module a\n\nfn halve(n: Int) -> Int\n  where\n    n >= 0,\n{\n    n / 2\n}\n\nfn answer() -> Int { halve(10) }\n\ntest \"a proven precondition does not get in the way\" {\n    assert answer() == 5\n}\n",
            call: "answer",
            expect: 5,
        },
        Agreed {
            name: "a refinement the checker proves",
            source: "module a\n\ntype Positive = Int where value > 0\n\nfn one() -> Positive { 1 }\n\nfn answer() -> Int { one() + 41 }\n\ntest \"a proven refinement answers as itself\" {\n    assert answer() == 42\n}\n",
            call: "answer",
            expect: 42,
        },
        Agreed {
            name: "a closure called through a name",
            source: "module a\n\nfn apply(f: Fn(Int) -> Int, n: Int) -> Int { f(n) }\n\nfn answer() -> Int { apply(|n: Int| n + n, 21) }\n\ntest \"a closure is a value that can be called\" {\n    assert answer() == 42\n}\n",
            call: "answer",
            expect: 42,
        },
        Agreed {
            name: "a closure that captures",
            source: "module a\n\nfn apply(f: Fn(Int) -> Int, n: Int) -> Int { f(n) }\n\nfn answer() -> Int {\n    let by = 10\n    apply(|n: Int| n * by, 4)\n}\n\ntest \"a closure reads what it captured\" {\n    assert answer() == 40\n}\n",
            call: "answer",
            expect: 40,
        },
        Agreed {
            name: "a generic function at one type",
            source: "module a\n\nfn firstly<T>(items: List<T>, fallback: T) -> T {\n    for item at i in items with found = fallback {\n        if i == 0 {\n            item\n        } else {\n            found\n        }\n    }\n}\n\nfn answer() -> Int { firstly([7, 8, 9], 0) }\n\ntest \"a generic function answers at the type it was called with\" {\n    assert answer() == 7\n}\n",
            call: "answer",
            expect: 7,
        },
        // Two copies of one declaration, which is what monomorphization is
        // for and what a single copy would get wrong.
        Agreed {
            name: "a generic function at two types",
            source: "module a\n\nfn count_of<T>(items: List<T>) -> Int { length(items) }\n\nfn answer() -> Int { count_of([1, 2]) + count_of([true]) }\n\ntest \"one declaration, two element types\" {\n    assert answer() == 3\n}\n",
            call: "answer",
            expect: 3,
        },
        // Handlers. The compiled form is a frame linked into a stack and a
        // search down it, so what these check is that the search finds what
        // the interpreter's scoping says it should.
        Agreed {
            name: "a handler answering an operation",
            source: "module a\n\neffect Counter {\n    fn value() -> Int\n}\n\nhandler Fixed implements Counter {\n    state count: Int\n\n    fn value() -> Int { count }\n}\n\nfn answer() -> Int {\n    with Fixed { count: 7 } {\n        Counter.value()\n    }\n}\n\ntest \"a handler answers what it holds\" {\n    assert answer() == 7\n}\n",
            call: "answer",
            expect: 7,
        },
        Agreed {
            name: "handler state that changes",
            source: "module a\n\neffect Counter {\n    fn value() -> Int\n    fn bump(by: Int) -> ()\n}\n\nhandler InMemory implements Counter {\n    state count: Int\n\n    fn value() -> Int { count }\n\n    fn bump(by) -> () {\n        count = count + by\n    }\n}\n\nfn answer() -> Int {\n    with InMemory { count: 0 } {\n        Counter.bump(2)\n        Counter.bump(3)\n        Counter.value()\n    }\n}\n\ntest \"state carries between operations\" {\n    assert answer() == 5\n}\n",
            call: "answer",
            expect: 5,
        },
        Agreed {
            name: "a return inside a handler operation",
            source: "module a\n\neffect Counter {\n    fn value() -> Int\n}\n\nhandler Fixed implements Counter {\n    state count: Int\n\n    fn value() -> Int {\n        return count\n    }\n}\n\nfn answer() -> Int {\n    with Fixed { count: 6 } {\n        Counter.value()\n    }\n}\n\ntest \"an operation can return early\" {\n    assert answer() == 6\n}\n",
            call: "answer",
            expect: 6,
        },
        // The frame is found at runtime rather than read off the call site,
        // and this is the program that needs it: `report` is compiled once
        // and performs into whichever handler is installed when it runs.
        Agreed {
            name: "performing from a function the handler does not enclose",
            source: "module a\n\neffect Counter {\n    fn value() -> Int\n}\n\nhandler Fixed implements Counter {\n    state count: Int\n\n    fn value() -> Int { count }\n}\n\nfn report() -> Int uses Counter.value { Counter.value() * 2 }\n\nfn answer() -> Int {\n    with Fixed { count: 4 } {\n        report()\n    }\n}\n\ntest \"a performing function finds the handler its caller installed\" {\n    assert answer() == 8\n}\n",
            call: "answer",
            expect: 8,
        },
        Agreed {
            name: "the innermost handler is the one that answers",
            source: "module a\n\neffect Counter {\n    fn value() -> Int\n}\n\nhandler Fixed implements Counter {\n    state count: Int\n\n    fn value() -> Int { count }\n}\n\nfn answer() -> Int {\n    with Fixed { count: 1 } {\n        with Fixed { count: 2 } {\n            Counter.value()\n        }\n    }\n}\n\ntest \"nesting decides\" {\n    assert answer() == 2\n}\n",
            call: "answer",
            expect: 2,
        },
        // What the frame being unlinked buys: the outer handler is back once
        // the inner block ends, rather than being shadowed forever.
        Agreed {
            name: "a handler stops answering when its block ends",
            source: "module a\n\neffect Counter {\n    fn value() -> Int\n}\n\nhandler Fixed implements Counter {\n    state count: Int\n\n    fn value() -> Int { count }\n}\n\nfn answer() -> Int {\n    with Fixed { count: 10 } {\n        let inner = with Fixed { count: 3 } {\n            Counter.value()\n        }\n        inner + Counter.value()\n    }\n}\n\ntest \"the outer handler comes back\" {\n    assert answer() == 13\n}\n",
            call: "answer",
            expect: 13,
        },
        // The frame-list shape survives an inner `with` block for a
        // different effect. After the inner block ends, the outer handler is
        // restored and its state is intact. This is the same property a
        // scheduler needs after a resumption completes inside a nested
        // context: the outer frame must not be corrupted or lost.
        //
        // step() bumps and reads Counter. Called from inside the Flag block
        // (count goes to 1), then after the Flag block ends (count goes to
        // 2). If the outer frame were stale the second call would find the
        // wrong handler or the wrong count.
        Agreed {
            name: "state persists across a nested handler block for a different effect",
            source: "module a\n\neffect Counter {\n    fn value() -> Int\n    fn bump(by: Int) -> ()\n}\n\nhandler InMemory implements Counter {\n    state count: Int\n\n    fn value() -> Int { count }\n\n    fn bump(by) -> () {\n        count = count + by\n    }\n}\n\neffect Flag {\n    fn mark() -> ()\n    fn marked() -> Bool\n}\n\nhandler Toggle implements Flag {\n    state set: Bool\n\n    fn mark() -> () {\n        set = true\n    }\n\n    fn marked() -> Bool { set }\n}\n\nfn step() -> Int\n  uses Counter.bump, Counter.value\n{\n    Counter.bump(1)\n    Counter.value()\n}\n\nfn answer() -> Int {\n    with InMemory { count: 0 } {\n        let a = with Toggle { set: false } {\n            Flag.mark()\n            step()\n        }\n        let b = step()\n        a + b\n    }\n}\n\ntest \"state survives a nested block for a different effect\" {\n    assert answer() == 3\n}\n",
            call: "answer",
            expect: 3,
        },
        // A handler operation performs into a co-installed handler across
        // the frame boundary. Summer.add uses Log.note, and Sink answers
        // Log from the outer `with` block. When Summer.add runs, the frame
        // search finds Sink further down the list, not Summer itself.
        //
        // This is the cross-frame search a suspended operation relies on
        // when it is resumed: the operation's handler must be found in the
        // list even when the search passes through frames the resumption did
        // not itself install.
        Agreed {
            name: "a handler operation that performs into a co-installed handler",
            source: "module a\n\neffect Tally {\n    fn add(n: Int) -> ()\n    fn total() -> Int\n}\n\neffect Log {\n    fn note() -> ()\n}\n\nhandler Summer implements Tally {\n    state sum: Int\n\n    fn add(n) -> ()\n      uses Log.note\n    {\n        Log.note()\n        sum = sum + n\n    }\n\n    fn total() -> Int { sum }\n}\n\nhandler Sink implements Log {\n    state calls: Int\n\n    fn note() -> () {\n        calls = calls + 1\n    }\n}\n\nfn answer() -> Int {\n    with Sink { calls: 0 } {\n        with Summer { sum: 0 } {\n            Tally.add(3)\n            Tally.add(4)\n            Tally.total()\n        }\n    }\n}\n\ntest \"an operation performs into a co-installed handler\" {\n    assert answer() == 7\n}\n",
            call: "answer",
            expect: 7,
        },
        // State that is not a number. A field narrower than a word is
        // widened on the way in, and a handler is the only place anything
        // gets written twice, so this is where getting that wrong shows.
        Agreed {
            name: "handler state that is a boolean",
            source: "module a\n\neffect Switch {\n    fn on() -> Bool\n    fn flip() -> ()\n}\n\nhandler Toggle implements Switch {\n    state lit: Bool\n\n    fn on() -> Bool { lit }\n\n    fn flip() -> () {\n        lit = !lit\n    }\n}\n\nfn answer() -> Int {\n    with Toggle { lit: false } {\n        Switch.flip()\n        if Switch.on() {\n            1\n        } else {\n            0\n        }\n    }\n}\n\ntest \"a boolean field survives being written\" {\n    assert answer() == 1\n}\n",
            call: "answer",
            expect: 1,
        },
        // `Result`, which is a choice with two variants that nobody writes
        // down. Every `Io` operation that can fail hands one back, so this
        // is most of what stands between the backend and the corpus.
        Agreed {
            name: "a Result that holds a value",
            source: "module a\n\nfn halve(n: Int) -> Result<Int, String> {\n    if n % 2 == 0 {\n        ok(n / 2)\n    } else {\n        err(\"odd\")\n    }\n}\n\nfn answer() -> Int {\n    match halve(10) {\n        ok(half) => half,\n        err(why) => 0 - 1,\n    }\n}\n\ntest \"an even number halves\" {\n    assert answer() == 5\n}\n",
            call: "answer",
            expect: 5,
        },
        Agreed {
            name: "a Result that holds an error",
            source: "module a\n\nfn halve(n: Int) -> Result<Int, String> {\n    if n % 2 == 0 {\n        ok(n / 2)\n    } else {\n        err(\"odd\")\n    }\n}\n\nfn answer() -> Int {\n    match halve(7) {\n        ok(half) => half,\n        err(why) => length(why),\n    }\n}\n\ntest \"an odd number does not\" {\n    assert answer() == 3\n}\n",
            call: "answer",
            expect: 3,
        },
        // Two element types, two layouts. One shared shape would read the
        // wrong width out of memory for one of them.
        Agreed {
            name: "two Results holding different things",
            source: "module a\n\nfn number() -> Result<Int, String> { ok(4) }\n\nfn text() -> Result<String, String> { ok(\"abc\") }\n\nfn answer() -> Int {\n    let first = match number() {\n        ok(n) => n,\n        err(why) => 0,\n    }\n    let second = match text() {\n        ok(s) => length(s),\n        err(why) => 0,\n    }\n    first + second\n}\n\ntest \"each Result keeps what it holds\" {\n    assert answer() == 7\n}\n",
            call: "answer",
            expect: 7,
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

/// A proof about `Int` is only sound if every engine shares the same boundary.
///
/// `grow` is Proven because `Int` does not wrap: `n + 1` either answers with
/// another positive integer or fails with arithmetic that has no answer. If the
/// backend wrapped here, this would come back `i64::MIN` and the proof would be
/// a lie.
#[test]
fn overflow_near_the_boundary_fails_the_same_way_under_both_engines() {
    let source = "module a\n\n\
         type Positive = Int where value > 0\n\n\
         fn grow(n: Positive) -> Positive { n + 1 }\n\n\
         test \"the boundary\" {\n\
         \x20 assert grow(9223372036854775807) == 0\n\
         }\n";
    let (_, one) = checked(source);

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
    let failure = outcomes[0]
        .failure
        .as_ref()
        .expect("the interpreter should stop at the overflow");
    assert_eq!(failure.code, codes::ARITHMETIC);

    let lowered = deed_mir::lower(&one.module, &one.resolutions, &one.types).expect("this lowers");
    let module = compile(&lowered).expect("this compiles");
    assert_eq!(
        call(&module, "grow", &[Value::I64(i64::MAX)]),
        Err(Trap::Failed {
            code: codes::ARITHMETIC.to_string(),
            message: "this arithmetic has no answer".to_string(),
            span: None,
            blame_caller: false,
        })
    );
}

/// What a tier is worth at runtime, which is the whole of why the checker
/// bothers.
///
/// Two functions with the same body and the same `where` clause. One is
/// called with something the checker can prove satisfies it and the other
/// with something it cannot, and only the second compiles to a check. This
/// is the claim `design/05-backend.md` makes about proven obligations
/// costing nothing, asked as a count of instructions rather than as a
/// sentence.
#[test]
fn a_proven_precondition_compiles_to_nothing_and_a_guarded_one_does_not() {
    let proven = "module a\n\nfn halve(n: Int) -> Int\n  where\n    n >= 0,\n{\n    n / 2\n}\n";
    let guarded = "module a\n\nfn halve(n: Int) -> Int\n  where\n    n >= 0,\n{\n    n / 2\n}\n\nfn any(m: Int) -> Int { halve(m) }\n";

    let sizes: Vec<usize> = [proven, guarded]
        .iter()
        .map(|source| {
            let (_, one) = checked(source);
            let lowered =
                deed_mir::lower(&one.module, &one.resolutions, &one.types).expect("this lowers");
            let module = compile(&lowered).expect("this compiles");
            module.funcs[0].body.len()
        })
        .collect();

    assert!(
        sizes[0] < sizes[1],
        "a call the checker could not prove should compile to more than one it could: {sizes:?}"
    );
}

/// An `assert refuses` keeps the check it is aiming at, and only that one.
///
/// The check is dropped when every recorded call proved the clause, and the
/// checker records no tier for a call inside `assert refuses`: a
/// precondition meant to fail is not an obligation anybody discharged. So
/// the one caller that needs the check is the one caller nothing knew about,
/// and the clause it aims at has to be named rather than inferred from the
/// tiers.
///
/// Both halves matter. A backend that kept every check whenever any `assert
/// refuses` appeared would pass the first assertion and lose what the tier
/// is worth, which is the claim `design/05-backend.md` makes.
#[test]
fn an_assert_refuses_keeps_the_check_it_aims_at_and_no_other() {
    let source = "module a\n\n\
fn halve(n: Int) -> Int\n  where\n    n >= 0,\n{\n    n / 2\n}\n\n\
fn twice(n: Int) -> Int\n  where\n    n >= 0,\n{\n    n + n\n}\n\n\
fn answer() -> Int { twice(4) }\n\n\
test \"a negative one is turned down\" {\n    assert refuses halve(0 - 1)\n}\n";

    let (_, one) = checked(source);
    let lowered = deed_mir::lower(&one.module, &one.resolutions, &one.types).expect("this lowers");
    let sizes: Vec<usize> = lowered
        .functions
        .iter()
        .map(|function| function.body.stmts.len())
        .collect();

    let halve = lowered
        .functions
        .iter()
        .position(|function| function.name == "halve")
        .expect("`halve` is declared");
    let twice = lowered
        .functions
        .iter()
        .position(|function| function.name == "twice")
        .expect("`twice` is declared");

    assert!(
        sizes[halve] > sizes[twice],
        "`halve` is what the `assert refuses` aims at, so it keeps its check \
         and `twice` does not: {sizes:?}"
    );
    assert_eq!(
        sizes[twice], 0,
        "every call to `twice` proved its clause, so it should compile to no \
         check at all: {sizes:?}"
    );
}

/// A program the backend cannot compile has to say so, rather than compile
/// into something that runs and answers wrongly.
#[test]
fn what_the_backend_cannot_compile_is_refused_by_name() {
    let (_, one) = checked(STILL_REFUSED);
    let lowered = deed_mir::lower(&one.module, &one.resolutions, &one.types)
        .expect("this lowers, compiling it is what does not");
    let refused = compile(&lowered).expect_err("this is not compiled yet");
    assert_eq!(refused.function, "same");
    assert!(refused.to_string().contains("in memory"), "{refused}");
}

/// The interpreter stays the reference implementation, so a program the
/// backend refuses still runs. That is the whole of why the interpreter is
/// not being replaced, and it is worth a test rather than a sentence.
#[test]
fn a_program_the_backend_refuses_still_runs_under_the_interpreter() {
    let (_, one) = checked(&format!(
        "{STILL_REFUSED}\ntest \"two of the same\" {{\n    assert same(Point {{ x: 1 }}, Point {{ x: 1 }})\n}}\n"
    ));

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

/// A shape the backend does not compile, for the two tests above.
///
/// Comparing two records is structural in this language and two addresses
/// being equal is not two records being equal, so the backend refuses rather
/// than answering the wrong question. When it stops refusing, these two tests
/// need another shape rather than deleting: what they hold is that a refusal
/// says which function it met and that the interpreter still answers.
const STILL_REFUSED: &str = "module a\n\nrecord Point {\n    x: Int,\n}\n\nfn same(one: Point, other: Point) -> Bool { one == other }\n";

/// One generated run, by what happened.
enum Finding {
    Agreed,
    Unsupported(String),
    Crash(String),
    Disagreement { interpreted: i64, compiled: i64 },
}

const GENERATED_SUBJECT: &str = "module a\n\nchoice Mood {\n    Calm,\n    Loud { by: Int },\n}\n\nrecord Sample {\n    numbers: List<Int>,\n    word: String,\n    mood: Mood,\n    yes: Bool,\n}\n\nfn score(sample: Sample) -> Int {\n    let tone = match sample.mood {\n        Calm => 0,\n        Loud { by } => by,\n    }\n\n    let sign = if sample.yes {\n        1\n    } else {\n        0 - 1\n    }\n\n    length(sample.numbers) + length(sample.word) + tone + sign\n}\n";

fn escaped(text: &str) -> String {
    let mut out = String::new();
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn to_deed_literal(value: &InterpretedValue) -> String {
    match value {
        InterpretedValue::Unit => "()".to_string(),
        InterpretedValue::Int(n) => n.to_string(),
        InterpretedValue::Bool(true) => "true".to_string(),
        InterpretedValue::Bool(false) => "false".to_string(),
        InterpretedValue::Str(text) => escaped(text),
        InterpretedValue::List(elements) => {
            let inner: Vec<String> = elements.iter().map(to_deed_literal).collect();
            format!("[{}]", inner.join(", "))
        }
        InterpretedValue::Record(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(name, value)| format!("{name}: {}", to_deed_literal(value)))
                .collect();
            format!("Sample {{ {} }}", inner.join(", "))
        }
        InterpretedValue::Variant(variant) => {
            if variant.fields.is_empty() {
                variant.name.clone()
            } else {
                let fields: Vec<String> = variant
                    .fields
                    .iter()
                    .map(|(name, value)| format!("{name}: {}", to_deed_literal(value)))
                    .collect();
                format!("{} {{ {} }}", variant.name, fields.join(", "))
            }
        }
        InterpretedValue::Result { ok, value } => {
            let name = if *ok { "ok" } else { "err" };
            format!("{name}({})", to_deed_literal(value))
        }
        InterpretedValue::Capability(_)
        | InterpretedValue::Closure(_)
        | InterpretedValue::Function { .. } => {
            panic!("this generator should not produce callable or capability values")
        }
    }
}

fn generated_program(input: &InterpretedValue) -> String {
    format!(
        "{GENERATED_SUBJECT}\nfn main() -> Int {{\n    score({})\n}}\n",
        to_deed_literal(input)
    )
}

fn run_generated_case(input: &InterpretedValue) -> Finding {
    let source = generated_program(input);
    let (sources, one) = checked(&source);

    let mut interpreted = Interpreted::new();
    interpreted.add(
        one.file,
        &one.module,
        &one.resolutions,
        one.guards(),
        one.rows(),
    );
    let interpreted_run = run_main(&interpreted, one.file, std::path::Path::new("."), &[])
        .expect("generated source should declare main");
    let interpreted_value = match interpreted_run.result {
        Ok(value) => match value.as_int() {
            Some(value) => value,
            None => {
                return Finding::Crash("the interpreter returned a non-integer value".to_string());
            }
        },
        Err(why) => {
            let text = deed_diagnostics::render_human(&sources, &why);
            return Finding::Crash(format!("the interpreter stopped:\n{text}"));
        }
    };

    let lowered = match deed_mir::lower(&one.module, &one.resolutions, &one.types) {
        Ok(lowered) => lowered,
        Err(why) => return Finding::Unsupported(why.to_string()),
    };
    let module = match compile(&lowered) {
        Ok(module) => module,
        Err(why) => return Finding::Unsupported(why.to_string()),
    };
    let compiled_value = match call(&module, "main", &[]) {
        Ok(Some(value)) => value.as_i64(),
        Ok(None) => return Finding::Crash("the backend returned no value".to_string()),
        Err(why) => return Finding::Crash(format!("the backend trapped: {why}")),
    };

    if interpreted_value == compiled_value {
        Finding::Agreed
    } else {
        Finding::Disagreement {
            interpreted: interpreted_value,
            compiled: compiled_value,
        }
    }
}

#[test]
fn generated_programs_keep_the_interpreter_and_backend_in_agreement() {
    let (_, generated) = checked(GENERATED_SUBJECT);
    let mut program = Interpreted::new();
    program.add(
        generated.file,
        &generated.module,
        &generated.resolutions,
        generated.guards(),
        generated.rows(),
    );

    let function = generated
        .module
        .items
        .iter()
        .find_map(|item| match item {
            deed_ast::Item::Function(function) if function.sig.name.name == "score" => {
                Some(function)
            }
            _ => None,
        })
        .expect("`score` should exist");

    let cases = generate_inputs(
        &program,
        generated.file,
        &generated.module,
        &generated.resolutions,
        function,
        PropertyConfig {
            cases: 60,
            ..PropertyConfig::default()
        },
    );
    assert!(
        !cases.cases.is_empty(),
        "the generator produced no usable input (seed {:#x}, rejected {})",
        cases.seed,
        cases.rejected
    );

    let mut agreed = 0usize;
    let mut unsupported = Vec::new();
    let mut crashes = Vec::new();
    let mut disagreements = Vec::new();

    for args in &cases.cases {
        let input = args[0].clone();
        match run_generated_case(&input) {
            Finding::Agreed => agreed += 1,
            Finding::Unsupported(why) => unsupported.push((input, why)),
            Finding::Crash(why) => crashes.push((input, why)),
            Finding::Disagreement {
                interpreted,
                compiled,
            } => disagreements.push((input, interpreted, compiled)),
        }
    }

    assert!(
        agreed > 0,
        "no generated input reached both engines; unsupported: {:?}; crashes: {:?}",
        unsupported
            .iter()
            .map(|(value, why)| format!("{} -> {why}", to_deed_literal(value)))
            .collect::<Vec<_>>(),
        crashes
            .iter()
            .map(|(value, why)| format!("{} -> {why}", to_deed_literal(value)))
            .collect::<Vec<_>>()
    );

    if let Some((input, interpreted, compiled)) = disagreements.into_iter().next() {
        let shrunk = shrink_inputs(
            &program,
            generated.file,
            &generated.module,
            &generated.resolutions,
            vec![input.clone()],
            |candidate| {
                matches!(
                    run_generated_case(&candidate[0]),
                    Finding::Disagreement { .. }
                )
            },
        );
        panic!(
            "generated disagreement: input {} shrinks to {}, interpreter {}, backend {}",
            to_deed_literal(&input),
            to_deed_literal(&shrunk[0]),
            interpreted,
            compiled
        );
    }

    assert!(
        crashes.is_empty(),
        "generated crashes: {:?}",
        crashes
            .iter()
            .map(|(value, why)| format!("{} -> {why}", to_deed_literal(value)))
            .collect::<Vec<_>>()
    );
}
