# Decision: a Deed row maps to a WIT world with four named gaps

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

A Deed row and a WIT world express similar things from different angles. A row on a function
lists what operations that function may perform. A world on a component lists what the host
must supply before the component can run. If those two things are in correspondence, the
world can be derived from the unhandled rows at the program boundary rather than written by
hand, and no language targeting components can do that today.

Before writing any derivation code, this decision writes down the mapping entry by entry,
names every place it does not hold, decides what happens at each gap, and records what
evidence would falsify the whole argument.

`design/05-backend.md` already notes the correspondence in one sentence: "a compiled
program's import section is its capability requirements." This decision is the sentence
expanded into an account that can be checked.

## Decision

A Deed effect row and a WIT world are in partial correspondence. The derivation is sound for
the subset covered by the mapping. The four named gaps below are not rounding errors; each
requires an explicit answer before any code generates a world file.

### The mapping, entry by entry

A row entry has the form `Effect.operation` or bare `Effect` (meaning all of its
operations). The claim is that each such entry, when the effect is a built-in host-backed
effect and the operation is unhandled at the program boundary, corresponds to a WIT import.

**`Io.write`** -> import of an interface that provides a single `write` function taking a
`console` handle and a string. In WIT terms: an import in the world that names the
`deed:io/write` interface.

**`Io.now` / `Io.epoch`** -> import of a `deed:io/clock` interface. Both operations act on a
`Clock` handle; they share one import rather than two because the resource they act on is
the same.

**`Io.open` / `Io.read` / `Io.save` / `Io.remove` / `Io.make` / `Io.list`** -> import of a
`deed:io/filesystem` interface. All six operations act on a `Dir` handle.

**`Io.args`** -> import of a `deed:io/args` interface.

**`Diverge`** -> no WIT import. Divergence is not a host service; it is an annotation that
the function may not terminate. A WIT world says nothing about non-termination.

**A user-declared effect handled inside the program** (`with Ledger { ... }`) -> no WIT
import. The handler is program-internal. The effect does not reach the host.

**A user-declared effect not handled at the program boundary** -> not representable in the
current WIT model as an import. See gap 3 below.

**A row variable** (`<R>` in a polymorphic function) -> not representable in WIT. See gap 4
below.

### Where it does not hold

**Gap 1: granularity.** Deed's row entries are individual operations (`Io.read`,
`Io.save`). WASI's interfaces are grouped by resource and access pattern, not by operation
name. `wasi:filesystem/types` and `wasi:filesystem/preopens` do not correspond
one-to-one to Deed's `Io.*` entries. A program that uses only `Io.read` and no `Io.save`
still imports one WASI interface that declares both `read` and `write` descriptors.

The mapping used here is `deed:io/*` rather than `wasi:*`. A shim layer translates between
them when a WASI host is the target, as noted in `design/05-backend.md`. The derived world
names Deed's own interface, and a WASI-targeting host provides the shim. This is not a gap
in the derivation; it is a gap between Deed names and WASI names that the shim fills. What
would turn it into a real gap: a single Deed operation that maps to two distinct WASI
interfaces with no single shim function possible.

**Gap 2: capabilities are arguments.** A WIT world import grants authority unconditionally:
if the world imports `deed:io/write`, every part of the component can call it. A Deed row
does not work that way. A function that declares `uses Io.write` still needs a `Console`
handle as an argument. Holding the row entry is not the permission; holding the handle is.

The derived world imports the interface. That is necessary but not sufficient. The component
still needs a capability value (a `Console`, a `Dir`, a `Clock`) to pass to the operation.
In the compiled module those values are handle indices the host supplies at instantiation
time, not ambient globals. So the WIT world says "this component may call `deed:io/write`"
and the capability argument says "this specific call uses this specific console." Both halves
are present; they are not redundant.

What would turn this into a real gap: a host that hands out authority through the world
import alone, with no per-call capability argument. That host would give every function in
the component the same console, which is wider than a Deed program's row guarantees. The
host implementing the capability argument protocol is what keeps the derived world from being
too wide in practice.

**Gap 3: effects that are not `Io`.** A user-declared effect with a `with` block is handled
inside the program and produces no import. That is handled correctly: the derivation skips
any effect for which a handler is installed at the program boundary.

The harder case is a user-declared effect with no `with` block at the program boundary. That
effect escapes. In the current model, an escaped user effect is a type error at `main`
unless the effect row of `main` explicitly declares it. A world file cannot represent it as
an import because no WIT interface exists for it. The correct answer for this case is that
such a program does not produce a compilable component today: it would require a host
implementation of the effect, which is outside what `deed-rt` provides. The derivation
skips escaped user effects and records them as unresolved, not silently drops them.

