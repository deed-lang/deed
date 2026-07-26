# Syntax

A sketch, not a grammar. Nothing here is settled. The goal is to have something concrete
enough to argue with, and concrete enough to write a lexer against.

Most of what follows is now built, and the rest is not, and a reader cannot tell the two
apart by looking. So: every rule stated as a rule, and every diagnostic code named, describes
what the compiler does today. Four constructs appear in the illustrations and do not parse at
all, because the sections around them would be thin without something to name:

- traits, which have no keyword and no node
- `matches(value, EMAIL_PATTERN)`, which is not in the prelude
- `Int.parse(input)`, since a builtin type has no members

The reason to be exact about this is that a document once described `Diverge` as an effect
the compiler tracked, and the word did not resolve anywhere in the repository. Sketching is
fine. Sketching in a voice that cannot be told apart from a specification is not.

Where there is no reason to differ, syntax follows Rust and TypeScript. Familiarity is free
recognition and novelty buys nothing (see P2 in [01-principles.md](01-principles.md)).

## Modules

One module per file. The path is the module name, so there is nothing to keep in sync.

```vow
module payments/transfer

use std/result.{Result, ok, err}
use ledger.{Ledger, Entry}
```

Imports are explicit and named. No wildcards, no re-exports, no ambient prelude beyond a
small fixed set of primitives. If a name is in scope, some line in this file put it there.

**A module's name says where it lives.** A module named `a/b` is at `<root>/a/b.vow`, and the
root is worked out from a file that was named on the command line: take its own module path
off the end of its own file path, and what is left is the root. So `vow run examples/todo.vow`
finds `examples/list.vow` for a `use examples/list`, and nothing has to be listed twice.

There is no search path, no config file and no manifest. One layout, and the `use` line
already says where to look. That rule had been true of every file in this repository since
the first one and nothing said it out loud, which meant a program that imported anything
could not be run by naming its own file.

What was named on the command line is the subject, and what an import needed is context. So
`vow test app.vow` does not run the tests of a library it happens to import, and
`vow check app.vow` does report an error in that library, because a program that cannot
compile its own dependency does not compile. It is the same split the language server makes
between the workspace and the open document.

This is not a package manager. Nothing is fetched, nothing is versioned, and the search stops
at the root the named files imply.

## Types

```vow
type AccountId = Id<Account>

record Money {
    units: Int,
    currency: Currency,
}

choice TransferError {
    InsufficientFunds { available: Money },
    AccountClosed { account: AccountId },
    LimitExceeded,
}
```

`record` is a product, `choice` is a sum. No classes, no inheritance, no interfaces with
default bodies. Behaviour is attached with traits, and a trait cannot carry state.

Refinements are part of the type, which is how most validation stops needing to exist:

```vow
type Positive = Int where value > 0
type Email = String where matches(value, EMAIL_PATTERN)
```

A `Positive` cannot be constructed without the check passing, so no function taking one
needs to re-check it. This is P6.

## Operators

There are fewer than it looks, because none of them are overloadable and there is no trait
system for them to hang off.

| | |
| --- | --- |
| `+ - * / %` | `Int`, and nothing else. Overflow and division by zero are errors, not wraps |
| `+` | also joins two `String`s |
| `< <= > >=` | `Int` and `String`, and nothing else |
| `== !=` | anything, structurally |
| `&& \|\|` | `Bool`, and they short circuit |

`+` is the one operator with two meanings. There is no conversion between `Int` and
`String`, so no expression is ambiguous about which one it wanted, and spelling
concatenation any other way would be a tax on the most ordinary thing a program does.

Ordering is deliberately narrow. Comparing two records used to pass the type checker and
fail at runtime, and the runtime message blamed the interpreter for not implementing
something that has nothing to implement: there is no order on a record that anyone could
define, because there is no trait to define it in. Refusing the comparison is the honest
answer until there is one. Equality is different, because structural equality is total and
wanting to know whether two records are the same is reasonable.

Strings order by character. For text in one script that is the order anyone expects, and for
text that mixes them it is a decision that needs a locale, which is not something an operator
should be guessing at.

The prelude carries one function for measuring, `length`, which counts a `String` in
characters rather than bytes. Otherwise a refinement written against it would mean something
different depending on which letters turned up. It is in the prelude because a `String` you
cannot measure is a `String` you cannot check, and the prelude stays small enough that every
entry can be argued for.

It carries four more for taking a string apart and putting one back together:

```vow
split("a,b,c", ",")      // ["a", "b", "c"]
split("gün", "")         // ["g", "ü", "n"]
join(["a", "b"], ",")    // "a,b"
to_string(0 - 12)        // "-12"
to_int("41")             // ok(41)
to_int("forty one")      // err("`forty one` is not a number")
```

Two pairs of inverses, and that is the argument for all four at once. Before them a program
could hold text and hold a number and get from neither to the other, so nothing could read
input, print a count, or write a file back out.

`split` and `join` stay inverses in the corners as well as the middle, which is the only
property either of them has: a separator at the edges leaves an empty piece rather than being
tidied away, and splitting something with no separator in it gives one piece rather than
none. An empty separator gives the characters. The alternatives are an error the return type
cannot express, or an empty string between every pair of characters, and this way walking a
string costs the prelude no second name.

`to_int` hands back a `Result`. Text that is not a number usually came from a file or an
argument, so it is not a mistake in the caller and there is nothing to trap about.

One more, on a narrower argument:

```vow
trim("  a  ")            // "a"
trim("  a  b  ")         // "a  b"
trim("line\r")           // "line"
```

The test for whether something belongs in the prelude is whether it can be written in the
language, and most of the obvious candidates can. `contains(text, needle)` is
`length(split(text, needle)) > 1`. `replace(text, from, to)` is `join(split(text, from), to)`.
An indexed walk is `for item at index in items`.

`trim` cannot be. Deciding what whitespace is needs to look at characters, and taking it off
the ends needs a walk that stops early, which a fold does not do. It is also the difference
between a program working and not: splitting a file on `"\n"` leaves a `\r` on every line of
a file written on Windows, and `examples/todo.vow` printed its own output backwards over
itself until there was a way to take one off.

