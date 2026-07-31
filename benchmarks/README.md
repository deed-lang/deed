# Can something else write this language?

Every `.deed` file in this repository was written by one person. That is the
largest open question about Deed and it is not a question about the compiler:
the corpus is one author's idea of what the language is for, the diagnostics
were tuned against one author's mistakes, and every "this reads well" judgement
in the design documents has a sample size of one.

`design/00-motivation.md` says most code being written today is not typed out by
a person. This directory is where that claim gets a number instead of a
sentence.

## What this measures

A task hands over a prompt and nothing else. An answer is one Deed module. The
harness checks it against tests the task brought with it, which the answer never
sees, so an answer cannot pass by writing tests it already satisfies.

```text
cargo run -p deed-driver --example agent_bench --release -- <answers-directory>
```

The directory holds one `<task>.deed` per task. A missing file is a task that
was not answered, which is reported rather than skipped: a model that produces
nothing for the hard half and perfect answers for the easy half should not score
the same as one that tries everything.

Four numbers come back per task:

| Number | What it says |
| --- | --- |
| `checks` | The module has no errors. Anything else, and nothing further is measured. |
| `passes` | The task's own tests all pass against the answer. |
| `proven` | Obligations the checker settled at compile time. |
| `guarded` | Obligations that fell to a runtime check, with the reason. |

The last two are the interesting ones, and they are the reason this is not a
generic code benchmark. Two answers can both check and both pass while one
carries contracts the compiler proved and the other carries none at all. A
language that claims a signature is a promise the compiler checks should be able
to tell those apart, and an author who cannot get past `guarded` is telling us
something about the checker rather than about themselves.

## What this does not measure

Whether the answer is good code. Whether it would be written that way twice.
Whether the prompt was fair. This is a floor, not a grade: an answer that does
not check is definitely a problem, and an answer that checks is not definitely
fine.

It also does not call a model. Nothing here has a network or an API key, which
is the same rule the compiler itself follows. Producing the answers is the
caller's job, and `deed mcp` is the intended way to let something produce them
against the real compiler rather than against its memory of one.

## The tasks

Each directory under `tasks/` holds three files:

- `prompt.md`, handed over as-is. It names the module and the signatures and
  says nothing about how to write the bodies.
- `checks.deed`, a module that imports the answer and tests it. Never shown.
- `reference.deed`, one answer that works, written by hand.

The reference is not there to be copied. It is there so the harness itself is
tested: `crates/deed-driver/tests/benchmark.rs` requires every reference to
score full marks and requires a deliberately broken answer to score less, which
is what stops this from being a scorer that says yes to everything.

The tasks cover the language rather than a domain, one feature each:

| Task | What it is about |
| --- | --- |
| [twice](tasks/twice/prompt.md) | Arithmetic, and a contract that has to constrain its own input before it can promise anything |
| [total](tasks/total/prompt.md) | Walking a list with the one loop this language has, and carrying a record through it |
| [grade](tasks/grade/prompt.md) | A choice and a `match` that has to name every variant |
| [split_evenly](tasks/split_evenly/prompt.md) | `Result`, and refusing rather than returning a wrong answer |
| [audit](tasks/audit/prompt.md) | An effect, a handler, and a `with` block that discharges it |
| [stock](tasks/stock/prompt.md) | A refinement, and getting the checker to prove it rather than guard it |

## What would falsify the whole exercise

If answers score the same here as the same answers translated into a language
with no contracts, then the contracts are decoration and the pitch is wrong.
That is a real possible outcome and the reason to run this before believing the
pitch, rather than after.
