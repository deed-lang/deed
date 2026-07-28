# Deed for VS Code

Syntax highlighting for `.deed` files.

## Installing it

There is nothing to build. The extension is three JSON files and no code, so
copying the directory into the extensions folder is the whole install:

```
# Windows
xcopy /E /I editors\vscode %USERPROFILE%\.vscode\extensions\deed

# macOS and Linux
cp -r editors/vscode ~/.vscode/extensions/deed
```

Then restart VS Code and open any file in `examples/`.

## What it does and does not do

It colours. That is all it does today.

The compiler already has a language server with hover, go to definition, find
references, rename, formatting, document symbols, completion, signature help,
workspace search and quick fixes, and none of it is wired up here yet. Until
it is, run `deed check` in a terminal. `../README.md` has the editors that do
start it.

Highlighting comes from a TextMate grammar rather than from the compiler,
which means it works on a file that does not parse. That is the right way
round for the case that matters, a file being typed, but it also means the
grammar is a second copy of the words the compiler reads as keywords: the ones
the lexer reserves, and the ones the parser reads by name in a single position
such as `state` and `refuses`. It is held to both by
`crates/deed-parser/tests/grammar.rs`, which fails if they disagree in either
direction.

## Not published

This is not on the marketplace. `deed-lang` publishes nothing yet, and an
extension is a promise to keep something working, which is worth making once
there is more here than colours.
