# Editors

The compiler ships a language server. `deed lsp` speaks LSP over stdin and
stdout, which is all any of these editors want.

## What each one gets today

|  | highlighting | language server |
| --- | --- | --- |
| VS Code | yes, `editors/vscode` | not yet |
| Helix | no | yes |
| Neovim | no | yes |
| anything else that speaks LSP | no | yes |

The split is not an accident. VS Code takes a TextMate grammar, which is a
data file this repo can carry and hold to the compiler with a test. Helix and
Neovim highlight with tree-sitter, which needs a grammar written in a
different language and built as a shared library, and there is not one yet.

The other half is the reverse. Helix and Neovim start a language server from
a few lines of configuration, and VS Code needs an extension with code in it.
There is not one of those yet either.

## Getting the binary

```
cargo install --path crates/deed-cli
```

Then `deed lsp` is on the path. Everything below assumes that.

## Helix

In `~/.config/helix/languages.toml`:

```toml
[language-server.deed]
command = "deed"
args = ["lsp"]

[[language]]
name = "deed"
scope = "source.deed"
file-types = ["deed"]
language-servers = ["deed"]
```

`hx --health deed` says whether it found the binary.

## Neovim

Needs 0.10 or later for `vim.fs.root`. In your config:

```lua
vim.filetype.add({ extension = { deed = "deed" } })

vim.api.nvim_create_autocmd("FileType", {
  pattern = "deed",
  callback = function()
    vim.lsp.start({
      name = "deed",
      cmd = { "deed", "lsp" },
      root_dir = vim.fs.root(0, { "Cargo.toml", ".git" }),
    })
  end,
})
```

No plugin. `vim.lsp.start` is in the editor.

## Anything else

The server reads `workspaceFolders` from `initialize` and answers about every
`.deed` file under them, so point whatever you use at the folder rather than
at one file. The modules that ship inside the compiler are checked alongside
them, so a `use std/list` is not an error in an editor any more than it is on
the command line. They are context rather than files: nothing is reported
against one, and go to definition and rename do not lead into one, because
there is no file behind it to open or to change.

It publishes diagnostics on open and on change, and answers hover, inlay hint,
go to definition, type definition, references, document highlight, rename, formatting, code actions,
document symbol, folding range, selection range, document link, completion, signature help and workspace symbol.

A hover carries the part of a signature a diagnostic never gets to. On a
function's name it quotes the contract, which is what that function requires,
performs and guarantees. On anything an obligation covers it names the tier
that obligation landed in, the same three words `deed check --obligations`
prints and the same table it prints them from, `proven` included: an editor
that showed only the ones that went wrong would leave a reader unable to tell a
discharged contract from a question nobody asked. The contract comes from the
file being hovered, so a name imported from another module answers with its
type and not its contract, and go to definition is one keypress away from the
rest.

The inlay hints are the same tiers, said without being asked. A hover answers
about one position and a screen of code has many, so finding out how much of
what you are looking at was proven meant hovering each call in turn. Every
obligation in the range the editor asks about gets its tier written at the end
of what it covers, and two that end in the same place and landed in the same
tier are one hint rather than two.

Full sync only. The server asks for whole documents rather than incremental
changes, because a sync that can drift out of step with the file is an
optimisation worth measuring before taking.

## What holds this file to the compiler

Nothing here is prose on its own. `crates/deed-lsp/tests/session.rs` starts a
server and compares what it advertises against the sentence above naming what
it answers, in both directions, so a provider that lands without a line here
fails the build and so does a line here with no provider behind it. What it
compares them through is a rule and not a list: a capability is written here
the way its field name reads with `Provider` taken off, and the handful that
are not are each held to the field they claim to be about. What a hover says
about a contract is held in the same file, one test for each clause a contract
can have and one for a name that came from another module. That the tier is the
terminal's answer and not a second one is held by
`crates/deed-cli/tests/agreement.rs`, which runs the real binary and an
in-process server over the same file and compares them position by position.
The keywords the VS Code grammar paints are compared with the lexer's in
`crates/deed-parser/tests/grammar.rs`. The module named in `use std/list` above
is checked against the ones that really ship in
`crates/deed-driver/tests/documentation.rs`, whose header says why these checks
sit beside the things they read rather than together.
