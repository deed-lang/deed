# How do I embed a compiled Deed program?

`deed build` writes a WebAssembly module. Running it is somebody's job, and
this page is for whoever is doing it: what the module asks for, what it hands
back, and what a host owes it in return.

Nothing here is specific to a runtime. The module is a plain core module, and
the engine Node ships loads it, which is what
[`crates/deed-codegen/smoke.mjs`](../crates/deed-codegen/smoke.mjs) does on
every commit.

## What the module asks for is its capability row

A Deed function names what it does outside itself, and the compiler turns that
into the module's import section. So the imports are the whole of what a
program can reach, readable before a byte of it runs:

```
$ deed build hello.deed
hello.wasm

# what any engine sees in it
deed:sys.console   deed:io.write   deed:sys.clock   deed:io.now
```

A host decides one import at a time. What it does not supply, the module
cannot call: the function index for an import that was never declared does not
exist in the module's index space, so offering it later reaches nothing. And a
module declaring an import the host cannot answer should be refused at
instantiation, which is what every engine does anyway.

`deed build --component` writes a `.wit` world beside the module saying the
same thing in the component model's own words.

## A capability is a number the host made up

`Io.write` takes the console it writes to. That console arrives as an `i64`,
and it is whatever the host decided: an index into a table it keeps, most
simply. The module cannot look inside one, because there is nothing inside
one, and it cannot make one, because the only place they come from is a host
call.

That means a host must look its arguments up rather than trust them. A checked
Deed program cannot pass a number where a capability belongs, but a host is
handed modules, not programs.

- `deed:sys.console`, `deed:sys.clock`, `deed:sys.files`, `deed:sys.net` take
  the root and hand back something narrower.
- Every `deed:io.*` takes the capability it acts on as its first argument.

The root is what a compiled `main` is called with, one per parameter it
declares.

## Values wider than a number live in memory

The module exports its memory as `memory`. A host reads and writes it
directly, and everything below is the whole of the layout.

Every field, element and tag is eight bytes, so nothing has to be told how
wide anything is.

| What | Where it is | How big |
| --- | --- | --- |
| the bump pointer | 0 | 8 |
| the code of a failed contract | 16 | 8 |
| the message of a failed contract | 24 | 8 |
| a word | anywhere | 8 |
| a string | `[characters][bytes][the bytes]` | 16 + bytes rounded up to 8 |
| a list | `[length][element 0]...` | 8 + 8 per element |
| a record | `[field 0]...` | 8 per field |
| a choice | `[tag][field 0]...` | 8 + 8 per field |

A `Result` is a choice with one field: `ok` is tag 0 and `err` is tag 1.

The two counts a string carries answer different questions. `length` in the
language counts characters; reading the bytes back needs the byte count. A
host that derives one from the other is right until somebody writes an accent.

## Allocating

Everything the module makes comes from a bump pointer at address 0, and a host
that hands back a string or a list allocates the same way: read the word at 0,
write the value there, and store the new end back at 0. What comes back is
then an ordinary value of the program's rather than something out of a second
heap it has no way to name.

A host that has run out of room has to say so rather than write past the end.
The module's memory can grow, and a host allocating into it should grow it the
way the module does instead of failing at the first page boundary.

## Nothing is given back, so plan for that

The bump pointer only goes up. A compiled program reclaims one thing, a handler
frame, and nothing else: the value a block produces outlives the block, so
rewinding would free something the caller is about to read. **What a module
allocates in total is what its memory reaches.**

That is not a detail of this release. It is measured, and the shape is worth
knowing before it is discovered in production. The same call, five times, on
one instance — bytes past where the bump pointer started:

```
3216   6432   9648   12864   16080
```

Every identical call costs the same again. There is no point at which the
module notices it has answered this before.

The way to get the memory back is to make another instance. The same module,
instantiated a second time, after the first had run five times:

```
3216
```

So a host that calls an export in a loop should decide how long an instance
lives. Per request, per batch, per document — anything with an end to it. An
instance that lives as long as the process does is one whose memory only grows,
and the program cannot tell you it is happening.

This also applies from the outside now. A component's caller allocates through
the module's `cabi_realloc` to pass a string, and that allocator is the same
bump pointer, so passing arguments in grows the memory as surely as computing
answers does.

What would change this: `design/decisions/2026-08-09-what-a-callee-does-with-its-argument.md`
names the one missing fact, and
`design/decisions/2026-07-31-compiled-memory-reclamation.md` is why there is no
collector. Until one of them moves, the paragraph above is the whole story and
this page would rather say so than let a host find out on a long day.

## Where a failure ends up

A compiled program that breaks a contract writes two strings before it stops:
the diagnostic code at address 16 and the sentence at 24, each the address of
an ordinary string. Both are zero when nothing has failed. Reading them is how
a host says "DEED6002: this call does not meet `n > 0`" instead of
"unreachable".

## A host, in full

This one answers a console. It is the same shape the smoke test uses, and the
same shape `crates/deed-codegen/src/grant.rs` uses in Rust.

```js
const held = ["System", "Console"];              // the table
const handle = (what) => BigInt(held.indexOf(what) + 1);
const stands = (given) => held[Number(given) - 1];

let memory;

function text(address) {
  const view = new DataView(memory.buffer);
  const at = Number(address);
  const bytes = Number(view.getBigUint64(at + 8, true));
  return new TextDecoder().decode(new Uint8Array(memory.buffer, at + 16, bytes));
}

const instance = await WebAssembly.instantiate(module, {
  "deed:sys": {
    console: (root) => {
      if (stands(root) !== "System") throw new Error("not a capability I gave out");
      return handle("Console");
    },
  },
  "deed:io": {
    write: (console_, address) => {
      if (stands(console_) !== "Console") throw new Error("not the console");
      process.stdout.write(text(address) + "\n");
    },
  },
});

memory = instance.exports.memory;
instance.exports.main(handle("System"));
```

Nothing about that is a Deed rule. It is a table, a lookup, and a read, and
the table is the reason a handle cannot be forged.

## The program that host runs

Playground: [open](https://deed-lang.github.io/)

```deed embed-hello
module hello

fn greet(out: Console, name: String) -> ()
  uses
    Io.write,
{
    Io.write(out, "hello, ")
    Io.write(out, name)
}

fn main(sys: System) -> Int
  uses
    Io.write,
{
    greet(sys.console, "world")
    0
}
```

## What is kept green

- [`crates/deed-codegen/smoke.mjs`](../crates/deed-codegen/smoke.mjs) does all
  of the above in an engine that is not this repository's, on every commit.
- [`crates/deed-driver/tests/embedding.rs`](../crates/deed-driver/tests/embedding.rs)
  reads the table on this page and compares every number in it to
  [`crates/deed-codegen/src/layout.rs`](../crates/deed-codegen/src/layout.rs),
  so a layout change fails here rather than in somebody's host.
- [`crates/deed-codegen/src/grant.rs`](../crates/deed-codegen/src/grant.rs) is
  a host written against this page, and `deed run --compiled` is it running.
