# Effects

An effect is anything a function does that is not returning a value: reading a database,
getting the time, writing a log, failing to terminate.

In most languages these are invisible. A function typed `String -> String` might hit the
network and you would never know. Vow makes the effect row part of the signature, which is
what lets absence mean something.

## Declaring an effect

```vow
effect Ledger {
    fn balance(account: AccountId) -> Money
    fn post(entry: Entry) -> Result<(), LedgerError>
}

effect Clock {
    fn now() -> Instant
}

effect Audit {
    fn append(event: Event)
}
```

An effect is an interface with no implementation. It names operations and their types, and
nothing more.

## Using effects

```vow
fn daily_report(account: AccountId) -> Report
  uses Ledger.read, Clock.now
{
    let today = Clock.now().date()
    let balance = Ledger.balance(account)
    Report { account, balance, today }
}
```

The row is fine-grained. `Ledger.read` and `Ledger.write` are separate obligations, so a
reporting function can be given read access without also gaining the ability to move money.
Naming the whole effect grants all its operations.

## Propagation

Rows are inferred bottom up and checked against declarations.

```vow
fn a() uses Ledger.read { ... }
fn b() uses Audit.append { ... }

fn c() uses Ledger.read, Audit.append {
    a()
    b()
}
```

Two rules, both errors rather than warnings:

- **Too narrow:** the body performs an effect the signature does not declare.
- **Too wide:** the signature declares an effect the body cannot perform.

The second one matters more than it looks. If over-declaring were allowed, every signature
would drift toward listing everything, and the annotation would stop carrying information.
An effect row is only worth reading if it is tight.

## Purity is the default

No `uses` clause means no effects. A pure function can be evaluated at compile time, cached,
reordered, run in parallel, and tested with no setup at all.

Most code is pure and does not currently get to say so.

## Handlers

An effect is performed by the body and interpreted by a handler further out. This is what
makes testing mechanical instead of ceremonial.

```vow
handler InMemoryLedger implements Ledger {
    state accounts: Map<AccountId, Money>

    fn balance(account) -> Money {
        accounts.get(account).unwrap_or(Money.zero)
    }

    fn post(entry) -> Result<(), LedgerError> {
        accounts.update(entry.account, |m| m + entry.amount)
        ok(())
    }
}
```

```vow
test "transfer moves the money" {
    with InMemoryLedger { accounts: [alice -> 100.try, bob -> 0.try] },
         FrozenClock { at: 2026-01-01T00:00:00Z },
         NullAudit
    {
        let result = transfer(alice, bob, 40.try)
        assert result.is_ok()
        assert Ledger.balance(alice) == 60.try
        assert Ledger.balance(bob) == 40.try
    }
}
```

No mocking library, no monkey patching, no dependency injection framework. The effect row
already said what the function reaches for, so substituting an implementation is just
supplying a different handler.

## What this buys

**Colour-free async.** The `async`/`await` split exists because a language noticed one
effect and gave it dedicated syntax. If suspension is one row entry among many, a function
that suspends is not a different kind of function, and the two-worlds problem does not
arise.

**Determinism for free.** `Clock`, `Random` and `Net` are effects, so a function with an
empty row is reproducible by construction. Replay, and therefore durable execution, becomes
something the runtime can offer rather than something a library reimplements.

**Sandboxing without a container.** If a function's row is `Ledger.read`, it cannot open a
socket. That is a compile-time fact, not a runtime policy, and it is enforced without a
process boundary. Container startup is milliseconds and it is paid per test. This is not.

## The honest part

Effect systems have been understood for decades and keep failing to escape research. The
reason is almost never expressiveness, it is that annotations propagate and real programs
end up drowning in rows nobody wants to maintain.

Vow does not have a solution to that yet. The ideas on the table are:

- Infer rows everywhere except at module boundaries, so most functions never write one
- Effect aliases, so `uses Storage` expands to a named group
- Polymorphic rows, so a `map` does not need to enumerate what its callback might do

Row polymorphism is the one that worries me, because it is where the type system gets big,
and P2 is watching. If the annotation burden cannot be made to disappear for ordinary code,
this whole design fails on ergonomics, exactly like its predecessors.

## Open questions

- Can rows be inferred well enough that most functions carry none, and does that then
  undermine the review argument, which depends on the signature being complete?
- How do effects interact with data structures. Does a `Map` holding closures need a row?
- Is fine granularity worth it, or does `Ledger.read` versus `Ledger.write` just double the
  annotation cost for a distinction few people use?
- What does an effect row mean across a network boundary, where the callee is not compiled
  with you?
