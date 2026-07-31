<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/deed-cipher-lockup-dark.svg">
    <img alt="Deed" src="assets/brand/deed-cipher-lockup.svg" width="380">
  </picture>
</p>

A contract-first language where a function signature is a promise the compiler checks.
Built for code that machines write and humans review.

> **Status: it runs.** The compiler lexes, parses, resolves names, type checks and checks
> effect rows, and a tree walking interpreter executes `test` blocks and `main` with
> contracts enforced at runtime. Programs get their authority from a `System` capability
> handed to `main`, and a `Dir` narrows to a subdirectory and cannot be walked back out of.
> There is no code generation, and what a run costs without it has been measured rather
> than guessed at: `design/01-principles.md` has the table and the reason a compiler is not
> the next thing. Criticism of the design is still the most useful
> contribution. See [issue #228](https://github.com/deed-lang/deed/issues/228) for where
> this is going.

## Getting it

Every release carries a `deed` binary for Linux, macOS and Windows. Take the one for your
machine from [releases](https://github.com/deed-lang/deed/releases), unpack it, and run it.
There is nothing else to install: the compiler, the formatter, the test runner and the
language server are all in that one file.

Start with the [tutorial](TUTORIAL.md) if you want one program built up one step at a time.

```
$ tar xzf deed-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
$ ./deed-v0.1.0-x86_64-unknown-linux-gnu/deed --version
deed 0.1.0
```

From source instead, which needs Rust 1.85 or newer:

```
$ cargo install --path crates/deed-cli
```

**What the version promises.** `0.x` says the language can still move, so a file that
compiles against one minor release may not compile against the next. Breaking moves are
called out in release notes, and `deed fix` carries the mechanical part when possible. The
full policy, including diagnostics and `std/`, is in
[design/07-versioning.md](design/07-versioning.md).

```
$ deed run examples/config.deed --dir examples
found it
`..` would leave the directory, and there is no way out of a `Dir`
`../Cargo.toml` is not a single name, and a `Dir` only takes one at a time
`/etc/passwd` is not a single name, and a `Dir` only takes one at a time
`nowhere` is not there
used the fallback
```

```
$ deed test examples/
examples/counter.deed
  ok    bumping twice adds twice
  ...
examples/transfer.deed
  ok    moves the money and conserves the total
  ok    refuses to overdraw and leaves the ledger alone
  ok    refuses a currency mismatch and leaves the ledger alone

127 passed, 0 failed
```

That number used to be 102. Seven of those tests are in `std/table` now, because the table
library stopped living under `examples/` and a module that ships with the compiler is context
rather than subject, so the corpus does not run its tests any more. Nothing was deleted: the
modules that ship carry sixty-four tests of their own, and
`crates/deed-driver/tests/shipped.rs` runs every one of them on every commit.

```
$ deed check examples/transfer.deed --obligations
obligations: 7 proven, 0 tested, 6 guarded
  guarded  examples/transfer.deed:94:5  transfer ensures ok
  ...
  proven   examples/transfer.deed:202:76  Positive
```

For an editor, see [`editors`](editors). VS Code gets syntax highlighting from three JSON
files and no code. Helix and Neovim get the language server from a few lines of
configuration and no plugin. Neither gets both yet, and the table there says why.

## How-to guides

If you already know the language and want the shortest path to a specific task,
start with [how-to/README.md](how-to/README.md). Those pages index the existing
corpus: checking a module set, writing tests, contracts and `assert refuses`,
effects and capabilities, formatting, backend compilation, and editor setup.

The shipped library API is indexed separately in [docs/std.md](docs/std.md), with one
page per module and every exported function held there by a documentation ratchet.

There is no REPL. A Deed input is a module of declarations, and `deed run` enters through
`main` rather than evaluating a top-level expression. For quick experiments, use a scratch
`.deed` file with `deed check`, `deed test` or `deed run`. The right browser answer is a
playground that edits and runs a whole program in that same shape.

There is also a small [conformance suite](conformance/README.md) for checks that should hold
across implementations.

## The idea

Most of the code being written today is not typed out by a person. That changes which
costs matter. Producing a line of code is close to free. Reading it, trusting it, and
finding out three weeks later that it was subtly wrong are not.

Every language we use was shaped by the opposite assumption. They optimize for the
comfort of someone typing character by character: implicit behaviour, clever shorthand,
five ways to express the same thing. All of that is now working against us.

Deed makes a different trade. Signatures carry the entire contract, bodies are checked
against it, and nothing in the language lets a function reach outside what its signature
admits to.

```deed
fn transfer(from: AccountId, to: AccountId, amount: Amount)
    -> Result<Receipt, TransferError>
  where
    from != to,
  uses
    Ledger.balance,
    Ledger.post,
    Audit.append,
  ensures
    ok  => result.from == from,
    ok  => result.amount == amount,
    ok  => Ledger.balance(from).units == old(Ledger.balance(from).units) - amount.units,
    ok  => Ledger.balance(to).units == old(Ledger.balance(to).units) + amount.units,
    ok  => Ledger.total() == old(Ledger.total()),
    err => unchanged(Ledger),
{
    ...
}
```

This is not an illustration. It is the top of [examples/transfer.deed](examples/transfer.deed),
which the compiler lexes, parses, resolves, type checks and effect checks on every commit.

Three things follow from that block, and they are the whole pitch:

**You review the signature, not the body.** It fits on one screen and it is complete.
Whoever wrote the body had nothing else to satisfy.

**The body needs no other context.** No global state, no implicit conversions, no
inheritance, no exceptions thrown from four frames down. Everything that can affect this
function is written above the brace.

**The function cannot reach the network.** Not by accident and not on purpose. It did not
ask for that capability in `uses`, so it does not have it, and it holds no value that would
let it name one. Sandboxing stops being a container and becomes a type.

## Why this is a language and not a library

A library can offer contracts. It cannot stop you from ignoring them, it cannot see the
whole call graph, and it cannot make the absence of an effect mean anything. Contracts,
effects and capabilities are only useful if they are total, and total means the compiler
enforces them.

The longer version, including the cost model this is all based on, is in
[design/00-motivation.md](design/00-motivation.md).

## Design documents

| Document | What it covers |
| --- | --- |
| [00-motivation.md](design/00-motivation.md) | The cost model, and why a library cannot do this |
| [01-principles.md](design/01-principles.md) | The constraints we are willing to be held to |
| [02-syntax.md](design/02-syntax.md) | Contract blocks, types, errors, modules |
| [03-effects.md](design/03-effects.md) | Effect declarations, propagation, handlers |
| [04-capabilities.md](design/04-capabilities.md) | Authority, how it enters a program, and why |
| [05-backend.md](design/05-backend.md) | Compiling to WebAssembly, and what does not change |
| [06-grammar.md](design/06-grammar.md) | The grammar, checked against the parser both ways |
| [07-checker-semantics.md](design/07-checker-semantics.md) | The checker rules the implementation enforces |
| [07-editions.md](design/07-editions.md) | How a breaking change ships without splitting the ecosystem |
| [characters.md](design/characters.md) | Why characters remain one-scalar strings |
| [fractional-values.md](design/fractional-values.md) | The decision on fractional values and money |
| [refusals.md](design/refusals.md) | Everything deliberately left out, and why |
| [07-versioning.md](design/07-versioning.md) | Compatibility policy for language, diagnostics, and `std/` |

Read them in order. Each one leans on the one before it.

## What is built

| Crate | Does |
| --- | --- |
| `deed-diagnostics` | Spans, source maps, and structured diagnostics with machine-applicable fixes |
| `deed-lexer` | Source text to tokens |
| `deed-ast` | The syntax tree |
| `deed-parser` | Tokens to a tree, with recovery |
| `deed-resolve` | Every name bound to a declaration, including across module boundaries |
| `deed-typeck` | Every expression given a type, including types from other modules |
| `deed-effects` | Every effect row checked against what the body does |
| `deed-interp` | Runs `test` blocks, property tests and `main`, with contracts enforced |
| `deed-fmt` | The one canonical form, with no options for the output |
| `deed-lsp` | A language server: diagnostics, hover, go to definition, references, rename, completion, signature help, quick fixes, an outline, workspace search and formatting |
| `deed-driver` | Runs all of the above, in one place, so nothing drifts |
| `deed-cli` | The `deed` binary: `check`, `test`, `run`, `fmt`, `fix` and `lsp` |

`deed lsp` is a language server, and most of it is plumbing over things that already existed:
the compiler produces structured diagnostics with spans, `Types::type_of` can say what an
expression turned out to be, `Resolutions` can say where a name was declared, and the
formatter has one canonical answer with no options. It publishes diagnostics as you type,
says the type of whatever is under the cursor, jumps to a declaration in whichever file
declares it, lists every use of a name across the workspace, renames one everywhere it is
written, offers what could be written where the cursor is, offers the patch a diagnostic is
already carrying, says what the file declares, and formats a file.

Signature help is the one that is most this language's own. Everywhere else the contract is
on the screen already, because it is written on the declaration. At a call site it is not:
`transfer(a, b, amount)` says nothing about what the call performs or what it needs to be
true first, and that is exactly where somebody is deciding whether they can afford it. So the
label carries the row as well as the types, and `shout(Console, String) uses Io.write -> ()`
is the whole signature rather than the part that fits an editor's idea of one.

It is read off the text rather than the tree, for the same reason completion is. `add(` with
nothing after it does not parse, and that is the moment the question gets asked. Nesting is a
stack rather than a counter, so the comma in `add(add(1, 2), 3)` belongs to the inner call
and the ones in `total([1, 2, 3], 4)` belong to the list.

That last one was data with nowhere to go. A diagnostic carries the edit that resolves it
where the repair is unambiguous, and the only thing that could reach one was `deed fix` on a
command line, which applies the certain ones and skips the guesses. So the guesses were
reachable by nothing at all, and "cannot find `lenght`, did you mean `length`" knew the answer
and made the reader type it. They are quick fixes now, and `Applicability` carries straight
over: a machine-applicable fix is the one `deed fix` would apply without being asked, so it is
the preferred action, and a guess is offered and never preferred. The editor already had a
word for the distinction and the compiler already had the distinction.

Completion is the only one that is a different shape. Everything else answers a question
about a name that already exists, and a document does not parse while somebody is typing into
it, so it answers a narrower question instead: what is in scope here. After a `.` that is the
fields of whatever is to the left; inside `use a/b.{ }` it is what that module exports, which
is the one place a name has to match another file exactly; anywhere else it is what the file
declared, what it imported, and the prelude. Nothing it offers inserts more than a name. No
snippets, no argument placeholders, no auto-import, because each of those is a decision about
what somebody meant.

The outline is the one answer that does not go through a check. Everything else this server
says is about a name that resolved; an outline is about what is written, and half a
declaration is still worth drawing while somebody is typing the rest of it, so it reads the
parse tree of the one file and nothing else. What belongs to a declaration is nested under it,
because a variant belongs to its choice and a handler operation belongs to its handler and a
list that said otherwise would be a worse version of scrolling. The protocol has fewer kinds
than this language has declarations, so a `type` and an `effect` share one: both are a named
thing declared by its shape, and picking a different icon for one of them would be picking it
for how it looks rather than for what it is.

Rename and find references are the same walk with two answers, which is the point of them
sharing one. A rename that edited the declaration and left the `use` line that brought the
name into another file would fix one file and break another, and that is worse than not
having rename at all. What it deliberately does not decide is whether the new name is a good
one: a collision with something already in scope is something the checker has a diagnostic
for, and answering it twice is how the two answers come to disagree. It does refuse a name
the language cannot hold, and it asks the lexer rather than holding a second definition of
what an identifier is.

It checks a document together with every other `.deed` file in the folders the editor said it
has open, and an open file's text comes from the buffer rather than from disk, so removing an
export in one file puts a squiggle on the import in another before either is saved. The set
of files is the workspace rather than a guess: `initialize` carries it, and taking what the
editor says is the same answer `deed check src/` gives when a person says which directory they
mean. An editor that names no folder gets the single file behaviour. That was the only
behaviour until recently, and it meant every file with a `use` in it had a red line under the
import, which is exactly the failure a server is not allowed to have.

It rechecks the whole workspace on every keystroke and on every hover. That used to be
followed by "and nothing has measured it", which is now not true:
`cargo run -p deed-driver --example edit_loop --release` says the recheck is linear at about
70 microseconds per file, so 512 files cost about 38ms per keystroke, inside P9's 100ms
budget, and a few thousand files is where it leaves. It also says 99% of that work is spent
on files that did not change, which is what a cache would take off. So the cache is worth
writing when the size arrives and is not worth writing now, and that is a conclusion with a
number behind it rather than a feeling. `design/01-principles.md` has the table.

It has no dependencies either. The protocol is a `Content-Length` header, a blank line and a
handful of object shapes, so the JSON reader and the framing are written out. Two parts are
worth reading. Positions: the protocol counts UTF-16 code units and the compiler counts
bytes, which agree for ASCII and stop agreeing the moment somebody writes a comment in
Turkish. And URIs: a space arrives as `%20`, a Windows drive as `/c%3A/`, and a Turkish
letter as two escapes that are bytes rather than characters. Getting either wrong is silent.

The examples are [transfer.deed](examples/transfer.deed),
[counter.deed](examples/counter.deed), [hello.deed](examples/hello.deed),
[config.deed](examples/config.deed), [todo.deed](examples/todo.deed),
[journal.deed](examples/journal.deed), [logs.deed](examples/logs.deed),
[proven.deed](examples/proven.deed),
[closures.deed](examples/closures.deed), [diverge.deed](examples/diverge.deed),
[strings.deed](examples/strings.deed), [lists.deed](examples/lists.deed),
[generics.deed](examples/generics.deed),
[generic_types.deed](examples/generic_types.deed),
[using_list.deed](examples/using_list.deed), [calculator.deed](examples/calculator.deed),
[calendar.deed](examples/calendar.deed), [ranking.deed](examples/ranking.deed),
[kv_store.deed](examples/kv_store.deed), [json.deed](examples/json.deed),
[stack_machine.deed](examples/stack_machine.deed), [tic_tac_toe.deed](examples/tic_tac_toe.deed),
[markdown.deed](examples/markdown.deed), [tree.deed](examples/tree.deed), and the three that see
each other: [names.deed](examples/names.deed), [sink.deed](examples/sink.deed) and
[greeting.deed](examples/greeting.deed). All are checked by every pass on every commit,
`hello.deed`, `config.deed`, `todo.deed`, `journal.deed` and `logs.deed` have a `main`, and
the rest run their own tests.

`todo.deed` is the one written to find out what is missing rather than to show what is there.
It reads a list of tasks out of a directory it was handed, adds one or marks one done if it
was given anything on the command line, counts them, and prints the ones that are not done.
That is the smallest thing anybody would call a program and it was not writable at all a week
ago. It found four things. Three of its functions were the same function: start at zero, look
at each element, stop when the index runs out, and declare `Diverge` for the privilege. An
accumulator had to be threaded through as a parameter, because handler state is the only
mutable thing in the language and reaching for a handler to collect strings would be using an
effect to avoid a loop. Those two are what decided what a loop looks like here, and the three
walks are `for` loops now with nothing declared. The file format was `x|title` rather than
`[x] title` because splitting was all there was, so the data got bent to fit the tool. And
the first run printed its own output backwards over itself, because splitting on `\n` leaves
a carriage return on every line of a file written on Windows and there was nothing that
trimmed one off. The last two are what `trim` is for, and the file format is `[x] title` now.
None of those were obvious from inside the compiler.

Marking a task done is the thing that file said for months it could not do, and it turned out
to be one `map` over the tasks with one of them replaced. What took the time was being able
to write that `map` once: generic functions, generic types and a row variable all had to
exist first, and once they did the feature was four lines. It found one more thing on the way
out, a small one. `state` was a keyword, so `with state = ..` did not parse, and `state`
means something in exactly one position where the only alternative is `fn`. Reserving a word
that common for one position is a cost nobody had paid until a program wanted the word, so it
is a name again and the parser recognises it where it matters.

The last thing on its list was two tasks with the same title, where `done buy milk` finishes
both because a title is being used as a name and it is not one. `done 2` finishes the second
task now, and no part of the language changed to allow it, which is what makes it worth
reporting. A position is a fact about where an element is and a callback is only ever handed
the element, so `map` and `filter` stop being enough the moment position matters and the walk
goes back to a counter in a record written out by hand. There were three of those in that file
when this was written, one of them replacing a `map(filter(..))` that used to be a single
line. `for task at here in tasks` came out of exactly that and took all three back to one
line each.

And nothing bounds the number. `done 9` on a file with two tasks walks both, marks neither
and hands back the list it started with, so whether it meant anything has to be asked again
by a second function comparing it against the length. That is the shape an index has when the
type system cannot hold "in range", and it is the argument for the refinement tier made by a
program rather than by a design document. What to do about it is the next question rather
than this one.

`for` is a fold with syntax rather than a loop with a variable in it:

```deed
let total = for n in numbers with sum = 0 {
    sum + n
}
```

The block's value is the accumulator for the next turn, and `sum` is a fresh binding every
time rather than something assigned to, so iteration exists and a handler's `state` is still
the only mutable thing in the language. The second reason for that shape is `Diverge`: there
is no termination proving, so a function that can reach itself has to declare it may not
return, and without a loop every walk over a list is recursion. A row that almost every
function carries the same entry in has stopped saying anything. A `for` walks a list that is
already there, so it stops, so it declares nothing.

It also says where in the list it is, which came out of `todo.deed` for the third time.
`for task at here in tasks with kept = []` binds the position, zero-based like the `at` that
indexes a list, and it is known to be a real one: not negative, and below the length of what
is being walked. Before it, three walks in that file carried a counter in a record so that
something could ride alongside the answer, and every branch had to remember to bump it. The
reason this is in the language rather than in the library is that the library cannot have it
otherwise: everything in `std/list` is written with a `for`, so `map` cannot hand a callback
something the walk never knew. With it, `map_at` is four lines of Deed and no part of the
language grew a second `map`. `at` is still an ordinary name, since the only thing that can
follow a `for` binder is `at` or `in`.

`journal.deed` is the half `todo.deed` could not write. `Io` could read a file and open a
directory and there was no operation that wrote one, so `Dir` was a read capability wearing a
more general name. `Io.save` is that operation, and the thing worth looking at is what stops
it writing anywhere else. Two separate things do. The row: a function that does not declare
`uses Io.save` cannot write, whatever it is holding, so reading and writing are different
authorities over the same capability and which one a caller is handing over is written in the
signature. The capability: `Io.save` takes the `Dir` it writes into and there is no way to
construct one, and the name goes through the same check reading goes through, so `..`, an
absolute path, a separator and a symlink pointing out are refused for writing exactly as they
are for reading. Not a second implementation that agrees today.

`Io.remove` is the case where a second kind of `Dir` was most tempting. Reading, listing and
writing all leave what was there; deleting does not, and a program that writes the wrong
bytes can be put back from what it overwrote while one that deletes the wrong file cannot be
put back from anything. It got the same answer as writing did, and for the same reason:
`uses Io.remove` cannot be reached from `uses Io.save`, holding a `Dir` says nothing about
which of the four operations a function may do, and nothing new had to be built for it. That
is three tests of the same claim now, and the third was the one meant to break it. `todo.deed`
can say `clear`, and the function that does the deleting declares `uses Io.remove` and
nothing else, so it cannot read the file it is about to delete.

`Io.make` is the fourth test and the one that looks like it breaks the rule that authority
only ever shrinks. It hands back a `Dir`, and a `Dir` is authority, so this document said for
a while that creating a directory was authority being made rather than narrowed. That was
wrong. A `Dir` reaches everything under its root and which of those paths happen to exist is
not part of what it grants, which is why `Io.save` writing a file that was not there is not
authority creation either. What `Io.make` returns is rooted inside what it was given, so it
reaches strictly less, and there is a test that makes a directory and then fails to climb out
of the result. Nothing may already be at the name: "I made it" and "it was already there" are
different answers.

`Io.epoch` is the same split about something other than authority. `Io.now` counts calls
rather than reading a wall clock, because P8 says the default is deterministic and a real
clock would make every run different, which is the right answer for a test and the wrong one
for a program that needs the actual time. So the machine's clock is a second entry in the
row on the same `Clock`. `uses Io.now` cannot reach it and it cannot reach `uses Io.now`,
and what that buys is not narrower authority: a signature saying `uses Io.epoch` is a
function whose output can change between two runs of the same program, and that is now
something you can see without reading the body.

`proven.deed` is the one that argues with itself. Every function in it either proves its
postcondition or explains, in a comment, why the checker cannot, and the file is written so
that the two halves sit next to each other. The `Proven` tier used to hold constant
expressions and nothing else, which made a refinement in real code a runtime check with
ceremony around it. It now reasons about intervals, about the difference between two names,
and about what a callee promised: a `where` clause, a refined parameter type, an `if`
condition, a guard that returns, `low < high`, and `ensures ok => result == n` at a call site
are all facts the rest of the body can use.

How long something is is one of them now. `length(items)` used to come back as a range and
nothing else, which meant it could not be one side of a difference, so `index < length(items)`
was invisible to the machinery that existed for `low < high`. A length is a term keyed on the
thing being measured, and the rest falls out of the two rules that were already there: the
default is zero and up, because there is no list with fewer than no things in it, and past
`if length(items) <= 0 { return err(..) }` one less than the length is an index that is really
there. Two lists are two terms, and `length(f(items))` is not a term at all, since two calls
could hand back two different lists.

That was prompted by `todo.deed`, which marks a task by position now and could not say no to
`done 9` before running. What made the fact worth having is the other half of the same
change: a `where` clause is read at the call site now. It was not before. A precondition was
a fact for the callee's body and a check inside the callee at runtime, and nothing ever
looked at it from where the call was written, so `halve(0 - 5)` against `where n >= 0` passed
the checker in silence. The design doc had described the call-site check for months. Now a
call that plainly breaks a clause is an error where the call is, a caller that can show the
clause holds is `Proven`, and a caller that cannot is `Guarded` with the runtime check still
standing. `proven.deed` went from four proven obligations to forty-five that way, and most
of them are calls rather than values.

What crosses into a clause is the caller's facts said in the callee's parameter names: each
argument's range, how long it is, and the differences between them. That last part is what
settles `index < length(items)`, which is the clause the whole detour was about. The clause
itself crosses a module boundary whole, unlike a refinement predicate, because a refinement
is a proof the declaring module already did and a precondition is a question only the caller
can answer. What travels with it is what each name in it refers to, worked out where it was
written, since a clause's spans are offsets into that file and nowhere else.

A refinement could say the same thing about a length and could not be settled, which is the
kind of gap you only find by writing the two side by side. `value` has no declaration of its
own, so it travelled as a range, and a range answers `value > 0` and cannot answer
`length(value) > 0`: that is a question about a term and a term needs something to be keyed
by. So `if length(s) > 0` proved a `where` clause saying `length(s) > 0` and left a refinement
saying the same thing Guarded, over the same fact, two lines apart. A value being checked now
carries what is known about it three ways, the range it lands in, how long it is, and the name
it was given when it has one. The name is the strongest of the three, since it is the entry
the body has been narrowing all along. The length covers the case with no name to give, so
`""` where a non-empty string is wanted is refused outright instead of being left to a check
at runtime, and a string written on the spot proves the refinement it is being passed into.

The same thing was missing from both of the other directions, which is what a gap like that
usually turns out to mean. A parameter already of a refined type is a fact without a `where`
clause repeating it in prose, and that held for what the value is worth and not for how long
it is, so a `NonEmptyList` knew nothing about its own length inside the body that declared it.
And narrowing into a refinement asked for the base type exactly rather than asking the
question the base type would have been asked, so `first_of([])` came back as "expected
`NonEmptyList`, found `List<_>`". The empty list fits a `List<Int>` perfectly well. What is
wrong with it is the predicate, and that is what it says now.

That also answered a question the design doc had listed as open. A refinement has no
conversion form, and it turns out not to need one: `try_positive(n)` returning a `Result` is
what `Positive.try(n)` would have been, it is ordinary code, and the `ok` is Proven because
the guard above it says what the predicate wants. What a built-in form would buy is the
predicate living in one place rather than being restated at every converter, and a
restatement weaker than the predicate is the mistake it would rule out. That one is still
accepted today with a runtime check rather than refused, which is honest and is not the same
thing as catching it.

It still does not finish the job. `at` returns a `Result` whatever is known about its index,
so a caller that proved the bound still writes a `match` for a failure that cannot happen.
A total indexing form is a precondition on a prelude function, which is a thing the language
can now express, and whether the prelude should carry one is a question rather than an
oversight.

It also used to refuse `n + 1` on a `Positive`, and this paragraph used to say that was the
reasoning working. It was not. Overflow is an error rather than a wrap, so `n + 1` either
produces a value or stops the program, and it never produces a wrong one; so a value that
exists is inside `Int`, and any sum that exists is greater than one. The interval clamps at
the edge instead of collapsing, and the runtime check that used to be emitted there could
never have fired. Two of the three warnings in the file went away and the comments explaining
them turned out to be the more interesting half of the fix.

What it still cannot do is relate two names through a product, so `result == n * n` says
nothing. [design/02-syntax.md](design/02-syntax.md) lists the rest.

`transfer.deed` used to model something that could not exist. `Money.units` was `Positive`,
which made a zero balance and a debit unwritable, and the type checker said so. The fix was
to separate the type that can be zero from the type that cannot, which is the sort of thing
the language is supposed to force and did.

`hello.deed` was worse. The test written to prove that a function without a `Console` cannot
write to one failed, because `Io.write(Console, "hi")` type checked: a type name in
expression position had no type, and no type agrees with everything. Capability safety was
decorative for about an hour. That is now `DEED4019`.

`closures.deed` is the same shape of bug found twice in one place. A parameter could be
written with no type, which made it the unknown type, and a closure's effects were charged to
nobody. Either alone is arguable. Together they meant a closure could carry any effect into
any function with the row staying empty the whole way. A parameter now needs a type, and a
closure's effects are charged to whoever wrote it. A closure can leave the function that
wrote it, and the type it leaves through says what it may do on the way: `Fn(Int) -> Int`
performs nothing, `Fn(Int) uses Log.note -> Int` performs that and no more. Leaving a row off
cannot mean any row, or a value could carry an unstated effect through a signature.

`diverge.deed` is what a design document claiming something the compiler did not do looks
like when it gets fixed. "Non-termination is an effect" had a section of its own and
`Diverge` appeared nowhere else in the repository, so the word did not even resolve. Running
an unbounded recursion overflowed the host stack and killed the process, with no diagnostic
and no exit code anyone could read. Now a function that can reach itself has to declare it,
mutual recursion included, and the interpreter reports `DEED6009` instead of dying. There is
still no termination proving, so `factorial` has to declare it too, and the design document
says that rather than hoping.

`strings.deed` exists because until recently there was no way to join two strings. A program
could not build a message out of pieces, so nobody could write a program, so every other
decision here was untested. Fixing it turned up the same bug from the other side: `<` was
accepted on anything as long as both sides had the same type, so comparing two records passed
the type checker and failed at runtime with a message blaming the interpreter for not
implementing something that has nothing to implement. It now also carries `split`, `join`,
`to_string` and `to_int`, which are two pairs of inverses and are there for one reason: a
program could hold text and hold a number and get from neither to the other, so it could not
read input, print a count, or write anything back out.

`lists.deed` is the same complaint one size up. Until it, nothing in the language could hold
more than one of something, so every program was one that worked on a fixed number of named
variables. `List` is built in rather than declared, and it is the same shortcut `Result`
takes: element types are compared componentwise and an unknown one absorbs, so `[]` fits
wherever a list was wanted and no
unification was needed anywhere. What the file is honest about is iteration: at the time
there was no `for`, so walking a list was recursion and every walk declared `Diverge`. That
was not an oversight. An accumulator loop wants mutation or a fold, and mutation here is
supposed to be handler state and nothing else, so the shape of `for` was an argument about
the central claim of the language rather than a piece of syntax to add.

`generics.deed` is what the two shortcuts were actually costing. Not the shortcuts: nobody
could write a library. `first`, `last`, `map` and `count_where` are one function at different
element types and none of them could be written down anywhere in this repository. A generic
function costs this design almost nothing, because there is still no unification: at a call
site the declared parameter types are matched against the argument types, walking down both
in step, and `List<T>` against `List<String>` gives `T = String`. One rule carries the rest.
Every type parameter has to appear in a parameter's type, which means a call always knows
what every parameter is, which means no type arguments are ever written, which means
`f<a>(b)` versus `f < a > (b)` is not a problem this parser has. It is also the same claim
everything else here makes: a signature is complete.

`generic_types.deed` is the other half, and it is where the shortcut stops being one.
`Option` was always the third generic type people reach for, and it is declared there rather
than built in. A generic type is a head plus arguments compared componentwise, which is
exactly how `Result` and `List` were already compared, so the mechanism was half built
before it was asked for. What decides the arguments is the same matching a call does: a
literal matches its declared field types against the values it was given. A field then reads
at the type it was applied to, and so does a pattern binder, which is the part a test caught:
`Some { value }` on an `Option<Int>` has to bind an `Int` rather than the `T` the choice was
declared with.

`std/list` and `using_list.deed` are the point of the three changes above. `std/list` is a
list library written in Deed: `map`, `map_at`, `filter`, `filter_at`, `fold`, `fold_at`, `any`,
`all`, `count_where`, `filtered_with`, `first`, `last`, `reversed`, `prepend`, `find`, `take`, `drop`, `concat`, `flatten`, `partition`, `zip`, `enumerate`, `windows`, `chunks`, `intersperse`, `unzip`, `flat_map`, `scan`, `transpose`, `group_by` and `sort`, none of them known to the
compiler, no builtin, no special case, no name in the prelude. It is the first thing in this
repository anybody else could have written.

It ships with the compiler, and it did not always. It sat under `examples/` for months, and a
module's name says where it lives, so the only way to import it was `use examples/list` and a
program written anywhere else had to copy the file. Nothing about the library changed when it
moved.

It is also what got the language a way to stop a walk. `any` and `all` in it both used to
open with a branch whose only job was to notice that the answer was already in, which is
control flow inside a fold, which is the thing a fold exists to not have, and the branch could
skip the work but not the turn. The design doc said `break` and `continue` had not come up in
a program written here, and this file was the counterexample sitting in the repository. So a
`for` takes a `while` now, read before each turn with the accumulator in scope and the element
out of it, since the element belongs to the turn the condition is deciding whether to take.
Both functions lost their branch. It is not `break`: nothing is abandoned, the value of the
loop is the accumulator it stopped holding, and the list still bounds how many turns there can
be, so the termination argument that keeps a `while` statement out is untouched.

Pointing `todo.deed` at it found the last thing missing. The compiler only looks at the files
it was handed, which is a rule worth having, and it meant `deed run examples/todo.deed` could
not find the library and the workaround was to name every file the program transitively
needs. A library nobody can use without knowing its file layout is not a library. A module's
name says where it lives now: a module named `a/b` is at `<root>/a/b.deed`, and the root comes
from taking a named file's module path off the end of its own path. No search path, no config
file, no manifest, and it is a rule every file here already followed.

What was named is the subject and what an import needed is context, so `deed test app.deed`
does not run a library's tests and `deed check app.deed` does report a library's errors. That
is the same split the language server already makes between the workspace and the open
document.

What `std/list` needed last was a row variable. Before that there were two ways to write
`map` and both were wrong: `Fn(A) -> B` promises to perform nothing, so the callback could
not log or read a file, and `Fn(A) uses Log.note -> B` works for one effect and needs a
second copy for the next one. `uses r` stands for whatever the callback performs and passes
it through to the function's own row, so `map(ns, |n| n + n)` performs nothing and
`map(ns, |n| { Log.note(..); n })` performs `Log.note`, and the second caller has to say so.
The library says "whatever you gave me" and the caller says what that was.

Inside the body a row variable is an ordinary entry: calling the callback performs `r`, the
contract says `uses r`, and the same two rules that check every other function check that
one. Which is why a variable that reaches no parameter was already an error before anything
was written for it, since nothing can fill it, so nothing performs it, so the row is too wide.

The design doc spent a while listing "can a row variable appear in two parameters" as an open
question, on the assumption that the second occurrence was checked against the first the way
a type parameter's is. Measuring it first said otherwise: nothing compares them, and the call
site unions the row of every argument the variable came from. That is the useful answer, so
`filtered_with(ns, keep, dropped)` takes a pure `keep` and a `dropped` that logs and charges
the caller `Log.note`. A variable is not a name for one row that every parameter carrying it
has to agree on, it is a name for the places a call reads a row off. The behaviour was right
and untested, which is a worse position than being wrong and tested, so it is pinned now.

Looking at that machinery again turned up a hole that had been open the whole time. The
pass worked out a function value's row by matching on the shape of the expression: it knew a
closure written on the spot and a bare name, and answered "performs nothing" for everything
else. An empty row is a claim rather than an absence, so every other shape was a claim nobody
had checked, and there were five of them. A function that came back from a call, one chosen
by an `if`, one taken out of a list, one read out of a record field, and a call applied
straight to the result of another call. Each one ran an effect through a caller that declared
none, and the argument in the comment for why that was safe confused two different questions:
whether a value performs more than its type allows, which was checked, and what calling it
costs the caller, which was not.

The row is part of the type, and the type checker already works out a type for every
expression, so it hands over the row of every function-typed one and the guessing is gone.
The rule that had to come with it is where a row variable may be written: the row of a
parameter that is a function type, and the declaration's own `uses` clause, and nowhere else.
A variable in a return type reaches a caller standing for something that caller has no word
for, so it gets dropped, and a dropped entry is an effect that happens and is not declared.
That is the same rule as the one saying a type parameter has to appear in a parameter's type,
and for the same reason: a signature whose call sites cannot work out what it means is not a
signature.

Five holes in one place, all found by hand, is a bad way to find out. So the interpreter
holds the program to its own signatures while it runs. Each active call carries the row its
declaration wrote down, every effect performed is checked against every call on the stack,
and one nobody declared is an error reported against the compiler rather than the program,
since the file was accepted and so the check that accepted it was wrong. A `with` block
discharges what is inside it and a contract does not contribute to a row, so both are exempt,
which are rules the language already had rather than allowances made here. Pointed at
`examples/`, it passes, and with the fix above taken back out it fails on the first program
that used to slip through. The point is not what it finds today. It is that the next one
reports itself, with a stack, from a real program.

It found one the same day it was written, which is sooner than I expected and worse than I
expected. A `with` block discharges the effect the handler implements, which is what a
handler is for, and it said nothing about what the handler does to implement it. So a
function handed a `Console` could install a handler whose operation writes to that console,
run it, and declare an empty row. Everything checked clean and the screen still got written
to. Those effects go to whoever installed the handler now, which is the function that made
the decision, and a handler carries what it performs across a module boundary the same way a
function carries its row.

The worst one so far was found the same afternoon. The `Guarded` tier did not guard a return
value. `deed check` printed "so it becomes a runtime check" and there was no check: the
interpreter guarded arguments and annotated `let`s, because those were the two places
somebody had happened to write the call. A function declared to return a `Positive` would
hand back a `-5` and every caller downstream was entitled to believe it. The warning was the
part that made it dangerous, since that is what convinces a reader they are covered. The two
passes now read the same table, and `crates/deed-driver/tests/guards.rs` has a test for every
place a refined value can come into existence, each one handing the guard something it is
supposed to refuse. The test that should have caught this did exist, and it passed, because
it only ever handed the guard values it accepts.

Those tests all had to be written in Rust, which was itself a gap and `proven.deed` said so in
a comment for months. A contract failure ends the run, so a file of examples showing a guard
refusing something could not pass, and then preconditions started being read at the call site
and the checker began refusing such a file outright. The better the checking got, the further
out of reach the check itself went. `assert refuses order_of(0)` says it from inside the
language now: it passes when a `where` clause, an `ensures` clause or a refinement turns the
value down, and fails when anything else happens, including the call producing a value. That
is the one thing in the language that catches, and it catches those three and nothing else,
because overflow and a missing handler are a program going wrong rather than a signature
doing its job.

The `Tested` tier had a quieter version of the same problem. `deed test` generates a hundred
inputs from a contract and shrinks whatever fails, and for a while it only shrank integers
and the fields of records. So a counterexample of `[95]` came out when `[1]` says the same
thing, and a variant would shrink its field to zero and stop rather than becoming the
`Nothing` sitting next to it. A counterexample built out of something that does not shrink
looks exactly like a small one that happens to be awkward, and nothing tells you which it
was. Everything the generator can build shrinks now: lists get shorter before their elements
get smaller, strings get shorter and then plainer, `true` gives way to `false`, and a variant
gives way to a sibling that carries no fields. That last one is the only part that cannot be
read off the value, so the choices are walked once and every variant is told which of its
siblings are empty.

Five of these have now had the same shape: something has no type, the unknown type agrees
with everything, and checking quietly stops. So there is an invariant for it now. In a file
that checks cleanly, no expression is unknown, because an unknown one is an expression nothing
done with it was checked against. Written as a test and pointed at the examples, it found two
more in the first run and a third once those were fixed: closure bodies, handler literals, and
every call to an imported effect's operation. `crates/deed-driver/tests/fully_typed.rs` is
where the next one gets found on purpose.

Looking for the same shape one level up found the biggest one. A function's effect row did
not cross a module boundary, so every call into another file was free, and in a program with
more than one file that is most calls. The pass this language exists for was doing its work
on the calls that mattered least. Rows travel now, and a caller that inherits an effect it
never imported is told which module to import it from, because a row that cannot name what it
grants is not a row.

The grammar had one of its own. Statements are separated by nothing, so what ends one is the
next token not being able to continue the expression before it, and the design doc said that
held only by coincidence. It did not hold at all: `(` starts a parenthesised expression and
continues a call, `-` starts a negation and continues a sum, so `let a = 1` with `-2` under
it read as `let a = 1 - 2` and the second line was gone with nothing said about it. A line
break ends an expression now, and the rule is the same inside brackets as outside them,
because a rule that switches off somewhere is a rule people have to remember. Nothing
canonical changes shape under it, since `deed fmt` never breaks a binary expression across
lines.

Being right is half of it. The rule makes a new mistake possible, which is carrying an
operator down to the next line, and both halves of that are answered. `* 2` on its own line
is a parse error that says why rather than only what, and takes the rest of the line with it
so one mistake gets one complaint. `- 2` on its own line is a perfectly good expression and
there is nothing to refuse, so what is left is a statement whose value nobody reads, and
saying that is `DEED4026`. It is a warning, because `let _ = f()` is how a program says it
meant to throw a value away and working code should not have to be rewritten to keep
compiling, and dropping a `Result` gets its own sentence, since that one loses the failure
rather than a line.

Then the escape hatch turned out to have a hole. `let _ = f()` throws the value away and
says so, but `let b = f()` silences the warning too, and now there is a name nobody reads,
which is the same statement doing nothing with an explanation attached. So a `let` binding
one plain name that no expression mentions is `DEED3009`. The measurement is the interesting
part: pointing it at every binder fired twenty seven times across the examples and twenty
five of those were not mistakes, because a pattern is there to match and `err(why)` names
what the shape holds, a parameter's shape is the signature and a handler's belongs to the
effect it implements, and a `for` binder walks whether or not the element is wanted. A `let`
is the one form whose entire reason for existing is the name. Narrowed to that, it fires
once in the whole corpus, on a binding an example had already called `unused`.

`deed fmt` prints one canonical form and takes no options for the output. P4 said formatting
is not configurable long before anything enforced it, which meant the files were formatted
the way they happened to have been typed. A test now asserts that every `.deed` file in the
repository is already canonical, so the principle either holds or the build fails.

`deed fix` is the same move for P7. Diagnostics already carried a patch and a note about
whether that patch is certain or a guess, and nothing applied them, so what P7 described was
a data structure. `deed fix` applies the certain ones and refuses the guesses, with no flag to
override that. Which left the guesses reachable by nothing, and the certain ones reachable
only from a place the reader has already left, until the language server started offering
both as quick fixes.

It writes rows now, which is the part that costs something day to day. A fix is a span and a
replacement, so most of them are handed over where the problem is found, and the row
diagnostics could not: `DEED5001` names the effect, names the function and tells you to add it
to the `uses` clause, and saying that as a span means knowing about commas, about indentation,
and about a clause that may not be there yet. The effect checker has the answer and no
business knowing any of that, so the driver writes those, where the text, the tree and the one
canonical layout are all in scope. `deed fix` fills in a row that is too narrow, takes out an
entry that is never performed, and what it writes is exactly what `deed fmt` would have.

It declines twice, and both are the point. A contract holding a `where` or an `ensures` is
left alone, since the region between a signature and its body holds all three and nothing in
the tree says where one stops and the next starts. And a comment anywhere in that region stops
it, because rewriting the block would eat the comment, and a machine-applicable fix that
deletes a comment is a fix nobody should have applied.

An unused import goes the same way, one pass earlier. `DEED3003` knows which name nothing uses
and knows nothing about the comma beside it, and taking the last name out of a list is a
question about a line and the blank line next to it. One fix for the whole block rather than
one per name, which is not tidiness: two names out of the same list are two edits over
overlapping text and both get dropped, and two whole lines removed separately each leave
behind the blank that used to sit beside them, so the file ends up with a gap that no longer
separates anything. Rewriting the block once has neither problem, and what it writes is what
`deed fmt` would have.

The two warnings about something going nowhere carry a guess each, and a guess is what the
editor is for. `DEED3009` offers `_spare`, `DEED4026` offers `let _ = ` in front of the line,
and neither is ever applied in bulk, because the other way to arrive at either one is a value
that was supposed to be read and papering over that is the single thing they must not do.
`DEED4026` is also the first fix the type checker has ever carried. A type that does not fit
has no obvious repair, which is why there were none, and this one has exactly one mechanical
answer that is still not certain to be what was meant.

`deed check --timings` is the same move for P9, which said check latency is budgeted and had
never been measured. The guard is not a wall clock budget, since a test that fails on a busy
machine is a test people learn to rerun. It is the shape of the curve: ten times the input
has to cost well under a hundred times the time. That test found a quadratic on its first
run, in the "did you mean" suggestions, which cost an edit distance against every name in
scope for every name that failed to resolve.

`names.deed` and `greeting.deed` are two modules that see each other. A module is named by its
own `module` line, and the unit of compilation is the set of files you handed the compiler,
so `deed check examples/` resolves the `use` in one against the declarations in the other. A
`use` of a module that is not there is an error, so is a name it does not declare, so is a
field an imported record does not have, so is a `match` on an imported choice that forgets a
variant. None of that was checked before: an imported name had no type, and a name with no
type agrees with everything, so a module boundary was a place where checking stopped.

Running crosses it too. A call through an import walks into the other module's body with
that module's own names in scope, and a variant carries the module that declared it, so the
same variant reached two ways is one value. Effects cross as well: `sink.deed` declares an
effect and two handlers, `greeting.deed` performs the effect and installs the handlers, and
the row is checked against the declaration in the other file in both directions.

No dependencies. `cargo test` runs the whole thing, and one of the tests is that
`examples/transfer.deed` survives every pass.

`deed check` runs every pass even when an earlier one failed. Stopping at the first failure
would cost a round trip for everything the later passes would have found, and the design
makes running on safe: parse errors become error nodes, unresolved names become unknown
types, and unknown agrees with everything. Diagnostics come out in source order rather than
pass order, because a reader works down the file and which pass noticed is not their problem.

## What Deed is deliberately not doing

No macros, no inheritance, no exceptions, no operator overloading, no implicit
conversions, no optional syntax. Each of those makes a function impossible to understand
without reading something else, which is exactly the cost this language exists to remove.

Deed is less pleasant to write by hand than most modern languages. That is the trade, made
on purpose.

## The obvious problem

There is no training data for a language nobody has written yet, and no ecosystem either.
The only real answer to the first is to keep the specification small enough to read in
full, in one sitting, so the language never has to be remembered. That is a hard
constraint on the design, not a nice-to-have. The answer to the second is interop, and it
is not solved yet.

If you think there is a better answer, please open an issue.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Short version: the design is the thing that needs
attacking right now, not the code.

## License

Apache-2.0. See [LICENSE](LICENSE).
