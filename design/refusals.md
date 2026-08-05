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
for now: keep refusing every fractional number type. That page is no longer only an argument.
`std/ratio` ships, holds exact fractions as two `Int`s in a record, and `examples/logs.deed`
has the percentage column that raised the question. Writing it settled two of the three
worries: canonical equality turned out to be a constructor, and the proof model was never
asked anything, because no contract in the library says anything about a ratio. The third,
that `1/2 + 1/3` had to be spelled `added(half, third)`, was an argument about operator
overloading rather than about numbers, and that argument has since been made:
[decisions/2026-08-03-operators-bound-to-functions.md](decisions/2026-08-03-operators-bound-to-functions.md)
lets a module bind `+`, `-` and `*`, and `std/ratio` binds all three. What is left is a
contract that has to say something about a fractional quantity. Binary floating point is
still the wrong answer for money and decimal is still the leading candidate for stored base
ten quantities, but neither has a program here that wants one.

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

## No REPL

A Deed file parses as one `module`, an optional `edition`, zero or more `use` declarations,
and then items. The tree is the same shape: a `Module` has `uses` and `items`, and an item
is a declaration or a `test` block. A bare expression only exists inside a block as its tail
expression, so there is nowhere today to put `1 + 2` at a prompt without inventing a module,
a function, or both.

The rest of the language makes that invention load-bearing rather than cosmetic. `deed run`
enters through one `main`, handing it the `System` capability there is; a prompt would have
to decide which row the session holds before the first line was checked. Contracts are also
checked at call sites, and a prompt has no enclosing function body to prove a precondition
from, so prompt-time obligations would become guarded by default. That is an interesting
model, but it is a different surface from "evaluate this one expression".

The answer for now is a whole-program surface: a scratch `.deed` file locally, or a browser
playground built to the same shape. `deed check` answers whether the module is well formed,
`deed test` runs executable examples, and `deed run` executes a `main` that names its
capabilities up front. That matches the parser, the checker and the interpreter that already
exist, rather than teaching each of them a second top level.

**What would change the answer:** an explicit session model that says what synthetic module a
prompt grows, which capability row it holds, and how prompt-time contract obligations are
represented, very likely as guarded rather than proven.

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
twenty crates in this workspace. Declaring `Result` in a prelude module moves the first,
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

Measured, not assumed, and measured again rather than quoted. Running
`cargo run -p deed-driver --example edit_loop --release` reports that at 512 files a full
recheck costs about 42ms on one developer machine, inside the 100ms budget
[P9](01-principles.md) sets for the edit loop, and effectively all of that time is spent on
files that did not change. That last number is the one that matters: it says a cache is worth
writing when the corpus is large enough to need it, rather than on a feeling that it might
be, and nobody has written a few thousand files of Deed yet.

The number moved since it was first written down. It was 38ms, at about 70us a file; it is
42ms now, at about 82us. Two shipped modules and a larger corpus arrived in between, and the
per-file cost is what grew rather than the shape: the curve still flattens (60us at one file,
82us at 512), which is what `crates/deed-driver/tests/scaling.rs` guards. A number in a
design document that nobody re-runs is the thing this repository keeps finding, so it is
written here with the command that produces it.

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

## No detached spawn

A detached spawn starts a task and immediately disconnects: the spawning block exits while
the task keeps running, owned by nothing, with no scoped lifetime. Most concurrent languages
start here and then add structured concurrency on top, paying the cost of two models for the
lifetime of the language.

Deed chooses the opposite order. The decision is recorded in
`design/decisions/2026-07-31-structured-concurrency.md`. The short version: a task in Deed
is tied to the block that started it and cannot outlive it. That is the same shape as `with`,
which already says "inside this block, this effect is handled, and outside it, it is not".
A task group would say the same thing about tasks rather than effects, and the scoping
mechanism is the same.

The refusal is `DEED2014`. `spawn(f())` at statement level is read by the parser and refused
with a message that explains the structural alternative rather than treating `spawn` as an
unknown name. The conformance case at `conformance/cases/reject-spawn/` is the test where a
parent would return before its child, which is the exact shape this decision refuses.

**What would change the answer:** a program where scoped concurrency is demonstrably unable
to express something that detached concurrency can, not merely less convenient, and where
carrying both models is clearly cheaper than the structural change that would let scoped
concurrency cover the case.

## Checked against the compiler

Two different things on this page can go stale, and they need different checks.

**The refusals.** The float, trait, range, `while`/`break`/`continue`, and detached-spawn
refusals are each read by an existing test that keeps this page's claims tied to the parser's
and prelude's actual behaviour rather than to a description of it that can go stale. The
first four are read by `crates/deed-driver/tests/documentation.rs`; the spawn refusal is held
by `crates/deed-parser/tests/parsing.rs` and `crates/deed-cli/tests/conformance.rs`. The rest
are structural claims about the type checker and the backend that do not have a single
number to check against; they are read against the reasoning in `design/02-syntax.md`,
`design/04-capabilities.md` and `design/01-principles.md`, which is where each one was
first written down in full.

**The thresholds.** Every "What would change the answer" paragraph is a condition under
which a decision here should be reopened, and for a long time nothing watched any of them.
A refusal going stale is loud, because a `trait` keyword appearing would fail a test. A
threshold going quietly true is silent, and leaves this page saying a question is settled
after the thing that would unsettle it has happened.

`crates/deed-driver/tests/thresholds.rs` watches the three that a test can watch, and fails
when the decision should be reopened rather than when something regressed:

- the trait threshold, by sorting and rendering `Ratio` and `Date` with passed functions, so
  that a passed function ceasing to be enough is what fails
- the fractional threshold's second half, by counting the contract clauses in `std/ratio`,
  which is zero and is why the proof model was never asked anything
- the `Result`/`List` threshold's second half, by checking that `for` still walks a list and
  nothing else

Three are conditions somebody would have to build rather than conditions that become true on
their own: a session model for the REPL, a program scoped concurrency cannot express, and a
realistic codebase large enough that a full check is slow. A test for those would be a test
that watches nothing. The last one has the closest thing to a watch anyway, in
`crates/deed-driver/tests/scaling.rs`, which guards the shape of the curve rather than the
clock, and the incremental-checking decision names a change in that curve as half its
threshold.

The count of thresholds is itself pinned, so a seventh arriving is a decision about whether
it can be watched rather than something nobody looked at.
