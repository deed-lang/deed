# Decision: A place to stand, and everything else outside

- Status: Accepted
- Date: 2026-08-04
- Supersedes: `design/decisions/2026-07-31-debug-adapter-feasibility.md`
- Superseded by: None

## Context

The feasibility record staged this work and then stopped, on the grounds that a
transport-only adapter "would look complete to editors but fail the core
runtime behavior users need". That was right. What it left open was the part
that decides everything else:

> Which public API boundary should own snapshot and pause control: `deed-interp`
> directly or a new runtime facade crate?

A facade is not available. The place a program can be observed is inside the
recursive evaluator, and a crate wrapped around `deed-interp` cannot reach it.
A facade could only offer what `run_main` already offers, which is an answer
after the fact.

So the hook is in `deed-interp`. The question that mattered was how much goes
with it.

## Decision

One method, and it decides nothing.

```rust
pub trait Watcher {
    fn at(&mut self, paused: Paused<'_, '_>);
}
```

The interpreter calls this before each statement and carries on when it
returns. **A watcher stops by not returning.** That is the whole of suspension:
the host stack is the program's stack, so leaving it alone is what makes the
state a debugger reads the state the program is in. Nothing is re-run, nothing
is checkpointed, and there is no second execution model to keep in step with
the first.

Everything the feasibility record listed as stages 3 and 4 — what a breakpoint
is, what "over" means, when to stop — lives in `deed-dap`. The interpreter does
not know what a line is. Teaching it would put one decision in two crates, and
the failure mode of that is a debugger that stops somewhere the compiler does
not agree is anywhere.

### The three open questions, answered

**Pause granularity: the statement, and a block's tail expression.**

Not every expression: `total(x, y) + 1` would stop four times on one line and a
reader stepping through it could not say what had happened between two of them.
Not statements alone either, because `fn answer() -> Int { 42 }` has none, and a
debugger that cannot stop in a function that is one expression long is a
debugger that cannot stop in most of `std`.

**Ownership: `deed-interp`, with the policy outside it.** Above.

**How to describe a `perform`.** It is not described. An operation is called
through `Interp::call` like anything else, so its frame appears in the stack
with the handler's module on it, and stepping into a `perform` lands in the
handler body with no special sentence attached. The feasibility record expected
this to need "clear stop reasons so frontends can render the jump", and it does
not, because the jump is a call and the stack says where it went. A stop reason
that said "you are in a handler now" would be a second explanation of something
the stack trace already shows, and the two would drift.

### What crosses a thread

The program runs on its own thread so that the adapter is still able to answer
while it is held still. Only rendered text crosses: a `Value` is full of `Rc`,
and a stack trace made of `String` cannot be a reference count that got loose.

`Session::handle` is synchronous, including for the requests that resume: it
comes back with the events up to the next stop. That is what makes a session a
function from a message to messages, which is the property that makes
`crates/deed-dap/tests/session.rs` possible to write without timing in it.

### What is paid when nobody is watching

One `Option::is_some` per statement, and a call stack that is only maintained
while a watcher is installed.

Measured with `cargo run -p deed-driver --example interpreting --release`,
against the same tree with the two `self.watch(..)` calls removed:

| | with the hook | without |
| --- | --- | --- |
| a turn of a `for` | 54ns, 48ns | 50ns |
| a call with a contract on it | 578ns, 555ns | 538ns |
| an operation that reads state | 539ns, 508ns | 489ns |

**The difference is inside this harness's run-to-run spread**, which is what
the two numbers in the first column are there to show: repeating the same
build moves "a turn" by about as much as removing the hook does. So this is
not a claim that the hook is free, it is a statement that a 100,000-turn
benchmark cannot see it, and a smaller claim than that would not be honest.

What would catch a real regression is `deed-driver::scaling`, which fails when
checking stops being close to linear, and re-running the table above.

## Drawbacks (required)

