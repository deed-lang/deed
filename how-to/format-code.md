# How do I format code?

Run `deed fmt path/to/file.deed` to rewrite a file into the one canonical form,
or use `deed fmt --check path/to/file.deed` when you want the command to fail
instead of rewriting.

The formatter is intentionally simple: one output shape, no style options. The
examples directory is a good source of real input, because those files are kept
formatted in the test suite already.

```text
deed fmt examples/hello.deed
deed fmt --check examples/
```

Read next:

- [`examples/hello.deed`](../examples/hello.deed)
- [`README.md`](../README.md)
- [`crates/deed-fmt/tests/repository.rs`](../crates/deed-fmt/tests/repository.rs)

Playground: [open](https://deed-lang.github.io/)

```deed formatting-example
module examples/hello

fn greet(out: Console, name: String) -> ()
  uses
    Io.write,
{
    Io.write(out, "hello, ")
    Io.write(out, name)
}
```
