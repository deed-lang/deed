# Deed for VS Code

Syntax highlighting, language-server wiring and a debugger for `.deed` files.

## Installing it

The extension has a small client entrypoint plus the grammar and language
configuration. Install its runtime dependency before copying it into your
extensions folder:

```
# from the repository root
cd editors/vscode
npm install

# Windows
cd ..\..
xcopy /E /I editors\vscode %USERPROFILE%\.vscode\extensions\deed

# macOS and Linux
cd ../..
cp -r editors/vscode ~/.vscode/extensions/deed
```

Then restart VS Code and open any file in `examples/`.

## What it does

It colours and starts `deed lsp` over stdio when a Deed file opens.

The compiler's language server provides hover, go to definition, find
references, rename, formatting, document symbols, completion, signature help,
workspace search and quick fixes.

By default the extension launches `deed lsp`. If your binary is somewhere else,
set `deed.server.path`. To pass additional arguments after `lsp`, set
`deed.server.args`.

## Debugging

Press F5 in a `.deed` file, or add a launch configuration:

```json
{
  "type": "deed",
  "request": "launch",
  "name": "Run this file",
  "program": "${file}",
  "stopOnEntry": false
}
```

Breakpoints, step in, step over, step out, the call stack and the bindings of
every active call. A program writes through a `Console` and those lines arrive
in the debug console as it runs.

The adapter is the same binary, started with `debug` instead of `lsp`, so
`deed.server.path` is the only setting. Two paths to one executable would let
an editor talk to two versions of the compiler at once.

Two things it does not do. There is no **pause**: a program stops where it was
told to and runs otherwise, so one with no breakpoints and no end has to be
killed. And there is no **watch expression or conditional breakpoint**, because
both mean running Deed code inside a program that is currently held still.
`design/decisions/2026-08-04-a-place-to-stand.md` has the reasoning for both.

Highlighting comes from a TextMate grammar rather than from the compiler,
which means it works on a file that does not parse. That is the right way
round for the case that matters, a file being typed, but it also means the
grammar is a second copy of the words the compiler reads as keywords: the ones
the lexer reserves, and the ones the parser reads by name in a single position
such as `state`, `refuses` and the `ok` and `err` of an `ensures` clause. It
is held to both by `crates/deed-parser/tests/grammar.rs`, which fails if they
disagree in either direction.

A grammar with no positions colours a word everywhere, so `at(items, 0)` and
`ok(value)` are coloured like the markers they share a spelling with. The
parser's `SOFT_KEYWORDS` says why that is the trade being made.

## Not published

This is not on the marketplace. `deed-lang` publishes nothing yet, and an
extension is a promise to keep something working, which is worth making once
there is more here than colours.
