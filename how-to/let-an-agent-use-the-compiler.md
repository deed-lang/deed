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

## What it can answer

| Tool | Argument | What comes back |
| --- | --- | --- |
| `deed_check` | `source` | One JSON object a line: `diagnostic` for anything wrong, `obligation` for every contract clause the checker looked at. Silence means the program is well formed. |
| `deed_test` | `source` | One line per `test` block, with the failing diagnostic when one failed, then a `summary` line counting them. |
| `deed_run` | `source` | The lines `main` printed, then whether it finished. |
| `deed_fmt` | `source` | The one layout the formatter chooses, or the parse diagnostics. |
| `deed_fix` | `source` | The program with every machine-applicable repair applied, and how many went in. |
| `deed_explain` | `code` | The page for one diagnostic code, like `DEED4025`. |

Every tool takes a whole module, because that is the unit this language has.
[`design/refusals.md`](../design/refusals.md) says why there is no REPL, and the same
reasoning applies: there is no expression to evaluate on its own.

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

## What it may not do

Nothing, and that is on purpose.

The server holds no capability. A program arrives as text and the answer leaves as text: it
opens no file, resolves no path, and `deed_run` refuses a program whose row asks for a
directory *before* running it rather than part way through.

That is [`design/04-capabilities.md`](../design/04-capabilities.md)'s rule applied to the
compiler's own tooling. It also has a cost worth knowing before you hit it: there is no root
here, so a `use` that names another one of your files cannot be resolved and comes back as
`DEED3007`. Send the module set as one program, or check those files with `deed check` on a
real path.

The reasoning in full, including what was rejected, is in
[`design/decisions/2026-07-31-agent-surface.md`](../design/decisions/2026-07-31-agent-surface.md).

## Related

- [How do I check a file or module set?](check-a-file-or-module-set.md) for the same
  questions from a terminal.
- [How do I use Deed in an editor?](use-deed-in-an-editor.md) for the same questions from an
  editor. `deed lsp` and `deed mcp` are two doors into one compiler.
