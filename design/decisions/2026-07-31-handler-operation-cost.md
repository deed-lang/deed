# Decision: tail-resuming operations are not worth a separate compiler path yet

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

Koka distinguishes tail-resuming operations from general control operations.
A tail-resuming operation always resumes the caller exactly once, immediately,
at the end of the handler body. A general operation may capture the
continuation and resume it later, zero times, or more than once. Koka compiles
the tail-resuming case to something close to a virtual call, skipping the
continuation machinery entirely.

Deed's operations are all one shape today. The question is whether the corpus
uses only the tail-resuming shape in practice, and if so whether separating
the two paths in a future compiler would be worth the added complexity.

The measurement tool is `crates/deed-driver/examples/interpreting.rs`, which
was extended to report a handler operation table. Run it with:

```text
cargo run -p deed-driver --example interpreting --release
```

## Decision

The tail-resuming distinction is not worth implementing until the compiled
backend exists and can measure its own overhead directly.

The interpreter overhead for an operation is around 128 ns above a plain call
taking nothing (stateless handler) and around 142 ns (handler with one state
field read). That gap is real. But the interpreter's overhead comes from
HashMap lookups and dynamic dispatch through the syntax tree, which a compiled
backend would replace with a stack slot or a direct function pointer. The
compiled cost of the search would be a few pointer reads rather than a hash
walk, so the absolute numbers from the interpreter say little about whether
the tail-resuming path would recoup its complexity cost in compiled code.

The corpus evidence is unambiguous: all ten operations across six effects in
the examples corpus tail-resume. No operation captures or discards the
continuation. That makes the corpus a natural fit for the fast path once one
exists.

The right moment to build the fast path is after the first backend lands and
after the same benchmark can be run against compiled output. At that point
the gap between a general operation and a virtual call will be measurable
directly, and the decision will have numbers that predict the payoff rather
than numbers that only confirm the overhead exists.

## Drawbacks (required)

Programs that use effect operations heavily pay the full cost of the general
dispatch path for every call, even though that path is never exercised on
resumption. The interpreter overhead is around 128 ns per operation turn.

## Rejected Ideas (required)

- Option: implement the tail-resuming fast path now, before the backend.
  - Rejected because: the interpreter overhead comes from HashMaps and tree
    walking, not from continuation machinery. Splitting the interpreter's
    dispatch would reduce a cost that the compiler will remove entirely,
    and the complexity is non-trivial.

- Option: declare all current operations tail-resuming by fiat and stop
  measuring.
  - Rejected because: the corpus only covers the examples directory. A
    general-purpose language needs to keep the door open for operations that
    do not tail-resume, and the overhead should be measured in compiled output
    rather than assumed away.

## Open Questions (required)

- What is the overhead of an operation in compiled WebAssembly output, once
  the backend exists?
- Is one handler level deep typical, or do real programs nest three or more
  handlers? The stack search is O(depth), so the answer changes what the
  gap looks like at scale.
- Should the language let an effect *declare* that all its operations are
  tail-resuming, the way Koka does, so the compiler can skip the check
  without the programmer having to annotate each operation?

## References

- deed-lang/deed#611
- deed-lang/deed#575
- `crates/deed-driver/examples/interpreting.rs` (the measurement)
- Koka language reference, section on tail-resumptive operations
