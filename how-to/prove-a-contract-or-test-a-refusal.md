# How do I prove a contract, or test that something is refused?

Put the rule in a refinement or a `where` clause, then either let the checker
prove it or say `assert refuses ...` in a test when the interesting case is that
the rule turns something down.

[`examples/proven.deed`](../examples/proven.deed) is the repository's map of
that territory. It has proofs that discharge at check time, and tests that make
the guarded cases actually refuse.

```text
deed check examples/proven.deed --obligations
deed test examples/proven.deed
```

Read next:

- [`examples/proven.deed`](../examples/proven.deed)
- [`crates/deed-driver/tests/proving.rs`](../crates/deed-driver/tests/proving.rs)
- [`crates/deed-driver/tests/guards.rs`](../crates/deed-driver/tests/guards.rs)

Playground: [open](https://deed-lang.github.io/)

```deed proves-and-refuses
module examples/proven

type Positive = Int where value > 0

record Order {
    quantity: Positive,
}

fn order_of(n: Int) -> Order
  where
    n > 0,
{
    Order { quantity: n }
}

test "a guard turns down what it should" {
    assert order_of(1).quantity == 1
    assert refuses order_of(0)
}
```
