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

It publishes diagnostics on open and on change, and answers hover, go to
definition, references, rename, formatting, code actions, document symbol,
completion, signature help and workspace symbol.

Full sync only. The server asks for whole documents rather than incremental
changes, because a sync that can drift out of step with the file is an
optimisation worth measuring before taking.