What would turn this into a real gap: a mechanism for a host to implement a user-declared
Deed effect. That would require the effect's operation signatures to appear in a generated
WIT interface, which is a new kind of output this design does not produce.

**Gap 4: row variables.** A polymorphic function has a row variable `<R>` meaning "whatever
the caller adds." A WIT world is not parameterised. A polymorphic function cannot be
compiled to a WIT world on its own; only its concrete instantiations can.

Monomorphization, already present in the compiled backend (`design/05-backend.md`, section
"Monomorphization is affordable"), resolves this: each instantiation has a concrete row and
a concrete world can be derived from it. The row variable is gone by the time the derivation
runs. What would turn this into a real gap: a polymorphic function exposed as a component
export, which would require the world to name a concrete interface. That shape of export
does not exist today.

### Whether the derived world is too wide, too narrow, or right

*Too wide* means the world imports something the program does not use. The derivation
produces exactly the `deed:io/*` interfaces whose operations appear in the unhandled rows at
the program boundary. "Appears in the unhandled rows" is the same condition the effect
checker already enforces: a row entry that is not performed is an error
(`design/03-effects.md`). So the derivation cannot produce a too-wide world unless the
effect checker has a bug.

*Too narrow* means the world omits an import the program needs. The only way that happens
is if an `Io` operation is performed but its row entry is absent, which is also an effect
checker error. So the derivation cannot produce a too-narrow world unless the effect checker
has a bug.

*Right* is the conclusion, conditional on the effect checker being correct and conditional
on the four gaps being handled as described above: `Diverge` omitted, handled effects
omitted, unhandled user effects recorded as unresolved rather than silently dropped, and row
variables resolved by monomorphization before the derivation runs.

## Drawbacks (required)

The claim "the derived world is right" is only as strong as the effect checker. A bug that
allows an over-declared row would produce a too-wide world. A bug that allows an
under-declared row would produce a too-narrow world, which is worse because the component
would fail at link time rather than at the host's authority check.

The `deed:io/*` names are Deed-specific and require a shim to run on a real WASI host. A
program whose intended host is `deed-rt` can use the derived world directly; a program
targeting a WASI host needs the shim layer that `design/05-backend.md` defers. Using a
derived world before the shim exists would produce a component that does not link to any
real host.

## Rejected Ideas (required)

- Option: derive the world from the syntactic row of `main` only, ignoring callee rows.
  - Rejected because: `main`'s row is inferred bottom up from everything it calls. It is
    already the union of all unhandled effects reachable from `main`. Reading it directly
    produces the same result as the derivation and is not a different option.

- Option: produce a WIT world that names `wasi:*` interfaces directly instead of `deed:io/*`.
  - Rejected because: WASI's interface groupings do not match Deed's operation-level
    granularity (gap 1). Producing `wasi:*` names requires the shim mapping to be correct
    first. The derivation produces `deed:io/*` names and the shim is a separate, later step.

- Option: include `Diverge` as a WIT import.
  - Rejected because: WIT has no interface for non-termination. No host implements one.
    Divergence is a property of the program, not a host service.

- Option: silently drop unhandled user effects that have no corresponding host interface.
  - Rejected because: silent dropping produces a world that does not reflect the program's
    actual requirements. The correct behaviour is to record them as unresolved so tooling
    can report them.

## Open Questions (required)

- Whether the shim mapping from `deed:io/*` to `wasi:*` is always one-to-one, or whether
  any Deed operation maps to two distinct WASI interfaces with no single function covering
  both. If it does not hold, gap 1 becomes real and the derivation needs a richer interface
  table.

- Whether a host implementing a user-declared Deed effect (gap 3) is a use case worth
  designing for. If it is, the derivation would need to produce generated WIT interface
  definitions for escaped user effects rather than recording them as unresolved.

- Under what conditions the four-gap account here should be replaced by a tighter account
  because the shim and the host capability protocol have both been implemented and
  measured.

## References

- `design/03-effects.md` (row propagation rules, too-narrow and too-wide errors)
- `design/04-capabilities.md` (capabilities as arguments, not bits in the row)
- `design/05-backend.md` (import section as capability requirements; shim deferral; monomorphization)
- `design/decisions/2026-07-31-wasm-backend-target.md` (WASM as first backend target)
- `crates/deed-typeck/src/check.rs` (row checking)
- `crates/deed-driver/tests/host.rs` (capability-as-argument check against compiled module)
- `https://github.com/deed-lang/deed/issues/627` (this issue)
- `https://github.com/deed-lang/deed/issues/577` (parent issue)
- `https://github.com/deed-lang/deed/issues/569` (capabilities as opaque handles)
