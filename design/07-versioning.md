# Versioning

This document states what a Deed version number promises.

Rust's framing is the model: stability without stagnation. "We never break
anything" and "we never fix anything" are the same promise. The way out is
editions. Deed adopts the same direction and sets the policy below.

## 1. What a version number promises

Today Deed is `0.x.y`.

- Patch release (`0.x.y -> 0.x.(y+1)`): no intentional source break.
  Programs that compiled before should keep compiling under the same settings.
- Minor release (`0.x.y -> 0.(x+1).0`): language and `std/` changes may break
  source compatibility. Any break must be called out in release notes, and
  `deed fix` should carry the mechanical part when possible.

Deprecations are the required step before removals:

- Marking a declaration `deprecated old -> new` is non-breaking and may ship in
  a patch or minor release.
- In `0.x`, a deprecated declaration must survive until at least the next minor
  release (`0.(x+1).0`) after the release that first marked it.

After `1.0.0`, Deed follows SemVer with editions:

- Patch and minor releases do not break programs that stay on the same edition.
- Breaking language changes are edition-gated. Old editions keep compiling.
- A major release may drop support for old editions, with a migration path.
- A declaration deprecated in one edition is removed only in a later edition
  (or when that edition is dropped), never by patch or minor release alone.

Until editions land, breaking changes are only allowed in `0.x` minor releases.

## 2. Where diagnostics sit in that promise

Diagnostics are part of compatibility in Deed.

This repository ratchets diagnostic output in tests such as:

- `crates/deed-driver/tests/codes.rs`
- `crates/deed-parser/tests/messages.rs`
- `crates/deed-typeck/tests/messages.rs`

Policy:

- Diagnostic codes are stable identifiers. A code is never reused.
- Renumbering or reassigning a code is a breaking change.
- The human wording of diagnostics is a compatibility surface for users.
  Rewording that changes meaning, guidance, or expected phrasing is a breaking
  change for that release line.
- Small copy edits that preserve meaning are allowed, but should be deliberate
  and reviewed because message wording is tested as a ratchet.

For tools, `--format json` is the stable machine surface.

## 3. Standard library compatibility

`std/` follows the same compatibility policy as the language.

- Removing or changing behavior of an existing public `std` API is a breaking
  change.
- Adding new APIs is non-breaking.
- Edition-gated breaks apply to `std/` the same way they apply to syntax and
  type checking.

## 4. Mechanical migration and `deed fix`

Deprecation warnings name the replacement declaration and carry a
machine-applicable rename when the replacement is in scope. `deed fix` applies
those automatic renames, so a deprecation that fits this shape can be migrated
mechanically.

## 5. What is outside the promise

The version promise does not cover:

- Internal Rust crate APIs under `crates/` unless explicitly documented as
  public.
- Exact formatting details of human diagnostic rendering such as whitespace,
  color, or line wrapping, when code and meaning stay the same.
- Performance characteristics, compile times, and memory use.
- Experimental or explicitly unstable behavior.

When any of these change, release notes should still call out user-visible
impact.
