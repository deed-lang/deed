# Backend

## Why this exists

There is no code generation today: `deed run`, `deed check` and `deed test` all go through
a tree-walking interpreter, and `design/01-principles.md`'s P10 section measured what that
costs. That measurement is still open. Call cost has an argument list and a binding that
cost something, a frame pool was tried and ruled out, and a slot per name was measured and
found not to matter. Nothing in this document depends on that question closing, and it does
not reopen it.

The reason to compile Deed is not that the interpreter is slow. It is that an interpreter is
a single artifact: it needs its own toolchain, its own process, and it cannot be embedded
somewhere that only accepts a self-contained binary or a sandboxed module. Three things a
compiler buys and an interpreter cannot:

- **Standalone binaries.** A program somebody wrote in Deed should be something they can
  hand to somebody else without also handing them `deed`.
- **A sandboxed embedding target.** Deed's capability model already says a function can do
  nothing its signature does not mention. WASM's import model says the same thing about a
  module and its host. Compiling to WASM makes that promise enforceable by something outside
  the compiler, not only by it.
- **No new toolchain for the person running the result.** `deed` ships as one binary today,
  with the standard library embedded in it. A JIT that runs inside `deed` keeps that story;
  an AOT step that needs a system linker is the one place this trades it away, and that
  trade is confined to `deed build` alone.

## What does not change

The interpreter is not going away. It stays the reference implementation, the thing every
other pass is checked against, and the fast path for `deed check`, `deed test` and the
language server's hover and evaluation. Nothing about P9's edit-loop budget changes: a JIT
or an AOT compile is not on the hot path a keystroke takes.

A backend is a new consumer of the same typed, effect-checked tree the interpreter already
reads, the same way `deed-fmt` and `deed-lsp` are. It does not get to change what a program
means. Anywhere the backend and the interpreter disagree about a program's output, that is a
bug in the backend, found by running both over the same corpus and comparing.

## Target

Cranelift was the plan and is not what happened. Its build scripts compile for the host
target, this machine has no MSVC linker, and none of the ways around that survive contact
with the second and larger objection: Cranelift is about sixty transitive crates, and this
workspace has none at all. `deed-lsp` writes its own JSON and its own message framing for
exactly that reason, and the part of the WebAssembly binary format a compiler needs is
smaller than either: numbers, locals, calls, blocks, branches, and a linear memory.

So the encoder is written by hand, in `crates/deed-codegen/src/wasm.rs`. What runs the
result is a small runner over the instructions the compiler emits, which is a test oracle
rather than a WebAssembly implementation, and says so.

WASM first, native object code second. A WASM module runs inside `deed`, so `deed run
--compiled` never needs anything the user's machine does not already have. Native AOT
output (`deed build`) comes once the WASM-shaped design has proven itself, and it is the one
place a system linker is required, which is written down as its own tradeoff rather than
folded into the rest of the story.

## Effect handlers are one-shot, and that decides the dispatch

The question a backend has to answer before it compiles `with` and `perform` is whether an
operation can resume more than once. If it can, dispatch needs real continuations and the
whole shape of a compiled function changes. If it cannot, dispatch is a stack search and an
ordinary call.

It cannot, and the interpreter is where that is settled rather than argued. `deed-interp`'s
`perform` finds the innermost installed handler for the effect with an `rposition` over a
handler stack, looks up the operation the handler declares for it, and calls it the same way
it calls any other function. What comes back is the operation's return value, going to the
site that performed it, once. There is no continuation captured anywhere, nothing that can
be invoked twice, and nothing that resumes. `Expr::With` is a `truncate` back to the stack
depth it found, so a handler's lifetime is exactly its block.

So a compiled `perform` is: walk the handler stack from the top, find the entry whose effect
matches, and call the operation. The set of handlers a program declares is known when it is
compiled, so which operation to call can be chosen by comparing the entry's handler against
each of them rather than through an indirect call, and the module needs no function table.
Handler state is a mutable cell, which is what the language already calls it and the only
mutable thing it has.

What would change this: a `resume` in the language, or an operation that returns to
somewhere other than where it was performed. Neither exists, and adding either would be a
change to `design/03-effects.md` before it was a change here.

## What would falsify this

If a WASM host turns out unable to express the calling convention or the effect-handler
dispatch this needs without contortions that erase the sandboxing benefit, native object
output moves first instead. If embedding never turns out to matter to anyone building with
Deed, the case for a backend at all gets weaker, independent of anything about dispatch
speed above.
