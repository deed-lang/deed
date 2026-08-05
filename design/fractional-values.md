# Fractional values

`examples/logs.deed` is the first real program here that wants a fraction.

If its report grows a percentage column, the program has to say things like "2 of 3 lines were ERROR" and print `66.7%`, or "1 of 200 lines were WARN" and print `0.5%`. That quantity is not an integer. It is a ratio between two integers that stays exact until the last step, where the program chooses how many digits to print.

The money example does not force the question yet. It already stores the smallest unit as `Int`, which is an honest model for the program that exists today. Fractions would only become necessary there once the language needs rates, taxes, interest, exchange prices, or some other quantity that is not naturally counted in minor units.

Binary float is not the answer. A quantity that is written as one tenth and stored as a nearby approximation is already a bad fit for money, and Deed cannot hide that behind a type name because `==` is structural and total. If a value carries a float, equality has to say whether two nearby approximations are the same value, and either answer is dishonest somewhere. The checker also proves refinements with integer intervals and simple linear facts. Floating point arithmetic does not fit that model.

Fixed point or decimal are better candidates for money, but they are not the need `examples/logs.deed` shows first. A report percentage is not a stored base ten quantity. `2 / 3` does not have a finite decimal expansion, so the program must choose a scale and a rounding rule immediately. That is a presentation policy, not the shape of the underlying value. Choosing decimal because the first example wants a percentage would hard code a printing decision into the language.

Rational matches the report more honestly. The natural internal value for that percentage is `2/3`, not `0.667` and not `667` at scale three. But rational is not a small addition either. Structural equality is total, so `1/2` and `2/4` must compare equal, which means rationals need a canonical form with the sign in one place and the common divisor removed on construction. Refinements are intervals over one integer today. A rational value has at least two integers and an invariant on the denominator. `facts` also reasons in linear arithmetic over integers. Even proving `ratio < 1` or `ratio == other_ratio` wants cross multiplication and nonzero denominator facts, which is a different proof model from the one the language has now.

The decision is to refuse all fractional number types for now.

The real program that needs one first is a report, not a ledger. Its need is exact ratios that become text at the edge. That is enough to rule binary float out, and enough to say that picking decimal now would solve the wrong problem first. It is not enough to justify adding rational either, because doing so would reach into equality, refinements, and contract reasoning at once.

If `examples/logs.deed` needs percentages before the language gains a fractional type, the honest first move is a library level formatter that takes two integers and returns text. That keeps the rounding policy where the program can see it, and it does not pretend the language has settled what a fractional value is.

What would change the answer is a real program that must store, compare, and pass fractional quantities around as values rather than only print them. If that program is about money, decimal is the front runner because exact base ten arithmetic is the point. If that program is about exact ratios that participate in contracts, rational is the front runner, but only with an explicit design for canonical equality and a proof story that goes beyond integer intervals.

## What the library answered

The suggestion above was taken, and it went further than a formatter. `std/ratio` ships with the compiler and holds a `Ratio` as two `Int`s in a record. It adds, subtracts, multiplies, divides, orders, and renders to a chosen number of decimal places, and `examples/logs.deed` has the percentage column that motivated this page.

Three things came out of writing it, and they are the reason this page is now a measurement rather than an argument.

**Canonical equality was the cheap part.** `simplified` puts every ratio in lowest terms with the sign on top, and once it does, the language's own structural `==` says `2/4 == 1/2` without the checker knowing anything about ratios. The concern above was that structural equality forces a canonical form. It does, and a constructor is where it goes.

**The proof story came up, and it was smaller than the worry.** This paragraph used to say the question had never been asked, because no contract in `std/ratio` said anything. Contracts have since been written, and the answer is that a clause about a fractional quantity is a clause about integers: a `Ratio` is two `Int`s, so `ensures ok => result.bottom > 0` is a fact about a field the interval model already reasons about, and `where n > Int.min` is a bound on an ordinary number. Ninety-two of the obligations in that module are proven, three are tested by generated inputs, and eleven are guarded, and every guarded one is a call whose arguments are arithmetic rather than anything about fractions. The worry that a fractional type would reach into `facts` was a worry about a fractional *representation*; a fraction made of two integers reaches into nothing.

**Writing them found a bug nothing else had.** `absolute(n)` answers `0 - n` for a negative `n`, and the smallest `Int` has no positive counterpart, so `ratio(Int.min, 1)` overflowed and stopped the program. Nothing had noticed, because nothing had been asked to say what `absolute` promises. The contract is `where n > Int.min`, `ratio` turns the number away at the door, and everything below it is proven rather than checked again.

**One thing the checker could not follow.** The door is written `if top <= Int.min` rather than `if top == Int.min`. The two say the same thing about an `Int`, since nothing is below the smallest one, but a comparison narrows a range and a disequality does not: after `==` fails, all the checker holds is that one number is out. Written with `==` the call below is guarded; written with `<=` it is proven. That is a gap in the prover rather than in the design, and it is small, but it is the one place writing these contracts pushed on something.

**What the library could not have was operators, and now it has three.** This was written when `1/2 + 1/3` had to be `added(half, third)`, and it called that the whole remaining cost and an argument about operator overloading rather than about numbers. It was, and the argument got made: `design/decisions/2026-08-03-operators-bound-to-functions.md` lets a module say that `+`, `-` or `*` means a function it declares, and `std/ratio` binds all three. What is left of this paragraph is ordering. `first < second` is still `is_below(first, second)`, because `<` on a user type meets `sort` and that is the trait question rather than a question about notation.

The decision therefore stands, with the reason narrowed. Refusing float is still right. Refusing decimal is still right, because no program here stores base ten quantities. Refusing rational *as a language type* is now supported by something better than an argument: a rational library exists, it is exact, and it is written with the operators arithmetic is written with.

**What would change the answer now.** Both halves of the original worry have been tested and neither moved the decision. What is left is the thing neither half was about: a program that has to *store* fractional quantities, compare them across a boundary and hand them to something that keeps them, rather than compute with them and print them. Nothing in this repository does that yet. A contract that has to relate two ratios by something other than their fields would be the other one, and `<` on a user type is where that runs into the trait question rather than into numbers.

AI assistance: drafted with GitHub Copilot and reviewed by the author.
