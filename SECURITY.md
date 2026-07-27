# Security policy

## What is actually a security boundary here

Most of this repository is a compiler for a language with no users, so most bugs in it are
bugs rather than vulnerabilities. Two things are different, and both are claims the project
makes out loud.

**The `Dir` sandbox.** A function holding a `Dir` rooted at some directory must have no way
to reach outside it. `crates/deed-interp/src/sandbox.rs` refuses separators, `.` and `..`,
absolute paths, drive prefixes and empty names, and canonicalizes before comparing so that a
symlink cannot walk out. If you find any input that escapes the root, that is a
vulnerability, not a bug report. `design/04-capabilities.md` is the claim it is measured
against.

**Capability safety.** A function that was never handed a capability must have no way to
obtain one. Naming a capability type where a value belongs is `DEED4019` precisely because it
was once a way to conjure authority out of a type name. Any other route from "holds nothing"
to "holds a `Console`, a `Clock` or a `Dir`" is the same class of problem.

Running untrusted Deed source through `deed run` is not otherwise a supported threat model.
The interpreter has a depth limit and traps on overflow, but it has not been hardened
against a program written to attack it, and the sandbox is about the filesystem rather than
about CPU or memory.

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