**There is no `pause`.** While the program runs, nothing reads the client's
stream, so a request to interrupt cannot arrive. A program with no breakpoints
and no end runs until the process is killed. This is the price of the
synchronous session, and it is stated in `deed --help` rather than discovered.

**Watching costs a branch on the hot path of every run.** The alternative was a
compile-time feature, which would mean the debugger is not there unless
somebody built it in, and a debugger you have to rebuild for is a debugger
nobody has when they need one.

**A watcher can be wrong about depth in a way the interpreter cannot correct.**
Stepping is defined against the number of active calls, which is a number the
adapter is handed rather than one it can check.

**Only the interpreter is debuggable.** A program run through the compiled
backend has no watcher and cannot get one; the hook is in the evaluator, and
WebAssembly has its own debugging story that this does not touch.

**Imports are found beside the program, not through a manifest.** The rule is
the one `deed check` uses for a file that is where its module name says it
should be. A program whose modules arrive through `deed.manifest` — a component
root or a fetched module — is compiled by the CLI and not by the adapter, so
debugging one is not yet possible.

**Nothing is evaluated in the debugger.** There is no `evaluate` request, so a
watch expression and the debug console have nothing to answer them. Evaluating
an expression means running Deed code inside a watcher, which means the
interpreter is lent out and re-entered, and the shape that makes suspension free
is exactly the shape that makes that unsafe. It is a real gap and it is not a
small one to close.

**Values do not expand.** A record is rendered whole into one string rather than
being a tree a client can open. For a list of a thousand elements this is a
thousand elements on one line.

## Rejected Ideas (required)

- Option: rewrite the evaluator as a state machine so it can be suspended and
  resumed without a thread.
  - Rejected because: it is a rewrite of the thing every test in the project is
    about, to obtain something a blocking call already gives. The recursive
    evaluator is also what makes the stack a debugger reads real.

- Option: put breakpoints and stepping in `deed-interp`, and have the adapter
  only carry messages.
  - Rejected because: the interpreter would need lines, files as an editor
    spells them, and a notion of what a client meant by "over". Every one of
    those is a protocol question, and answering them twice is how a debugger
    stops somewhere the compiler says is nowhere.

- Option: call the watcher at every expression rather than every statement.
  - Rejected because: stepping would stop several times on one line with no way
    to say what changed between two stops. Statement granularity is what a
    reader can follow.

- Option: give `Paused` the values themselves rather than rendered text.
  - Rejected because: a `Value` holds `Rc`, so it belongs to the thread that
    made it. Handing one across would either force the interpreter onto atomic
    reference counts — paid by every run, for the runs that are being debugged —
    or be a data race with a type signature saying otherwise.

- Option: run the adapter and the program on one thread, calling back into a
  reader from inside the watcher.
  - Rejected because: the watcher would be reading the protocol stream from
    inside the interpreter, and a malformed message would surface in the middle
    of an evaluation with nowhere to report it.

- Option: report a distinct stop reason when execution enters a handler.
  - Rejected because: the stack already says which module and which function.
    See above.

## Open Questions (required)

- Whether `evaluate` can be answered at all, and if so what a re-entrant call
  into a held interpreter is allowed to do. An expression with an effect in it
  would be performed against handlers installed by the program, which is
  either exactly what somebody wants or the reason their next step is wrong.

- Whether a conditional breakpoint is the same question. It is `evaluate` with
  the answer read rather than shown, so it waits on the same thing.

- Whether a program run through the compiled backend should be debuggable, and
  by what. Nothing in this decision extends to it.

- Whether the adapter should read a `deed.manifest`, or whether that resolution
  should move out of `deed-cli` so that everything which compiles a program
  finds the same files.

## References

- deed-lang/deed#584, deed-lang/deed#659
- `design/decisions/2026-07-31-debug-adapter-feasibility.md`
- `crates/deed-interp/src/watch.rs`
- `crates/deed-dap/src/stepper.rs`
- `crates/deed-dap/tests/session.rs`
