# Decision: a WIT world is not a component

- Status: Superseded
- Date: 2026-08-07
- Supersedes: None
- Superseded by: `2026-08-09-a-component-for-what-crosses-unchanged.md`

> The third thing this record lists as missing is written. `deed build
> --component` writes a component binary for exports that cross unchanged, and
> the measurement below now runs one rather than finding it empty. The other
> two are still missing, and the record that supersedes this one says which
> exports they cost.

## Context

`design/decisions/2026-07-31-row-to-wit-world-mapping.md` decided that a program's
effect row derives its WIT world, and that is the claim this language is for: nobody
else derives a world from code, everybody writes it by hand and nothing checks that
the code matches it.

`deed build --component` has produced a `.wasm` and a `.wit` since #747, and the help
text said it "produces a component". Nothing in this repository read the `.wit` back,
and nothing outside it had been asked to. That is the shape of a claim that has never
been tested by anybody who would have to believe it.

Measured, with the Bytecode Alliance's own tooling, on a program with no `main` and no
capability in a signature:

```
$ deed build --component adder.deed
adder.wasm
adder.wit

$ cat adder.wit
package deed:adder;

world component {
    export add: func(p0: s64, p1: s64) -> s64;
    export greet: func(p0: string) -> string;
}

$ jco new adder.wasm -o adder.component.wasm     # wasm-tools component new
$ jco wit adder.component.wasm
package root:component;

world root {
}
```

A component came out, and it exports nothing.

## Decision

Say so, and keep measuring it.

`--component` writes a core module and the world its exports describe. It does not
write a component binary, and the help text, `design/05-backend.md` and this record
say that in those words.

`crates/deed-codegen/component.mjs` runs the transcript above on every commit and
fails if it stops matching, in either direction: if the world stops naming both
functions, or if the component stops being empty. The second one failing is the good
news, and the file says which lines to rewrite when it happens.

## What is actually missing

Three things, and none of them is the world derivation:

1. **The canonical ABI at the boundary.** A component's exports pass a string as a
   pointer and a length and return anything wider than a word through a return area
   the caller provides. This backend passes one address into its own layout, which is
   the right thing for its own host and is not what a component's caller does.
   `crates/deed-codegen/src/abi.rs` already transcribes the rules; nothing generates
   the adapters.

2. **`cabi_realloc`.** A component's caller allocates inside the callee's memory
   through an export it does not have. The bump pointer is already there and
   `Helper::GrownMemory` already grows the memory, so this is small once the first
   one exists.

3. **The component binary.** A wrapper format around the core module: a component
   type section, a canon lift per export, a core instance, and the exports. Or the
   `component-type` custom section that `wasm-tools component new` reads, which is
   the same information written where an existing tool already looks.

The third is the one worth doing first, because it is the one that turns the other
two from a design into a failing test.

## Drawbacks (required)

The measurement pins something being wrong, which is a test that passes because a
thing does not work. That reads badly and is still better than the alternative this
replaces, which was a help text saying it worked.

CI now installs an unpinned npm package. That is the same trade the MCP smoke test
makes with `pip install mcp`: pinning the consumer would take away the only thing the
job measures.

Somebody reading the `.wit` may still take it for something a toolchain consumes. The
help text now says otherwise in the same paragraph, which is the most that can be done
short of not writing the file.

## Rejected Ideas (required)

- Option: emit the component binary now.
  - Rejected because: it is not the small half. Without the canonical ABI adapters a
    component whose exports carry text would be one that answers wrongly rather than
    one that answers nothing, and answering wrongly is the outcome this repository
    spends its tests avoiding. The order is adapters, then binary.

- Option: stop writing the `.wit` until it is consumed by something.
  - Rejected because: the world is the derivation, and the derivation is the claim.
    It is checked against the rows by `crates/deed-driver/tests/wit_world.rs` and it
    is what somebody writing the adapters starts from.

- Option: leave the help text as it was and write the gap down only here.
  - Rejected because: the help text is what a person reads before deciding whether
    this does what they need, and it said it produced a component.

- Option: pin the toolchain version so the measurement cannot move under us.
  - Rejected because: the question the job asks is whether the toolchain people
    actually run still agrees, and a pinned one stops answering that.

## Open Questions (required)

- Whether the adapters belong in the compiler or in a generated shim module, given
  that a shim is what every other language's toolchain produces and this workspace
  has no dependencies to produce one with.
- Whether the exported interface should stay "every function the module declares".
  A component's world is a public API and a Deed module has no visibility markers,
  so today those are the same list by accident rather than by decision.
- What a capability becomes at a component boundary. `--component` refuses signatures
  holding one, and the component model's answer is a resource, which is a type this
  language does not have.

## References

- `crates/deed-codegen/component.mjs`, the transcript above, on every commit
- `crates/deed-codegen/src/abi.rs`, `crates/deed-driver/tests/wit_world.rs`
- `design/decisions/2026-07-31-row-to-wit-world-mapping.md`
- `design/05-backend.md`
- `how-to/embed-a-compiled-program.md`, which is what works today
