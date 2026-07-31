# How do I write and run tests?

Write `test "..." { ... }` blocks in the same file as the code they exercise,
then run them with `deed test`.

[`examples/counter.deed`](../examples/counter.deed) is the smallest complete
example in the corpus: it declares an effect, installs a handler in a test, and
checks both the return value and the state afterwards.

```text
deed test examples/counter.deed
```

Read next:

- [`examples/counter.deed`](../examples/counter.deed)
- [`README.md`](../README.md)

Playground: [open](https://deed-lang.github.io/)

```deed counter-tests
module examples/counter

type Positive = Int where value > 0

effect Counter {
    fn value() -> Int
    fn bump(by: Positive) -> ()
}

handler InMemory implements Counter {
    state count: Int

    fn value() -> Int {
        count
    }

    fn bump(by) -> () {
        count = count + by
    }
}

fn bump_twice(by: Positive) -> Int
  where
    by > 0,
  uses
    Counter.bump,
    Counter.value,
  ensures
    ok  => Counter.value() == old(Counter.value()) + by + by,
{
    Counter.bump(by)
    Counter.bump(by)
    Counter.value()
}

test "bumping twice adds twice" {
    with InMemory { count: 0 } {
        assert bump_twice(5) == 10
        assert Counter.value() == 10
    }
}
```