Whitespace here is four characters, space, tab, carriage return and newline, and not the
Unicode whitespace table. That is a large amount of behaviour to hide behind a four letter
name, and it would make what `trim` does depend on a table nobody reading the signature can
see. A program that needs the full definition can say so in a name of its own.

What is still missing is slicing, searching, case and padding. Those want a standard library,
and there is no story for one yet.

## Lists

```vow
let names: List<String> = ["ada", "grace"]
let none: List<Int> = []

length(names)          // 2
at(names, 0)           // ok("ada")
at(names, 9)           // err("index 9 is outside a list of 2")
push(names, "katherine")
```

`List` is built in, and that is not where it should end up. It is the same shortcut `Result`
takes: the checker compares element types componentwise and lets an unknown one absorb, which
is why `[]` fits wherever a list was wanted with nothing written on the literal itself. The
first element of a literal decides the element type, because with no unification there is
nothing to meet two candidates with.

The reason it is still built in is no longer that it could not be declared. It can be now,
and `Option` in `examples/generic_types.vow` is the proof. It is that `[1, 2, 3]` is a
literal with syntax of its own, and moving `List` out of the language means deciding what
that literal builds.

It is built in at all because nothing else in the language could hold more than one of
something. Every program written before it worked on a fixed number of named variables,
which ruled out reading input, building a report, and almost anything else worth writing.

`at` hands back a `Result` rather than the element. An index nobody promised is there is not
a mistake in the caller, and nothing in this language stops a program, so it is an error
value like everything else that can fail. `push` returns a new list: handler state is meant
to be the only mutable thing here, and a collection that could be written through would
quietly be a second one.

## Iteration

```vow
let total = for n in numbers with sum = 0 {
    sum + n
}

for line in lines {
    Io.write(out, line)
}
```

A `for` is a fold with syntax, not a loop with a variable in it. The block's value is the
accumulator for the next turn, and the value of the whole expression is the last one. Leaving
`with` off means an accumulator of `()`, so the body has to produce `()` and the loop is
there for its effects.

**Nothing here is assigned.** `sum` is a fresh binding on every turn. That is the entire
reason this shape was chosen over the familiar one: Vow has exactly one mutable thing, a
handler's `state`, and the claim that an empty effect row means a function cannot observe or
cause a change to anything rests on there being no second one. A `for` with a mutable
accumulator would have been the more familiar spelling of a weaker language.

**The other reason is `Diverge`.** There is no termination proving, so a function that can
reach itself has to declare that it may not return. Without a loop, walking a list is
recursion, and walking a list is the most ordinary thing a program does, so almost every
function in a real program would carry `Diverge`. 03-effects says a row that drifts toward
listing everything stops carrying information, which is exactly what that is. A `for` walks a
list that is already there, so it stops, so it declares nothing. `examples/todo.vow` had
three recursive walks and three `Diverge` declarations before this existed and has none of
either now.

The accumulator is not in scope in what it starts as: `with sum = sum` names something that
does not exist yet rather than a value that refers to itself. The binder is a binding like
any other, so shadowing an outer name with it is the same error it is anywhere else.

**A `for` can say where in the list it is.**

```vow
for task at here in tasks with kept = [] {
    if task.done {
        kept
    } else {
        push(kept, to_string(here + 1) + ". " + task.title)
    }
}
```

`at` is optional and there is no second loop form: everything above still holds, the index is
a fresh binding on every turn like the element and the accumulator, and nothing became
mutable. It counts from zero, like the `at` that indexes a list, because a walk that
disagreed with the only way to index would be a trap rather than a convenience. It is known
to be a real position, so it is not negative and it is below the length of what is being
walked, which is what makes it worth binding rather than counting: something that indexes with
it can say so in a `where` clause and be believed.

`at` stays an ordinary name, and it is the name of the prelude function that indexes a list.
Reserving the word for one position would cost a name people already use, and the only thing
that can follow a `for` binder is `at` or `in`, so there is nothing here for it to be
confused with. That is the same reasoning that took `state` back out of the keyword list.

This is in the language rather than in the library because the library cannot have it
otherwise. Everything in `examples/list.vow` is written with a `for`, so `map` cannot hand a
callback something the walk never knew. With this, the library builds its own indexed forms:
`map_at` is four lines and no part of the language grew a second `map`. Before it,
`examples/todo.vow` had three walks carrying a counter in a record, all the same shape, each
with branches that existed only to remember to bump it.

What can be walked is a `List`, and nothing else. A `for` over a `String` or a `Result` would
need a way to say what walking one means, which is a trait system, which does not exist.

What is deliberately absent is `while`, `break` and `continue`. A `while` cannot be shown to
terminate, so it would bring `Diverge` back with it and undo the reason for having `for`.
`break` and `continue` want a loop with control flow in it rather than a fold, and neither
has come up yet in a program written here.

What is deliberately absent from lists: slicing, searching, and any operation that takes a
function. The last one is now writable, and `examples/list.vow` is a list library written in
Vow rather than built into the compiler. It stays out of the prelude, because the prelude is
where names go to become unavailable to everyone else and a library does not need to be
there.

## Functions and the contract block

The centre of the language. Everything between the return type and the opening brace is the
contract.

```vow
fn transfer(from: AccountId, to: AccountId, amount: Money)
    -> Result<Receipt, TransferError>
  where
    from != to,
  uses
    Ledger.balance,
    Ledger.post,
    Audit.append,
  ensures
    ok  => Ledger.balance(from).units == old(Ledger.balance(from).units) - amount.units,
    ok  => Ledger.balance(to).units   == old(Ledger.balance(to).units)   + amount.units,
    err => unchanged(Ledger),
{
    let available = Ledger.balance(from)
    if available.units < amount.units {
        return err(InsufficientFunds { available })
    }

    Ledger.post(Entry { account: from, amount })
    Ledger.post(Entry { account: to, amount })
    Audit.append(TransferRecorded { from, to, amount })

    ok(Receipt { from, to, amount })
}
```

### `where`, preconditions

