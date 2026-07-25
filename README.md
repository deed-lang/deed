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

12 passed, 0 failed
```

```
$ cargo run -p vow-cli -- check examples/transfer.vow --obligations
obligations: 6 proven, 0 tested, 0 guarded
  the tested tier needs property test generation, which does not exist yet
  proven   examples/transfer.vow:126:50  Positive
  ...
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
| `vow-typeck` | Every expression given a type |
| `vow-effects` | Every effect row checked against what the body does |
| `vow-interp` | Runs `test` blocks, property tests and `main`, with contracts enforced |
| `vow-fmt` | The one canonical form, with no options for the output |
| `vow-driver` | Runs all of the above, in one place, so nothing drifts |
| `vow-cli` | The `vow` binary: `check`, `test`, `run` and `fmt` |

There are four examples, [transfer.vow](examples/transfer.vow),
[counter.vow](examples/counter.vow), [hello.vow](examples/hello.vow) and
[config.vow](examples/config.vow). All are self contained and checked by every pass on every
commit. The first two run their own tests, the last two have a `main`.

`transfer.vow` used to model something that could not exist. `Money.units` was `Positive`,
which made a zero balance and a debit unwritable, and the type checker said so. The fix was
to separate the type that can be zero from the type that cannot, which is the sort of thing
the language is supposed to force and did.

`hello.vow` was worse. The test written to prove that a function without a `Console` cannot
write to one failed, because `Io.write(Console, "hi")` type checked: a type name in
expression position had no type, and no type agrees with everything. Capability safety was
decorative for about an hour. That is now `VOW4019`.

`vow fmt` prints one canonical form and takes no options for the output. P4 said formatting
is not configurable long before anything enforced it, which meant the files were formatted
the way they happened to have been typed. A test now asserts that every `.vow` file in the
repository is already canonical, so the principle either holds or the build fails.

`names.vow` and `greeting.vow` are two modules that see each other. A module is named by its
own `module` line, and the unit of compilation is the set of files you handed the compiler,
so `vow check examples/` resolves the `use` in one against the declarations in the other. A
`use` of a module that is not there, or of a name that module does not declare, is now an
error. What still does not cross is the types: an imported name has no type behind it, so
the type checker treats it as unknown and unknown agrees with everything. That is
[issue #37](https://github.com/onatozmenn/vow/issues/37) and the example says so in a
comment rather than pretending otherwise.

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
