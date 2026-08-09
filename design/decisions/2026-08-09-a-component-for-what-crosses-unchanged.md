# Decision: a component for what crosses unchanged

- Status: Accepted
- Date: 2026-08-09
- Supersedes: `2026-08-07-a-wit-world-is-not-a-component.md`
- Superseded by: None

## Context

The record this supersedes measured a gap and named three things that stood in
it. The measurement was that `deed build --component` wrote a core module and a
`.wit`, and that handing the core module to the Bytecode Alliance's own tooling
produced a component exporting nothing:

```
$ jco new adder.wasm -o adder.component.wasm
$ jco wit adder.component.wasm
package root:component;

world root {
}
```

Two of the three things are still missing. The one that is not is the third:
nothing wrapped the module in the format a component runtime reads.

That record also contradicted itself about the order. "What is actually
missing" said the binary was worth doing first, because it turns the other two
from a design into a failing test. "Rejected Ideas" said the order was adapters
then binary, because a component whose exports carry text would answer wrongly
without them.

Both are right, about different exports. A number is an `s64` on both sides of
the boundary and a boolean is a `bool` on both sides, so lifting one is a
declaration rather than a conversion, and there is nothing an adapter would do.
A string is a pointer and a length into memory the caller helped allocate
through `cabi_realloc`, and this backend passes one address in its own layout,
so lifting one without adapters is exactly the wrong answer the record refused.

## Decision

Write the component for the exports that cross unchanged, and refuse it by name
for the ones that do not.

`deed build --component` writes three files. `<name>.wasm` is the core module,
unchanged, which is what `deed build` writes and what
`how-to/embed-a-compiled-program.md` describes. `<name>.wit` is the world, also
unchanged, because the derivation is the claim and it does not depend on any of
this. `<name>.component.wasm` is a component binary carrying that core module
verbatim, with one lift per export.

A module with an export wider than a word does not get the third file. It gets
a line naming the function, the type, and the adapters that are missing, and it
keeps the first two.

The component's encoding is written by `crates/deed-codegen/src/component.rs`,
with no dependency: a preamble, the core module as a section, one instance of
it, an alias per export, a component function type per export, a canonical lift
per export, and the exports. Every constant in it was read out of a component
the toolchain itself built, rather than out of the specification.

## What this buys

`crates/deed-codegen/component.mjs` no longer measures a gap. It builds a
module of scalars, reads the component's world with `jco wit`, transpiles it
with `jco transpile`, and **calls it**:

```
ok    the component it wrote has a world with both of them in it
ok    a component runtime runs it
ok    and the second export is the second function rather than the first
```

Reading a world is not running one, which is why the last two are separate. A
lift wired to the wrong core function, or with its parameters in the wrong
order, still produces a component whose world reads correctly.

The same file measures the other half in the same run: a module carrying text
is told which export needs the adapters, is not given a component, and keeps
its world.

## Drawbacks (required)

Two files that are both `.wasm` and are not the same kind of thing. The names
differ and the four bytes after the magic number differ, but somebody who
copies the wrong one gets a confusing error from whatever they hand it to. The
alternative was for `<name>.wasm` to change meaning depending on the signatures
inside it, which is worse.

A component whose world is `s64` and `bool` and nothing else is a component
almost nobody wants. What it is for is turning the remaining two gaps from a
design into a failing test, which is what the superseded record said the binary
would do.

The refusal is per module rather than per export. One function carrying a
string costs the whole module its component, even though the others could have
been lifted. Exporting a subset would mean a component whose world is not the
world in the `.wit` beside it, and two worlds that disagree is the thing this
language exists to avoid.

## Rejected Ideas (required)

- Option: lift strings and lists now, by writing the adapters first.
  - Rejected because: the adapters need `cabi_realloc`, a return area, and a
    lowering for every aggregate, and none of that has anything to check it
    against until a component exists to run. With one, the next change is
    testable the day it is written.

- Option: emit the `component-type` custom section instead, and let
  `wasm-tools component new` do the wrapping.
  - Rejected because: it is the same encoder for less. The section's payload is
    itself a component, so the work is the same and the outcome is a file that
    still needs another tool before anybody can run it.

- Option: write `<name>.wasm` as the component and drop the core module.
  - Rejected because: the core module is what a host embeds today, and it is
    the only thing `deed build` and `deed build --component` share. Making one
    filename mean two formats depending on the source would break the one thing
    that reads it for the sake of the one that does not exist yet.

- Option: refuse the whole command for a module carrying text, the way it
  refuses one holding a capability.
  - Rejected because: a capability has no world-level type at all, and text
    has one this backend cannot yet produce. The world for a string-carrying
    module is correct and useful; only the binary is out of reach.

## Open Questions (required)

- Where the adapters belong when they are written: in the compiler, or in a
  generated shim module, which is what every other toolchain produces and what
  this workspace has no dependencies to produce with.
- Whether the exported interface should stay "every function the module
  declares". A component's world is a public API and a Deed module has no
  visibility markers, so today those are the same list by accident.
- What a capability becomes at a component boundary. `--component` refuses
  signatures holding one, and the component model's answer is a resource, which
  is a type this language does not have.
- Whether a module should be able to export the subset that lifts. See the
  third drawback; the answer probably depends on the `.wit` gaining a way to
  say which of its exports the binary carries.

## References

- `crates/deed-codegen/src/component.rs`, the encoder
- `crates/deed-codegen/component.mjs`, the measurement, on every commit
- `crates/deed-codegen/src/abi.rs`, which transcribes the rules the adapters
  will have to follow
- `design/decisions/2026-08-07-a-wit-world-is-not-a-component.md`, superseded
- `design/decisions/2026-07-31-row-to-wit-world-mapping.md`
- `design/05-backend.md`
