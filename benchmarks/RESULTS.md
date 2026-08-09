# What five runs said

One model, one build, six tasks, five times, and a control arm with the
compiler taken away.

This is a record, not a ratchet. Reproducing it needs a network and an API key,
and neither is allowed anywhere in this repository, so nothing in CI re-derives
these numbers. What CI does hold is everything the numbers are *about*: the
tasks, the scorer, and the diagnostics named below.

## The setup

| | |
| --- | --- |
| Model | `gpt-5.6-luna`, one family, temperature left at the provider's default |
| Compiler | one build, unchanged across all five runs |
| Tasks | the six in [`tasks/`](tasks), one language feature each |
| Runs | five, recorded 2026-08-07 |
| Arms | `mcp` (the model may call `deed mcp`) and `blind` (prompt only) |

Before this, the number of runs against any single compiler was one, and one
run cannot tell a fixed problem from a coin landing the same way twice.

## The scores

Each answer is checked against tests the task brought with it, which the answer
never sees.

| Run | Answered | Check | Pass their tests |
| --- | --- | --- | --- |
| 1 | 6/6 | 6 | 5 |
| 2 | 6/6 | 6 | 5 |
| 3 | 6/6 | 6 | 5 |
| 4 | 6/6 | 5 | 5 |
| 5 | 6/6 | 6 | 5 |

`stock` came back with one proven obligation in all five runs, which is the
number this benchmark exists to produce: the model did not merely write a
refinement, it wrote one the checker could settle at compile time rather than
defer to a runtime guard.

The task that fails is `total`, in every run, and it is not a compiler problem.
The answer starts a maximum from a caller-supplied fallback and never lets a
negative element past it, so a list that is entirely negative returns the
fallback. That is a wrong answer, not a missing message, and no checker catches
it. It is here because a benchmark whose failures are all the compiler's fault
is a benchmark that is measuring the compiler.

## The control arm

The same six prompts with the compiler taken away:

| Arm | Answered | Check | Pass their tests |
| --- | --- | --- | --- |
| blind, `gpt-5.6-luna` | 6/6 | 0 | 0 |
| blind, `gpt-5.6-sol` | 6/6 | 0 | 0 |
| blind, `gpt-5.6-terra` | 6/6 | 0 | 0 |

Six answers every time, and not one of them compiles. Deed is not in anybody's
training data, so this arm measures the floor: without something to ask, a
model produces a confident file in a language it has never seen.

⚠️ These were recorded on 2026-08-01, against an earlier build than the five
runs above. The comparison is therefore between arms **and** builds, which is
weaker than it looks; what it supports is "zero versus not zero", not a
difference of one or two.

## What the compiler was asked

One hundred and two tool calls across the five runs:

| Tool | Calls |
| --- | --- |
| `deed_check` | 83 |
| `deed_test` | 8 |
| `deed_fmt` | 8 |
| `deed_fix` | 3 |
| `deed_run` | 0 |
| `deed_explain` | 0 |

`deed_run` is zero for a reason that is written into the tasks: every prompt
asks for a module and no `main`, so there is nothing to run.

Three of the eight `deed_test` calls are the interesting ones. They came back
with a property test — a hundred cases, generated from the contract the model
had just written, passing — in a task whose prompt says "no tests". Nobody
wrote that test. A signature being a promise the compiler checks is the whole
claim of this language, and this is the smallest possible demonstration of it
happening to somebody who did not know it would.

## What it kept saying

The score is not the interesting column. This is: the same sentence gets said
in every run.

| Code | Times | Tasks | Runs | The sentence, most often |
| --- | --- | --- | --- | --- |
| DEED2003 | 49 | 6 | 5 | expected a declaration, found identifier `export` |
| DEED3001 | 44 | 3 | 5 | cannot find `head` in this scope |
| DEED2001 | 36 | 4 | 5 | expected `.` while parsing a `use` declaration |
| DEED2015 | 28 | 2 | 5 | match arms are separated by commas |

Forty-five of the forty-nine `DEED2003`s are the word `export`, in all six
tasks in all five runs. That one already says the right thing and already
carries a repair that deletes the word, so the measurement's verdict on it is
"nothing left to fix here". A measurement that can return that is worth more
than one that only ever returns work.

The one still worth fixing came from the third row's neighbours: an answer
reaching for `join` writes `use std/string.{join}` and used to get two
messages, an error saying the module declares no `join` and a warning saying
`join` hides a builtin. Both true, and together they point at the module, which
is the one place the answer is not. It now gets one message saying the name is
already in scope, with a repair that deletes the import.
`crates/deed-resolve/tests/messages.rs` holds that sentence.

## How many turns it took

Turns per task, across the five runs:

| Task | Turns | Median |
| --- | --- | --- |
| audit | 5, 4, 5, 5, 8 | 5 |
| grade | 2, 3, 2, 2, 2 | 2 |
| split_evenly | 2, 2, 2, 2, 3 | 2 |
| stock | 2, 2, 3, 2, 2 | 2 |
| total | 3, 4, 2, 12, 2 | 3 |
| twice | 5, 2, 2, 4, 6 | 4 |

`audit` is the effect-and-handler task and it is the expensive one in every
run. `grade`, `split_evenly` and `stock` are two turns almost always, which is
one answer and one confirmation.

## What this does not say

- **One model family.** Everything above is `gpt-5.6`. Whether any of it holds
  elsewhere is unmeasured, and every number here should be read with that
  attached.
- **No comparison language.** The six tasks have not been run in a language
  with no contracts. `README.md` names that as the thing that would falsify the
  pitch, and until it is run, "contracts helped" is not something this file
  establishes.
- **Six tasks is a floor.** They cover the language rather than a domain. An
  answer that checks is not an answer that is good.
- **The control is from a different build.** See the warning above.

## Reproducing it

The harness that scores answers is in this repository and needs nothing:

```text
cargo run -p deed-driver --example agent_bench --release -- <answers-directory>
```

Producing the answers is the caller's job, deliberately: the compiler's
workspace has no network and no API key, and this directory follows the same
rule. The runner that drives a model against `deed mcp` lives outside this
repository for that reason.
