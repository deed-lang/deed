# How do I check what the backend compiles today?

It compiles every program in this repository, so the question is now about
what is left rather than what works. Read the backend design note for the
boundary, then the ratchet tests for what is kept green.

The important split is:

- [`design/05-backend.md`](../design/05-backend.md) explains what the backend is for and what it still leaves open.
- [`crates/deed-driver/tests/corpus_backend.rs`](../crates/deed-driver/tests/corpus_backend.rs) compiles every `.deed` file in `examples/` and `std/` and fails naming whatever stopped.
- [`crates/deed-driver/tests/agreement.rs`](../crates/deed-driver/tests/agreement.rs) runs the same program through both engines and compares, which is the one that matters: a backend that compiles more and answers differently is worse than one that refuses.
- [`crates/deed-driver/tests/backend_prelude.rs`](../crates/deed-driver/tests/backend_prelude.rs) names which prelude calls and `Io` operations compile through the backend today.
- [`examples/hello.deed`](../examples/hello.deed) and [`examples/journal.deed`](../examples/journal.deed) are the smallest real programs to compare against those tests.

The page to read in the test suite is `agreement.rs`. A list of what compiles
is worth something only alongside a check that what compiles answers the same
way the interpreter does.

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
