# How do I check what the backend compiles today?

Read the backend design note for the current boundary, then look at the backend
ratchet tests for the exact floor the repository keeps green.

The important split is:

- [`design/05-backend.md`](../design/05-backend.md) explains what the backend is for and what it still leaves open.
- [`crates/deed-driver/tests/backend_prelude.rs`](../crates/deed-driver/tests/backend_prelude.rs) names which prelude calls and `Io` operations compile through the backend today.
- [`examples/hello.deed`](../examples/hello.deed) and [`examples/journal.deed`](../examples/journal.deed) are the smallest real programs to compare against those tests.

The page to read in the test suite is `backend_prelude.rs`: it is the ratchet
that turns "it compiled when somebody happened to try it" into a checked list.

Playground: [open](https://deed-lang.github.io/)

```deed backend-hello
module examples/hello

fn greet(out: Console, name: String) -> ()
  uses
    Io.write,
{
    Io.write(out, "hello, ")
    Io.write(out, name)
}
```
