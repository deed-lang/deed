# Vow

A contract-first language where a function signature is a promise the compiler checks.
Built for code that machines write and humans review.

> **Status: design phase.** There is no compiler yet. This repository is the
> specification and the reasoning behind it. Criticism of the design is the most
> useful contribution right now. See [issue #1](https://github.com/onatozmenn/vow/issues/1)
> for where this is going.

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
    amount.units > 0,
    from != to,
  uses
    Ledger.read,
    Ledger.write,
    Audit.append,
  ensures
    ok  => balance(from) == old(balance(from)) - amount,
    ok  => balance(to)   == old(balance(to))   + amount,
    err => unchanged(Ledger),
{
    ...
}
```

Three things follow from that block, and they are the whole pitch:

**You review the signature, not the body.** It fits on one screen and it is complete.
Whoever wrote the body had nothing else to satisfy.

**The body needs no other context.** No global state, no implicit conversions, no
inheritance, no exceptions thrown from four frames down. Everything that can affect this
function is written above the brace.

**The function cannot reach the network.** Not by accident and not on purpose. It did not
ask for that capability in `uses`, so it does not have it. Sandboxing stops being a
container and becomes a type.

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
