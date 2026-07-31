# Deed tutorial: write one program, one step at a time

This tutorial teaches the language in one order:

1. a function
2. a contract
3. what `where` buys
4. an effect row
5. why the row is not a comment
6. a capability
7. a handler
8. the three tiers: proven, tested, guarded

Each step starts when the last step is not enough.

## 1) Start with a function

You can write a plain function first.

```deed step-01-function
module tutorial/step01

fn greet(name: String) -> String {
    "hello, " + name
}

test "a function call returns a value" {
    assert greet("deed") == "hello, deed"
}
```

This works, but it says nothing about guarantees.

## 2) Add a contract

A contract can say what a function promises about the result.

```deed step-02-contract
module tutorial/step02

fn doubled(n: Int) -> Int
  ensures
    ok => result == n + n,
{
    n + n
}

test "the postcondition matches the body" {
    assert doubled(21) == 42
}
```

This is better, but callers can still pass values you do not want.

## 3) Use `where` so callers must meet a precondition

`where` lets you state what must be true before the body runs.

```deed step-03-where
module tutorial/step03

fn half_of(n: Int) -> Int
  where
    n > 0,
{
    n / 2
}

test "a bad call is refused" {
    assert half_of(10) == 5
    assert refuses half_of(0)
}
```

Now the call site must prove the precondition, or the runtime checks it.

## 4) Add an effect row when the function touches the outside world

Next problem: this function writes to the console, but it does not declare that effect.

```deed step-04-missing-row fails
module tutorial/step04

fn shout(out: Console, message: String) -> () {
    Io.write(out, message)
}
```

That fails to check. The effect must be declared.

```deed step-05-row
module tutorial/step05

fn shout(out: Console, message: String) -> ()
  uses Io.write,
{
    Io.write(out, message)
}
```

## 5) The row is checked, not a comment

If you declare an effect you do not perform, that also fails.

```deed step-06-row-not-comment fails
module tutorial/step06

fn shout(out: Console, message: String) -> ()
  uses
    Io.write,
    Io.read,
{
    Io.write(out, message)
}
```

So the row is a checked boundary.

## 6) A capability controls which resource you can touch

Another failure: naming `console` out of thin air does not work.

```deed step-07-missing-capability fails
module tutorial/step07

fn shout(message: String) -> ()
  uses Io.write,
{
    Io.write(console, message)
}
```

You need a capability value, and you pass it in.

```deed step-08-capability
module tutorial/step08

fn shout(out: Console, message: String) -> ()
  uses Io.write,
{
    Io.write(out, message)
}
```

## 7) A handler supplies an effect implementation

Now define an effect and handle it with state.

```deed step-09-handler
module tutorial/step09

effect Log {
    fn note(message: String) -> ()
}

handler ConsoleLog implements Log {
    state out: Console

    fn note(message) -> ()
      uses Io.write,
    {
        Io.write(out, message)
    }
}

fn announce(name: String) -> Int
  uses Log.note,
{
    Log.note("hello, " + name)
    0
}

fn main(sys: System) -> Int
  uses Io.write,
{
    with ConsoleLog { out: sys.console } {
        announce("deed")
    }
}
```

## 8) End with one working program and read its tiers

This final program is the same shape, with one more goal: show all three obligation tiers.

- **proven**: a caller satisfies a `where` clause at check time
- **tested**: an `ensures` obligation is checked by generated tests
- **guarded**: the checker cannot prove it, so runtime keeps the guard

```deed final-program
module tutorial/final

type Positive = Int where value > 0

effect Log {
    fn note(message: String) -> ()
}

handler ConsoleLog implements Log {
    state out: Console

    fn note(message) -> ()
      uses Io.write,
    {
        Io.write(out, message)
    }
}

fn checked_port(port: Int) -> Positive
  where
    port > 0,
{
    port
}

fn doubled(n: Int) -> Int
  ensures
    ok => result == n + n,
{
    n + n
}

fn uncertain(n: Int) -> Positive {
    n
}

fn announce(port: Positive) -> Int
  uses Log.note,
{
    Log.note("server on " + to_string(port))
    0
}

fn main(sys: System) -> Int
  uses Io.write,
{
    with ConsoleLog { out: sys.console } {
        let port = checked_port(8080)
        let _same = doubled(port)
        announce(port)
    }
}
```

Run this and you get a small program that writes a line. Check it with obligations enabled and you can inspect the three tiers on code you just built.
