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

Today the Proven tier holds constant expressions and nothing else, which is a thin slice of
what it should eventually cover. Everything it cannot discharge becomes Guarded and says so
as a warning at the point of use. Thin is fine. Silent would not be.

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

`Result` has no methods. There is no `unwrap`, and there is no pattern for matching on it
yet, so the only thing you can do with one is compare it or propagate it. That is a gap, and
it is listed below rather than hidden.

Error types are `choice`s, matching is exhaustive, and adding a variant breaks every caller
that has to care, on purpose.

## Pattern matching

```vow
match result {
    Ok(receipt) => log(receipt.id),
    Err(InsufficientFunds { available }) => notify(available),
    Err(AccountClosed { account }) => escalate(account),
    Err(LimitExceeded) => retry_later(),
}
```

Exhaustive. No fallthrough, and no catch-all arm when the scrutinee is a `choice`. A
wildcard there would mean adding a variant stops being a compile error, which is the entire
value of having variants. Where the cases cannot be enumerated, such as matching an `Int`, a
wildcard is fine and necessary.

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
value > 0` has nothing else to talk about. It is the one name the language introduces
implicitly.

**The prelude is seven names:** `Int`, `String`, `Bool`, `System`, `Result`, `ok`, `err`.
Everything else is imported. Each prelude entry is a name that cannot be looked up in any
file, which is the kind of thing P2 is a budget for, so the list is short on purpose.
**Names reached through an import are not checked.** `Ledger.read` where `Ledger` came from
`use ledger.{Ledger}` is left alone, because the compiler has not loaded that module and
cannot honestly say anything about its contents. When cross module loading exists this
becomes a real check.

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

**The prelude is seven names:** `Int`, `String`, `Bool`, `System`, `Result`, `ok`, `err`.
Importing a name the language already provides is a warning, because silently shadowing a
builtin would put everything that depends on it quietly back to being unchecked.

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

- There is no way to take a `Result` apart. No `unwrap`, and no `ok(x)` or `err(e)` pattern
  for `match`. Comparing the whole value works and propagating with `?` works, and that is
  not enough for real code.
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
