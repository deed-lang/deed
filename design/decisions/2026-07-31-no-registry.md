# Decision: no registry

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

The no-search-path decision established that a module's name says where it lives.
A dependency is therefore already a location. The remaining question is what
identifies which version of what is at that location, and whether a central
registry is needed to answer it.

Zig and Unison both reached the same position from a similar starting point:
a dependency is a location and a content hash. The hash adds what a path never
had to provide: identity. It answers "which version of what is at this location"
without any further infrastructure.

## Decision

Deed has no registry. A dependency is a location (a URL or a file path) and a
content hash. This extends the rule already in place: a module's name says where
it lives, and the hash says which version of it.

What this gives without a single line of defensive code:

- No name to squat. Locations are not allocated through a central authority.
- No version that can change under you. The hash pins the content exactly.
- No yanked release. A location and hash that worked before keep working.
- No registry outage. There is no single server that can take down the ecosystem.
- No namespace to govern. There is no central namespace.

Swift's package registry evolution list has had proposals for registry auth,
mirrors, search, and SBOM generation. Each of those proposals is a consequence
of having a registry. Deed avoids all of them by not having one.

## Drawbacks (required)

This choice costs something real, and it should be stated plainly rather than
minimised.

- No discovery. Nobody can find a library by searching a central index.
- Version selection is done by hand. The user finds the location and chooses
  the hash.
- No automated notification of a security fix. There is no channel that knows
  who depends on a library and can reach them.
- No human-readable version string in the dependency declaration. "Version 2.1"
  communicates intent; a hash does not.

An unfindable ecosystem may as well not exist. This is the strongest argument
against the decision, and it is true.

## Rejected Ideas (required)

- Option: a central registry, like crates.io or npm.
  - Rejected because: name squatting, version mutation, yanked releases,
    registry outages, and namespace governance are all real, documented problems
    in registries. Each one has generated proposals, migrations, and defensive
    code in existing ecosystems. Deed's module system gives these properties
    for free by extending the rule already in place.

- Option: a registry for discovery only, backed by location-plus-hash
  dependencies.
  - Rejected because: a read-only index still needs an operator, governance,
    and uptime. It solves discoverability but trades the outage and governance
    costs back in. The benefit is real, but the costs appear before the ecosystem
    is large enough to need the index.

- Option: version strings at a location, without a registry (semver with URL).
  - Rejected because: a version string without a registry is not pinned. A tag
    can be moved. The hash is the only thing that identifies content without a
    third party.

- Option: a SBOM or advisory database integrated at the toolchain level.
  - Rejected because: a security advisory database keyed on library names
    requires stable names, which is what the registry provides. An advisory
    database keyed on hashes is feasible but is a distinct design question from
    whether a registry exists.

## Open Questions (required)

- Whether a hash-keyed vulnerability database is viable and, if so, what
  toolchain surface it requires. That would recover the security-notification
  property without requiring a registry.
- Whether a read-only, community-operated index of known locations-and-hashes
  is worth building once the ecosystem has enough libraries to benefit from it,
  and what the minimum governance shape for that index looks like.
- Whether discovery is a real problem at Deed's current scale, or whether it
  becomes one only after the language is more complete.

## References

- `design/decisions/2026-07-31-no-search-path.md`
- `design/02-syntax.md` (Modules, "A module's name says where it lives")
- `https://github.com/deed-lang/deed/issues/634`
