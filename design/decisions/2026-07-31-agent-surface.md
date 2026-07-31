# Decision: the agent surface is a capability-free MCP server over program text

- Status: Accepted
- Date: 2026-07-31
- Supersedes: None
- Superseded by: None

## Context

`design/00-motivation.md` opens with the claim the whole language rests on: most of the code
being written today is not typed out by a person, which changes which costs matter. Every
other decision in this repository followed from that. The surfaces did not.

Today the compiler answers a person at a terminal (`deed check`), a person in an editor
(`deed lsp`, sixteen capabilities), and a person in a browser (`deed-wasm`, the playground).
An agent had no way in. It could shell out and scrape stdout, which is what every other
language's agent integration does, and which breaks the first time a message is reworded.
This repository rewords messages deliberately and often.

That gap is not cosmetic. The language's pitch is that a signature is a promise the compiler
checks, so the reader does not have to read the body. If the reader is a machine and the
machine cannot reach the compiler, the pitch is untested.

## Decision

`deed mcp` speaks the Model Context Protocol on stdin and stdout, offering six tools:
`deed_check`, `deed_test`, `deed_run`, `deed_fmt`, `deed_fix` and `deed_explain`.

Three properties are load-bearing, and each is held by a test rather than by this document.

### 1. The server holds no capability

A program arrives as text in a tool argument and the answer leaves as text. This server
opens no file, resolves no path against a root, and runs nothing that could reach one:
`deed_run` refuses a program whose row mentions a directory operation *before* running it,
so a program cannot print half its output and then discover it has no filesystem.

This is the language's own rule applied to its own tooling. `design/04-capabilities.md` says
a function can do nothing its signature does not mention. A server that checks untrusted
programs on behalf of an agent is exactly the place where saying that and not meaning it
would be worst.

It costs something real, written here rather than discovered later: an agent working on a
module set has to send the files together, because there is no root to resolve a second
file's `use` against. The shipped library is the one exception, and only because it already
travels inside the binary.

### 2. The answers come from the surface the playground already uses

`deed-wasm` answers the same five questions for a browser: text in, JSON out, one file, no
filesystem. `deed-mcp` calls those functions rather than writing a second copy, and
`crates/deed-mcp/tests/agreement.rs` compares the two byte for byte over a corpus.

The failure this avoids is this repository's most common one: two consumers of one idea
drifting apart, with the drift landing on whichever one nobody is looking at. Nobody watches
an agent's transcript.

### 3. A wrong program is a successful call

MCP has `isError` for a tool call that failed. A program with diagnostics in it did not fail
the call; the list of what is wrong with it *is* the answer to "check this". `isError` is
kept for the call itself going wrong, which here means an unknown tool or a missing
argument. A client that read a diagnostic as a transport failure would retry rather than
read.

## Drawbacks (required)

The single-file limit is the real one. An agent refactoring across modules cannot ask about
the module set today, and the honest answer is that it has to send whole files and will get
`DEED3007` for an import this server cannot resolve.

A second cost: the tool descriptions are prose, and prose an agent reads is prose that can
go stale the way any other prose here can. `every_tool_says_what_it_takes` holds the shape
but cannot hold the wording.

A third: this adds a crate and a protocol to a workspace that has neither dependencies nor
much appetite for surfaces. The JSON reader and writer are `deed-lsp`'s, so the new code is
the transport and the tool table and nothing else.

## Rejected Ideas (required)

- Option: give the server a root directory so `use` resolves and `Io` works.
  - Rejected because: it would hand an agent's arbitrary program the filesystem through a
    server the person running it did not read. If that is wanted later it should be an
    explicit flag naming the directory, which is the same shape `deed run --dir` already
    has, and it should be a separate decision with its own drawbacks section.
- Option: expose the language server over MCP instead, since it already answers everything.
  - Rejected because: LSP is about positions in documents an editor owns. An agent has no
    documents and no cursor; it has a program and a question. Translating between the two
    would be a second protocol's worth of code answering questions nobody asked.
- Option: write the tool results as MCP structured content rather than JSON-in-text.
  - Rejected because: the shape `deed check --format json` writes is already published and
    already read by the playground. A second shape here would be the drift this decision
    spends a whole test avoiding. Worth revisiting when the structured-content field is
    universally supported by clients.
- Option: ship it as a separate binary.
  - Rejected because: `deed` is one file that carries the compiler, the formatter, the test
    runner and the language server. A second file is a second thing that can be missing or
    the wrong version, which is the argument the standard library being embedded already
    made.

## Open Questions (required)

- Whether a module-set tool (several files in one call) is worth adding, or whether that is
  really asking for the root this decision refused.
- Whether agents read the `reason` on a guarded obligation at all. The handshake's
  `instructions` field points at it explicitly, which is a guess about what an agent needs
  told. Nothing measures whether it helps.
- Whether the tool names should be `deed_check` or `check`. Prefixed here because an agent
  host mixes servers and an unprefixed `check` says nothing about what it checks.

## References

- `design/00-motivation.md` (who writes code now, and which costs that changes)
- `design/04-capabilities.md` (authority is an argument, not an ambient right)
- `design/refusals.md` (no REPL: why the unit is a module and not an expression)
- `crates/deed-mcp/src/lib.rs` (the transport, and the capability claim in full)
- `crates/deed-mcp/tests/agreement.rs` (the agent and the page get the same answer)
- `crates/deed-cli/tests/cli.rs` (`mcp_speaks_the_protocol_on_stdin_and_stdout`)
