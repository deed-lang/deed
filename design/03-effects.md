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
  uses Ledger.balance, Clock.now
{
    let today = Clock.now().date()
    let balance = Ledger.balance(account)
    Report { account, balance, today }
}
```

The row is fine-grained. A `uses` entry names an operation the effect declares, so a
reporting function can be granted `Ledger.balance` without also gaining `Ledger.post` and
the ability to move money. Naming the effect on its own, as `uses Ledger`, grants all of its
operations.

An earlier version of this document wrote `Ledger.read` and `Ledger.write` next to an effect
that declared `balance` and `post`. That was two ideas at once, permission groups and
operations, and only one of them survived. Entries name operations. The read and write
distinction is carried by which operations you ask for.

## Propagation

Rows are inferred bottom up and checked against declarations.

```vow
fn a() uses Ledger.balance { ... }
fn b() uses Audit.append { ... }

fn c() uses Ledger.balance, Audit.append {
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

### Across a module boundary

A call into another module is not free. A function's declared row travels with it, so calling
something that logs means declaring that you log, wherever the callee is.

It did not, for a long time. The row stopped at the file boundary, so anything calling into
another module looked pure, and in a program with more than one file that is most calls. The
effect checker was doing its work on the ones that mattered least.

A row entry cannot travel as a definition, for the same reason a type could not: a definition
is an index into one module's table. It travels as the module the effect was declared in, its
name there, and the operation, which is the same identity the interpreter already uses for an
effect at runtime. The declaring module knows the path from its own syntax, either because the
effect is declared in it or because it is on a `use` line, which is what keeps exports
computable with nothing else resolved first.

**A caller has to be able to name what it inherits.** If it calls something that uses `Log`
and has not imported `Log`, that is an error, and the message says which module to import it
from. This is a real constraint rather than an implementation detail: a row that could not
name what it grants would not be a row. Declaring an effect means having a word for it.

Effects the language provides are the exception, and only because they need no word. `Io` and
`Diverge` are in the prelude, so every module can already name them.

## Specification is not action

A `where` or `ensures` clause may mention any effect operation and contributes nothing to
the row. `ok => Ledger.total() == old(Ledger.total())` does not require `Ledger.total` in
`uses`.

An obligation describes state rather than changing it. Making one cost permissions would
make obligations expensive to write, and obligations that are expensive to write do not get
written, which defeats the entire point of the language.

The cost of this rule is that a contract can observe something a body is not allowed to
touch. That looks wrong at first and is probably right: what a specification is allowed to
talk about and what an implementation is allowed to do are different questions.

## Purity is the default

No `uses` clause means no effects. A pure function can be evaluated at compile time, cached,
reordered, run in parallel, and tested with no setup at all.

Most code is pure and does not currently get to say so.

## Handlers

An effect is performed by the body and interpreted by a handler further out. This is what
makes testing mechanical instead of ceremonial.

The block below does not parse. `Map`, datetime literals, method calls and `Money.zero` are
all invented for it, because a handler is worth showing with something in its state and
there is nothing to put there yet. List literals used to be on that list and are real now,
which is one item off it and no more: a `Map` is still invented, and so is every method call
here. `examples/counter.vow` and `examples/transfer.vow` are the versions that run.

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

A handler operation writes no parameter types, because the effect already declared them and
saying it twice would be a second place for them to disagree. That only means anything if the
effect is actually consulted, and for a long time it was not: every parameter in every handler
body was the unknown type, unknown agrees with everything, and so the piece of code holding
the state and talking to the outside world was the least checked in the language. A refined
parameter raised no obligation, no warning and no runtime check.

The types come from the effect now, including one from another module, and a handler
operation that does not line up with the effect is `VOW4021`: an operation the effect never
declared, or one taking a different number of arguments.

**Installing a handler is a decision, and it costs what the handler costs.** A `with` block
answers for the effect the handler implements, which is what a handler is for. It says
nothing about what the handler does to implement it, and a handler that writes to a console
is a program writing to a console. Those effects go to whoever installed it:

```vow
fn looks_pure(n: Int, screen: Console) -> Int
  uses
    Io.write,
{
    with Loud { out: screen } {
        talks(n)
    }
}
```

`Log.note` is discharged, because that is what `Loud` is for. `Io.write` is not, because
nothing discharged it, and without the `uses` clause this function holds a console, writes to
it, and declares an empty row. That was true for a while. It is `VOW5001` now, and it works
across a module boundary too, because a handler carries what it performs in its export the
same way a function carries its row.

The handler's own row goes in with the body's row rather than straight onto the enclosing
function, so a handler whose operations perform the effect it implements is answered by
itself, and stays installable.

## What this buys

**Colour-free async.** The `async`/`await` split exists because a language noticed one
effect and gave it dedicated syntax. If suspension is one row entry among many, a function
that suspends is not a different kind of function, and the two-worlds problem does not
arise.

**Determinism for free.** `Clock`, `Random` and `Net` are effects, so a function with an
empty row is reproducible by construction. Replay, and therefore durable execution, becomes
something the runtime can offer rather than something a library reimplements.

**Sandboxing without a container.** If a function's row is `Ledger.balance`, it cannot open
a socket. That is a compile-time fact, not a runtime policy, and it is enforced without a
process boundary. Container startup is milliseconds and it is paid per test. This is not.

## The honest part

Effect systems have been understood for decades and keep failing to escape research. The
reason is almost never expressiveness, it is that annotations propagate and real programs
end up drowning in rows nobody wants to maintain.

Vow does not have a complete solution to that yet. The ideas on the table are:

- Infer rows everywhere except at module boundaries, so most functions never write one
- Effect aliases, so `uses Storage` expands to a named group

Row polymorphism was the third one and it was the one that worried me, because it is where
the type system gets big and P2 is watching. It is built now and it cost less than expected:
one more kind of definition, one more item in a row, and one substitution at a call site. See
the section below.

If the annotation burden cannot be made to disappear for ordinary code, this whole design
fails on ergonomics, exactly like its predecessors.

## Closures

A closure holds code without being a declaration, which makes it the obvious place for a row
to leak, and it did. Effects performed inside a closure were charged to nobody, and a
parameter written without a type became the unknown type, which agrees with everything. Put
together, a closure could carry any effect into any function and the row stayed empty all the
way.

The rule now is that a closure's effects are charged to the function that wrote it. That is
conservative rather than correct: the correct place is the call site, because that is where
the effect actually happens. It stays sound when a closure leaves, because it can only leave
through a function type, and a function type carries the row that says what it may do.

It over-approximates in one direction only. A closure that is written and never called still
charges its author, because deciding otherwise means deciding whether a function value
escapes, and not having to answer that is the point.

Closure parameters still need types, for the same reason every other parameter does.

### Function values

A function value can cross a boundary, and the type it crosses through says what it may do
on the way:

```vow
fn apply(f: Fn(Int) -> Int, n: Int) -> Int {
    f(n)
}

fn apply_logging(f: Fn(Int) uses Log.note -> Int, n: Int) -> Int
  uses
    Log.note,
{
    f(n)
}

fn adder() -> Fn(Int) -> Int {
    |x: Int| x + 1
}
```

`Fn(Int) -> Int` says three things. It takes an `Int`, hands back an `Int`, and performs
nothing. The third is not decoration and it is not a default that could be relaxed later
without breaking anything: leaving a row off cannot mean "any row". A value carrying an
unstated effect through a signature would undo the point of having rows at all, which is that
a signature is complete.

The row goes before the arrow. After the return type it would be indistinguishable from the
declaration's own contract, because that also starts with `uses` and also follows a return
type, so `fn make() -> Fn(Int) -> Int uses Log.note` would have two readings. Before the
arrow there is nothing to confuse it with, since the `->` ends the list.

Two rules follow, and they are the same two the rest of this document is about. A value may
not perform more than its type allows, so a closure that logs is refused by
`Fn(Int) -> Int`. And calling a function value costs whatever its type said it costs, so
`apply_logging` has to declare `Log.note` itself. Without the second the row would be checked
where the value was handed over and then forgotten, which is where an effect system usually
starts leaking.

A value may perform *less* than its type allows. That is the one place in the checker where
one type fits another without being it, and it gives way in the safe direction: a function
that stays inside the room it was given breaks nothing, and one that goes outside it is
exactly what the row was written to catch. It is what lets a pure `doubled` be passed to
`apply_logging` without anybody writing a second overload.

Checking any of this needs both passes: which values owe a row is a question about types, and
whether a value keeps one is a question about rows, so the type checker records the places
and the effect checker settles them.

A declared function named where a value belongs is a value too, and its row is its contract.
It is not a closure: a closure carries no contract and a function does, so calling one that
arrived as a value goes through the same path a written out call takes, and its `where`, its
`ensures` and every refinement on it still run.

What is still unwritable is a function type that is polymorphic in its row, so a `map` has to
name what its callback may do rather than passing it through. That is the row polymorphism
question above, and rows on function types are what makes it possible to ask.

### Row variables

A row on a function type names an effect. What a combinator needs is to name *whichever* row
its callback has and pass it through to its own:

```vow
fn map<A, B, uses r>(items: List<A>, step: Fn(A) uses r -> B) -> List<B>
  uses
    r,
{
    for item in items with out = [] {
        push(out, step(item))
    }
}
```

Before this there were two ways to write that and both were wrong. `Fn(A) -> B` promises to
perform nothing, so the callback could not log or read a file. `Fn(A) uses Log.note -> B`
works for exactly one effect and needs a second copy for the next one.

**A row variable is declared, not inferred from where it turns up.** `<A, B, uses r>` is one
list holding everything a call has to work out, and `uses` says which kind each entry is. A
name that meant one thing in one position and something else elsewhere would be a thing a
reader has to work out, and `uses` is already a keyword so this costs the grammar nothing.

**Inside the body it is an ordinary row entry.** Calling `step` performs `r`, the contract
says `uses r`, and the same two rules that check every other function check this one. That is
why a variable that reaches no parameter is already an error: nothing can fill it, so nothing
performs it, so the row is too wide and `VOW5002` says so with no new code behind it.

**At a call site it is replaced by what was passed.** The variable is dropped, because it
names nothing a caller could declare, and the row of whatever went in at that parameter goes
in instead. So `map(ns, |n| n + n)` performs nothing and `map(ns, |n| { Log.note(..); n })`
performs `Log.note`, and the second caller has to say so. The library says "whatever you gave
me" and the caller says what that was.

**It crosses a module boundary as a position.** Which parameter a variable comes from is what
travels, for the same reason a type parameter travels as a position: a `DefId` is an index
into one module's table. `examples/list.vow` is a list library written in Vow and
`examples/using_list.vow` imports it, which is the test for whether any of this worked.

**It may only be written where a call can fill it in**, which means the row of a parameter
whose type is a function type, and the declaration's own `uses` clause. `VOW5008` refuses it
anywhere else. This follows from the paragraph above rather than being a separate idea: a
variable is replaced by what was passed, so if there is nothing passed at that position there
is nothing to replace it with, and the entry reaches a caller naming something the caller has
no word for. It would then be dropped, and a dropped entry in a row is an effect that happens
and is not declared.

The two positions this rules out are a return type, as in
`fn pick<A, uses r>(f: Fn(A) uses r -> A, ..) -> Fn(A) uses r -> A`, and a variable buried
inside a parameter rather than being that parameter's own row, as in
`steps: List<Fn(A) uses r -> A>`. Both used to check clean and both let an effect through. It
is the same rule as `VOW4023`, where a type parameter has to appear in a parameter's type so
that a call always knows what it is, and it is worth stating for the same reason: a signature
whose call sites cannot work out what it means is not a signature.

What is deliberately absent is row subtraction, or anything that says "this row minus `Log`".
A `with` block already handles an effect; saying that in a type is a much larger thing.

## The run checks too

Everything above is one pass deciding whether a program keeps its promises. For a long time
that pass was also the only thing that ever read a row, which meant a hole in it was a hole
nothing could see. Five separate ways of getting an effect past it were open at once, all
found by hand, all of them the same mistake: a function value arriving from somewhere the
pass did not recognise, given an empty row, and an empty row means "performs nothing".

So the interpreter holds the program to its own signatures while it runs. Each active call
carries the row its declaration wrote down, every effect performed is checked against every
call on the stack, and one that nobody on the stack declared is `VOW6010`. That is reported
against the compiler rather than against the program: the file was accepted, so if an effect
got through then the check that accepted it was wrong.

Three things are exempt, and each is a rule stated elsewhere rather than a hole here.

A `with` block discharges what is inside it, so a call is only asked about effects answered
by a handler installed before that call started. A contract does not contribute to a row, so
a `where` or `ensures` clause may read state without anything having to admit to it. And a
declaration whose row holds a row variable is not asked, because the variable stands for
whatever the caller passed and the declaration alone does not say what that was. The caller's
own frame is where that question has an answer, and it is still asked.

A handler operation is included, and it has to be handled carefully in both directions. It is
held to its own row, which matters because that is where an effect is implemented. But what
it performs is charged to whoever installed the handler and not to the frames in between: a
function calling `Log.note` does not choose the handler and cannot know what that handler
does, so asking it to declare that would be asking it to know something it has no way to
find out. The walk goes innermost first for exactly this reason, and stops asking at the
`with` block that installed the operation it is inside.

The check found something on the day it was written, which is the argument for it. A `with`
block discharged the effect a handler implements and charged what the handler itself performs
to nobody, so a function holding a `Console` could install a handler that writes to it and
declare an empty row. That is fixed above, and it is the kind of thing that was never going
to be found by reading the pass that had the bug.

## Open questions

- Charging a closure's effects to the call site rather than to its author. Now that a
  closure's row can be written down where it crosses a boundary, the conservative rule is
  doing less work than it used to. Changing it changes the soundness argument above, which is
  worth its own change rather than a line in someone else's.
- Whether a row variable should be able to appear in two parameters. Today the second one is
  checked against the first, the same way a type parameter's second occurrence is, so
  `fn both<uses r>(f: Fn() uses r -> (), g: Fn() uses r -> ())` forces two callbacks to have
  the same row. The useful version is a union, and a union is a different thing from a
  variable that stands for one row.
- Can rows be inferred well enough that most functions carry none, and does that then
  undermine the review argument, which depends on the signature being complete?
- How do effects interact with data structures. Does a `Map` holding closures need a row?
- What does an effect row mean across a network boundary, where the callee is not compiled
  with you?
