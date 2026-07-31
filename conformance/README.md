# Deed conformance suite (initial slice)

This directory is a neutral artifact: each case states a program and what must happen.

A case is a directory under `conformance/cases/` with:

- `case.txt` metadata
- either a `program.deed` file in the same directory, or a `path` entry pointing to an existing `.deed` file

`case.txt` is line based:

- `mode: check | test | run`
- `expect: accept | reject | run`
- `code: DEEDnnnn` (required for `expect: reject`)
- `stdout: ...` (repeat for each expected output line, used by `expect: run`)
- `path: relative/path.deed` (optional, relative to repository root)

The suite is intentionally small in this PR:

- one accept case from `examples/`
- one reject case with an explicit diagnostic code
- one run case with expected output

Follow-up work can migrate more of the existing corpus and diagnostic-code coverage.
