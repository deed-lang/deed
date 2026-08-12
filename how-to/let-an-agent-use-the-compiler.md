# How do I let an agent use the compiler?

`deed mcp` speaks the [Model Context Protocol](https://modelcontextprotocol.io) on stdin and
stdout, so a coding agent can ask the compiler questions instead of guessing at the answers
or scraping them out of terminal output.

Start it the way an agent host starts any other MCP server. In a host that reads a JSON
config, that is:

```json
{
  "mcpServers": {
    "deed": { "command": "deed", "args": ["mcp"] }
  }
}
```

There is nothing else to install. `deed` is one binary and the standard library is inside
it, so the server can check a program that says `use std/list` without a `std` directory
existing anywhere.

If you would rather see the answers before wiring anything up,
<https://deed-lang.github.io/agents/> runs the same compiler in a browser tab and prints
what each of the calls below comes back with.

## What it can answer

| Tool | Argument | What comes back |
| --- | --- | --- |
| `deed_check` | `source` | One JSON object a line: `diagnostic` for anything wrong, `obligation` for every contract clause the checker looked at. Silence means the program is well formed. |
| `deed_test` | `source` | One line per `test` block and one per property a contract generates, with the failing diagnostic when one failed, then a `summary` line counting them. |
| `deed_run` | `source` | The lines `main` printed, then whether it finished. |
| `deed_fmt` | `source` | The one layout the formatter chooses, or the parse diagnostics. |
| `deed_fix` | `source` | The program with every machine-applicable repair applied, and how many went in. |
| `deed_explain` | `code` | The page for one diagnostic code, like `DEED4025`. |
| `deed_review` | `before`, `after`, optional `policy` | One receipt for a patch: authority additions, weaker obligation tiers, new Guarded obligations, and any policy violations. |

The order matters, and the server says so in its handshake because an agent that is not
told picks one. `deed_check`, then `deed_fix` for the repairs the compiler is sure about,
then `deed_test`, then `deed_run`. Skipping the third step is the easy mistake: the first
model to be pointed at this server made sixty-five checks across six tasks and never ran a
test, on tasks that were scored on whether their tests pass. Before finishing a patch,
`deed_review` compares every module before and after it.

Checking is not passing. That is not a caveat, it is the distinction the whole language is
arranged around: the check settles what the contract can settle, and `deed_test` runs what
is left, which is the `test` blocks and the properties generated from the contracts.

The one-file tools take a whole module, because that is the unit this language has.
[`design/refusals.md`](../design/refusals.md) says why there is no REPL, and the same
reasoning applies: there is no expression to evaluate on its own. `deed_review` takes arrays
of whole modules so each side of a patch can resolve imports entirely in memory.

## Review the patch

Send every module in each version, including modules needed to resolve a local `use`:

```json
{
  "before": ["module app\n\nfn save() -> () { () }\n"],
  "after": ["module app\n\neffect Audit { fn note() -> () }\n\nfn save() -> () uses Audit.note, { Audit.note() }\n"],
  "policy": {
    "denyNewAuthority": true,
    "denyWeakerPromises": true,
    "denyNewGuarded": true
  }
}
```

The answer is one `review_receipt` JSON object. Without `policy`, findings are evidence and
nothing is denied. With it, read `policy.passed`: `false` is still a successful MCP tool
call, because the receipt is the answer rather than a transport failure. A side with
compiler errors returns its diagnostics and `review_refused`, never a partial receipt.

## The line worth reading

An agent that only reads `diagnostic` lines is reading half the answer. Send this program,
which has nothing wrong with it:

Playground: [open](https://deed-lang.github.io/)

```deed agent-obligation-example
module guide

type Positive = Int where value > 0

fn keep(n: Int) -> Positive {
    n
}
```

`deed_check` reports no diagnostic, because the program is well formed. What it does report
is an obligation:

```json
{"kind":"obligation","tier":"guarded","file":"main.deed","line":6,"column":5,
 "subject":"Positive","reason":"nothing narrowed this name"}
```

`tier` is `proven` when the checker settled the clause at compile time, `tested` when a test
pins it, and `guarded` when it falls to a runtime check. A `guarded` line carries `reason`,
which says what stopped it from being proven, and therefore what would have to change for it
to be. Here it is that nothing told the checker `n` is positive, so adding `where n > 0` to
`keep` turns that line into `proven`.

That is the difference between "the compiler could not prove this" and "here is what to do
about it".

## Check first, and it will hold you to it

`deed_test` and `deed_run` refuse a program that does not check, with one line:

```json
{"kind":"refused","errors":2,"message":"this program does not check, and running it would report the wrong mistake"}
```

They could run it. The interpreter would get partway in and complain about whatever it hit
first, which is a real sentence about the wrong thing: an agent reading it goes looking for a
bug in the code it was executing, when the answer was two lines up in `deed_check`. The
command line has refused this for the same reason since it had a `test` subcommand.

## The test you did not write

`deed_test` runs two kinds of thing. The `test` blocks in the file, and one property per
function whose contract can be exercised: the checker generates inputs that satisfy the
`where` clause and holds the function to its `ensures`.

The second kind is the one worth waiting for. This checks cleanly and passes its written
test:

Playground: [open](https://deed-lang.github.io/play/)

```deed agent-property-example
module guide

fn twice(n: Int) -> Int
  where
    n > 0,
  ensures
    ok  => result > n,
{
    n + n
}

test "twice doubles" {
    assert twice(3) == 6
}
```

and fails its property, because `n + n` overflows near the top of the range and `Int` does
not wrap, so the `ensures` is not true for every `n > 0`. Nobody wrote that test and nobody
had to.

```json
{"kind":"property","function":"twice","cases":9,"seed":"0x5eed1234abcd0001","passed":false,
 "diagnostic":{"code":"DEED6007", ...}}
```

The seed is on the line because a property test you cannot reproduce is a rumour. The
diagnostic names the input it failed on.

## What it may not do

Nothing, and that is on purpose.

The server holds no capability. A program arrives as text and the answer leaves as text: it
opens no file, resolves no path, and `deed_run` refuses a program whose row asks for a
directory *before* running it rather than part way through.

That is [`design/04-capabilities.md`](../design/04-capabilities.md)'s rule applied to the
compiler's own tooling. It also has a cost worth knowing before you hit it: there is no root
here, so the one-file tools cannot resolve a `use` that names another one of your files.
`deed_review` can resolve modules explicitly included in its arrays, but it cannot discover
or fetch one. Use `deed check` on a real path when discovery is the question.

The reasoning in full, including what was rejected, is in
[`design/decisions/2026-07-31-agent-surface.md`](../design/decisions/2026-07-31-agent-surface.md).

## Related

- [How do I check a file or module set?](check-a-file-or-module-set.md) for the same
  questions from a terminal.
- [How do I use Deed in an editor?](use-deed-in-an-editor.md) for the same questions from an
  editor. `deed lsp` and `deed mcp` are two doors into one compiler.
