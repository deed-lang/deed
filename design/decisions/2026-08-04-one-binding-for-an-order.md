# Decision: one binding for an order

- Status: Accepted
- Date: 2026-08-04
- Supersedes: None
- Superseded by: None

## Context

`design/decisions/2026-08-03-operators-bound-to-functions.md` made `+`, `-` and `*`
bindable and left ordering out with one sentence: "Ordering is left out on purpose and is a
separate decision, because it does not stop at notation. `<` on a user type meets `sort`,
which takes a comparator today, and that is where the trait threshold sits."

That was a decision to decide later, not a refusal. This is the later.

The cost of leaving it out is visible in `std/ratio`, which has an exact order
(`is_below`, by cross multiplication) that no expression can spell. A program that moves a
quantity from `Int` to `Ratio` keeps `+`, `-` and `*` and loses `<`, so it rewrites every
comparison as a call. That is the same tax the arithmetic decision was made to remove, paid
on the half that was left.

## Decision

`<` is bindable, and one binding answers all four comparisons.

```deed
fn is_below(left: Ratio, right: Ratio) -> Bool { .. }

operator < = is_below
```

`a > b` is `b < a`. `a >= b` is not `a < b`. `a <= b` is not `b < a`. The operands are
swapped and the answer negated at the point of use, in both engines, from the single
binding.

Two things follow and both are the point.

**Four bindings would let them disagree.** A module could bind `<` and `>` to two functions
that answer differently about the same two values, and nothing would say so, because
nothing can check that a function orders any more than it can check that `+` adds. One
binding removes the possibility rather than the mistake.

**The shape is different from the other three, and it has to be.** An operator that
combines hands back what it was given, so that `a + b + c` reads the way it is written. An
order answers a question about two values, so it hands back a `Bool`. The shape check asks
for the right one per operator, and a binding that handed back its operand type is refused
with `DEED4031` saying so.

**This is notation, not dispatch.** `<` on a type parameter is still refused. `sort` still
takes a comparison. The trait threshold is exactly where it was: it is about generic code
choosing an implementation, and a binding is for a type the binding module names.

## Drawbacks (required)

**A module can bind `<` to a function that is not an order.** Nothing checks transitivity,
irreflexivity, or that it agrees with `==`. That is the same hole `+` already has, and the
derivation makes it wider: a function that is not a total order makes `a <= b` mean
something other than "below or equal", because the derivation assumes trichotomy.

**Two spellings for the same question.** `std/ratio` now has `is_below` and `<`, and a
reader has to choose. The function form is what a value needs, since it can be passed; the
operator form is what an expression needs. That is the same split the arithmetic decision
took, and it drifts the same way.

**`is_above` is now redundant.** It stays, because it is exported and removing it is a
breaking change for a reason no user asked for, and because passing a comparison is still
how `sort` is called.

**Nothing derives `==` from it.** A type with a bound `<` still gets structural equality,
and if the order disagrees with structural equality — two values that are neither below nor
above each other but compare unequal — the language will say both things. `Ratio` avoids
this by construction, since it is always in lowest terms, and nothing checks that a type
does.

## Rejected Ideas (required)

- Option: bind all four separately.
  - Rejected because: they could disagree about the same two values and nothing would say
    so. An order is one thing, and four bindings say it is four.

- Option: bind `<` and leave the other three unspelled, so `a > b` is written `b < a`.
  - Rejected because: it is the arithmetic tax again in a smaller place. A reader of
    `b < a` has to invert it mentally at every use, and the inversion is exactly what a
    compiler can do without being asked.

- Option: derive `<` from a three-way comparison instead, the way `Ord` does.
  - Rejected because: a three-way answer needs a type to be, and this language has no
    ordering type. Adding one would be a prelude data type and a bigger decision than the
    operator it exists to serve. `Bool` is the answer `<` already has on `Int`.

- Option: also make `<` work on a type parameter, so generic `sort` picks it up.
  - Rejected because: that is dispatch, which is the trait threshold, and this decision
    does not move it. `sort` taking a comparison is a working answer that costs one
    argument.

- Option: keep ordering out because it "meets `sort`".
  - Rejected because: it does not. A binding is looked up by the type the operands have,
    and a type parameter has no binding to look up, so nothing about `sort` changes.
    The original sentence read the meeting as unavoidable and it is not.

## Open Questions (required)

- Whether a type that binds `<` should be allowed to disagree with structural `==`. Nothing
  checks it and nothing could without a proof obligation. What would change this: a program
  in the corpus where the two disagree and something reads the difference.

- Whether `sort` should look for a binding when it is not given a comparison. That is the
  trait question wearing a different hat, and the answer is still that a type parameter has
  no binding.

- Whether the derivation should be stated in the language rather than performed by the
  compiler, so that a reader of `a <= b` can see it is `!(b < a)`. Today it is in this
  document and in `BinaryOp::from_less_than`.

## References

- `design/decisions/2026-08-03-operators-bound-to-functions.md`, which left this open.
- `crates/deed-ast/src/lib.rs`, `BinaryOp::BINDABLE` and `BinaryOp::from_less_than`.
- `crates/deed-driver/tests/operators.rs`,
  `one_binding_answers_every_comparison` and
  `an_order_answers_with_a_bool_rather_than_the_type_it_was_given`.
- `std/ratio.deed`, which binds it beside the function it means.
