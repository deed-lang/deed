# Decision: a dependency is a location and a hash

- Status: Accepted
- Date: 2026-08-07
- Supersedes: None
- Superseded by: None

## Context

`design/decisions/2026-07-31-fetch-verify-no-execute.md` decided how bytes are fetched,
verified and cached, and `crates/deed-cli/src/lock.rs` decided how a build records what
went into it. Both were written as though the larger question had been settled. It had
not: nothing said whether Deed would have a registry, and the epic that asked
(#579, "registry or URL and hash") was closed by the work rather than by an answer.

The consequence was not a missing feature. Everything a dependency needs is already
here -- a manifest directive, content-addressed fetch, hash verification, a lock file --
and there was no page that said so and no record that said why it stops there. A
mechanism nobody can find is the same as one that does not exist, which is the finding
this repository has now made three times in one day.

## Decision

A dependency is a URL and a SHA-256, written in `deed.manifest`:

```text
module https://example.com/ledger.deed sha256:ba7816bf...
```

There is no registry, no version resolver, and no name service. The name of a module
is the one it declares in its own first line; the manifest says only where the bytes
are and what they hash to.

Written up for a reader in `how-to/depend-on-another-module.md`.

## Why not a registry

A registry is a name service. The value of one is discovery -- finding out that a
module exists at all -- and the cost is that the meaning of a name lives somewhere
other than in your file. Every dependency crisis of the last decade is a variation on
that cost: a name that resolved to different bytes than it used to, whether by
compromise, by a maintainer's decision, or by a version range doing what it was asked.

This language's whole argument is that a signature is a promise you can read. A
dependency whose identity is a name in a database is a promise about a thing that can
be swapped. `sha256:` is the same argument applied one level out: the identifier *is*
the bytes, so "has this changed" is not a question anybody has to trust an answer to.

The cost is discovery, and it is real. There is no `deed search`, no index, and no way
to find out that somebody wrote the module you need. That is a genuine loss and this
record is not pretending otherwise; it is saying that discovery is a problem a website
can solve and that mutable identity is a problem nothing can.

## Drawbacks (required)

No discovery at all. Somebody wanting a JSON parser has no way to learn that one
exists.

No version resolution, so a diamond -- two dependencies wanting different versions of
a third -- is not something the compiler can help with. Today it does not arise,
because nothing has two dependencies, and saying that plainly is better than shipping
a resolver for a shape nobody has.

Upgrading is editing a hash by hand. For a project with one dependency that is
correct and pleasant; for one with forty it would be neither, and forty is a number
this decision has never been tested against.

The bytes have to stay somewhere. A URL that stops answering is a build that stops
working, and the content-addressed cache only helps a machine that fetched them once
already. A registry's other real value is that it is a mirror.

## Rejected Ideas (required)

- Option: a registry with immutable published versions, like crates.io.
  - Rejected because: it moves identity into a service, and then availability and
    integrity are properties of that service rather than of the file in front of you.
    Immutability is a policy the operator can revise; a hash is not.

- Option: a registry that only maps names to hashes, so identity stays with the bytes.
  - Rejected because: that is a search index, and a search index does not have to be
    in the compiler. If it is worth building it can be built beside the language and
    hand somebody a `module` line to paste, which is exactly the split this decision
    prefers.

- Option: version ranges, resolved at build time.
  - Rejected because: a range is a statement that some bytes nobody has seen will be
    acceptable. That is the opposite of every other claim this language makes, and
    a lock file is the industry's own admission that it did not work.

- Option: git URLs with a revision instead of a file URL with a hash.
  - Rejected because: a revision identifies a tree in a service, and resolving it
    means speaking a protocol and trusting a server. A hash identifies bytes and can
    be checked by anything that can read them.

- Option: leave the question open, since the mechanism already works.
  - Rejected because: it had been left open for months and the result was a working
    mechanism no page mentioned. An unanswered question is not a neutral state.

## Open Questions (required)

- Whether a mirror or a vendoring directory belongs in the compiler, given that a URL
  going away is the one failure this decision has no answer for.
- What a second dependency does to this, and a third. Nothing here has been tested
  against a project with a dependency graph rather than a dependency.
- Whether `deed build --lock` should record the URL a hash came from, so a rebuild on
  a machine with an empty cache knows where to look.
- Whether discovery should exist at all as a thing this project builds, or whether it
  is somebody else's website that happens to know about Deed modules.

## References

- `how-to/depend-on-another-module.md`
- `crates/deed-driver/src/manifest.rs`, `crates/deed-fetch/src/lib.rs`,
  `crates/deed-cli/src/lock.rs`
- `design/decisions/2026-07-31-fetch-verify-no-execute.md`