What the caller must guarantee. Read at the call site with the facts in scope there, and
checked at the boundary on every call whatever the reading found. A precondition failure is a
bug in the caller, and it is reported that way: a call the checker can see breaks the clause
is `VOW4025` where the call was written, not a runtime failure inside a function the author
of the call did not write.

For a long time this section described a check that did not exist. A `where` clause was two
things, a fact for the callee's body and a check inside the callee at runtime, and nothing
ever looked at it from where the call was. So `halve(0 - 5)` against `where n >= 0` passed in
silence and failed when it ran.

What crosses into a clause is the caller's facts said in the callee's parameter names: the
range of each argument, how long each one is, and the differences between them where the
arguments are things a fact can be about. That last part is what settles a clause relating
two arguments, which is most of what a `where` clause is for:

```vow
fn nth(items: List<Int>, index: Int) -> Result<Int, String>
  where
    index >= 0,
    index < length(items),
{
    at(items, index)
}
```

A caller that checked the length first proves both clauses. A caller that did not is
`Guarded`, which is the ordinary case and not a mistake. A caller passing `0 - 1` is refused.

The runtime check stays either way, for the same reason an `ensures` clause is evaluated on
every call whatever tier it landed in: the tier says how much was settled ahead of time, not
whether the check happens. And a predicate does not cross a module boundary, the same as a
refinement's does not, so a caller in another file answers for a precondition at runtime and
the checker says nothing about it in either direction.

### `uses`, the effect row

Every effect the body may perform. Nothing else is reachable. An empty `uses` means the
function is pure, and that is the default when the clause is absent.

Effects propagate: a caller needs at least the union of what it calls, and the compiler
infers the minimum and complains when a declaration is wider than the body needs. Declaring
authority you do not use is an error, not a warning, because an over-wide row is exactly the
thing that makes the annotation stop meaning anything.

Details in [03-effects.md](03-effects.md).

### `ensures`, postconditions

What the function guarantees. Written per outcome, so `ok =>` and `err =>` are separate
obligations and neither is optional.

`result` is what the function produced. In an `ok =>` clause it is the success value, in an
`err =>` clause it is the error, and for a function that does not return a `Result` it is
whatever was returned. So an obligation never has to unwrap anything, and the two outcomes
cannot be confused with each other.

Without it a pure function could not have a postcondition at all, since its return value is
the only thing it produces. That was true from the first draft of this document and nobody
noticed, because every example was effectful.

`old(expr)` is the value of `expr` on entry. `unchanged(Effect)` says nothing observable
through that effect was modified, which is how rollback gets stated without describing the
mechanism.

Postconditions are the review surface. A person reads the contract, the compiler is
responsible for the body agreeing with it.

## Generic functions and types

```vow
fn first<T>(items: List<T>) -> Result<T, String> {
    at(items, 0)
}

record Pair<A, B> {
    left: A,
    right: B,
}

choice Option<T> {
    None,
    Some { value: T },
}
```

`Result` and `List` are built in because there was no way to declare either. That shortcut
was never the expensive part. The expensive part was that nobody could write a library:
`first`, `last`, `map` and `count_where` are all one function at different element types, and
until this existed not one of them could be written down. `Option` was the third generic type
people reach for, and it is declared rather than built in.

**There is no unification and no inference beyond the expression at hand.** At a call site
the declared parameter types are matched against the argument types, walking down both in
step. `List<T>` against `List<String>` gives `T = String`. A literal does the same thing with
its fields: `Pair { left: 1, right: "a" }` matches the declared field types against the
values and gets `A = Int`, `B = String`. One mechanism, two places, and it is the same kind
of local reasoning the checker already does everywhere else.

**The first answer for a parameter wins.** `fn same<T>(a: T, b: T)` called with `same(1, "two")`
binds `T` from the first argument and then reports an ordinary mismatch on the second:
"expected `Int`, found `String`", pointing at the parameter. That is a better message than
anything about a variable the caller never wrote.

**An argument the checker gave up on decides nothing.** An unknown agrees with whatever the
parameter turns out to be, so treating it as an answer would let one argument nobody could
type decide the type of every other, and turn one mistake into several.

**Every type parameter has to appear in a parameter's type.** `fn empty<T>() -> List<T>` is
`VOW4023`. Without this rule a call would sometimes have to write its type arguments, which
needs `empty<String>()` to parse, which is the `f<a>(b)` versus `f < a > (b)` ambiguity, and
P2 has a budget for exactly this kind of thing. The rule costs nothing today: `[]` is already
a `List<unknown>` and unknown absorbs, so `empty()` would add nothing the empty literal does
not already do.

It is also the same claim everything else here makes. A signature is complete, and one with
a hole a caller has to fill in from somewhere else is not.

**A type parameter is not a free pass inside the body.** `T` is whatever the caller decided,
so the body may hold one, count them and put them in a list, and may not add two together.
Nothing said they could be added.

**A generic function is not a value.** `let f = first` is `VOW4024`. One expression has one
type here, and a generic function named rather than called has as many as there are ways to
call it. Making that work needs a polymorphic value, which is a much larger thing than
substituting into a signature at a call site.

**A type parameter crosses a module boundary as a position, not as a name.** The same reason
an imported type crosses as a module path and a name: a `DefId` is an index into one module's
table. An imported generic function arrives with its parameters still in it and the call site
does exactly the work it would have done at home.

**A generic type is a head plus arguments, compared componentwise.** Two `Pair`s are the same
type when the declaration matches and every argument matches, which is exactly how `Result`
and `List` were already compared. A field reads at the type it was applied to, so `left` on a
`Pair<Int, String>` is an `Int`, and so does a pattern binder: `Some { value }` on an
`Option<Int>` binds an `Int`.

**A type is written with exactly as many arguments as it declared.** `Pair` bare is
`VOW4013`, and so is `Pair<Int>`. A signature is complete, so a missing argument is as much a
hole in one as a parameter with no type, and filling it in with unknowns would make every use
of it agree with everything.

