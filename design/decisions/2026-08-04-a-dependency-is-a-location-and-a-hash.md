# Decision: a dependency is a location and a hash

- Status: Accepted
- Date: 2026-08-04
- Supersedes: None
- Superseded by: None

## Context

`design/decisions/2026-07-31-fetch-verify-no-execute.md` decided how a dependency would be
obtained: content-addressed, offline first, hard failure on mismatch, nothing executed. It
was implemented as `deed-fetch`. Nine months later the only thing in the workspace calling
that crate was `deed-cli`'s lock file, for its SHA-256 helper. `verify_and_cache` and
`Cache` had no caller at all.

`design/decisions/2026-07-31-minimal-manifest.md` decided how a project says where to look:
`component <path>`, a directory that already exists on this machine.

So the language could depend on a directory somebody already had, and had a fully
implemented, tested, unreachable way of getting one they did not. There was nothing to
depend on because there was no way to say where anything was.

## Decision

A second manifest directive:

```
component ../other-project
module https://example.com/list.deed sha256:9f2c...
```

**The directive does not say what the module is called.** A fetched module is named by its
own `module` line, the same way every other module in this language is. The location says
where the bytes were, the hash says what they are, and neither says what they mean. Two
projects that fetch the same bytes from two mirrors get one module and one cache entry.

**The hash is not optional.** A location without one is a dependency whose bytes are
whatever the other end felt like today, and a build that accepts those cannot be repeated.
`DEED7004` is its own code rather than a variant of "malformed", because knowing where the
module is and being told that is not enough is a different lesson from not having said
anything.

**Offline first, and that is the whole shape.** The cache is keyed by the hash the manifest
states, so a hit means the bytes are already known to be the right ones and nothing goes
over the network. A build that has run once runs again on an aeroplane.

**A location may be a path.** A dependency being developed alongside its user is the
ordinary case, and requiring publication before testing is what makes people vendor
instead. The hash is checked either way, so a local file is not a way around the check.

**The retrieval half now exists.** Step 2 of the fetch decision — "retrieve the bytes from
the location" — had no implementation, because this workspace has no dependencies and
therefore had no HTTP client. It has one now, written for the network capability, and this
is its second caller.

## Drawbacks (required)

**`http` and not `https`, for the same reason as everywhere else.** A dependency fetched
over plain HTTP is one a network can replace. What stops that mattering is the hash: bytes
that do not match are refused and never stored, so the worst a hostile network can do is
make the build fail. That is a real reduction in what TLS would give and it is the honest
extent of it. A path or a `file://` location has no such exposure.

**Nothing writes the hash for you.** A person adding a dependency has to obtain the digest
some other way. `deed build --lock` writes a lock file over inputs and does not produce
manifest lines.

**A manifest a component root brought with it is read while imports are being resolved**,
so anything it fetches arrives after the seed set was built. Those modules are compiled,
but their own imports are resolved on the next invocation rather than this one. A direct
dependency is unaffected; a dependency of a dependency of a component root is not.

**The cache is never pruned.** Entries accumulate under a platform cache directory and
nothing removes them. That was already true of the fetch decision and is now reachable.

**There is still nothing published to depend on.** This makes the mechanism exist and be
reachable. It does not make an ecosystem.

## Rejected Ideas (required)

- Option: give the directive a name, as `module far/away https://... sha256:...`.
  - Rejected because: the file already says what it is called, on its `module` line, and a
    second place to say it is a second place to be wrong. It would also make the same bytes
    two different modules depending on who fetched them.

- Option: make the hash optional, with a warning.
  - Rejected because: the value of a content-addressed dependency is entirely in the
    address being the content. A warning is a thing people turn off.

- Option: accept an archive, so a dependency can be more than one module.
  - Rejected because: it needs an archive format and a decompressor, which is either a
    dependency or several hundred lines of one. The fetch decision already said the unit is
    bytes handed to the compiler as source, and one file is that. A project wanting several
    can declare several, one line each, which is also what makes a diff readable.

- Option: a registry, so a dependency is a name and a version.
  - Rejected because: `design/decisions/2026-07-31-no-registry.md` decided that, and
    nothing here reopens it. A name that resolves through an index is a name whose meaning
    is somebody else's to change.

- Option: fetch lazily, only when a module turns out to be missing.
  - Rejected because: a manifest that declares a dependency and a build that silently does
    not need it are two different facts, and finding out which one you have by whether the
    network was touched is not a way to find out.

## Open Questions (required)

- Whether `deed build --lock` should record fetched modules alongside the files it hashes.
  Today the lock file is about inputs on disk, and a fetched module is an input on disk by
  the time the compiler sees it, so it is recorded by its cache path, which is a path
  containing the hash. That is either exactly right or a coincidence and nobody has decided
  which.

- Whether a manifest should be able to say where a dependency's own manifest is not to be
  read from, so that a dependency cannot pull in more than it declared. Today a fetched
  module is a file, and a file has no manifest, so the question does not arise yet.

- Whether the compiler should offer to write the hash after fetching once, the way a lock
  file is written. That would mean a first build that trusts the network, which is the
  thing this is against, unless it is a separate command a person runs on purpose.

## References

- `design/decisions/2026-07-31-fetch-verify-no-execute.md`, whose step 2 this implements.
- `design/decisions/2026-07-31-minimal-manifest.md`, which this adds a directive to.
- `design/decisions/2026-07-31-no-registry.md`, which this does not reopen.
- `crates/deed-driver/src/manifest.rs`, `parse_module`.
- `crates/deed-cli/tests/cli.rs`,
  `a_module_declared_in_a_manifest_is_fetched_and_checked`.
