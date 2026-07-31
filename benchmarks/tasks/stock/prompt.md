# Task: stock

Write a module named `bench/stock`.

Declare a refinement type:

```
type InStock = Int where value > 0
```

Then export:

```
fn take_one(count: InStock) -> Int
fn restock(count: Int, delivered: Int) -> InStock
```

`take_one(count)` gives back one fewer than it was handed.

`restock(count, delivered)` gives back the count with the delivery added to it.

The second one is the point of the task. `restock` promises to give back
something the checker will accept as `InStock`, and the checker only accepts it
if it can see that the result is above zero. Constrain the parameters with a
`where` clause so that it can, rather than leaving it to a runtime check.

You can tell whether you managed it: `deed check --obligations` says `proven`
when the checker settled it and `guarded` when it did not.

Write only the module. No `main`, no tests.