**A bare variant says nothing about its arguments.** `None` is an `Option<unknown>`, and
unknown absorbs, so it fits wherever an `Option` was wanted. That is not a special case: `[]`
is a `List<unknown>` and `ok(x)` is a `Result<T, unknown>` for the same reason. Three places,
one answer.

What is still missing is a type parameter on an alias. An alias with no predicate is expanded
away and one with a predicate is a refinement, and a generic refinement is a different
question about what the predicate may say about a value whose type nobody knows yet.

A declaration's list may also hold a **row variable**, written `uses r`:

```vow
fn map<A, B, uses r>(items: List<A>, step: Fn(A) uses r -> B) -> List<B>
  uses
    r,
{ ... }
```

One list rather than two, because a reader wants everything a call has to work out in one
place, and `uses` says which kind each entry is rather than leaving it to be inferred from
where it turns up. `design/03-effects.md` has the rest.

## Verification, honestly

Full static proof of arbitrary postconditions is undecidable and shipping a solver as a
hard dependency would violate P9. So contracts are handled in three tiers, and the tier is
always visible:

| Tier | Mechanism | When |
| --- | --- | --- |
| Proven | discharged statically | arithmetic, refinements, exhaustiveness, simple algebraic facts |
| Tested | property tests generated from the contract | anything decidable by sampling |
| Guarded | runtime check at the boundary | everything else |

`vow check` reports which tier each obligation landed in. A contract silently degrading to a
runtime check would be the single most dishonest thing this language could do, so it does
not happen quietly.

There is a worse thing, and it happened. For a while the checker recorded a `Guarded`
obligation on a return value, printed "so it becomes a runtime check", and the interpreter
had no check there: it guarded arguments and annotated `let`s and nothing else, because those
were the two places somebody happened to write the call. The warning was what convinced a
reader they were covered.

The two passes now read the same table. An obligation recorded as `Guarded` at a span is a
check at that span, and there is no second place where the runtime decides what to check, so
they cannot drift apart without a test noticing.

All three tiers exist. `Tested` covers pure functions whose parameters can be generated:
`vow test` runs a hundred generated inputs against the contract and shrinks any
counterexample it finds. Everything else is `Guarded`, checked on every call.

**A test can say that a contract turns something down.**

```vow
test "a guard refuses what it should" {
    assert refuses order_of(0)
}
```

`assert refuses e` passes when evaluating `e` fails a `where` clause, an `ensures` clause or
a refinement, and fails when anything else happens, including `e` producing a value. It is
the one thing in the language that catches, and it catches those three and nothing else:
overflow, a missing handler and a run that went too deep are a program going wrong rather
than a signature doing its job, and catching those would be a `try` with a small vocabulary
rather than a statement about what was promised.

It exists because the `Guarded` tier was the one thing a Vow program could not test about
itself. A contract failure ends the run, so a file of examples showing a guard refusing
something could not pass, and every such test had to be written in Rust against the compiler.
Then preconditions started being read at the call site, and the checker began refusing those
files outright, so the better the checking got the further out of reach the check itself
went. Inside an `assert refuses` the checker is quiet about a contract it can see will break,
because that is the statement being right rather than a mistake, and nothing is recorded as
discharged.

`refuses` is a name everywhere else. It is the marker only when an identifier follows it, and
no statement could ever have been two names in a row, so `assert refuses(x)` is still a call
to a function somebody called `refuses`. Same reasoning as `state` and as the `at` in a `for`.

### What `Proven` can decide

Interval reasoning with one relation on top. Each integer in scope has a known range, and
a refinement is discharged by evaluating its predicate over that range rather than over a
value. Ranges come from the things that state one:

- a `where` clause, so `n > 0` makes `n` at least one for the whole body
- a parameter already of a refined type, so a `Positive` parameter needs no `where` clause
  repeating the type in prose
- the condition of an `if`, narrowed one way in the then branch and the other way in the else
- a guard that leaves, so after `if n <= 0 { return err(..) }` the rest of the body knows
- the contract of a function being called, so a proof one function did is worth something to
  the next one

How long something is counts as a name here. `length(items)` used to come back as a range and
nothing else, which meant it could not be one side of a difference, and `index < length(items)`
was a shape the relation below could not see even though `low < high` was the shape it existed
for. A length is a term keyed on the thing being measured, so the two are one rule:

```vow
fn how_many_after(items: List<Int>, index: Int) -> Positive
  where
    index >= 0,
    index < length(items),
{
    length(items) - index
}
```

The default for one is zero and up rather than anything at all, because there is no list with
fewer than no things in it. Nobody writes that down and no call has to hand it back. Two lists
are two terms, so a bound on one says nothing about the other, and `length(f(items))` is not a
term at all: two calls could hand back two different lists and a fact about one of them is not
a fact about the other.

The relation is the difference between two names. An interval has nowhere to put `low < high`,
so every contract that says how two arguments relate used to be thrown away, and that is half
of what a `where` clause is for. A range per pair of names holds exactly the orderings, which
is what comparisons produce and nothing more:

```vow
fn span(low: Int, high: Int) -> Positive
  where
    low >= 0,
    high <= 100,
    low < high,
{
    high - low
}
```

A difference and a range each tighten the other, so a bound arriving after the comparison that
needed it still counts, and two differences sharing a name make a third, so `a < b` and `b < c`
settle `a < c`. That last one is a fixpoint, and it is run for a fixed small number of rounds
rather than to exhaustion, because P9 is a budget and not a preference.

A name multiplied by a number is still a name, counted more than once, so `n + n` and `n * 2`
read as two of one name. `n * 3 > 0` puts `n` at one or above, which needs dividing a bound by
three, and that rounds inwards: `3 * n <= -2` admits `n <= -1` and not `n <= 0`. A name
multiplied by a name is not linear and that is where this stops.

The other two clauses in that example are not there for the relationship. They are there so
`high - low` has an answer: without a bound on either name the difference can be larger than an
integer, and an expression that cannot be computed proves nothing about what it computes.

A refined value can also reach a different refinement over the same base. `Positive` widens to
`Int` and narrows back into `NonNegative`, and the predicate that arrives is usually enough to
discharge the predicate that is wanted. The narrowing is an obligation like any other, so the
direction that does not follow is `Guarded` rather than rejected.

