# Effects

An effect is anything a function does that is not returning a value: reading a database,
getting the time, writing a log, failing to terminate.

In most languages these are invisible. A function typed `String -> String` might hit the
network and you would never know. Deed makes the effect row part of the signature, which is
what lets absence mean something.

## Declaring an effect

```deed
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

```deed
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

```deed
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

**The boundary does not soften a rule either.** A `uses` entry naming something that is not
an effect is an error whether the name was declared here or imported: which kind of thing it
is comes off the export, so the compiler is exactly as sure of it either way. That was a
warning on the imported side for a while, which read as if it could not tell, and it also
switched both rules above off for the rest of the function, so one wrong entry bought silence
about the whole row.

**What the boundary does lose is a starred entry.** A row is exported without its starred
entries and marked as not the whole story, and a caller is told at the call site rather than
quietly inheriting a row that is missing something and looking pure. The rule is the star and
only the star, read straight off the syntax, because a module's surface is computed before
anything is resolved and which kind of thing a name is does not show there.

That is right for `sys.*`, the case it was written for. It is too coarse in both directions
elsewhere, and both directions are reachable today. `Log.*` on a declared effect means the
whole of `Log`, which a caller can read perfectly well; the declaration is checked and stays
silent, and every call site is told the row is not checked anyway. A bare `sys` is not
starred at all, so it crosses as an ordinary entry, and the caller is told to import a name
that is one of the callee's parameters. Deciding this properly means resolving names before
computing exports, which is a change to how rows cross the boundary rather than to what the
message says, so it is an open question below rather than a fix in passing.

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

There is a second cost, and that one was wrong. Contributing nothing to the row was read as
permission to reach an effect the row does not mention at all, and a clause that does is a
call that passes every check and then cannot run: it needs a handler, installing one is the
caller's job, and the signature is the only place the caller could learn that. So the rule
has a floor. **A contract may perform an effect only if the signature names it.**

Naming it is the whole requirement. The operation is still free, which is what keeps the
paragraph above true: `transfer` reads `Ledger.total()` in an `ensures` clause, declares
`Ledger.balance` and `Ledger.post` and not `Ledger.total`, and is fine, because a handler is
installed for an effect rather than for an operation. Asking for the operation would not even
be available as a choice, since the tightness rule would immediately reject the entry as
declared and never performed.

That last part cuts the other way too. A function whose body performs nothing and whose
postcondition reads the ledger has to be able to write `uses Ledger.balance`, so an entry
counts as used when the body or the contract performs it. Before that, the honest row was an
error and the dishonest one was accepted, which left the shape with no writable form.

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
here. `examples/counter.deed` and `examples/transfer.deed` are the versions that run.

```deed
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

```deed
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
operation that does not line up with the effect is `DEED4021`: an operation the effect never
declared, or one taking a different number of arguments.

The same claim read from the other end is `DEED4029`: a handler implements every operation
its effect declares, or it is not a handler for that effect. Only one of those two directions
was checked for a while, so a handler could leave an operation out and the program found out
when a call reached the gap. Half a handler is not a smaller handler, because a `with` block
discharges the effect rather than the operations written inside the braces. Installing one is
a claim that every call underneath has somewhere to go, and a caller declaring
`uses Counter.total` is taking that claim at its word.

**Installing a handler is a decision, and it costs what the handler costs.** A `with` block
answers for the effect the handler implements, which is what a handler is for. It says
nothing about what the handler does to implement it, and a handler that writes to a console
is a program writing to a console. Those effects go to whoever installed it:

```deed
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
it, and declares an empty row. That was true for a while. It is `DEED5001` now, and it works
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

Deed does not have a complete solution to that yet. The ideas on the table are:

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

### A closure may not be written over handler state

Handler state is the only mutable thing in the language and its lifetime is the `with` block
that installed the handler. Everything that reads it runs inside one of that handler's
operations, so the question of which handler a read belongs to never comes up.

Except for a closure. A closure captures the frame, and state is not in the frame; it is in
the handler instance. So a closure written inside a handler operation and called later read
whichever handler was innermost when the call landed. Called after the operation returned
that was no handler at all, and called inside another handler's operation it was that
handler's table. When the two handlers happened to use the same state name there was no
refusal and no message: `deed check` exited 0, the run exited 0, and the closure answered
with the other handler's number.

There were two ways out and they are not the same size.

The obvious one is to capture the handler along with the frame. It gives the right answer to
the case above, and it changes what a value is: a closure would keep a handler alive past the
`with` block that installed it, so the program would hold a mutable cell with no name, no
declared lifetime, and two live copies of "the" state of one handler as soon as the block ran
twice. The reason to refuse it is not that, though. It is that the closure's type would not
say any of it. `Fn() -> Int` says the value takes nothing, hands back an `Int` and performs
nothing, and the section above argues that a row left off a function type cannot mean "any
row", because a signature here is complete. A value that is also a live window onto one
particular handler's state carries an input and a lifetime through a signature that mentions
neither, and there is no notation for either. Whatever `with` discharges, it would stop being
the whole story about what installing a handler costs.

