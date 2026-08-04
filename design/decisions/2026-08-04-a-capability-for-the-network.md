# Decision: a capability for the network

- Status: Accepted
- Date: 2026-08-04
- Supersedes: None
- Superseded by: None

## Context

`design/04-capabilities.md` opened with `uses Net.send` as an illustration and then said
the illustration was "the shape of the argument rather than the state of the compiler."
That was true for nine months. The consequence was larger than a missing operation:
`deed build --component` derives a WIT world from a program's effect rows, which is the
one thing in this repository nobody else does, and the components it produced could not
make a request. A component model story whose components can only compute is a
demonstration.

The ten `Io` operations were console, clock and filesystem. Nothing reached a machine
other than the one running the program.

## Decision

A fifth capability, `Net`, taken out of `System` as `sys.net`, with three operations.

| Operation | Signature | What it is |
| --- | --- | --- |
| `Io.reach` | `(Net, String) -> Result<Net, String>` | `Io.open` for the network |
| `Io.fetch` | `(Net, String) -> Result<String, String>` | `Io.read` for the network |
| `Io.send` | `(Net, String, String) -> Result<String, String>` | `Io.save` for the network |

Three things follow from the existing design rather than being chosen here.

**Three entries in the row, not one.** Holding a `Net` says nothing about whether a
function may read from a host or change something on it, which is the same split `read`
and `save` already get about the same `Dir`. This is the sixth time that split has been
applied and the sixth time no new mechanism was needed.

**Narrowing, and no way back.** `Io.reach` is the third operation that hands a capability
back, after `Io.open` and `Io.make`. The rule those two rest on is not that a capability
cannot be produced but that what comes back reaches strictly less than what went in.
`crates/deed-rt/src/reach.rs` decides this for every runtime rather than for the
interpreter, the way `sandbox` already does for `Dir`, and
`reaching_narrows_and_there_is_no_way_back` is the escape test the other two have.

**A host matches by equality, never by suffix.** Granting `example.com` does not grant
`evil-example.com` or `example.com.evil.net`. Suffix matching is how an allowlist becomes
a suggestion, and it has its own test naming four pretenders.

**The grant starts empty.** `--dir` defaults to the working directory and `--allow`
defaults to nothing, which looks inconsistent and is not the same question. Running a
command in a directory is already a choice about that directory. There is no equivalent
ambient choice about the network, so a default has nothing to inherit, and "the network"
is not a place anyone is standing.

## Drawbacks (required)

**The interpreter speaks `http` and not `https`.** A TLS client is a cryptographic
implementation and this workspace has no dependencies; writing one to make a test go green
would be the least trustworthy code here. `https` is refused by name with that reason
rather than failing to connect, because a reader who gets "connection reset" from an
`https` URL goes looking at their own network. A compiled component is unaffected: it asks
its host for `deed:io.fetch` and the host speaks whatever it likes. The cost is real all
the same, since almost nothing on the public internet answers on `http` in 2026, so the
interpreter's network is mostly good for talking to a machine you also run.

**The status code is not available as a number.** A `Result` carries one value and this
language has no tuple, so the shapes available were "the body" and "a string with the
status in it". A status outside the two hundreds becomes an `err` carrying both, and a
program that wants to branch on `429` rather than `404` has to read a message. See the
open question below.

**No redirects, no connection reuse, no streaming, no timeouts a program can set.** Each
of those is written down in `crates/deed-rt/src/http.rs` with the reason. The redirect one
is a decision rather than an omission: a redirect is a second request to a second host,
and deciding to make it is exactly what the capability exists to put in the caller's hands.

**Time is still unbounded from the program's side.** The client carries a thirty second
timeout so that a host that accepts a connection and never answers cannot hang a run
forever. A program cannot choose a different one, and `design/04-capabilities.md` still
lists time and memory as resources nothing bounds.

## Rejected Ideas (required)

- Option: a `Net` that reaches everything unless narrowed, matching `--dir`'s default.
  - Rejected because: the working directory is a choice somebody already made by being
    there. There is no ambient network choice for a default to inherit, and the failure
    mode of getting this wrong is a program that phones home because it was run.

- Option: suffix matching, so `example.com` grants `api.example.com`.
  - Rejected because: it is one character away from granting `example.com.evil.net`, and
    a reader cannot tell by looking which of the two a given implementation does. A
    program that wants a subdomain names it.

- Option: one operation, `Io.request(net, method, url, body)`, with the method as a string.
  - Rejected because: the method would be a value rather than part of the row, so a
    signature would stop saying whether a function may change something on the other end.
    That is the distinction `read` and `save` exist to make, and putting it in an argument
    hides it exactly where the reviewer is looking.

- Option: bundling TLS by vendoring a crypto implementation.
  - Rejected because: zero dependencies is a rule this workspace has kept through a
    compiler, a language server, a WASM backend and an MCP server, and the first thing to
    break it should not be a component nobody here can audit. The compiled path does not
    need it, which is where a real program runs.

- Option: refusing `https` by failing to connect, and letting the message come from the
  operating system.
  - Rejected because: the reader would look at their network. The runtime knows exactly
    why it cannot and says so.

- Option: an `Io.listen`, so a program can serve as well as call.
  - Rejected because: a server is a program that does not finish, and nothing in this
    language expresses that yet. It also needs concurrency the compiled backend does not
    have. Nothing in the corpus asks for it.

## Open Questions (required)

- Whether the prelude should declare a record so an answer can carry its status, its body
  and its headers separately. That is a larger decision than one operation gets to make:
  the prelude is twenty-three names and every addition is argued for, and a record there
  would be the first data type the language ships that is not a container. `Io.list` and
  `Io.read` did not need one, and this is the first operation that does.

- Whether `--allow` should also gate the compiled path. Today it gates the interpreter,
  and a compiled component is gated by whichever host offers `deed:io.fetch`, which is
  stronger. But `deed run --compiled` runs a compiled module through this repository's own
  test oracle, and that oracle has no host at all, so the question has not come up.

- Whether a `Net` should carry a path prefix as well as a host, the way a `Dir` carries a
  directory rather than a filesystem. `Io.reach` narrows to a host and stops there. A
  grant of `example.com/v1` is expressible and nothing has asked for it.

- What an effect row means when the callee is on the other end of one of these. This was
  already open in `design/03-effects.md` and is now open with something pointing at it.

## References

- `crates/deed-rt/src/reach.rs`, and `crates/deed-rt/src/sandbox.rs` beside it.
- `crates/deed-rt/src/http.rs`.
- `crates/deed-driver/tests/capabilities.rs`, the network section, which serves every
  request from a loopback port the operating system picked rather than from a real host.
- `crates/deed-driver/tests/wit_world.rs`, `reaching_the_network_appears_in_the_world`.
- `design/decisions/2026-07-31-row-to-wit-world-mapping.md`, which this makes worth
  something.
