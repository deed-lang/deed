# Decision: Debug adapter feasibility over existing LSP framing

- Status: Proposed
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

Issue deed-lang/deed#659 asks for a Debug Adapter Protocol path that reuses the stream framing already implemented for the language server.

The framing and JSON pieces are already present and reusable:

- `crates/deed-lsp/src/protocol.rs` implements `Content-Length` framed message read and write.
- `crates/deed-lsp/src/json.rs` implements the JSON values and parser needed for request and response payloads.

The runtime has useful state for debugging, but the execution model does not currently expose pause points:

- The interpreter keeps active call bindings in `frames: Vec<Frame>` and uses `Value` that is already printable.
- Handler state exists and is centralized in interpreter handler instances.
- Execution is recursive and single-pass; there is no suspend and resume API.

## Decision

Do not ship a partially working DAP endpoint in this change. Ship a feasibility decision with a concrete staged implementation path.

The current runtime hooks are not enough for breakpoints or stepping without adding explicit suspension points. A transport-only adapter would accept DAP messages but fail the core requirements around breakpoints, step actions, and stack inspection at arbitrary execution points.

Implement debugging in stages:

1. **Stage 1: shared transport**
   - Reuse `crates/deed-lsp/src/protocol.rs` and `crates/deed-lsp/src/json.rs` for DAP framing and payloads.
   - Keep transport code dependency-free, consistent with existing project constraints.

2. **Stage 2: execution snapshots**
   - Add interpreter snapshot APIs that expose:
     - current stack frames with source spans,
     - per-frame bindings resolved to names,
     - current handler state.
   - This enables `stackTrace`, `scopes`, and `variables` responses when execution stops.

3. **Stage 3: stoppable evaluator hooks**
   - Add explicit runtime hook points at expression boundaries (line-meaningful for Deed) to support:
     - breakpoint checks,
     - step in / step over / continue semantics,
     - stop events with accurate locations.

4. **Stage 4: perform stepping UX**
   - Model stepping into `perform` as a transition into handler operation frames and emit clear stop reasons so frontends can render the jump into handler code.

5. **Stage 5: stream-level tests**
   - Add integration tests mirroring LSP session tests: framed request streams in, framed protocol responses out, including breakpoints, stepping, stack, and variables.

## Drawbacks (required)

This does not immediately provide end-user debugging features.

It adds one more documented proposal before code lands, but avoids landing a protocol surface that claims stepping and breakpoint behavior the runtime cannot correctly provide.

## Rejected Ideas (required)

- Option: Add a DAP transport now with placeholders for breakpoints and stepping.
  - Rejected because: a transport-only adapter would look complete to editors but fail the core runtime behavior users need.

- Option: Expose only failure-time stack and variables as an MVP debugger.
  - Rejected because: this is post-mortem inspection, not an interactive debug adapter, and does not satisfy stepping and breakpoint requirements in the issue.

- Option: Implement stepping by rerunning the program repeatedly to synthetic checkpoints.
  - Rejected because: this changes program behavior around effects and handler state, and would produce misleading debugger semantics.

## Open Questions (required)

- What is the exact pause granularity for Deed stepping: expression boundary, statement boundary, or both?
- Which public API boundary should own snapshot and pause control: `deed-interp` directly or a new runtime facade crate?
- How should frontend messaging describe `perform` transitions so users understand entry into handler code paths?

## References

- deed-lang/deed#584
- deed-lang/deed#659
- `crates/deed-lsp/src/protocol.rs`
- `crates/deed-lsp/src/json.rs`
- `crates/deed-interp/src/interp.rs`
- `crates/deed-interp/src/value.rs`
- `crates/deed-lsp/tests/session.rs`
