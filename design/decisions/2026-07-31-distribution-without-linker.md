# Decision: distribution format is a WebAssembly module; no system linker required

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

A compiled Deed program needs to be distributable without requiring the recipient to install
a separate toolchain. The original plan said WASM first and native output second, but that
plan assumed Cranelift would be the backend. Without Cranelift, native output means either
taking that dependency back or writing an object-file encoder plus a machine-code emitter
per architecture. Neither is warranted by any current request, and both require a system
linker this build machine does not have.

The options needed to be written down and decided before any of them was started.

## Decision

The distribution artifact for a Deed program is a WebAssembly module. A system linker is
not required to build or distribute a program. A recipient runs the module through a host,
which today is `deed` itself.

An embedded-runner path (option 3 below) is deferred, not rejected. It is the natural next
step when a recipient needs a standalone binary with no `deed` dependency, and the module
format already supports the upgrade without redesign.

## Drawbacks (required)

A module cannot run on a machine that has neither `deed` nor any other WebAssembly host.
The recipient currently needs `deed` installed, or another capable host, to run the output
of `deed build`.

## Rejected Ideas (required)

- Option: use `cranelift-object` to produce a native object file and link it.
  - Rejected because: it reintroduces the large dependency tree that was already dropped,
    and it requires a system linker on the build machine. The cost is not justified without
    a concrete request for native output.
- Option: write a hand-rolled object-file encoder and machine-code emitter per architecture.
  - Rejected because: the scope is substantially larger than the WASM encoder that already
    exists, and it still requires a linker on every build machine.
- Option: compile to WASM and ship a small runner that embeds the module as a standalone
  binary, the same way `deed` ships its standard library.
  - Deferred rather than rejected: this path removes the host requirement, keeps the
    sandbox, and requires no linker. It is not the immediate choice only because nobody has
    yet asked to distribute to a machine with no `deed` and no module host. When that
    request arrives, this is the option that proceeds first.

## Open Questions (required)

- When the embedded-runner option is worth building, and what host surface it should expose
  to the embedded module.
- Whether a WASI shim would let the current module format target WASI-compliant hosts
  without changes to the compilation model or the capability import names.

## References

- `design/05-backend.md` (Distribution without a linker: options and the decision)
- deed-lang/deed#626
