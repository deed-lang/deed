# Decision: backend target is WebAssembly modules first

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

The backend exists to make Deed programs distributable and embeddable without making every user install a separate runtime toolchain.

## Decision

The first backend target is WebAssembly modules. Native object output is deferred.

## Drawbacks (required)

Producing a native executable with no host is deferred. A host is required to run a compiled module.

## Rejected Ideas (required)

- Option: use Cranelift as the first backend path.
  - Rejected because: it introduces a large dependency tree and host-linker constraints that this repository was deliberately avoiding.
- Option: prioritize native object output before WebAssembly.
  - Rejected because: it requires object writing, relocation, linker handling, and target-specific emitters before the backend shape is proven.

## Open Questions (required)

- Which host surface should be standardized first for capabilities in real embedding scenarios.
- Under which measured conditions native output should move from deferred to scheduled.

## References

- `design/05-backend.md`
- `crates/deed-codegen/src/wasm.rs`