Reading a callee's contract is only honest if the contract is kept, and it is: an `ensures`
clause is evaluated on the way out of every call whatever tier it landed in, and a refined
return type is checked against its predicate at the same point. The tier says how much was
settled ahead of time, not whether the check happens. So a caller holding a returned value is
holding something that already passed, and a broken promise cannot launder itself into a proof
somewhere else.

What a call carries is the same shape as what a body reasons with: the range the result lands
in, and the range of `result - argument` for each argument a clause ties it to. A promise that
never mentions its arguments is a rare thing to want, so a call used to lose most of what an
`ensures` said:

```vow
fn same(n: Int) -> Int
  ensures
    ok  => result == n,
{
    n
}

fn echoed(n: Positive) -> Positive {
    same(n)
}
```

`result - n` is zero, `n` is positive, so the result is. Before this, an `ensures` mentioning
an argument was decorative outside of tests, and most of them mention an argument.

What crosses a module boundary is those bounds, not the predicate. A refinement stays opaque
from outside, which is the rule modules already had, so an exported `fn one() -> Positive`
arrives as a pair of numbers and `fn same(n: Int) -> Int` arrives as a pair per argument. That
is the difference between exporting a proof and exporting the conclusion of one.

That last group is what made this worth building. Before it, `Proven` held constant
expressions and nothing else, so a refinement in real code was a runtime check with a
paragraph of ceremony around it, and the argument for having refinements at all is that they
replace checks rather than decorate them.

The branches are checked while their facts are still in scope, which is the reason the
checker pushes an expected type down through an `if` rather than inferring the whole thing
and comparing at the end. That is local bidirectional checking and it exists for exactly this.

### What `Proven` cannot decide

Every one of these is `Guarded`, with a warning, never a wrong answer.

- **Two names multiplied together.** `a < b * b` is not linear, and a pair of bounds has
  nowhere to put it. The same limit applies to a promise: `ensures ok => result == n * n`
  says something true and useful and there is nowhere to put it.
- **The payload of a call that can fail, until it is taken out.** The expression is the
  `Result` and the promise is about the number inside it, so the two meet at a `?`, at an
  `ok(..)` pattern, and where a `Result` is assigned into one with a refined success type,
  and nowhere else.
- **Anything that is not an integer.** No `String`, no record field, no variant.
- **Division and remainder.** The sign rules around zero and around the smallest integer are
  fiddly enough that getting them wrong is worse than not trying. It is also the only
  arithmetic left that can defeat a proof by having no answer, and the warning points at the
  operation and says so.

### What overflow does not cost

`n + 1` where `n` is `Positive` used to be `Guarded`, and this document used to argue that
refusing was the reasoning working. It was not.

Overflow is an error rather than a wrap, so `n + 1` either produces a value or stops the
program, and it never produces a wrong one. So a value that exists is inside `Int`, and if
`n > 0` then any sum that exists is greater than one. The interval clamps at the edge instead
of collapsing to "anything at all", and the runtime check that used to be emitted could never
have fired.

The same argument settles `high - low` under `low < high` alone, which used to be `Guarded`
for the same wrong reason.

It is also strictly more precise away from the edges. `n + 1` for an unbounded `n` used to
be "anything at all" and is now everything except the smallest integer, which is exactly the
set of values it can produce.

A solver would decide most of these and would be a hard dependency at check time, which P9
has a budget against. Whether that trade is right is an open question, not a settled one.

Generation discards inputs that violate a `where` clause rather than reporting them, since a
bad input makes the generator a bad caller and the runtime already says so. If too many get
discarded, that is reported: a property that only tested a handful of inputs is worse than no
property, because it looks like one.

A counterexample is shrunk before it is shown, and everything the generator can build
shrinks. Integers get a binary search toward zero, because greedy halving overshoots a
boundary and then crawls to it. Everything else is greedy: lists get shorter before their
elements get smaller, strings get shorter and then plainer, `true` gives way to `false`, the
payload of a `Result` shrinks without changing which outcome it is, and a variant gives way
to a sibling that carries no fields.

That last one is the only part that cannot be worked out from the value, since `One { n: 0 }`
says nothing about there being a `Nothing` next to it, so the choices are walked once and
every variant is told which of its siblings are empty. A variant whose siblings all carry
fields keeps its own name, because building one of those means inventing values for them,
which is generation rather than shrinking.

The rule the list has to keep is that it covers what the generator covers. A counterexample
built out of something nothing shrinks looks exactly like a small one that happens to be
awkward, and there is no way to tell them apart from the outside.

## Errors

Values, always. No exceptions, no panics in library code.

`Result`, `ok` and `err` are part of the language rather than a library. A language where you
cannot write a failing function without an import is not finished, and `?` cannot be checked
against a type the compiler does not know about.

```vow
fn parse_amount(input: String) -> Result<Money, ParseError> {
    let units = Int.parse(input)?
    ok(Money { units, currency: Currency.TRY })
}
```

`?` unwraps the success case and returns the failure one, so nothing after it runs when the
call fails. It is an error to use it on something that is not a `Result`, or inside a
function that does not return one.

A `Result` is taken apart by matching on it:

```vow
match small(n) {
    ok(value) => value,
    err(TooBig { limit }) => limit,
    err(Empty) => 0,
}
```

Both cases need an arm and a wildcard is rejected, exactly as for a `choice`, so a failure
cannot be swallowed by accident.

There is no `unwrap`. A language whose whole argument is that failure should be visible in
the signature has no business adding a way to ignore it.

Error types are `choice`s, matching is exhaustive, and adding a variant breaks every caller
that has to care, on purpose.

## Pattern matching

```vow
match result {
    ok(receipt) => log(receipt.id),
    err(InsufficientFunds { available }) => notify(available),
    err(AccountClosed { account }) => escalate(account),
    err(LimitExceeded) => retry_later(),
}
```

