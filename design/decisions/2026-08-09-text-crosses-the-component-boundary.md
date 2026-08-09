# Decision: text crosses the component boundary

- Status: Accepted
- Date: 2026-08-09
- Supersedes: `2026-08-09-a-component-for-what-crosses-unchanged.md`
- Superseded by: None

## Context

The record this supersedes wrote a component for the exports that need no
adapters and refused one, by name, for everything else. It said what that was
for:

> A component whose world is `s64` and `bool` and nothing else is a component
> almost nobody wants. What it is for is turning the remaining two gaps from a
> design into a failing test.

The failing test was there the same day. `crates/deed-codegen/component.mjs`
built a module with `fn greet(name: String) -> String` in it and measured the
refusal:

```
ok    a module carrying text is told which export needs the adapters
ok    and is not given a component that would answer wrongly
```

Two lines saying the compiler knows what it cannot do. The question this record
answers is what it takes to delete them, and the answer turned out to be
smaller than the record that deferred it assumed.

`crates/deed-codegen/src/abi.rs` already transcribes the canonical ABI's rules.
What was missing was not knowledge of them. It was that nothing in the module
could hold a value on the boundary's terms:

- a caller lowers a string by asking the callee for room, writing UTF-8 into
  it, and passing a pointer and a length, and no compiled module exported
  anything a caller could ask room from;
- a callee lifts one by writing a pointer and a length into a return area and
  giving back its address, and every compiled function gives back one address
  to a header of two words followed by the bytes;
- and the layout's first word is a count of characters, which the boundary
  does not carry, because a caller counts bytes.

## Decision

Write the adapters. Text crosses; a component's caller passes and receives a
`string` and neither side learns anything about the other's layout.

`crates/deed-codegen/src/adapter.rs` appends two kinds of function to the
compiled module:

`cabi_realloc(original, held, align, wanted)` is the allocator a caller looks
for by that spelling. It is the bump pointer every other allocation in a
compiled module already uses, rounded up to a word, copying the smaller of the
two sizes when something was there, and stopping on an alignment it cannot
promise.

A wrapper per export that carries text, exported as `<name>.lift`. A Deed name
cannot hold a full stop, so it collides with nothing a program could have
written. It takes the boundary's shape -- two `i32`s per string in, one `i32`
pointing at a return area out -- builds the value the module already uses,
calls the function the compiler already emitted, and writes the pointer and the
length back out.

The character count is counted rather than carried, one byte at a time, because
the boundary does not carry one. A byte begins a character unless it is a UTF-8
continuation byte, so the count is the number of bytes below `0x80` plus the
number at or above `0xc0`. Two comparisons and an add rather than a mask,
because this backend's own runner reads `i32.and` as the language's boolean
operator rather than WebAssembly's bitwise one and nothing may depend on which.

`crates/deed-codegen/src/component.rs` lifts with options when, and only when,
an export carries something that does not cross unchanged: an alias for the
memory, an alias for `cabi_realloc`, and a lift naming both. An export of
numbers and booleans in the same component still lifts with no options, so
every component this wrote before is byte for byte what it was.

The module inside the component is the module beside it with the adapters
appended. Every function keeps its index and every export keeps its name. The
`<name>.wasm` written beside the source has none of them: a host embedding that
file is not handed adapters it did not ask for.

## What this buys

Measured through `jco`, which bundles the Bytecode Alliance's own `wasm-tools`,
so the calls go through a real component runtime rather than anything here:

```
ok    the world of a component carrying text says string on both sides
ok    and says it for a string that is not the first parameter
ok    a component runtime hands it a string
ok    an empty one too
ok    a string among other parameters lands in the right place
ok    two of them do not overwrite each other
ok    and text outside ASCII arrives with the right character count
ok    while text going only one way still crosses
ok    and a string past the pages the module starts with
ok    an export needing no adapter still answers in a component that has them
```

Ten rather than one, because the ways this can be wrong are not one way. A
wrapper that read its parameters from fixed slots passes for `greet(name)` and
fails for `tag(n, name, on)`. One that built both strings into the same local
passes for one and fails for two. One that copied the byte count into the
character count passes for every ASCII string in the suite and fails for
`dünya 日本語`. And one whose allocator moved the bump pointer without growing
the memory passes for everything short -- which is the bug `str_concat` carried
for two releases, found by a program rather than a test, so the long string is
in there on purpose.

