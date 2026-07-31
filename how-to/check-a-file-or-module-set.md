# How do I check a file or module set?

Use `deed check path/to/file.deed` for one file, and `deed check path/to/folder`
when the file imports siblings beside it.

The smallest real example in this repository is
[`examples/greeting.deed`](../examples/greeting.deed) together with
[`examples/names.deed`](../examples/names.deed). `greeting.deed` imports types
and functions from `names.deed`, so checking the directory is what makes the
`use` line mean something:

```text
deed check examples/
```

Read next:

- [`examples/greeting.deed`](../examples/greeting.deed)
- [`examples/names.deed`](../examples/names.deed)
- [`crates/deed-driver/tests/modules.rs`](../crates/deed-driver/tests/modules.rs)

Playground: [open](https://deed-lang.github.io/)

```deed greeting-and-names
// file: examples/names.deed
module examples/names

choice Tone {
    Plain,
    Loud,
}

record Greeting {
    who: String,
    tone: Tone,
}

fn louder(tone: Tone) -> Tone {
    match tone {
        Plain => Loud,
        Loud => Loud,
    }
}

// file: examples/greeting.deed
module examples/greeting

use examples/names.{Greeting, Loud, Plain, Tone, louder}

fn describe(tone: Tone) -> String {
    match tone {
        Plain => "speaking",
        Loud => "shouting",
    }
}

test "a call goes into the other module and comes back" {
    let greeting = Greeting { who: "world", tone: Loud }

    assert describe(louder(Plain)) == "shouting"
    assert greeting.who == "world"
}
```
