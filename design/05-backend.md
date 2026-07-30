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

Cranelift, not LLVM: pure Rust, no external toolchain to build the compiler itself, and fast
enough to compile that a JIT path costs little more than starting the program would anyway.

WASM first, native object code second. A WASM module runs inside `deed` through an embedded
runtime, so `deed run --compiled` never needs anything the user's machine does not already
have. Native AOT output (`deed build`) comes once the WASM-shaped design has proven itself,
and it is the one place a system linker is required, which is written down as its own
tradeoff rather than folded into the rest of the story.

## What would falsify this

If a WASM host turns out unable to express the calling convention or the effect-handler
dispatch this needs without contortions that erase the sandboxing benefit, native object
output moves first instead. If embedding never turns out to matter to anyone building with
Deed, the case for a backend at all gets weaker, independent of anything about dispatch
speed above.
