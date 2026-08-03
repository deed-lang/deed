# `std/task`

_Generated from `/std/task.deed` and the module's own tests._

## Module

Cooperative tasks.

A task is a function value. `Task.fork` puts one in the queue and `run`
takes them out one at a time until the queue is empty. There is no
preemption and no parallelism: a task runs to completion, and a task that
wants to leave room for another splits itself into a step that runs now and
a closure for the rest, forks the closure, and returns.

The row variable is what makes this a library rather than a pattern to copy.
`Fn() uses r -> ()` is "a task, performing whatever tasks here perform", and
`r` is filled in at each `Task.fork` from the value that was passed. So a
program forking a task that logs is charged with logging, and one forking a
task that reads a file is charged with that, and neither of them is written
down in this file. `examples/scheduler.deed` is the same scheduler with the
row spelled out, which is what had to be done before an effect could take
one, and it is worth reading beside this to see what the variable bought.

What is not here is resumption. `Task.step` calls a task and the task
returns; there is no way to suspend one in the middle and pick it up later,
because `Resume<A, R>` is written down as a decision and not implemented.
See `design/decisions/2026-07-31-one-shot-resumptions.md`, whose own open
question is what the row of a `Resume` is. Until that is answered a task
that wants to yield splits itself, which is what `run_alternating` in the
tests does.

## `run`

### Behavior and limits

Run every task, including the ones the tasks fork, until none are left.

`Diverge` because nothing here proves the queue empties: a task that forks
itself never stops, and that is the program's decision rather than a
mistake this function can rule out.

### Signature

```deed
fn run() -> ()
```

### Row variables

`none`

### Declared row

`Task.more, Task.step, Diverge`

### Contract

```deed
uses
    Task.more,
    Task.step,
    Diverge,
```

### Examples from `std/task.deed`

#### `run empties the queue`

```deed
run()
```

## `run_up_to`

### Behavior and limits

Run at most `limit` tasks, and answer how many actually ran.

The bounded form, for a program that wants a scheduler it can prove
terminates. No `Diverge`: the walk is over a list of known length, so this
stops whatever the tasks do.

### Signature

```deed
fn run_up_to(limit: Int) -> Int
```

### Row variables

`none`

### Declared row

`Task.more, Task.step`

### Contract

```deed
where
    limit >= 0,
  uses
    Task.more,
    Task.step,
```

### Examples from `std/task.deed`

#### `a task forked by a task runs too`

```deed
run_up_to(5)
```

#### `a bounded run stops at the limit and says so`

```deed
run_up_to(2)
```

#### `a bounded run over an empty queue runs nothing`

```deed
run_up_to(3)
```
