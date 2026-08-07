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

Nothing is granted by default. `deed run --compiled` grants a console and a clock, so
`examples/hello.deed` now prints what the interpreted one prints, and a program that reads
a file is turned down before it runs:

```
$ deed run --compiled examples/todo.deed
the host does not offer `deed:io.read`
```

## Drawbacks (required)

The thing on the host's side of a handle is reachable by anything holding the table, so
the table is the security boundary and it is a `Vec` behind an `Rc<RefCell<_>>`. That is
right for one program in one process and is not a claim about anything else.

`Io.now` counts and `Io.epoch` reads the machine, which duplicates two lines the
interpreter already has. Two engines answering the same operation two ways is exactly the
drift this repository keeps finding, and there is no shared place to put them today: the
interpreter's version works on `Value`, this one on a handle table.

Only the console and the clock are granted. The filesystem, the network, arguments,
environment variables and standard input are still unanswered imports, and each of them
needs something this decision does not provide: a `Result` written into the module's
memory, and a `Dir` handle that narrows the way `Io.open` narrows.

Giving a host implementation the module's memory means giving it the ability to corrupt
the program it is answering. That is the same authority any embedder has and the reason
`write_text` refuses rather than truncates, but it is a wider surface than passing values.

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

- Option: grant everything `deed run` grants, so the two engines match today.
  - Rejected because: the filesystem operations answer with `Result`, and writing an
    aggregate into the module's memory is a second piece of work with its own decisions.
    Shipping it half-done would have meant a host that answers `Io.read` with something
    the program misreads, which is worse than a host that says it does not offer it.

## Open Questions (required)

- Where the shared answer to `Io.now` and `Io.epoch` should live, once the filesystem
  operations make the duplication between this host and the interpreter three times the
  size.
- Whether `deed run --compiled` should take the same `--dir`, `--allow`, `--env` and stdin
  grants `deed run` takes, or whether the compiled path should require them to be spelled
  out because a component's world is meant to be read before it runs.
- Whether a `Dir` handle should be handed out per `Io.open` call, which grows the table for
  as long as the program runs, or interned by resolved path.

## References

- `crates/deed-codegen/src/grant.rs`, `crates/deed-codegen/src/run.rs`
- `crates/deed-driver/tests/host.rs`, `crates/deed-cli/tests/cli.rs`
- `design/05-backend.md`, "A capability is a handle, and everything it reaches is an import"
- `design/04-capabilities.md`
- `design/decisions/2026-07-31-row-to-wit-world-mapping.md`
