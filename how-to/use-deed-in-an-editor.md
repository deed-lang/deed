# How do I use Deed in an editor?

Start with [`editors/README.md`](../editors/README.md). It has the exact Helix
and Neovim configuration, the current VS Code state, and the list of language
server features the repository checks.

The important habit is to open the folder, not one file by itself. The language
server reads the workspace roots it was given and answers about every `.deed`
file under them, which is the editor version of `deed check examples/`.

Read next:

- [`editors/README.md`](../editors/README.md)
- [`examples/greeting.deed`](../examples/greeting.deed)
- [`examples/names.deed`](../examples/names.deed)
- [`crates/deed-lsp/tests/session.rs`](../crates/deed-lsp/tests/session.rs)
- [`crates/deed-cli/tests/agreement.rs`](../crates/deed-cli/tests/agreement.rs)

Playground: [open](https://deed-lang.github.io/)

```deed editor-example
// file: examples/names.deed
module examples/names

choice Tone {
    Plain,
    Loud,
}

fn louder(tone: Tone) -> Tone {
    match tone {
        Plain => Loud,
        Loud => Loud,
    }
}

// file: examples/greeting.deed
module examples/greeting

use examples/names.{Loud, Plain, Tone, louder}

fn describe(tone: Tone) -> String {
    match tone {
        Plain => "speaking",
        Loud => "shouting",
    }
}

fn render() -> String {
    describe(louder(Plain))
}

test "the folder answers about both files" {
    assert render() == "shouting"
}
```
