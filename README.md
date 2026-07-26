# Vow

A contract-first language where a function signature is a promise the compiler checks.
Built for code that machines write and humans review.

> **Status: it runs.** The compiler lexes, parses, resolves names, type checks and checks
> effect rows, and a tree walking interpreter executes `test` blocks and `main` with
> contracts enforced at runtime. Programs get their authority from a `System` capability
> handed to `main`, and a `Dir` narrows to a subdirectory and cannot be walked back out of.
> There is no code generation. Criticism of the design is still the most useful
> contribution. See [issue #1](https://github.com/onatozmenn/vow/issues/1) for where this is
> going.

```
$ cargo run -p vow-cli -- run examples/config.vow --dir examples
found it
`..` would leave the directory, and there is no way out of a `Dir`
`../Cargo.toml` is not a single name, and a `Dir` only takes one at a time
`/etc/passwd` is not a single name, and a `Dir` only takes one at a time
`nowhere` is not there
used the fallback
```

```
$ cargo run -p vow-cli -- test examples/
examples/counter.vow
  ok    bumping twice adds twice
  ...
examples/transfer.vow
  ok    moves the money and conserves the total
  ok    refuses to overdraw and leaves the ledger alone
  ok    refuses a currency mismatch and leaves the ledger alone

75 passed, 0 failed
```

```
$ cargo run -p vow-cli -- check examples/transfer.vow --obligations
obligations: 4 proven, 0 tested, 6 guarded
  guarded  examples/transfer.vow:94:5  transfer ensures ok
  ...
  proven   examples/transfer.vow:202:76  Positive
```

## The idea

Most of the code being written today is not typed out by a person. That changes which
costs matter. Producing a line of code is close to free. Reading it, trusting it, and
finding out three weeks later that it was subtly wrong are not.

Every language we use was shaped by the opposite assumption. They optimize for the
comfort of someone typing character by character: implicit behaviour, clever shorthand,
five ways to express the same thing. All of that is now working against us.

Vow makes a different trade. Signatures carry the entire contract, bodies are checked
against it, and nothing in the language lets a function reach outside what its signature
admits to.

```vow
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

This is not an illustration. It is the top of [examples/transfer.vow](examples/transfer.vow),
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

Read them in order. Each one leans on the one before it.

## What is built

| Crate | Does |
| --- | --- |
| `vow-diagnostics` | Spans, source maps, and structured diagnostics with machine-applicable fixes |
| `vow-lexer` | Source text to tokens |
| `vow-ast` | The syntax tree |
| `vow-parser` | Tokens to a tree, with recovery |
| `vow-resolve` | Every name bound to a declaration, including across module boundaries |
| `vow-typeck` | Every expression given a type, including types from other modules |
| `vow-effects` | Every effect row checked against what the body does |
| `vow-interp` | Runs `test` blocks, property tests and `main`, with contracts enforced |
| `vow-fmt` | The one canonical form, with no options for the output |
| `vow-lsp` | A language server: diagnostics, hover, go to definition, references, rename, completion and formatting |
| `vow-driver` | Runs all of the above, in one place, so nothing drifts |
| `vow-cli` | The `vow` binary: `check`, `test`, `run`, `fmt`, `fix` and `lsp` |

`vow lsp` is a language server, and most of it is plumbing over things that already existed:
the compiler produces structured diagnostics with spans, `Types::type_of` can say what an
expression turned out to be, `Resolutions` can say where a name was declared, and the
formatter has one canonical answer with no options. It publishes diagnostics as you type,
says the type of whatever is under the cursor, jumps to a declaration in whichever file
declares it, lists every use of a name across the workspace, renames one everywhere it is
written, offers what could be written where the cursor is, and formats a file.

Completion is the only one that is a different shape. Everything else answers a question
about a name that already exists, and a document does not parse while somebody is typing into
it, so it answers a narrower question instead: what is in scope here. After a `.` that is the
fields of whatever is to the left; inside `use a/b.{ }` it is what that module exports, which
is the one place a name has to match another file exactly; anywhere else it is what the file
declared, what it imported, and the prelude. Nothing it offers inserts more than a name. No
snippets, no argument placeholders, no auto-import, because each of those is a decision about
what somebody meant.

Rename and find references are the same walk with two answers, which is the point of them
sharing one. A rename that edited the declaration and left the `use` line that brought the
name into another file would fix one file and break another, and that is worse than not
having rename at all. What it deliberately does not decide is whether the new name is a good
one: a collision with something already in scope is something the checker has a diagnostic
for, and answering it twice is how the two answers come to disagree. It does refuse a name
the language cannot hold, and it asks the lexer rather than holding a second definition of
what an identifier is.

It checks a document together with every other `.vow` file in the folders the editor said it
has open, and an open file's text comes from the buffer rather than from disk, so removing an
export in one file puts a squiggle on the import in another before either is saved. The set
of files is the workspace rather than a guess: `initialize` carries it, and taking what the
editor says is the same answer `vow check src/` gives when a person says which directory they
mean. An editor that names no folder gets the single file behaviour. That was the only
behaviour until recently, and it meant every file with a `use` in it had a red line under the
import, which is exactly the failure a server is not allowed to have.

It rechecks the whole workspace on every keystroke and on every hover. That used to be
followed by "and nothing has measured it", which is now not true:
`cargo run -p vow-driver --example edit_loop --release` says the recheck is linear at about
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

The examples are [transfer.vow](examples/transfer.vow),
[counter.vow](examples/counter.vow), [hello.vow](examples/hello.vow),
[config.vow](examples/config.vow), [todo.vow](examples/todo.vow),
[journal.vow](examples/journal.vow), [proven.vow](examples/proven.vow),
[closures.vow](examples/closures.vow), [diverge.vow](examples/diverge.vow),
[strings.vow](examples/strings.vow), [lists.vow](examples/lists.vow),
[generics.vow](examples/generics.vow),
[generic_types.vow](examples/generic_types.vow), [list.vow](examples/list.vow),
[using_list.vow](examples/using_list.vow), and the three that see
each other: [names.vow](examples/names.vow), [sink.vow](examples/sink.vow) and
[greeting.vow](examples/greeting.vow). All are checked by every pass on every commit,
`hello.vow`, `config.vow`, `todo.vow` and `journal.vow` have a `main`, and the rest run their
own tests.

`todo.vow` is the one written to find out what is missing rather than to show what is there.
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

`for` is a fold with syntax rather than a loop with a variable in it:

```vow
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

`journal.vow` is the half `todo.vow` could not write. `Io` could read a file and open a
directory and there was no operation that wrote one, so `Dir` was a read capability wearing a
more general name. `Io.save` is that operation, and the thing worth looking at is what stops
it writing anywhere else. Two separate things do. The row: a function that does not declare
`uses Io.save` cannot write, whatever it is holding, so reading and writing are different
authorities over the same capability and which one a caller is handing over is written in the
signature. The capability: `Io.save` takes the `Dir` it writes into and there is no way to
construct one, and the name goes through the same check reading goes through, so `..`, an
absolute path, a separator and a symlink pointing out are refused for writing exactly as they
are for reading. Not a second implementation that agrees today.

`proven.vow` is the one that argues with itself. Every function in it either proves its
postcondition or explains, in a comment, why the checker cannot, and the file is written so
that the two halves sit next to each other. The `Proven` tier used to hold constant
expressions and nothing else, which made a refinement in real code a runtime check with
ceremony around it. It now reasons about intervals, about the difference between two names,
and about what a callee promised: a `where` clause, a refined parameter type, an `if`
condition, a guard that returns, `low < high`, and `ensures ok => result == n` at a call site
are all facts the rest of the body can use.

It also used to refuse `n + 1` on a `Positive`, and this paragraph used to say that was the
reasoning working. It was not. Overflow is an error rather than a wrap, so `n + 1` either
produces a value or stops the program, and it never produces a wrong one; so a value that
exists is inside `Int`, and any sum that exists is greater than one. The interval clamps at
the edge instead of collapsing, and the runtime check that used to be emitted there could
never have fired. Two of the three warnings in the file went away and the comments explaining
them turned out to be the more interesting half of the fix.

What it still cannot do is relate two names through a product, so `result == n * n` says
nothing. [design/02-syntax.md](design/02-syntax.md) lists the rest.

`transfer.vow` used to model something that could not exist. `Money.units` was `Positive`,
which made a zero balance and a debit unwritable, and the type checker said so. The fix was
to separate the type that can be zero from the type that cannot, which is the sort of thing
the language is supposed to force and did.

`hello.vow` was worse. The test written to prove that a function without a `Console` cannot
write to one failed, because `Io.write(Console, "hi")` type checked: a type name in
expression position had no type, and no type agrees with everything. Capability safety was
decorative for about an hour. That is now `VOW4019`.

`closures.vow` is the same shape of bug found twice in one place. A parameter could be
written with no type, which made it the unknown type, and a closure's effects were charged to
nobody. Either alone is arguable. Together they meant a closure could carry any effect into
any function with the row staying empty the whole way. A parameter now needs a type, and a
closure's effects are charged to whoever wrote it. A closure can leave the function that
wrote it, and the type it leaves through says what it may do on the way: `Fn(Int) -> Int`
performs nothing, `Fn(Int) uses Log.note -> Int` performs that and no more. Leaving a row off
cannot mean any row, or a value could carry an unstated effect through a signature.

`diverge.vow` is what a design document claiming something the compiler did not do looks
like when it gets fixed. "Non-termination is an effect" had a section of its own and
`Diverge` appeared nowhere else in the repository, so the word did not even resolve. Running
an unbounded recursion overflowed the host stack and killed the process, with no diagnostic
and no exit code anyone could read. Now a function that can reach itself has to declare it,
mutual recursion included, and the interpreter reports `VOW6009` instead of dying. There is
still no termination proving, so `factorial` has to declare it too, and the design document
says that rather than hoping.

`strings.vow` exists because until recently there was no way to join two strings. A program
could not build a message out of pieces, so nobody could write a program, so every other
decision here was untested. Fixing it turned up the same bug from the other side: `<` was
accepted on anything as long as both sides had the same type, so comparing two records passed
the type checker and failed at runtime with a message blaming the interpreter for not
implementing something that has nothing to implement. It now also carries `split`, `join`,
`to_string` and `to_int`, which are two pairs of inverses and are there for one reason: a
program could hold text and hold a number and get from neither to the other, so it could not
read input, print a count, or write anything back out.

`lists.vow` is the same complaint one size up. Until it, nothing in the language could hold
more than one of something, so every program was one that worked on a fixed number of named
variables. `List` is built in rather than declared, and it is the same shortcut `Result`
takes: element types are compared componentwise and an unknown one absorbs, so `[]` fits
wherever a list was wanted and no
unification was needed anywhere. What the file is honest about is iteration: at the time
there was no `for`, so walking a list was recursion and every walk declared `Diverge`. That
was not an oversight. An accumulator loop wants mutation or a fold, and mutation here is
supposed to be handler state and nothing else, so the shape of `for` was an argument about
the central claim of the language rather than a piece of syntax to add.

`generics.vow` is what the two shortcuts were actually costing. Not the shortcuts: nobody
could write a library. `first`, `last`, `map` and `count_where` are one function at different
element types and none of them could be written down anywhere in this repository. A generic
function costs this design almost nothing, because there is still no unification: at a call
site the declared parameter types are matched against the argument types, walking down both
in step, and `List<T>` against `List<String>` gives `T = String`. One rule carries the rest.
Every type parameter has to appear in a parameter's type, which means a call always knows
what every parameter is, which means no type arguments are ever written, which means
`f<a>(b)` versus `f < a > (b)` is not a problem this parser has. It is also the same claim
everything else here makes: a signature is complete.

`generic_types.vow` is the other half, and it is where the shortcut stops being one.
`Option` was always the third generic type people reach for, and it is declared there rather
than built in. A generic type is a head plus arguments compared componentwise, which is
exactly how `Result` and `List` were already compared, so the mechanism was half built
before it was asked for. What decides the arguments is the same matching a call does: a
literal matches its declared field types against the values it was given. A field then reads
at the type it was applied to, and so does a pattern binder, which is the part a test caught:
`Some { value }` on an `Option<Int>` has to bind an `Int` rather than the `T` the choice was
declared with.

`list.vow` and `using_list.vow` are the point of the three changes above. `list.vow` is a
list library written in Vow: `map`, `filter`, `fold`, `any`, `all`, `count_where`,
`filtered_with`, none of them known to the compiler, no builtin, no special case, no name in
the prelude. It is the first thing in this repository anybody else could have written.

Pointing `todo.vow` at it found the last thing missing. The compiler only looks at the files
it was handed, which is a rule worth having, and it meant `vow run examples/todo.vow` could
not find `examples/list` and the workaround was to name every file the program transitively
needs. A library nobody can use without knowing its file layout is not a library. A module's
name says where it lives now: a module named `a/b` is at `<root>/a/b.vow`, and the root comes
from taking a named file's module path off the end of its own path. No search path, no config
file, no manifest, and it is a rule every file here already followed.

What was named is the subject and what an import needed is context, so `vow test app.vow`
does not run a library's tests and `vow check app.vow` does report a library's errors. That
is the same split the language server already makes between the workspace and the open
document.

What `list.vow` needed last was a row variable. Before that there were two ways to write
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
value. `vow check` printed "so it becomes a runtime check" and there was no check: the
interpreter guarded arguments and annotated `let`s, because those were the two places
somebody had happened to write the call. A function declared to return a `Positive` would
hand back a `-5` and every caller downstream was entitled to believe it. The warning was the
part that made it dangerous, since that is what convinces a reader they are covered. The two
passes now read the same table, and `crates/vow-driver/tests/guards.rs` has a test for every
place a refined value can come into existence, each one handing the guard something it is
supposed to refuse. The test that should have caught this did exist, and it passed, because
it only ever handed the guard values it accepts.

Five of these have now had the same shape: something has no type, the unknown type agrees
with everything, and checking quietly stops. So there is an invariant for it now. In a file
that checks cleanly, no expression is unknown, because an unknown one is an expression nothing
done with it was checked against. Written as a test and pointed at the examples, it found two
more in the first run and a third once those were fixed: closure bodies, handler literals, and
every call to an imported effect's operation. `crates/vow-driver/tests/fully_typed.rs` is
where the next one gets found on purpose.

Looking for the same shape one level up found the biggest one. A function's effect row did
not cross a module boundary, so every call into another file was free, and in a program with
more than one file that is most calls. The pass this language exists for was doing its work
on the calls that mattered least. Rows travel now, and a caller that inherits an effect it
never imported is told which module to import it from, because a row that cannot name what it
grants is not a row.

`vow fmt` prints one canonical form and takes no options for the output. P4 said formatting
is not configurable long before anything enforced it, which meant the files were formatted
the way they happened to have been typed. A test now asserts that every `.vow` file in the
repository is already canonical, so the principle either holds or the build fails.

`vow fix` is the same move for P7. Every diagnostic already carried a patch and a note about
whether that patch is certain or a guess, and nothing applied them, so what P7 described was
a data structure. `vow fix` applies the certain ones and refuses the guesses, with no flag to
override that.

`vow check --timings` is the same move for P9, which said check latency is budgeted and had
never been measured. The guard is not a wall clock budget, since a test that fails on a busy
machine is a test people learn to rerun. It is the shape of the curve: ten times the input
has to cost well under a hundred times the time. That test found a quadratic on its first
run, in the "did you mean" suggestions, which cost an edit distance against every name in
scope for every name that failed to resolve.

`names.vow` and `greeting.vow` are two modules that see each other. A module is named by its
own `module` line, and the unit of compilation is the set of files you handed the compiler,
so `vow check examples/` resolves the `use` in one against the declarations in the other. A
`use` of a module that is not there is an error, so is a name it does not declare, so is a
field an imported record does not have, so is a `match` on an imported choice that forgets a
variant. None of that was checked before: an imported name had no type, and a name with no
type agrees with everything, so a module boundary was a place where checking stopped.

Running crosses it too. A call through an import walks into the other module's body with
that module's own names in scope, and a variant carries the module that declared it, so the
same variant reached two ways is one value. Effects cross as well: `sink.vow` declares an
effect and two handlers, `greeting.vow` performs the effect and installs the handlers, and
the row is checked against the declaration in the other file in both directions.

No dependencies. `cargo test` runs the whole thing, and one of the tests is that
`examples/transfer.vow` survives every pass.

`vow check` runs every pass even when an earlier one failed. Stopping at the first failure
would cost a round trip for everything the later passes would have found, and the design
makes running on safe: parse errors become error nodes, unresolved names become unknown
types, and unknown agrees with everything. Diagnostics come out in source order rather than
pass order, because a reader works down the file and which pass noticed is not their problem.

## What Vow is deliberately not doing

No macros, no inheritance, no exceptions, no operator overloading, no implicit
conversions, no optional syntax. Each of those makes a function impossible to understand
without reading something else, which is exactly the cost this language exists to remove.

Vow is less pleasant to write by hand than most modern languages. That is the trade, made
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
