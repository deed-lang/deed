# Syntax

A sketch, not a grammar. Nothing here is settled. The goal is to have something concrete
enough to argue with, and concrete enough to write a lexer against.

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

What the caller must guarantee. Checked at the call site when it can be proven statically,
and compiled into a check at the boundary when it cannot. A precondition failure is a bug
in the caller, and it is reported that way.

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

All three tiers exist. `Tested` covers pure functions whose parameters can be generated:
`vow test` runs a hundred generated inputs against the contract and shrinks any
counterexample it finds. Everything else is `Guarded`, checked on every call.

### What `Proven` can decide

Interval reasoning, and nothing more. Each integer in scope has a known range, and a
refinement is discharged by evaluating its predicate over that range rather than over a
value. Ranges come from the things that state one:

- a `where` clause, so `n > 0` makes `n` at least one for the whole body
- a parameter already of a refined type, so a `Positive` parameter needs no `where` clause
  repeating the type in prose
- the condition of an `if`, narrowed one way in the then branch and the other way in the else
- a guard that leaves, so after `if n <= 0 { return err(..) }` the rest of the body knows
- the contract of a function being called, so a proof one function did is worth something to
  the next one

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

What crosses a module boundary is the range, not the predicate. A refinement stays opaque from
outside, which is the rule modules already had, so an exported `fn one() -> Positive` arrives
as a pair of bounds. That is the difference between exporting a proof and exporting the
conclusion of one.

That last group is what made this worth building. Before it, `Proven` held constant
expressions and nothing else, so a refinement in real code was a runtime check with a
paragraph of ceremony around it, and the argument for having refinements at all is that they
replace checks rather than decorate them.

The branches are checked while their facts are still in scope, which is the reason the
checker pushes an expected type down through an `if` rather than inferring the whole thing
and comparing at the end. That is local bidirectional checking and it exists for exactly this.

### What `Proven` cannot decide

Every one of these is `Guarded`, with a warning, never a wrong answer.

- **A relationship between two names.** An interval cannot hold `a < b`, so a `where
  low < high` proves nothing about `high - low`. This is the largest limitation and the
  first one anyone will hit. The same thing stops an `ensures ok => result == n` from
  saying anything at a call site: it is true, it is useful, and it is not an interval.
- **Arithmetic that could overflow.** `n + 1` where `n` is `Positive` is not provably
  positive, because `n` could be the largest integer there is. That is the reasoning working
  rather than a gap in it, and the runtime agrees: the sum has no answer.
- **The payload of a call that can fail.** The call site holds a `Result`, not the value
  inside it, so an `ensures` on a fallible function is not read.
- **Anything that is not an integer.** No `String`, no record field, no variant.
- **Division and remainder.** The sign rules around zero and around the smallest integer are
  fiddly enough that getting them wrong is worse than not trying.

A solver would decide most of these and would be a hard dependency at check time, which P9
has a budget against. Whether that trade is right is an open question, not a settled one.

Generation discards inputs that violate a `where` clause rather than reporting them, since a
bad input makes the generator a bad caller and the runtime already says so. If too many get
discarded, that is reported: a property that only tested a handful of inputs is worse than no
property, because it looks like one.

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
fn find_fixpoint(f: Fn(State) -> State, start: State) -> State
  uses Diverge
{
    ...
}
```

Functions are total by default. A loop the compiler cannot show terminates requires
`Diverge` in the row. This costs almost nothing to write and turns "does this ever return"
into something visible in the signature.

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

**There are no float literals.** That is what makes `40.try` unambiguously `40`, `.`, `try`
with no lookahead. If floats ever arrive, they will need a rule for that, and it is a debt
worth naming now rather than discovering later.

**Statements need no separator.** A newline is enough and `;` is accepted but never
required. This works because no statement can begin with a token that continues the
previous expression, which is a property that has to be maintained deliberately rather than
one that holds by luck. It is listed as an open question below.

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

**The prelude is ten names and one effect:** `Int`, `String`, `Bool`, `Result`, `ok`,
`err`, `System`, `Console`, `Clock`, `Dir`, and the `Io` effect with its `write`, `now`,
`open` and `read` operations. Everything else is imported. Each prelude entry is a name that
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

**Ordering is not tied to anything yet.** `<` currently insists only that both sides have
the same type. There are no traits, so there is nothing better to insist on. Listed below.

**`?` is checked.** The operand must be a `Result`, the enclosing function must return one,
and the error types must line up.

**`ok(x)` and `err(e)` each say nothing about the other side.** `ok(x)` has type
`Result<T, _>` and the unknown half agrees with whatever the expected error type turns out to
be. That is what makes them work with no unification anywhere, and it is the whole reason the
unknown type absorbs rather than unifies.

**The prelude is ten names and one effect:** `Int`, `String`, `Bool`, `Result`, `ok`,
`err`, `System`, `Console`, `Clock`, `Dir`, and `Io`. Importing or declaring a name the
language already provides is a warning, because silently shadowing a builtin would put
everything that depends on it quietly back to being unchecked.

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

This is a rule rather than a limitation. Mutation exists exactly where effects are
implemented and nowhere else, which is what lets an empty effect row mean that a function
cannot observe or cause a change to anything. Without the restriction, purity would be a
claim about IO only, and it is supposed to be a claim about everything.

It also keeps P1 intact. A mutable local would mean a name's value depends on where you are
in the function, which is the same objection that made shadowing an error.

The cost is that accumulator loops have to be written some other way. There are no loops
yet, so that bill has not arrived.

## Open questions

- Property tests only cover pure functions. Running an effectful one needs a handler, and
  inventing a handler means inventing the behaviour the property would then check against
  itself. A module declaring exactly one handler per effect might be a defensible default.
- Shrinking handles integers by binary search and record fields greedily. Nothing else
  shrinks, so a counterexample built from strings or nested choices comes out as generated.
- Ordering operators accept any two operands of the same type, including ones where ordering
  is meaningless. This needs a trait, or a fixed set of orderable types, and has neither.
- Refinements have no conversion form, so the only values that can enter a refined type are
  ones the compiler can evaluate. Real code will need `Positive.try(n)` or something like it,
  returning a `Result`.
- Banning shadowing outright may turn out to be more annoying than it is worth. It is easy
  to relax later and hard to tighten, which is the only reason it starts strict.
- Capitalisation carrying meaning in patterns is a wart, even though it earns its place.
  Explicit variant syntax would avoid it and would cost more to write everywhere else.
- The `with` handler list is disambiguated by a lookahead hack: a brace opens a struct
  literal only when followed by `name:`, which is what separates `with H { a: 1 }, Other`
  from the block that follows the handler list. It works for everything written so far and
  it is not a rule anyone should have to know. One concrete symptom: `with H { } { ... }`,
  a handler with no state, cannot be parsed at all. Better ideas welcome.
- Statement separation relies on no statement being able to start with `(`, `-`, `[` or `.`.
  That holds today and nothing enforces it. Either the grammar should guarantee it or
  statements need a real terminator.
- Generics: how much is needed before P2 breaks. Higher-kinded types are almost certainly out.
- Whether traits can be implemented outside the defining module, and what that does to
  local reasoning.
- `unchanged(E)` needs a definition. Handler state being the only mutable thing makes one
  possible, comparing that state before and after, but nothing implements it yet.
- Whether `uses sys.*` is a hole big enough to make `main` useless as a boundary.
- Concrete syntax for effect handlers, currently only sketched in 03.
