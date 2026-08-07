# Decision: what a host hands a compiled program

- Status: Accepted
- Date: 2026-08-07
- Supersedes: None
- Superseded by: None

## Context

`design/05-backend.md` says a compiled program's import section is its capability
requirements, and `crates/deed-driver/tests/host.rs` has checked that claim from both
sides for months: a module that does not name an operation has no index to call it
through, and a module that names one the host cannot answer is refused at link time.

Nothing crossed the boundary. Measured, on the day this was written:

```
$ deed run --compiled examples/hello.deed
`deed:sys.console` is the host's to answer, and this is not one
```

The interpreted program printed "hello, world". The compiled one could not say anything
at all, which made every sentence on that page a sentence about an import section rather
than about a program.

The obstacle was not wiring. A host implementation was `Fn(&[Value]) -> Option<Value>`,
and `Io.write(console, text)` passes the text as an address into the module's own memory,
so there was no host anybody could write for any operation carrying something other than
a number. The type had no way to say "and the memory".

The second question was one nobody had had to answer, because nothing had ever handed a
capability across: a capability compiles to an opaque number, and the module's memory is
full of numbers. What stops one being mistaken for the other?

## Decision

Three parts.

**A host is handed the call.** `HostCall` carries the arguments and the module's linear
memory, with `text` to read a Deed string out of it and `write_text` to put one in.
`write_text` allocates from the module's own bump pointer, so what a host answers with is
an ordinary value of the program's rather than something out of a second heap the program
has no way to name. A host that has run out of room says so instead of writing past the
end.

**A host may refuse.** `Fn(HostCall) -> Result<Option<Value>, Trap>`, with a new
`Trap::Refused` carrying the sentence. A host with no way to turn a call down has to
answer every call, including the ones it should not.

**A capability is an index into a table the host keeps, and the table says what each one
is.** `crates/deed-codegen/src/grant.rs`. `Grants` is built one grant at a time and turned
into a `Host` that offers exactly the imports those grants can answer. The handle is one
past the index, so zero is never a capability and an uninitialised word is refused rather
than mistaken for the console. Every operation looks its first argument up in the table
and checks the tag before doing anything, so:

- a number the host never handed out is not a capability, it is a number
- a handle for the console passed where a clock belongs is refused

Neither is reachable from a checked Deed program: a capability is a value with a type and
nothing in the language turns an integer into one. The check is there because a host is
handed modules rather than programs, and a host whose safety rests on the last compiler
that touched the module is not enforcing anything.

Nothing is granted by default. `deed run --compiled` grants what `deed run` grants: a
console, a clock, the directory `--dir` named, the variables `--env` named, the arguments,
and standard input when and only when `main`'s row says the program reads it. So
`examples/hello.deed` prints what the interpreted one prints, and a program that reaches
the network is turned down before it runs:

```
$ deed run --compiled examples/todo.deed --dir examples
examples
nothing asked for
5 of 6 done
still open: 6. work out what a trait is

$ deed run --compiled reaches.deed
the host does not offer `deed:io.fetch`
```

A `Dir` handle is interned by resolved path rather than one entry per `Io.open`, so a
program that opens the same place in a loop does not grow the table for as long as it
runs. Two handles to one directory reach the same things, so there is nothing to tell
apart.

## Drawbacks (required)

The thing on the host's side of a handle is reachable by anything holding the table, so
the table is the security boundary and it is a `Vec` behind an `Rc<RefCell<_>>`. That is
right for one program in one process and is not a claim about anything else.

`Io.now`, `Io.epoch` and the six filesystem operations duplicate what the interpreter
already does, sentence for sentence, because the answers have to match and there is no
shared place to put them: the interpreter's version works on `Value`, this one on a handle
table and the module's memory. The rules about what a `Dir` reaches are not duplicated --
both ask `deed_rt::sandbox` -- but the messages around them are, and
`crates/deed-cli/tests/cli.rs` holds the two engines to saying the same thing rather than
trusting that they will.

The network is still an unanswered import. `deed_rt::reach` and `deed_rt::http` are the
host half and `--allow` is the grant, so this is wiring rather than a decision, but it is
not wired.

Giving a host implementation the module's memory means giving it the ability to corrupt
the program it is answering. That is the same authority any embedder has and the reason
`write_text` refuses rather than truncates, but it is a wider surface than passing values.

A compiled `examples/logs.deed` still stops with "reached past the end of memory" where
the interpreted one finishes. That is not the host: a module gets sixteen pages and
nothing but handler frames is ever reclaimed, which `design/05-backend.md` already says
and which value reclamation is the answer to.

## Rejected Ideas (required)

- Option: keep `Fn(&[Value]) -> Option<Value>` and pass strings by copying them into a
  side channel the host reads.
  - Rejected because: the address is already in the argument, and a second channel is a
    second layout to keep in step with `crates/deed-codegen/src/layout.rs`. The bug that
    finds would be a silently wrong string.

- Option: `Fn(&[Value], &mut [u8]) -> ...`, memory as a second parameter.
  - Rejected because: every future addition to what a host is told is another parameter
    and another rewrite of every implementation. A struct grows a method.

- Option: let a host answer with `None` where a value was wanted, instead of adding
  `Trap::Refused`.
  - Rejected because: the caller's stack would be one value short and the module would
    fail somewhere else, about something else. A refusal has to be reported where it
    happened.

- Option: make the capability a pointer to a host-side object written into the module's
  memory.
  - Rejected because: the program could then read it, compare it, and construct one. The
    handle exists to be opaque, and a number that indexes a table on the far side of the
    boundary is the cheapest opaque thing there is.

- Option: trust the handle, since a checked Deed program cannot forge one.
  - Rejected because: a host links modules, not programs. Trusting the module makes the
    compiler part of the trusted computing base of everything that ever runs its output.

- Option: write the `ok` and `err` tags into the host, since they are 0 and 1.
  - Rejected because: nobody writes a `Result` down, so the order is the compiler's and
    a copy of it here is an answer that is inverted rather than wrong. `deed_mir` owns
    `RESULT_VARIANTS` and both sides read it.

- Option: hand out a new `Dir` handle per `Io.open`.
  - Rejected because: a program that opens the same directory in a loop grows the host's
    table for as long as it runs, and two handles to one directory are not two things.

- Option: keep the runner's instruction budget for `deed run --compiled`.
  - Rejected because: the budget is the size of a test, and it stopped
    `examples/logs.deed` part way through and called it running too long. Whoever runs a
    program says how far they are willing to count.

## Open Questions (required)

- Where the shared answer to the `Io` operations should live. There are two of them now,
  one per engine, held together by a test rather than by construction.
- Whether the network should be granted the same way, given that `--allow` already names
  hosts and `deed_rt::reach` already decides what a `Net` reaches.
- What a host should do when a module hands it a string the layout says is there and the
  bytes say is not valid UTF-8. Today that is a refusal, which is right for a module that
  does not match its own signature and may be wrong for one that read bytes off a disk.

## References

- `crates/deed-codegen/src/grant.rs`, `crates/deed-codegen/src/run.rs`
- `crates/deed-driver/tests/host.rs`, `crates/deed-cli/tests/cli.rs`
- `design/05-backend.md`, "A capability is a handle, and everything it reaches is an import"
- `design/04-capabilities.md`
- `design/decisions/2026-07-31-row-to-wit-world-mapping.md`