So it is refused instead, at the closure, as `DEED4030`. What to write is the snapshot the
rest of the language already takes:

```deed
handler A implements Give {
    state n: Int

    fn getter() -> Fn() -> Int {
        let current = n
        || { current }
    }
}
```

The closure now carries a number, which is what `Fn() -> Int` said it carried.

**What this rules out.** A closure cannot give a live view of handler state, so a handler
cannot hand out something that reads or writes its state later, and a handler holding a
capability in its state cannot let a closure carry that capability out of the `with` block.
It also refuses closures that never leave: one written inside a handler operation and called
on the next line is refused too. That is the price of the rule being lexical, and lexical is
deliberate. Deciding which closures escape is escape analysis, and not having to answer that
question is the same reason a closure's effects are charged to whoever wrote it rather than
to whoever calls it. A closure that is called on the next line is one line away from being
returned instead, and a rule that changes its answer over that line is a rule nobody can
apply while reading.

**What it leaves alone.** A closure written anywhere else is untouched, because handler state
is in scope inside the handler that declared it and nowhere else, so there is no name to
reach it by. Closures still cross into handler operations as arguments, still get stored in
handler state and called later, and still leave a `with` block carrying whatever they were
handed. `DEED4030` is about one thing: a closure written where the state is in scope, naming
it.

### Function values

A function value can cross a boundary, and the type it crosses through says what it may do
on the way:

```deed
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

```deed
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
performs it, so the row is too wide and `DEED5002` says so with no new code behind it.

**At a call site it is replaced by what was passed.** The variable is dropped, because it
names nothing a caller could declare, and the row of whatever went in at that parameter goes
in instead. So `map(ns, |n| n + n)` performs nothing and `map(ns, |n| { Log.note(..); n })`
performs `Log.note`, and the second caller has to say so. The library says "whatever you gave
me" and the caller says what that was.

**One variable in two parameters means the union of the two.** It does not force the two
callbacks to have the same row. A variable is not a name for one row that every place
carrying it has to agree on; it is a name for the places a call reads a row off, and the
declaration's `uses` is charged with the sum of them. So

```deed
fn filtered_with<T, uses r>(
    items: List<T>,
    keep: Fn(T) uses r -> Bool,
    dropped: Fn(T) uses r -> (),
) -> List<T>
  uses
    r,
```

takes a pure `keep` and a `dropped` that logs, and the caller declares `Log.note` and nothing
else. This follows from the paragraph above rather than being a second rule: each occurrence
is replaced by what was passed there, and two replacements are two rows.

Writing two variables instead, `uses r, uses s`, says the same thing at the call site. Both
are unioned into the caller either way, so separate names are not a way to keep two
callbacks' rows apart. Where the two spellings differ is inside the declaration, where each
name has to be performed for its own entry in `uses` to be justified.

**It crosses a module boundary as a position.** Which parameter a variable comes from is what
travels, for the same reason a type parameter travels as a position: a `DefId` is an index
into one module's table. `std/list` is a list library written in Deed and
`examples/using_list.deed` imports it, which is the test for whether any of this worked.

**It may only be written where a call can fill it in**, which means the row of a parameter
whose type is a function type, and the declaration's own `uses` clause. `DEED5008` refuses it
anywhere else. This follows from the paragraph above rather than being a separate idea: a
variable is replaced by what was passed, so if there is nothing passed at that position there
is nothing to replace it with, and the entry reaches a caller naming something the caller has
no word for. It would then be dropped, and a dropped entry in a row is an effect that happens
and is not declared.

The two positions this rules out are a return type, as in
`fn pick<A, uses r>(f: Fn(A) uses r -> A, ..) -> Fn(A) uses r -> A`, and a variable buried
inside a parameter rather than being that parameter's own row, as in
`steps: List<Fn(A) uses r -> A>`. Both used to check clean and both let an effect through. It
is the same rule as `DEED4023`, where a type parameter has to appear in a parameter's type so
that a call always knows what it is, and it is worth stating for the same reason: a signature
whose call sites cannot work out what it means is not a signature.

What is deliberately absent is row subtraction, or anything that says "this row minus `Log`".
A `with` block already handles an effect; saying that in a type is a much larger thing.

### An effect takes them too

An effect declares row variables the way a function does, and they are in scope in its
operation signatures and in the state of any handler that implements it.

```deed
effect Task<uses r> {
    fn fork(step: Fn() uses r -> ()) -> ()
    fn more() -> Bool
    fn step() -> ()
}

handler RoundRobin implements Task {
    state queue: List<Fn() uses r -> ()>
    ..
}
```

