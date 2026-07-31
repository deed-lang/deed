# Deed conformance suite

This directory is a neutral artifact: each case states a program and what must happen. It is
data rather than Rust test code, so a second implementation can consume the same cases
without consuming this compiler.

A case is a directory under `conformance/cases/` with:

- `case.txt` metadata
- either a `program.deed` file in the same directory, or a `path` entry pointing to an existing `.deed` file

`case.txt` is line based:

- `mode: check | test | run`
- `expect: accept | reject | run`
- `code: DEEDnnnn` (required for `expect: reject`)
- `stdout: ...` (repeat for each expected output line, used by `expect: run`)
- `path: relative/path.deed` (optional, relative to repository root)

## What is covered

The suite is organised around the three things an implementation has to agree about, and
`crates/deed-cli/tests/conformance.rs` requires at least one case of each kind so that it
cannot drift into being only one of them.

**Refusals.** Most of [`design/refusals.md`](../design/refusals.md) is a claim about what
this language does *not* accept, and a claim like that is worth as much as the case that
pins it. A float literal, a range, a positional variant, `let mut`, a type before a name, a
cast, a catch-all pattern, a detached spawn, an effect performed but not declared, an effect
declared but not performed, a precondition broken at the call site, a type parameter that
appears only in a return type, and an import of a module that is not there. Each names the
diagnostic code, so an implementation that refuses the right program for the wrong reason
does not pass.

**Acceptance.** The shapes that have to keep working: a generic function, an effect with a
handler, a refinement, alternative patterns, the shipped library, and a capability being
narrowed on its way down a call.

**Behaviour.** What a run has to produce, which is where two implementations are most likely
to differ quietly. Integer division truncating towards zero, structural equality over records
and lists, string ordering being by character, handler state being scoped to its block, and a
walk folding rather than mutating. Two more run under `deed test`: a broken precondition
being catchable with `assert refuses`, and a postcondition holding on every call.

## What is not covered

Nothing here checks diagnostic wording, only codes. Wording is deliberately not a conformance
concern: this repository rewords messages often and on purpose, and a second implementation
that says the same thing differently is not wrong.

Nothing here compares the compiled backend against the interpreter either. That comparison
exists, in `crates/deed-driver/tests/agreement.rs`, but it is about two engines inside one
implementation rather than about two implementations.
