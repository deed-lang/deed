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

51 passed, 0 failed
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
| `vow-lsp` | A language server: diagnostics, hover, go to definition and formatting |
| `vow-driver` | Runs all of the above, in one place, so nothing drifts |
| `vow-cli` | The `vow` binary: `check`, `test`, `run`, `fmt`, `fix` and `lsp` |

`vow lsp` is a language server, and most of it is plumbing over things that already existed:
the compiler produces structured diagnostics with spans, `Types::type_of` can say what an
expression turned out to be, `Resolutions` can say where a name was declared, and the
formatter has one canonical answer with no options. It publishes diagnostics as you type,
says the type of whatever is under the cursor, jumps to a declaration, and formats a file.

It checks a document together with every other `.vow` file in the folders the editor said it
has open, and an open file's text comes from the buffer rather than from disk, so removing an
export in one file puts a squiggle on the import in another before either is saved. The set
of files is the workspace rather than a guess: `initialize` carries it, and taking what the
editor says is the same answer `vow check src/` gives when a person says which directory they
mean. An editor that names no folder gets the single file behaviour. That was the only
behaviour until recently, and it meant every file with a `use` in it had a red line under the
import, which is exactly the failure a server is not allowed to have.

It rechecks the whole workspace on every keystroke and on every hover. That is fine at this
size and is the thing that will stop being fine first, which is what P9 is about and what
nothing has measured yet.

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
[strings.vow](examples/strings.vow), [lists.vow](examples/lists.vow), and the three that see
each other: [names.vow](examples/names.vow), [sink.vow](examples/sink.vow) and
[greeting.vow](examples/greeting.vow). All are checked by every pass on every commit,
`hello.vow`, `config.vow`, `todo.vow` and `journal.vow` have a `main`, and the rest run their
own tests.

`todo.vow` is the one written to find out what is missing rather than to show what is there.
It reads a list of tasks out of a directory it was handed, counts them, and prints the ones
that are not done, which is the smallest thing anybody would call a program and was not
writable at all a week ago. It found four things. Three of its functions are the same
function: start at zero, look at each element, stop when the index runs out, and declare
`Diverge` for the privilege. An accumulator has to be threaded through as a parameter,
because handler state is the only mutable thing in the language and reaching for a handler to
collect strings would be using an effect to avoid a loop. The file format is `x|title` rather
than `[x] title` because splitting is all there is, so the data got bent to fit the tool. And
the first run printed its own output backwards over itself, because splitting on `\n` leaves
a carriage return on every line of a file written on Windows and there is nothing that trims
one off. None of those were obvious from inside the compiler.

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
are all facts the rest of the body can use. What it still cannot do is relate two names
through a product, so `result == n * n` says nothing, and it will not prove anything about
arithmetic that could overflow, which is why `n + 1` on a `Positive` stays `Guarded` and says
so. [design/02-syntax.md](design/02-syntax.md) lists the rest.

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
closure's effects are charged to whoever wrote it. That is sound because the only closure
that can leave the function that wrote it is one that performs nothing: `Fn(Int) -> Int` is a
type, and the second thing it says is that this performs no effects. There is no syntax for a
row on a function type, and leaving one off cannot mean any row, or a value could carry an
unstated effect through a signature.

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
variables. `List` is built in rather than declared, because there is still no way to declare
a generic type, and it is the same shortcut `Result` takes: element types are compared
componentwise and an unknown one absorbs, so `[]` fits wherever a list was wanted and no
unification was needed anywhere. Whether that shortcut can carry a third type is the question
that decides whether generics get written. What the file is honest about is iteration: there
is no `for`, so walking a list is recursion and every walk declares `Diverge`. That is not an
oversight. An accumulator loop wants mutation or a fold, and mutation here is supposed to be
handler state and nothing else, so the shape of `for` is an argument about the central claim
of the language rather than a piece of syntax to add.

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
