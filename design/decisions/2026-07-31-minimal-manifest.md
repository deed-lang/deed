# Decision: a minimal manifest for external component roots

- Status: Accepted
- Date: 2026-07-31
- Supersedes: `design/decisions/2026-07-31-no-search-path.md`
- Superseded by: None

## Context

The no-search-path decision (`design/decisions/2026-07-31-no-search-path.md`) said there
is no search path, no config file, and no manifest, and gave the reasons for each refusal.
One of those reasons was that a manifest listing module roots would duplicate information
that file paths already provide.

That refusal was about modules in the same source tree. A different question arises when
a project wants to import modules that live in a separate tree entirely, such as a library
maintained in a different repository or a generated output directory. The normal root
derivation from a file named on the command line can never reach those trees, because it
can only derive roots from files it was given.

Issue `#633` asked what the smallest thing is that could answer that question without
reopening the question of search paths or general configuration.

The checklist from the issue:

- the format, and why that one
- the smallest thing it can say
- it cannot change what a program means, enforced
- a parse error in it is a diagnostic like any other

This document answers all four.

## Decision

A file named `deed.manifest` in a project root declares external component roots. The
compiler reads it automatically when it discovers that root during import resolution.

### Format

The format is a line-based plain text file:

```
# Comments start with a hash sign. Blank lines are ignored.

component ../other-project
component /absolute/path/to/lib
```

Each `component` line names one directory. That directory is an additional root for
module resolution, searched after all roots derived from the files named on the command
line have been asked and have not answered.

No other directives exist. A line that is not blank, not a comment, and not a `component`
declaration is diagnostic `DEED7001`. A `component` directive with no path following it
is `DEED7002`. Both are errors, but errors on one line do not suppress valid declarations
on the others.

### What it can say

Where to look for modules that are not in the current source tree, and nothing else.

It cannot select build targets, configure the compiler, set profiles, declare features, or
do anything else. Every directive adds somewhere to look. There is no directive that does
anything else, so there is no other effect.

> Since
> [`2026-08-04-a-dependency-is-a-location-and-a-hash.md`](2026-08-04-a-dependency-is-a-location-and-a-hash.md)
> there is a second directive, `module <url> sha256:<digest>`, which names bytes rather
> than a directory. It says where to look in exactly the sense above: the module it brings
> is named by its own `module` line and compiled like any other, so the paragraph below
> holds for it without change.

### What it cannot do

A manifest cannot change what a module means. Every module a component root supplies is
compiled identically to one found through normal root derivation. The manifest cannot
override a module from a named root, remap names, or affect the semantics of any program.

This is enforced structurally. The format has one directive and that directive adds a
root. Adding a root changes where the compiler looks, not what it finds when it looks.

### Parse errors as diagnostics

Parse errors in a `deed.manifest` are reported through the same diagnostic infrastructure
as every other compiler error. The file is added to the source map, so a parse error
underlines the offending line in the manifest, carries a diagnostic code, and appears in
the output alongside any errors in Deed source files.

A manifest with parse errors still contributes the component roots that could be read
before the error. An error on one line does not hide the others.

## Drawbacks (required)

A manifest is a second artifact that can drift from the code. If a component root moves
or is removed, the manifest still points at the old location, and the module that used to
be there is simply unfound, which is the same error as typing the path wrong in a `use`
line. That is not worse than the current state; it is the same diagnostic from a different
cause.

The rule that named roots come before component roots means a module in both places
shadows the one in the manifest, silently. That is the right direction for a file you can
read to win over one you cannot, but a reader who does not know about the manifest will
not know why.

There is now a file format to support. The format is as small as it can be, but it is
not zero.

## Rejected Ideas (required)

- Option: use TOML, YAML, or JSON for the manifest format.
  - Rejected because: each adds a parser dependency or a hand-written parser for
    somebody else's grammar. The format answers one question. A line-based format answers
    it with a parser that fits in a screenful.

- Option: write the manifest in Deed, using record declarations.
  - Rejected because: the Deed parser is a full language parser. A manifest is read
    before compilation begins, and reading a language file before the language is ready
    would require a restricted parse mode or a second, stripped-down parser. The line
    format is lighter and does not need the Deed parser.

- Option: accept the manifest path on the command line instead of discovering it.
  - Rejected because: a manifest in a fixed location relative to the project root is
    the thing a build tool or a shell alias can rely on without extra flags. A flag
    that names a manifest is a flag that every invocation must remember to pass.

- Option: allow the manifest to affect compilation in other ways, such as selecting
  a profile or declaring optional features.
  - Rejected because: the issue was explicit: the manifest should answer exactly one
    question. Every additional thing it can say is a place a program's meaning becomes
    invisible in source, which is what this language is specifically designed against.

- Option: search parent directories for the manifest (workspace-style discovery).
  - Rejected because: a root is derived from a file named on the command line. The
    manifest lives in that root. There is no workspace concept, and introducing
    parent-directory search would add one through the back door.

## Open Questions (required)

- Whether a component root should itself be allowed to have a manifest. Today it is not
  asked for one. If it were, component roots could chain, which is useful and complex.

- Whether the compiler should warn when a component root declared in the manifest does not
  exist on disk. Today it is silently ignored, which is the same treatment a missing
  module file gets from the resolver.

## References

- Issue `#633`: "A manifest that answers one question and no others"
- `design/decisions/2026-07-31-no-search-path.md`: the decision this supersedes
- `crates/deed-driver/src/manifest.rs`: the manifest parser
- `crates/deed-driver/src/codes.rs`: `DEED7001` and `DEED7002`
- `crates/deed-driver/tests/manifest.rs`: parser and diagnostic tests
- `crates/deed-cli/src/main.rs`: where the manifest is read during import resolution
