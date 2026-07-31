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

AI assistance: drafted with GitHub Copilot and reviewed by the author.
