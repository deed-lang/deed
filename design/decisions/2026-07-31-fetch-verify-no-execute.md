# Decision: fetch by location and hash, verify before caching, never execute during fetch

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

When a dependency is described as a location paired with an expected hash, the
tool that resolves it has to make four properties hold simultaneously:

1. A build where everything is already cached must not touch the network.
2. A hash that does not match what was fetched is a hard failure with a
   diagnostic, not a warning, not a silent substitution.
3. The cache is content-addressed, so two projects that need the same bytes
   store them once.
4. Nothing executes during a fetch. No build scripts, no post-install hooks.
   The fetched bytes are stored; they are not interpreted, spawned, or
   evaluated.

The fourth property is not enforced by any mechanism; it is enforced by the
absence of one. Deed has no build script mechanism, so there is no hook to
run. That absence is a deliberate choice and this record states it so that
future work does not fill the gap by accident.

## Decision

A dependency is a (location, expected-hash) pair. The fetch procedure is:

1. Derive the cache path from the expected hash alone. If that path exists,
   return it without making any network request.
2. Otherwise, retrieve the bytes from the location.
3. Compute the SHA-256 of the retrieved bytes.
4. If the digest does not equal the expected hash, emit a hard diagnostic and
   stop. The bytes are never written to the cache.
5. Write the bytes to a temporary file in the cache directory, then rename it
   to the content-addressed path. The rename is atomic on every target
   platform, so a concurrent fetch cannot produce a partial entry.
6. Return the cached path.

The cache lives in a platform-specific location:

- Linux: `$XDG_CACHE_HOME/deed` if the variable is set, otherwise
  `$HOME/.cache/deed`.
- macOS: `$HOME/Library/Caches/deed`.
- Windows: `%LOCALAPPDATA%\deed\cache`.

The hash is always SHA-256, encoded as 64 lowercase hexadecimal characters.
No other algorithm is accepted by this tool.

Nothing in steps 1 through 6 compiles, interprets, spawns, or evaluates the
fetched bytes. The bytes are opaque data until the build system passes them to
the compiler as source.

## Drawbacks (required)

The cache is not bounded. Two projects sharing a cache directory benefit from
deduplication, but nothing evicts stale entries. A separate garbage-collection
step will be needed when the cache grows large enough to matter.

SHA-256 is slower than newer alternatives such as BLAKE3. For the sizes of
dependency archives a language like Deed is likely to carry, the difference
is immaterial today. If it becomes material the algorithm field in a
dependency declaration would need a version, which is a migration cost.

Pinning to a single algorithm means the tool rejects any dependency that
specifies a different one, even if the hash itself would have been
sufficient. This is the intended behaviour: a tool that silently accepts
whatever algorithm a dependency requests gives up the property that every
cached file has a known digest.

## Rejected Ideas (required)

- Option: warn on hash mismatch rather than fail.
  - Rejected because: a warning on tampered bytes is not a security boundary.
    A mismatch means either the bytes have changed or the declared hash is
    wrong; either way the dependency is not what the author declared and
    continuing is wrong.

- Option: allow post-fetch hooks defined in the dependency manifest.
  - Rejected because: every supply-chain incident in npm, PyPI, and RubyGems
    that ran arbitrary code did so through an install hook. Deed has no such
    mechanism and this decision records that it will not grow one in the
    fetching layer.

- Option: use a content-addressed store keyed by URL rather than by hash.
  - Rejected because: two URLs that serve the same bytes would produce two
    cache entries, and the second fetch cannot be skipped by looking at the
    first. Content-addressing solves this at the lookup step.

- Option: compute the hash from the URL alone and skip verifying the content.
  - Rejected because: a URL is a mutable pointer. Content at a URL can change.
    The declared hash is the only stable identity.

- Option: support a configurable hash algorithm.
  - Rejected because: the cache key format would need a version, the
    verification code would need a dispatch table, and the property "every
    entry has an SHA-256 digest" becomes "every entry has some digest of some
    algorithm." Fixed algorithm keeps the guarantee simple.

## Open Questions (required)

- Whether the cache directory should be configurable via an environment
  variable or a CLI flag, for build systems that manage their own caches.
- What the garbage-collection policy should be: LRU, time-based, or
  reference-counted by which lock files mention each hash.
- How the offline flag should interact with a cache miss: hard error, soft
  error with a hint, or a separate exit code.

## References

- `crates/deed-fetch/src/lib.rs`
- `crates/deed-fetch/src/sha256.rs`
- `crates/deed-fetch/src/cache.rs`
- `crates/deed-fetch/src/verify.rs`
- `design/04-capabilities.md` (supply-chain discussion)
- Issue deed-lang/deed#579
- Issue deed-lang/deed#635