This is one rule applied to a second declaration rather than a second rule. `r` belongs to
the effect, so an operation signature and a handler's state that both write it are talking
about one variable, and it is filled in at the calls that supply it: `Task.fork(step)`
charges the caller with whatever `step` performs, exactly as a call to a function with a row
variable does. A program that forks a task which logs is a program that logs, and its row
says so.

A handler's `state` is the one position an effect's variable has that a function's does not.
It is not a signature, so nobody reads it at a call and nothing has to be able to fill it in
from there. What the queue holds is decided by the forks, and those are checked where they
are written.

And by the installation, which is the one way into a handler's state that does not go through
an operation. `with RoundRobin { queue: [noisy] }` puts a task in the queue without a
`Task.fork` anywhere, so a function value written into a handler at a `with` is charged to
the function that wrote it there, the same as one passed at a fork. What that does not reach
is a value arriving from a call, as in `queue: tasks()`: what `tasks` built is a question
about its body rather than about this expression.

`std/task` is the reason this exists. A library scheduler cannot know what the tasks it is
handed will perform, so before this its queue had to name a concrete row and every program
that wanted a scheduler had to copy one. `examples/scheduler.deed` is such a copy, kept for
the comparison: its queue holds `Fn() uses Schedule.fork, Schedule.yield, Log.note -> ()`,
and `Log.note` is in there because those particular tasks log.

Everything the previous section rules out is still ruled out for an effect's variable, and
for the same reasons. An operation's return type cannot carry one, and neither can a variable
buried inside a parameter that is not itself a function type, as in
`fn fork_all(steps: List<Fn() uses r -> ()>)`. `DEED5008` covers both.

What an effect does not take is a type parameter. A row variable costs nothing to carry
because it is erased; a type parameter would have to reach the handler, which is a value with
state and a single installation, and what `with Q { items: [] }` means when two different
`T`s go through it is not decided. `DEED2024` says that rather than leaving the reader with a
name-resolution error from inside an operation signature.

What has not crossed yet is the boundary. A row variable travels as a position, which is what
a call needs and not what a declaration needs, so a handler for an imported parameterised
effect cannot name the variable. `std/task` ships its handler alongside its effect, which is
what a program wants anyway, and `crates/deed-driver/tests/row_parameters.rs` holds the edge.

## The run checks too

Everything above is one pass deciding whether a program keeps its promises. For a long time
that pass was also the only thing that ever read a row, which meant a hole in it was a hole
nothing could see. Five separate ways of getting an effect past it were open at once, all
found by hand, all of them the same mistake: a function value arriving from somewhere the
pass did not recognise, given an empty row, and an empty row means "performs nothing".

So the interpreter holds the program to its own signatures while it runs. Each active call
carries the row its declaration wrote down, every effect performed is checked against every
call on the stack, and one that nobody on the stack declared is `DEED6010`. That is reported
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

- Whether a row can cross a module boundary knowing which of its entries are effects. Today
  the exported row is lowered from syntax alone and a call into a callee with any starred
  entry is reported, which is right for `sys.*` and wrong for `Log.*`, and misses a bare
  capability entirely. Answering it means resolving names before computing exports, and
  exports being computable first is what keeps module order from mattering.
  Measured: no starred entry (`uses X.*`) appears anywhere in `examples/` or `std/` today, so
  this imprecision has no real case behind it yet. Decided to keep the order-independence
  property rather than split exports into a pass that runs after resolution; revisit the day
  a program actually writes `uses Log.*` across a module boundary.
- Charging a closure's effects to the call site rather than to its author. Now that a
  closure's row can be written down where it crosses a boundary, the conservative rule is
  doing less work than it used to. Changing it changes the soundness argument above, which is
  worth its own change rather than a line in someone else's.
  Looked at again: the only case where this changes anything is a closure written but never
  called, and catching that needs escape analysis, a second inference pass for a case nobody
  has written. Kept the current rule: a closure's effect is charged to whoever wrote it.
- Can rows be inferred well enough that most functions carry none, and does that then
  undermine the review argument, which depends on the signature being complete?
  Not attempted, leaning no: an omitted `uses` clause defaulting to "infer it" is the silent
  effect P5 refuses everywhere else. Every function in the corpus already writes its own row,
  so there is no evidence either way yet, only the principle this would cut against.
- How do effects interact with data structures. Does a `Map` holding closures need a row?
  Measured: nothing in `examples/` or `std/` stores a closure in a Table, list element or
  record field; every closure in the corpus is passed as a plain parameter. `Types::
  function_rows()` keys by where a closure was written, not by where a value carrying it
  ends up, so a stored closure would need to carry its row through the value instead. No real
  case forces this yet.
- What does an effect row mean across a network boundary, where the callee is not compiled
  with you?
