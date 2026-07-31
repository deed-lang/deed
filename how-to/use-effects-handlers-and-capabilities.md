# How do I use effects, handlers, and capabilities?

Declare the effect where the operations belong, implement one or more handlers
for it, and pass capabilities as values so a callee receives only what it needs.

The corpus splits those questions across a few files:

- [`examples/sink.deed`](../examples/sink.deed) declares an effect and two handlers.
- [`examples/greeting.deed`](../examples/greeting.deed) installs those handlers from another module.
- [`examples/config.deed`](../examples/config.deed) narrows a `Dir` before handing it on.
- [`examples/journal.deed`](../examples/journal.deed) shows the row entries over a `Dir` that read, write, list, remove, and make.

Read next:

- [`examples/sink.deed`](../examples/sink.deed)
- [`examples/greeting.deed`](../examples/greeting.deed)
- [`examples/config.deed`](../examples/config.deed)
- [`examples/journal.deed`](../examples/journal.deed)
- [`design/03-effects.md`](../design/03-effects.md)
- [`design/04-capabilities.md`](../design/04-capabilities.md)

Playground: [open](https://deed-lang.github.io/)

```deed sink-and-greeting
// file: examples/sink.deed
module examples/sink

effect Sink {
    fn emit(line: String) -> ()
    fn count() -> Int
}

handler Collect implements Sink {
    state seen: Int

    fn emit(line) -> () {
        seen = seen + 1
    }

    fn count() -> Int {
        seen
    }
}

fn emit_twice(line: String) -> ()
  uses
    Sink.emit,
{
    Sink.emit(line)
    Sink.emit(line)
}

// file: examples/greeting.deed
module examples/greeting

use examples/sink.{Collect, Sink, emit_twice}

fn announce_loudly(line: String) -> ()
  uses
    Sink.emit,
{
    emit_twice(line)
}

test "an effect performed a module away still lands here" {
    with Collect { seen: 0 } {
        announce_loudly("shouting")
        assert Sink.count() == 2
    }
}
```

Playground: [open](https://deed-lang.github.io/)

```deed narrowed-dir
module examples/config

fn describe(files: Dir, name: String) -> String
  uses
    Io.read,
{
    match Io.read(files, name) {
        ok(text) => "found it",
        err(why) => why,
    }
}
```
