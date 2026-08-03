# Decision: an operator can be bound to a function a module already declares

- Status: Accepted
- Date: 2026-08-03
- Supersedes: None
- Superseded by: None

## Context

`std/ratio` closed the fractional-number question in the library rather than in the
language, and `design/fractional-values.md` said what that left behind: `1/2 + 1/3` has
to be spelled `added(half, third)`. `crates/deed-driver/tests/thresholds.rs` names the
same thing and is careful about what it is: "an argument about operator overloading
rather than about numbers."

That argument has not been made, and `design/decisions/2026-07-31-no-traits-for-now.md`
turned down the nearest thing to it. Its reasoning matters here, because the reason it
gave was not that operators are unwanted: it was that the measured examples stayed
*writable* with passed functions, so dispatch was not worth its cost. That reasoning
covers a generic sort over a type parameter. It does not cover this, and the difference
is the whole decision.

What is actually lost today is not the ability to add two ratios. It is that the
language's own precedence stops applying to them. Written out, a formula over `Int`

```
(a + b) * c - d
```

becomes, over `Ratio`

```
subtracted(multiplied(added(a, b), c), d)
```

The operators are gone and so is their precedence, so the author re-encodes it as
nesting, by hand, correctly, every time. Reading order inverts: the first operation is
now the innermost. And the type is supposed to be the thing that changed. A program that
moves a quantity from `Int` to `Ratio` rewrites every expression that touches it, not
because the arithmetic differs but because the notation for it does.

None of that needs traits, dispatch, or inference. Both operand types are known where
the operator is written, so the question "what does `+` mean here" is answered by the
same table lookup that answers "what does this call go to".

## Decision

A module may say that an operator means one of the functions it already declares:

```
fn added(left: Ratio, right: Ratio) -> Ratio { .. }

operator + = added
```

- The three total arithmetic operators, `+`, `-` and `*`, and nothing else.
- Both parameters and the return type must be the same type, and that type must be
  declared in the module doing the binding. There are no orphan bindings.
- The function must perform nothing. Its row must be empty.
- The binding is a binding, not a second definition. `added` stays a name, stays
  callable, and stays passable as a value.
- The operator is resolved from the types at the place it is written. A bare type
  parameter is still refused, which is the trait question and is untouched.

`==` is not on the list and will not be. Structural equality is total and applies to
every type, and several things in this language rest on that being true of every value
rather than of the values somebody wrote a function for.

Division and remainder are not on the list either, and the line is that operators are
for total operations. `divided` returns a `Result` because dividing by zero is a real
answer a caller has to handle, and the language already spells partial answers that way.
An operator that returns a `Result` would give `a / b + c` a shape nobody expects.

Ordering is left out on purpose and is a separate decision, because it does not stop at
notation. `<` on a user type meets `sort`, which takes a comparator today, and that is
where the trait threshold sits. Arithmetic does not touch it.

## Drawbacks (required)

Two ways to say one thing. `added(a, b)` and `a + b` are now both spellings of the same
call, where `Int` addition has only the operator. The split is not arbitrary -- the
function form is what a value needs, since it can be passed, and the operator form is
what an expression needs -- and it is the same split `is_below` and `<` already have.
But it is a choice a reader of `std/ratio` now has to make, and two spellings drift.

A module can bind `+` to a function that does not add. Nothing checks that `+` is
commutative, or that `-` undoes it, or that the three agree with each other. The
language has no way to state such a thing about `Int` either, but on `Int` there is only
one implementation and it is the compiler's.

The binding is invisible at the call site. A reader looking at `a + b` in a file that
imports `Ratio` has to know the type of `a` to know which function runs. That is true of
every language with operator overloading, and it is why this one keeps the set small and
the rule mechanical.

## Rejected Ideas (required)

- Option: declare the operator as its own item, `operator +(left: Ratio, right: Ratio) -> Ratio { .. }`.
  - Rejected because: it would make the operator the definition, so a module wanting
    both a passable function and an operator would write the body twice or write a
    wrapper. `std/ratio`'s `added` is already passed to a fold. Binding costs one line
    and keeps one body.

- Option: attach operators to the record declaration, the way a method would attach.
  - Rejected because: there are no methods here, and the receiver position is exactly
    what the language does not have. A binding at module level says the same thing
    without introducing the first place where a function belongs to a type.

- Option: allow any operator, including `/`, `%` and the comparisons.
  - Rejected because: `/` and `%` are partial and `Result` is how this language says so;
    ordering is a larger question that runs into generic sorting. Both can be added
    later without changing what is decided here, and neither can be removed later.

- Option: allow mixed operand types, so `Ratio * Int` could scale.
  - Rejected because: it multiplies the table by the pairs rather than the types, and
    `scaled(value, factor)` already exists and says which side is which. If a real
    program needs it, the need will name the pair.

- Option: add traits and hang operators off them.
  - Rejected because: this is the same decision `2026-07-31-no-traits-for-now.md` made,
    and nothing here reopens it. An operator bound to a named function needs no
    dispatch, no coherence rule and no bound on a type parameter.

## Open Questions (required)

- Ordering: `<` over a user type. It is the half of this that meets `sort` and the
  trait threshold, and it should be decided against a program rather than against
  taste.

- Whether a contract clause should be allowed to use a bound operator. It can today,
  because the clause is an expression and the operator is a call to a function that
  performs nothing, but the prover reasons about `Int` ranges and will answer Guarded
  about anything else. `thresholds.rs` counts contract clauses in `std/ratio` for
  exactly this reason, and the count is still zero.

- Whether a module should be able to bind an operator for a type it imported. Refused
  here, so that the meaning of `+` on a type is decided in one file, which is the same
  reasoning that keeps module resolution free of a search path.

## References

- deed-lang/deed#896
- `design/decisions/2026-07-31-no-traits-for-now.md` (the decision this does not reopen)
- `design/fractional-values.md` (the threshold that named this)
- `crates/deed-driver/tests/thresholds.rs` (the test that watches it)
- `std/ratio.deed` (the module that needed it first)