## Drawbacks (required)

The allocator never frees. `cabi_realloc` is the bump pointer, so a host calling
a component in a loop grows the module's memory in a loop, and a long-lived
component is a leak with a ceiling. This is not new and it is not about
components: `design/decisions/2026-07-31-compiled-memory-reclamation.md` is the
open question, and it covers the whole backend rather than this corner of it.
What is new is that a host can now drive the allocation from outside, which
makes the ceiling easier to reach than a program alone could.

Counting characters costs a pass over the bytes on every call. A boundary that
carried the count would not need one, and none does. The alternative is storing
a byte count and computing characters on demand, which moves the cost to
`length` and every slice, and this backend has no measurement saying which is
worse.

A wrapper per export that carries text, and the wrapper is not small. A module
of ten string-taking functions carries ten copies of the same lowering. Sharing
one lowering helper between them is a change with no measurement behind it yet;
the modules this compiles are small enough that nobody has noticed.

`<name>.wasm` and the module inside `<name>.component.wasm` are no longer the
same bytes when text is involved. The previous record could say "verbatim" and
this one says "with the adapters appended, and nothing else moved". That is a
weaker sentence and it is the true one, and
`crates/deed-codegen/src/adapter.rs` has the test that holds it: every function
and every export the compiler emitted is still there, at the index it had.

## Rejected Ideas (required)

- Option: put `cabi_realloc` in every compiled module rather than only in the
  one inside a component.
  - Rejected because: `deed build` writes a module for a host to embed, and an
    exported allocator is an invitation to a protocol that module is not part
    of. The adapters exist for a boundary; a module with no boundary should not
    carry them.

- Option: give the wrapper the export's own name and rename the compiled
  function.
  - Rejected because: the core module beside the component exports `greet`, and
    a reader comparing the two files should find the same name meaning the same
    thing. `greet.lift` is a name the language cannot produce, so it says what
    it is without taking anything.

- Option: carry the character count across the boundary as a second value, so
  nothing has to count.
  - Rejected because: it would not be a `string` any more. A world saying
    `func(p0: string, p1: s64)` is a world every other component toolchain
    reads as two parameters, and the whole point of the boundary is that the
    other side does not learn about this one's layout.

- Option: lower a list at the same time, since it crosses the same way.
  - Rejected because: a list crosses as a pointer and a length whose elements
    are laid out by the canonical ABI rather than by this backend, so every
    element has to be converted one at a time and the conversion depends on the
    element type. Text is one loop over bytes; a list is the general case, and
    the general case wants the measurement this change makes possible rather
    than the one it does not have yet.

- Option: write a `cabi_post_greet` to free the returned string.
  - Rejected because: nothing here frees anything, so the function would have
    nothing to do, and a post-return that does nothing is a claim that the
    memory was reclaimed.

## Open Questions (required)

- A list, a record and a choice, which are what is left. The lowering is per
  element and per field, which this now has somewhere to put and something to
  test against.
- Whether the wrappers should share one lowering helper. The measurement that
  would decide it is module size against export count, and nothing has asked.
- Whether a host driving allocation from outside changes the answer in
  `design/decisions/2026-07-31-compiled-memory-reclamation.md`, which was
  written when only the program could allocate.
- What a capability becomes at a component boundary. `--component` refuses
  signatures holding one, and the component model's answer is a resource, which
  is a type this language does not have.
- Whether the exported interface should stay "every function the module
  declares". A component's world is a public API and a Deed module has no
  visibility markers, so today those are the same list by accident.

## References

- `crates/deed-codegen/src/adapter.rs`, the adapters
- `crates/deed-codegen/src/component.rs`, the encoder
- `crates/deed-codegen/component.mjs`, the measurement, on every commit
- `crates/deed-codegen/src/abi.rs`, which transcribes the rules the adapters
  follow
- `design/decisions/2026-08-09-a-component-for-what-crosses-unchanged.md`,
  superseded
- `design/decisions/2026-08-07-a-wit-world-is-not-a-component.md`
- `design/decisions/2026-07-31-compiled-memory-reclamation.md`
- `design/05-backend.md`