Exhaustive. No fallthrough, and no catch-all arm when the scrutinee is a `choice` or a
`Result`. A wildcard there would mean adding a variant stops being a compile error, which is
the entire value of having variants. Where the cases cannot be enumerated, such as matching
an `Int`, a wildcard is fine and necessary.

`ok(x)` and `err(e)` are the only patterns that carry a value positionally. Variants have
named fields, so they are matched as `Variant { field }`, and `Variant(x)` is an error
rather than a pattern that can never match.

## Non-termination is an effect

```vow
fn factorial(n: Int) -> Int
  uses
    Diverge,
{
    if n <= 1 {
        1
    } else {
        n * factorial(n - 1)
    }
}
```

A call cycle is the only way a function can fail to return. There is a loop now, and it does
not change that: a `for` walks a list that already exists, so it stops, and it declares
nothing. `Diverge` is a built-in effect with no operations, in the prelude next to `Io` and
for the same reason: a program that could declare its own would be a program that could opt
out of saying it might not finish. It goes in the row like anything else a function does, so
the tightness rule applies as well, and declaring it without recursing is `VOW5002`.

**There is no termination proving.** The example above obviously terminates and still has to
declare it. "A loop the compiler cannot show terminates" currently means "any call cycle at
all", because the compiler shows nothing. That is over-approximation in the direction of
noise, and it is written here rather than left for someone to discover.

That over-approximation is also why iteration exists. Walking a list is the most ordinary
thing a program does, and without `for` every walk is a call cycle, so almost every function
in a real program would carry `Diverge` and the row would stop telling anyone anything.

Mutual recursion counts, worked out from the module's call graph. A cycle that leaves the
module and comes back does not, because the graph is local, and reading another module's
bodies is exactly what a module boundary exists to avoid. What crosses is the declared row,
so a function that admits to `Diverge` still passes it to its callers wherever they are.

Declaring it does not make anything stop. The interpreter refuses to go past a fixed call
depth and reports `VOW6009`, because a runner that can be taken down by the program it is
running is a runner nobody can point at a file they have not read.

## Entry point

```vow
fn main(sys: System) -> Result<(), Error>
  uses sys.*
{
    let ledger = Ledger.connect(sys.net, DATABASE_URL)?
    payments.serve(ledger, sys.clock)
}
```

`main` is where all authority enters the program, and it is the only place. Everything else
receives what it needs as a parameter. See [04-capabilities.md](04-capabilities.md).

## Formatting

Not configurable. One canonical rendering, and `vow fmt` is not optional in CI.

The reason is not tidiness. Canonical form means a diff only contains semantic change, and
that is what makes review scale.

## Rules the parser had to pin down

Writing the parser forced decisions that this document had been leaving vague. They are
recorded here rather than living only in the implementation.

**Contract clauses have a fixed order:** `where`, then `uses`, then `ensures`. Writing them
in any other order is an error, not a style preference. A signature is the review surface,
so it should read the same way every time. This is P4 applied to the part of the language
where it matters most.

**Handlers name their effect with `implements`,** as in `handler InMemory implements Ledger`.

**Every parameter needs a type, except a handler operation's.** The exception is the one place
the signature is not where the type would be written: a handler implements an effect that
already declared the whole thing, so repeating it would be redundancy nothing checks.
Everywhere else, including a closure, leaving it out means nobody knows it.

This was a hole rather than a choice. An untyped parameter became the unknown type, unknown
agrees with everything, and a closure could carry any effect through one into a function that
declared none. P5 says nothing implicit crosses a boundary, and a parameter is the boundary.

Closure parameters were briefly exempt, on the grounds that a closure cannot leave the
function that wrote it, so its parameters are not a boundary anyone reviews. That is true and
it is a different claim from "may be unchecked". With no types the parameters were unknown,
so the closure's body was checked against nothing at all: `|x| { x + "not a number" }` was
accepted, and so was calling it with a string. Not being a review surface does not make a
body exempt from type checking. It is also no longer true, since a closure can now cross a
boundary, carrying whatever row the function type it crosses through allows.

Nothing can infer them. A `let f = |x| ..` has no expected type to push down, and Vow does
not do global inference on purpose.

**A function type is written `Fn(Int, Int) -> Int`,** and a row goes before the arrow:
`Fn(String) uses Log.note -> ()`. The return type is written out even when it is `()`,
because a function type with no arrow reads like an unfinished one. Leaving a row off means
the function performs nothing, and it cannot mean "any row": a value that carried an unstated
effect through a signature would undo the point of having rows.

The row goes before the arrow rather than after the return type because a declaration's own
contract also starts with `uses` and also follows a return type, so
`fn make() -> Fn(Int) -> Int uses Log.note` would have two readings and no way to tell them
apart. Before the arrow the `->` ends the list and nothing is in doubt.
`design/03-effects.md` has the rest.

**There are no float literals.** That is what makes `40.try` unambiguously `40`, `.`, `try`
with no lookahead. If floats ever arrive, they will need a rule for that, and it is a debt
worth naming now rather than discovering later.

**Statements need no separator.** A newline is enough and `;` is accepted but never
required. It used to be the case that this worked only because no statement could begin
with a token that continues the previous expression, which was never true: `(` starts a
parenthesised expression and continues a call, and `-` starts a negation and continues a
sum. So `let a = 1` with `-2` under it read as `let a = 1 - 2` and the second line was
gone, with nothing said about it.

The rule now is that **a line break ends an expression**. A binary operator, or a postfix
`.`, `(`, `?` or `{`, continues what came before it only if it is on the same line. The rule
is the same inside brackets as outside them, because a rule that switches off somewhere is a
rule people have to remember. Nothing anyone writes changes shape under it: `vow fmt` never
breaks a binary expression across lines and never puts a call's `(` or a literal's `{` on a
line of its own, so an argument list or a record can still spread over as many lines as it
likes, the opening bracket just has to stay with what it opens.

**Type arguments close with single `>` tokens.** There is no shift operator in Vow, so
`Map<K, Vec<V>>` needs none of the special handling that costs other languages a real
amount of parser complexity.

**A brace after an expression is a struct literal, except in a condition.** `Point { x: 1 }`
is a literal, and the brace in `if a < b { ... }` starts a block. This is the standard rule
and it is the one genuinely ambiguous corner of the grammar. The `with` handler list needs a
third case, described below.

