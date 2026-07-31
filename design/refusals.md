# What Deed deliberately does not have

A language is mostly the things it refuses. This one has refused an unusual number of them
for written reasons, and every one of those reasons currently lives somewhere else: a
design document, a pull request body, a comment in the compiler. Collected in one place,
this is the clearest statement of what Deed is, and it answers the question every
experienced reader asks first: what did you leave out, and did you think about it.

The rule for this page: nothing goes on it without the reasoning, and without what would
change the answer. A list of refusals with no reasons reads as arrogance. The same list with
reasons reads as a design.

## No traits

Answered in PR [#246](https://github.com/deed-lang/deed/pull/246), and not reopened by
[#617](https://github.com/deed-lang/deed/issues/617). The usual motivation was mostly
absorbed by three decisions made for other reasons: equality is structural and total, so
`==` works on a bare type parameter and needs no bound; a function is a value, so a caller
that needs different behaviour passes one; and effect rows travel with function types, so a
passed operation already says what it is allowed to do, which is more than a trait would
say. What is left (`ok`/`err`, `length` on two receivers, ordering on a type parameter, `for`
over a `String`) each has a cheaper answer than a trait system.

`examples/tree.deed` found the closest thing to a crossing: nothing checks that two calls to
a keyed tree passed the same comparator. [#617](https://github.com/deed-lang/deed/issues/617)
answered why that does not reopen the question either: a hashed collection needs no bound at
all, because a structural hash is the same shape of claim as structural equality, and the
comparator gap is specific to *ordering*, which has no structural default the way equality
does.

**What would change the answer:** a program that needs a generic sort over a user type, or
needs to print a `T`, where a passed function is not merely uglier but impossible.
`examples/ranking.deed` tried both and neither was unwritable.

## No float literals

`1.5` is refused as `DEED1007`. [#210](https://github.com/deed-lang/deed/issues/210) added
that diagnostic because a reader from another language typed a decimal and got a warning
about a discarded value and a missing field, which was the wrong conversation entirely. That
diagnostic is a courtesy, not the decision: there is no fractional number of any kind here,
so an average, a percentage, a rate and a price are all currently unwriteable.

The absence of a float literal is also load-bearing elsewhere: it is what makes `40.try`
parse unambiguously as `40`, `.`, `try` with no lookahead. A fractional type, whatever shape
it takes, would need its own rule there.

**What would change the answer:** [fractional-values.md](fractional-values.md) takes the
first real case from `examples/logs.deed` and answers [#655](https://github.com/deed-lang/deed/issues/655)
for now: keep refusing every fractional number type. The first concrete need is an exact
ratio that only becomes text at the edge, which is not yet enough to choose one language
wide representation. Binary floating point is still the wrong answer for money, decimal is
still the leading candidate for stored base ten quantities, and rational is still the
leading candidate for exact ratios, but either one would need a new story for structural
equality, refinements, and `facts` beyond the integer interval model that exists today.

## No ranges

`for i in 0..10` is the first thing anyone writes who wants to count, and a range would
terminate perfectly well, so termination is not the argument. The argument is that a range
would be a second walkable thing, and the first program that genuinely needed to repeat
something a number of times got what it wanted from the prelude instead: `repeat(value,
count)` hands `for` the list it already knows how to walk, and `at` turns the index back
into the count it came from.

`repeat` itself cannot be written in Deed: having something a number of times has no list to
hand a `for`, so the only form left is a function that calls itself, which makes it
`Diverge` and spreads that to everything that uses it. That is the non-termination story
giving `for` its reason to exist, arriving from a direction nobody was watching for it.

## No `break`, `continue`, or `while` as a statement

`any` and `all` in `std/list` both wanted a branch whose only job was to notice the answer
was already in, which is control flow inside a fold, the thing a fold exists to not need.
Left without one, `any` over a thousand elements took a thousand turns to find the first.
That is the real case for `break` and `continue`, and it is answered by `for`'s own early
exit (`DEED2011` when the head does not declare one) rather than by adding either.

`while` as a *statement* stays absent because it cannot be shown to terminate. Adding it
would bring `Diverge` back into ordinary loops and undo the reason `for` exists at all.
`while` stays available as a name everywhere else, including as `for`'s own accumulator
keyword, on the same reasoning that kept `at` out of the keyword list.

## No search path, config file, or manifest

The unit of compilation is the set of files handed to the compiler on the command line, not
a discovered project root. `deed check src/` sees that set; `deed check one.deed` sees one
module with an empty universe, in which any `use` fails. That is a real cost, and what it
buys is not having a second set of rules about roots, extensions and case sensitivity, which
is the part of every module system that goes wrong. Nothing is fetched and nothing is
versioned either: the standard library is embedded in the compiler binary, so a downloaded
`deed` carries it and there is no second thing that can be missing or the wrong version.

## No visibility modifiers

Every item a module declares is exported, including a choice's variants in their own right
(a `test` block is the one exception, since it is not part of what a module offers). A
language with no wildcard imports already makes every name a file pulled in visible at its
`use` line, and a `pub` keyword on top of that would be a second, weaker version of the same
guarantee.

## No positional variants

`Result`'s `ok`/`err` are the one place a language-defined constructor reads like a function
call rather than a record literal, and that exemption is deliberate rather than a general
allowance: see "`Result` and `List` staying in the language" below for why moving away from
it is a bigger change than it looks.

## `Result` and `List` staying in the language rather than moving to a prelude module

Looked at more than once, most recently for the shape of `ok`/`err`. `Ty::Result` and
`Ty::List` are named in the type checker and in both crates the backend added, since a list
has to be laid out in memory and compiled as well as checked. What actually names `Result`'s
two variants, `?` and the outcome an `ensures` clause is keyed by, reaches into nine of the
seventeen crates in this workspace. Declaring `Result` in a prelude module moves the first,
small set of references and leaves the second, load-bearing set exactly where it was, with a
module lookup added in front of a type the syntax already knows the shape of: not a smaller
language, the same one with an indirection in it. `List` is the same shape: `[1, 2, 3]` has
to build something, and `for` is the only loop in the language and walks a list and nothing
else.

**What would change the answer:** a rule saying which variant of a two-variant choice means
"stop", so `?` and an outcome-keyed `ensures` could name a shape rather than a specific type;
and something for `for` to walk that is not spelled `List`. Both are larger questions than
moving a type. Looked at again, no smaller version has turned up: a "which variant means
stop" rule has to generalise to any two-variant choice, not only ones spelled like `Result`,
and a second walkable built-in is, by construction, a second `List`. Still open.

## No incremental checking

Measured, not assumed. `crates/deed-driver/examples/edit_loop.rs` reports that at 512 files
a full recheck costs about 38ms on one developer machine, inside the 100ms budget
[P9](01-principles.md) sets for the edit loop, and 99% of that time is spent on files that
did not change. That last number is the one that matters: it says a cache is worth writing
when the corpus is large enough to need it, rather than on a feeling that it might be, and
nobody has written a few thousand files of Deed yet.

**What would change the answer:** a realistic codebase where a full check is slow enough
that the absence of a cache stops being a footnote, or a change to the scaling test's curve
that says the constant factor is growing.

## No second `Dir` type

Tempting four separate times, against four separate operations that each looked like they
needed one: writing, removing, listing, and making a directory. Every time, the same move
was rejected for the same reason: a second type naming "may write" or "may destroy" would say
the same thing an effect row already says (`uses Io.save`, `uses Io.remove`), and would have
to be threaded through every signature that already carries a `Dir`. The row says what kind
of operation; the argument says which resource. Splitting the type as well would say the
first thing twice.

`Io.make` looked at first like it broke the pattern, since it hands back a new `Dir`, which
reads like authority being created rather than narrowed. It is not: the set of paths a
caller can reach does not grow, only the set of things that exist inside a place it could
already reach does, and the `Dir` `Io.make` returns is always rooted inside the one that was
handed to it.

## Checked against the compiler

The float, trait, range and `while`/`break`/`continue` refusals are each read by an existing
documentation test (`crates/deed-driver/tests/documentation.rs`) that keeps this page's
claims tied to the parser's and prelude's actual behaviour rather than to a description of
it that can go stale. The rest are structural claims about the type checker and the backend
that do not have a single number to check against; they are read against the reasoning in
`design/02-syntax.md`, `design/04-capabilities.md` and `design/01-principles.md`, which is
where each one was first written down in full.
