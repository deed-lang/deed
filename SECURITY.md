# Security policy

## What is a vulnerability in Deed

Most defects in this repository are correctness bugs. A report is a security vulnerability
when it crosses one of these boundaries:

- **Capability safety:** code that was not handed a capability can still obtain or forge one.
  `DEED4019` exists because a type name in expression position once allowed exactly that.
- **`Dir` containment:** a function holding a `Dir` can reach paths outside that root.
  `crates/deed-rt/src/sandbox.rs` is the implementation and `design/04-capabilities.md`
  is the design statement.

Anything else should normally be filed as a regular bug unless it can be shown to break one
of those two boundaries.

## Security boundary in the current implementation

The runtime boundary here is capability passing plus host-enforced operations:

- The interpreter and backend expose one built-in effect, `Io`, with `write`, `line`, `now`,
  `epoch`,
  `open`, `read`, `save`, `remove`, `make`, `list`, `args`, `env`, `reach`, `fetch` and
  `send`.
- Every `Io` operation takes the capability it acts on as its first argument. The row says
  what kind of operation may happen, and the capability value says which resource it may
  happen to.
- Runtime row checking reports `DEED6010` when a run performs an operation the checked row did
  not admit. That is treated as a compiler/runtime invariant failure, not as a program fault.
- Embedded std modules (`std/string`, `std/list`, `std/table`) are normal Deed code and run
  under the same row and capability rules as any other module.
- In compiled output, host imports are the authority boundary. A module can only ask for what
  it imports, and the host decides what those imports do.

## What this does not guarantee

- **No resource sandbox:** this is not a CPU, memory, or wall-clock isolation boundary.
  `MAX_DEPTH` (`DEED6009`) bounds call depth, but there is no general limit on allocation or
  total runtime.
- **No hostile-input hardening promise for the whole toolchain:** running an untrusted source
  file through the compiler or interpreter is expected to produce diagnostics or runtime
  failures, but denial-of-service resistance is not a stated guarantee.
- **No generic foreign import escape hatch in Deed source today:** capability crossings are the
  fixed host operations above. If components or FFI are added, they must preserve the same
  explicit capability boundary or they become a new security boundary to specify.

## Supported versions

None yet. There are no releases and the version is `0.0.0`, so the only supported version is
whatever `main` is today. Fixes land on `main` and nowhere else.

## Reporting

Use GitHub's private vulnerability reporting on this repository, under **Security** then
**Report a vulnerability**. That reaches the maintainer without the report being public
first.

If that is unavailable, open a normal issue. Given there are no users and no releases, a
sandbox escape reported in the open costs nobody anything, and having it written down is
worth more than the delay of arranging a private channel. There is no email address here on
purpose: an address nobody monitors is worse than none.

## What to expect

An acknowledgement, and then either a fix on `main` with the reporter credited in the commit,
or an explanation of why the behaviour is intended. There is one maintainer and no service
level agreement to promise.