## Rules name resolution had to pin down

**There is no shadowing.** Binding a name that is already bound is an error, not a style
warning. Binding a name that hides a module level declaration is a warning.

This is the most aggressive rule in the language and it is on purpose. P1 says you should be
able to point at a name in a function and know what it refers to. Shadowing means the answer
depends on which line you are looking at, which is exactly the property P1 exists to remove.
The cost is real: `let x = parse(x)?` needs a second name. That is the trade.

**In a pattern, capitalisation decides.** An uppercase initial names a variant, a lowercase
initial introduces a binding.

Without this rule a mistyped variant silently becomes a binding that matches everything, and
nothing in the compiler would ever mention it. It is the only place the language attaches
meaning to how a name is written, and it buys an entire class of silent bug.

**`value` is in scope inside a refinement and nowhere else.** `type Positive = Int where
value > 0` has nothing else to talk about. Along with `result` in an `ensures` clause, it is
one of only two names the language introduces implicitly, and both exist because the thing
they name has no other way to be written down.

**The prelude is nineteen names and two effects:** `Int`, `String`, `Bool`, `Result`, `ok`,
`err`, `length`, `List`, `at`, `push`, `split`, `join`, `trim`, `to_string`, `to_int`, `System`,
`Console`, `Clock`, `Dir`, and the effects `Io`, with its `write`, `now`, `open`, `read`,
`save`, `list` and `args` operations, and `Diverge`. Everything else is imported. Each prelude entry is a name that
cannot be looked up in any file, which is the kind of thing P2 is a budget for, so the list
is short on purpose. The four capability types are there because a capability that could be
imported would not be a capability.
**A module is named by its own `module` line, not by where it sits on disk.** `use
payments/ledger` is answered by looking for the file that says `module payments/ledger`
among the files handed to the compiler. The unit of compilation is that set of files, so
`vow check src/` sees the whole thing and `vow check one.vow` sees one module with an empty
universe, in which any `use` fails. That is a real cost and it buys not having a second set
of rules about roots, extensions and case sensitivity, which is the part of every module
system that goes wrong.

**Imports are checked at the level of names.** A `use` of a module that is not there is
`VOW3007`, a `use` of a name that module does not declare is `VOW3008`, and an operation an
imported effect does not have is `VOW3006`. All three used to be accepted in silence.

**Every item is exported, and a choice's variants are exported in their own right.** There
is no visibility modifier, because a language with no wildcard imports already makes the
reader of a file see every name it pulled in, and `pub` on top of that is a second and
weaker version of the same guarantee. Variants are separate names rather than arriving with
their choice, since a `use` that quietly brought in six more names would be exactly the
wildcard import that does not exist. A `test` is not exported: it is not part of what a
module offers.

**What crosses an import is a name and the type behind it.** An imported record's fields, an
imported function's signature and an imported choice's variants are all checked, and a
`match` on a choice from another module has to be exhaustive over that module's variants.
Identity for those types is the module path and the name together, rather than an index into
one module's table, so nothing about how one module was resolved leaks into another.

**Running crosses it too, and identity has to hold at runtime as well.** A variant value
carries the module that declared it, so a `Loud` built where it was declared and a `Loud`
named through an import are one value, and two modules that each declare a `Loud` are two.
An effect is identified the same way, since a `DefId` comparison would have made a handler
in one module answer an operation in another whenever the numbers happened to line up.

**An effect's operations and a choice's variants cross as declarations, not as names.** They
are part of what an `effect` or a `choice` says, so `Ledger.post` where `Ledger` was imported
gets a definition of its own and a row naming it is checked in both directions, the same as
a local one. A handler carries the one effect it implements, so a `with` block naming an
imported handler discharges that effect and no more.

**A refinement crossing a module boundary becomes opaque.** `type Positive = Int where value
> 0` exported and then imported is a distinct type that an `Int` does not fit, and the
predicate does not come with it, so nothing is proven on the far side. Carrying the
predicate means carrying the expression it is written in, which means carrying that module's
scope, and that is a much larger thing.

## Rules type checking had to pin down

**Checking is local, and there is no global inference.** Every function annotates its
parameters and its return type. Inside a body, `let` still infers from its initialiser.

Hindley-Milner would remove some of those annotations and cost a great deal of language to
do it, which P2 has a budget against. It would also move errors away from the expression
that caused them and towards wherever unification happened to notice, which is the opposite
of what P7 is for. And P1 wants a complete signature anyway, so most of what inference would
save is something the language asks for regardless.

**A type alias with a predicate is a distinct type. One without a predicate is transparent.**
An alias that adds no information should not add a distinction either. An alias that carries
a proof obligation has to be distinct, or the obligation attaches to nothing.

**Unknown agrees with everything.** A type that came from a module the compiler has not
loaded, or from an expression that already produced a diagnostic, is unknown, and an unknown
type is compatible with every other type in both directions.

That sounds like a hole and it is a deliberate one. While most of a realistic program still
comes from modules that cannot be loaded, a checker that guesses produces false positives,
and a false positive is more expensive than a missing check. The number of real checks grows
as the language lands. The number of wrong ones stays at zero.

The same rule applies to operators: if either side of `+` is unknown, nothing is said about
the other side either, because there is no basis for it.

**In a file that checks cleanly, no expression is unknown.** This is the other half of the
rule above, and it is what keeps the deliberate hole from becoming an accidental one. An
expression that ends up unknown is an expression nothing done with it was checked against, so
if the file also has no errors then the compiler only pretended to read that part of it.

Five holes have had exactly that shape. A type name in expression position. A function
parameter written without a type. A closure parameter, for the same reason. A handler
operation's parameters, which the effect declared and the handler never received. A call to an
imported effect's operation, whose signature was in the surface and never looked up. Each was
found by accident, one at a time, usually while doing something else.

The invariant is a test now, over every construct the language has and every example, so the
next one is found on purpose.

**Ordering is `Int` and `String`.** See the operator table above. It used to insist only that
both sides had the same type, which meant comparing two records passed and failed at runtime.

**`?` is checked.** The operand must be a `Result`, the enclosing function must return one,
and the error types must line up.

**`ok(x)` and `err(e)` each say nothing about the other side.** `ok(x)` has type
`Result<T, _>` and the unknown half agrees with whatever the expected error type turns out to
be. That is what makes them work with no unification anywhere, and it is the whole reason the
unknown type absorbs rather than unifies. `[]` is the same trick with one hole instead of
two.

**A refinement inside a list is discharged element by element.** `fn f() -> List<Positive>`
with a literal body puts an obligation on each element, because the list itself has no range
and nothing to check. A list that came back from a call has nothing naming its elements, so
that is a type mismatch rather than a guard: there is no expression to attach the check to.

**Importing or declaring a prelude name is a warning.** The list is above, under the naming
rules. Silently shadowing a builtin would put everything that depends on it quietly back to
being unchecked.

**A type name is not a value.** Writing `Console` where an expression is expected is
`VOW4019` rather than something with no type. This looks like a footnote and is not: an
expression with no type agrees with everything, so `Io.write(Console, "hi")` would have
checked, and a program could have conjured authority by spelling it.

## Mutation

There is exactly one mutable thing in Vow, and it is a `state` field of a handler.

```vow
handler InMemory implements Counter {
    state count: Int

    fn value() -> Int { count }
    fn bump(by: Positive) -> () { count = count + by }
}
```

Assignment is a statement, the target must be state of the enclosing handler, and nothing
else is assignable. Not a parameter, not a `let` binding, not a field of a record.

`state` is not a reserved word. It is read as a name at the head of a handler member,
where the only other thing a member can start with is `fn`, so nothing has to be worked
out and nothing is ambiguous. Everywhere else it is an ordinary identifier, which matters
because the accumulator of a fold is one of the most natural things to call `state`.

This is a rule rather than a limitation. Mutation exists exactly where effects are
implemented and nowhere else, which is what lets an empty effect row mean that a function
cannot observe or cause a change to anything. Without the restriction, purity would be a
claim about IO only, and it is supposed to be a claim about everything.

It also keeps P1 intact. A mutable local would mean a name's value depends on where you are
in the function, which is the same objection that made shadowing an error.

The cost is that accumulator loops have to be written some other way, and the other way is a
fold. `for n in numbers with sum = 0 { ... }` binds `sum` again on every turn rather than
assigning to it, so iteration exists and this rule is untouched. `examples/todo.vow` is where
that bill arrived: it threaded an accumulator through a recursive parameter because there was
nowhere else to put it, and reaching for a handler to collect four strings would have been
using an effect to get around not having a loop.

## Open questions

- Property tests only cover pure functions. Running an effectful one needs a handler, and
  inventing a handler means inventing the behaviour the property would then check against
  itself. A module declaring exactly one handler per effect might be a defensible default.
- Refinements have no conversion form. Real code will need `Positive.try(n)` or something
  like it, returning a `Result`, rather than relying on the runtime check a `Guarded`
  obligation leaves behind.
- Banning shadowing outright may turn out to be more annoying than it is worth. It is easy
  to relax later and hard to tighten, which is the only reason it starts strict.
- Capitalisation carrying meaning in patterns is a wart, even though it earns its place.
  Explicit variant syntax would avoid it and would cost more to write everywhere else.
- The `with` handler list is disambiguated by a lookahead hack: a brace opens a struct
  literal when followed by `name:`, which is what separates `with H { a: 1 }, Other` from the
  block that follows the handler list, or when it is `{ }` with a block behind it, which is
  the only way a record with no fields can be written there. The second half was added after
  the first turned out not to cover `with H { } { .. }` or a `for` whose accumulator starts
  as an empty record: a literal with no fields has no name to look at. It works for
  everything written so far and it is still not a rule anyone should have to know. Better
  ideas welcome.
- Whether a line break ending an expression should be relaxed inside brackets. It is uniform
  today, which is the version nobody has to remember, and it costs an expression that wants
  to run over several lines a pair of parentheses it would not otherwise need. Nothing
  written so far wants that, and the moment something does the answer is probably to let a
  line break inside an unclosed bracket keep going rather than to special case operators.
- Whether a `for` should be able to stop early. There is no `break`, so a search over a list
  walks all of it, and a fold cannot say it has seen enough. The honest version is probably a
  `for` whose accumulator is a `Result`, which stops meaning "done" and starts meaning
  "failed", so it needs its own answer rather than a reused one.
- Generic types on an alias. A `record` and a `choice` may carry type parameters and a `type`
  may not, because an alias with no predicate is expanded away and one with a predicate is a
  refinement, and a generic refinement is a different question. Higher-kinded types are
  almost certainly out.
- Whether `Result` and `List` should stop being built in now that they could be declared.
  `Option` already is. What holds `List` in place is `[1, 2, 3]`, which is a literal with
  syntax of its own, so moving it out means deciding what that literal builds.
- An index into a list has nowhere to say it is in range, and most of that is now fixed. A
  length is a term the prover can hold, so `index < length(items)` is a relation like any
  other, and a `where` clause saying it is read at the call site, so a caller that checked the
  length proves it and a caller passing something plainly out of range is refused. What is
  still missing is a way to spend the proof: `at` returns a `Result` whatever is known about
  its index, so the caller that proved the bound still writes a `match` for a failure that
  cannot happen. A total indexing form is a precondition on a prelude function, which is a
  thing the language can now express, and whether the prelude should carry one is the
  question.
- Position is not something a callback can be handed by a library that does not already have
  it. A `for` says where it is now, so `examples/list.vow` can write `map_at` and anything
  else it wants out of that. What is still unanswered is whether an indexed form of every
  walk belongs in that library at all, since one of each doubles it and nobody has wanted
  more than `map_at` yet.
- Whether traits can be implemented outside the defining module, and what that does to
  local reasoning.
- Whether `uses sys.*` is a hole big enough to make `main` useless as a boundary.
- Concrete syntax for effect handlers, currently only sketched in 03.
