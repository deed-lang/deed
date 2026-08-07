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
matches, and call the operation. Handler state is a mutable cell, which is what the language
already calls it and the only mutable thing it has.

What would change this: a `resume` in the language, or an operation that returns to
somewhere other than where it was performed. Neither exists, and adding either would be a
change to `design/03-effects.md` before it was a change here.

## The handler stack is a linked list in memory, and the search is at runtime

An earlier draft of the section above guessed that the handler could be picked at compile
time, since a module declares all of them. That is wrong, and the program that shows it is
three lines:

```deed
fn report() -> Int uses Counter.value { Counter.value() * 2 }

fn answer() -> Int {
    with Fixed { count: 4 } { report() }
}
```

`report` is compiled once. Which handler answers its `perform` depends on who called it, and
a second caller under a different `with` gets a different one. Nothing at the performing site
says which. So it is a search, and it happens while the program runs.

What it searches is one word of memory the source never names, holding the address of the
innermost frame or zero. A frame is `[next][effect][state][code 0][code 1]...`: the frame
under it, which effect it answers for, the address of its state, and a table index per
operation in the order the effect declared them. `with` builds a frame, links it in, runs the
body, and puts back what was there. Performing walks `next` until the effect matches, then
calls through the table with the state first and the arguments after.

Three things fall out of that rather than being arranged:

Nesting decides which handler answers, because the search starts at the innermost frame. A
handler stops answering when its block ends, because the block put back the frame under it.
And an operation is reached the same way a closure body is, through the table the module
already has, so handlers cost the backend no mechanism that closures had not already paid
for.

The one thing arranged on purpose is that a frame *is* freed, and it is the only thing in
this backend that is. A frame's lifetime is exactly its block, nothing in a program can hold
one, and blocks nest, so frames live on their own stack and a `with` rewinds it on the way
out. Values do not follow, for a reason that fits in a line: a block's value outlives the
block. The note in `crates/deed-codegen/src/layout.rs` about when monotonic allocation stops
being acceptable still applies to everything else, and
`design/decisions/2026-07-31-compiled-memory-reclamation.md` has the numbers.

There is now a size where this is what stops a program rather than a note about later. A
module gets sixteen pages, and building a thousand-key `std/table` copies the list once per
key, so it reaches the end of memory before the first lookup;
`design/decisions/2026-07-31-tree-vs-table-decision.md` measured both keyed modules and
neither of them survives a thousand keys. Below a few hundred the question that measurement
set out to answer is the algorithm; above it, the question is this paragraph.

## A capability is a handle, and everything it reaches is an import
A WebAssembly module cannot open a file, write a line or read a clock. It says what it wants
from its host and the host decides. That reads like a limitation and is the opposite of one:
it is the same shape `design/04-capabilities.md` already gives a `Dir`, and getting it from
the target for free is most of why WASM was the right one.

So a compiled program's **import section is its capability requirements**, written down
where a host can read them before running anything. `Io.write` becomes `deed:io.write`.
Narrowing `System` down to the console it carries becomes `deed:sys.console`, because
narrowing is something only the host can do and a compiled program reaching into its own
memory for it would be a program widening its own authority.

The decision for #660 is explicit:

- **Can Deed call outside Deed?** Yes, by calling host-provided imports only.
- **What must the signature say?** The row names the operation (`uses Io.write`), and the
  arguments carry the capability value it acts on (`Console`, `Dir`, `Clock`, or `System`).
- **What does the capability claim become?** A function can do only what its signature names
  *and* what the host actually provides under those names.
- **How does an ecosystem exist without ambient extern calls?** Through hosts and embedders
  shipping imports and libraries that are reached as capability operations, not by letting a
  module declare arbitrary process-level calls.

This is why there is still no user-declared `extern` escape hatch in the language. The set of
operations a module may request is the one the compiler already type-checks (`Io` plus capability
narrowing imports), and a host may deny any of them by not providing the import.

A capability itself is a handle: a number the host gave out and the program hands back. It
is not a pointer into the program's memory, so there is nothing inside one to look at, and
the program cannot make one up. What a program may do with a handle is decided by the effect
row, which the checker settled, and by which import it is passed to, which the host
implements. Neither is a question about its representation, so all four capabilities are one
type here.

The capability is still the argument. `Io.write` takes the console it writes to and the
compiled call keeps it, so holding the row is not a permission bit. That rule comes from
`crates/deed-typeck/src/check.rs` and is checked against a compiled module in
`crates/deed-driver/tests/host.rs`.

`deed-rt` is the host half. `sandbox.rs` moved there from `deed-interp`, because the rules a
`Dir` enforces have to be the same whether an interpreter or an embedder is enforcing them,
and a rule living inside one of two hosts is a rule about one of them.

This repo does have a host now: `crates/deed-codegen/src/grant.rs`, wired into
`deed run --compiled`. Until it existed, a compiled `examples/hello.deed` could not write
"hello, world" and the interpreted one could, which made every claim on this page about
capabilities a claim about an import section rather than about a program. It grants what
`deed run` grants -- a console, a clock, the directory `--dir` named, the variables
`--env` named, the arguments, and standard input when `main`'s row says so -- and each
grant offers exactly the imports it can answer, so a program asking for one nobody granted
is refused at link time by name: ``the host does not offer `deed:io.fetch` ``. See
`design/decisions/2026-08-07-what-a-host-hands-a-compiled-program.md` for how a handle
stays unforgeable across the boundary.

The rules about what a `Dir` reaches are `deed-rt`'s in both engines, and
`crates/deed-cli/tests/cli.rs` holds the two to saying the same thing about the same
program rather than trusting that they will.

The network is still an unanswered import, and so is anything a program declares itself
through `effect ... from`. Those stop with the name of what was wanted, which is the most
useful thing they could say about a module on its way to a real embedder.

**What would change this:** adopting the component/import declarations sketched in
`design/04-capabilities.md` would let modules name additional host imports in source, but only if
those imports are still capability-row entries with explicit capability arguments and host refusal
at instantiation time. A WASI target is one likely first mapping (`deed:io.read` to
`wasi:filesystem`), not a reason to add ambient `extern` calls.

## Monomorphization is affordable, and here is the number

A generic function is lowered once per set of type arguments it is called with. The obvious
worry is that a program leaning on generics compiles into something much larger than it
looks, and the alternative, a single copy taking everything by pointer, is real: it is what
the interpreter does and it costs no duplication at all.

Measured rather than argued, in `crates/deed-driver/tests/growth.rs`. Calling one generic
function forty times at one type produces the same number of functions as calling it once,
and the module grows by 99 to 107 bytes per call site with arithmetic overflow checks. The
range comes from WebAssembly's variable-width local indices crossing a LEB128 boundary, not
from another function body. A second element type costs one more body and nothing else.

So the growth is in distinct type arguments, and a program's distinct type arguments are
bounded by what it writes down. That is what makes this affordable, and it is why the
by-pointer version is not worth building: it would trade a bounded duplication for a boxed
representation on every generic value, and nothing here is asking for that trade.

**What would change this:** a program whose distinct instantiations are not bounded by what
it writes down. Deed has no type-level recursion that could produce one, so this would
arrive with a language change rather than with a large program.

## Distribution without a linker: options and the decision

The options for distributing a compiled Deed program, given no system linker and no
Cranelift dependency, are worth writing down explicitly before any of them is started.

**Option 1: `cranelift-object`.** Take Cranelift back as a dependency, emit a native
object file per architecture, and link it with the system linker. Cost: roughly sixty
transitive crates reappear, the build requires a linker on every supported machine, and
cross-compilation adds a second linker. The dependency argument was already lost once
before the WASM encoder existed, and the second time around it gets weighed against a
concrete request rather than a guess.

**Option 2: hand-written object file plus machine-code emitter.** Emit ELF or PE by hand
the same way `crates/deed-codegen/src/wasm.rs` emits WASM binary. Cost: one object format
per target platform, one instruction set per architecture, and a linker on every build
machine. The WASM encoder is about 700 lines. A production-quality x86-64 ELF emitter is
not, and nothing is asking for it yet.

**Option 3: a small runner that embeds the WASM module.** Compile the program to WASM,
then ship a single binary with the module embedded in it, the same way `deed` ships its
standard library. Cost: building the runner, which is additional work but not a new
dependency and not a linker. Benefit: the recipient needs nothing but the binary, and the
sandbox survives because the runner supplies the host. This is the path to reach when
somebody wants to distribute to a machine with no `deed` and no module host, and it does
not require changing the module format.

**The decision: the distribution format is a WebAssembly module**, and a runner is deferred
rather than rejected. Option 3 is the natural next step once somebody has a machine that
needs it; the module format already supports the upgrade without redesign. Option 1 and
option 2 both require a linker that this workspace does not have and no program has asked
for yet.

What the current choice costs is the one thing a native binary has that a module does not:
running with no host at all. What it saves is an object writer, a relocation model, a
linker search, and a per-architecture emitter.

Linker discovery is deferred with it. It only exists to serve an object file, and there is
no object file.

**What would change this:** somebody who wants to ship a Deed program to a machine that has
no `deed` on it and cannot run a module either. That request would move option 3 from
deferred to scheduled, and if option 3 proved insufficient for that case, option 1 would
follow, on a machine with a linker, with the dependency argument weighed against a real
need.

## What would falsify this

If a WASM host turns out unable to express the calling convention or the effect-handler
dispatch this needs without contortions that erase the sandboxing benefit, native object
output moves first instead. If embedding never turns out to matter to anyone building with
Deed, the case for a backend at all gets weaker, independent of anything about dispatch
speed above.

## Checking already compiles for `wasm32-unknown-unknown`

Measured for #586, before planning the rest of #573 (running Deed in a browser page with no
install step): `cargo check --target wasm32-unknown-unknown -p deed-driver` succeeds today,
with zero errors, and that pulls in every crate up through `deed-interp` and `deed-effects`.

The worry going in was `std::fs`, `std::time` and `std::env`, all reachable from
`deed-interp`'s `Io.now`, `Io.epoch` and file operations. They are reachable and they still
compile: `wasm32-unknown-unknown`'s standard library carries these APIs and returns an
`io::Error` at the call site rather than refusing to link. Nothing here needed `getrandom`,
`serde` or a time crate, matching the workspace's dependency-free shape.

This means the *checking* path (`check_all`, the thing #585 already reaches from named
strings) is a small job for the browser: it needs no code that fails to compile for that
target. Running a program (`deed run`) is the open question, because the compiling-cleanly
result says nothing about what happens when one of those calls executes: a `wasm32-unknown-unknown`
program has no filesystem and no clock underneath it, so `Io.*` needs a host answer, not a
recompile. That is the shape of #591 (what capabilities a page can hand a program), not a
finding about `deed-driver` itself.

## Which prelude names the backend compiles, enumerated rather than remembered

Measured for #621, against the compiler's own lists (`deed_resolve::PRELUDE`,
`deed_resolve::IO_OPERATIONS`) rather than typed by hand:
`crates/deed-driver/tests/backend_prelude.rs` tries one minimal program per name through
the real pipeline (check, lower, compile) and pins the result by name.

**All ten `Io` operations compile.** They reach the backend as host imports (#569), and a
call through one compiles the same way any other direct call does. `deed-codegen/src/compile.rs`'s
own doc comment used to say capabilities were refused by name; that was true before #569
and stayed written down after, which is the same mistake this document's other measured
sections keep finding elsewhere in this repository. Fixed alongside the test.

**All thirteen callable prelude functions compile.** Three of them (`ok`, `err`, `length`)
did from the beginning, because each is one instruction or none. The other ten (`at`,
`push`, `repeat`, `split`, `join`, `trim`, `upper`, `lower`, `to_string`, `to_int`) were
refused until #877, for the reason this document keeps finding: the prelude was written
once in the interpreter and nowhere else, so `deed run` had a body to call and `deed build`
did not. They are compiled now, not by lowering the interpreter's Rust but by writing each
one in WebAssembly directly (`deed-codegen/src/runtime.rs`), on the grounds that a hundred
lines the backend emits itself is cheaper to keep honest than a second source language.
What keeps the two answers together is `agreement.rs`, which runs the same program through
both engines and compares. Type names (`Int`, `String`, `Bool`, `Result`, `List`, `System`,
`Console`, `Clock`, `Dir`) are left out of this count on purpose: whether a *value* of one
of them passes through the backend is a different, already-answered question (`generics.rs`,
`result.rs`, `lists.rs`, `host.rs`), not one a function call can ask.

## One module at a time was the largest thing between the backend and the corpus

The interpreter has always been handed every file the compiler checked. The lowering was
handed one, so a program with a `use` in it was refused, which is most programs that do
any real work: eight of the thirty-four files in the corpus were waiting on this and
nothing else.

What made it more than plumbing is that a `DefId` is an index into one module's table and a
`Span` is an offset into one file, so neither means anything on the other side. A body from
another module has to be read with that module's resolutions and that module's types, and
the lowering swaps them in around the call rather than looking them up per read.

Two decisions worth writing down. Only what is reached is lowered, so a module that ships
thirty functions and is imported for one contributes one, which matters because the rest of
it may use shapes this backend cannot compile and refusing the whole program over a
function nobody called would be wrong. And a callee's `where` clause is kept whatever the
other side worked out about its own callers: the call that could break it was answered for
in the caller's table, which the callee's module never saw.

## Comparing two values that live in memory

Equality is structural in this language, and two addresses being equal is not two records
being equal, so the backend refused every comparison of a boxed value rather than answering
the wrong question. Text was the exception, because text had a helper.

What it needed was a comparison per shape, because none of what decides the answer survives
into a value at runtime: a record knows its fields, a choice knows its variants, a list
knows what it holds, and a value knows none of that. So `deed-codegen/src/equality.rs`
writes one function per shape a program compares two of, closed over transitively, numbered
after the runtime helpers. A shape holding another calls that shape's function.

The reason this is the last thing #877 closed is that it is the only refusal that was a
decision rather than a gap. The others were the backend not having got somewhere yet.

## What a tier costs at runtime, measured in both engines

`design/02-syntax.md` says an obligation the checker proves costs nothing at runtime and
one it cannot becomes a check. Both halves were only true of the interpreter. The compiled
backend emitted the `where` clause checks and no refinement checks at all, so `deed check`
printed "so it becomes a runtime check" over a check that `deed build` did not write, and a
value that violated its own type went through a compiled program without anything noticing.

Closed under #877. The lowering reads the checker's own table of `Guarded` obligations,
the same table `deed_driver::Checked::guards` hands the interpreter, and turns each one
into a bind, a predicate and a `Fail`. The predicate is an ordinary expression and is
lowered as one, which needed the checker to read it: a refinement predicate had no types
recorded for it because nothing had ever asked what `value > 0` meant.

Two things that were not obvious. The obligation on a `Result` that came back from a call
is about the number inside the `ok`, so the check has to look one level in and let an `err`
through untouched. And a `where` clause an `assert refuses` aims at has no recorded call
site at all: the checker deliberately records no tier there, since a precondition meant to
fail is not one anybody discharged, and the backend's rule for dropping a check is "every
recorded call proved it". So the one caller that needed the check was the one caller
nothing knew about, and `assert refuses` passed under the interpreter and failed under
`deed build`.

## The runner validates the module before it runs one, and a real engine is not tested against

`crates/deed-codegen/src/run.rs` says plainly it is a test oracle, not a WebAssembly
implementation: it runs the instructions the compiler emits and nothing checks in advance
that the module those instructions form is one a real engine would load. #567 found the
cost of that being incomplete by accident, when an `i32` stored through an `i64.store` ran
here and would have loaded nowhere else.

#625 closes the general case rather than the one instance: `crates/deed-codegen/src/validate.rs`
is the actual validation algorithm from the WebAssembly specification's own appendix (a
value stack and a control stack, both allowed to answer "anything" once a branch has made
the rest of a block unreachable), run over every function this backend emits. `run::call`
runs it first, so every existing test that calls a compiled module, `agreement.rs`,
`corpus_backend.rs`, and everything under them, validates the module it is about to run
without having to ask to. The runtime width check `run.rs` used to raise on a mistyped
store is gone: it is unreachable now that nothing gets that far without validating first.

Whether a real engine (wasmtime, wasmparser, or similar) is worth testing against as well:
decided no, for now. The specification's validation algorithm is the thing being
implemented, not approximated, so a real engine would mostly be confirming this workspace's
own transcription of a published algorithm rather than finding a different class of bug,
and it is the one dependency this backend has spent every other design decision in this
document avoiding. The number that would change this: a bug reaches a released `deed build`
that validation here missed and a real engine would have caught. None has, yet.

## What suspension does to the handler frame list

The handler stack is a linked list in memory rather than a flat stack of addresses because
one program asked for it: the three-line example above. A scheduler asks for the same
shape, and this section is that program, written before the scheduler is.

OCaml's implementation notes are the reference: their stack is a linked list of fibers, a
captured continuation points at a segment rather than copying frames, and resuming relinks
it. Deed's frame list already has that shape.

**What a stored resumption owns.** A suspended computation, in the frame model, is a saved
value of the HANDLERS global: the address of the innermost frame that was installed when the
computation paused. That frame's `next` pointer holds the frame below it, and so on to zero.
Nothing is copied. The resumption owns one word, the head-of-list address; the list is
already in memory, already linked, and already correct.

**Whether the frame list can be reattached.** Yes, without copying or rebuilding. To suspend
a computation inside a `with` block: save the current HANDLERS value. To resume it: restore
that value before continuing. The frame linked in for the `with` block is still at its
address, because nothing in this backend ever frees a frame. Its `next` pointer was stored
on entry and has not moved. The frame is ready to be the head of the list again.

**What the interpreter does.** The interpreter's handler list is `handlers: Vec<Instance>`
in `crates/deed-interp/src/interp.rs`, and `Expr::With` calls `self.handlers.truncate(base)`
when the body finishes evaluating. Suspension arrives as a value the body produces, and that
truncate runs before any resumption. A scheduler, once written, cannot use the return path of
the `with` body to truncate: it has to hold the range `base..handlers.len()` alive until the
last resumption resolves, then truncate at that point.

**What the backend does.** The backend already has the right shape, for the same reason the
linked-list design was right about the three-line example: frames are allocated once and
never freed, so a frame in the list is there for as long as the program runs. The restore
instruction at the end of a `with` block reads `frame.next` and writes it into HANDLERS,
putting back what was there before the block. For a scheduler: save HANDLERS before
suspending, restore it before resuming, and nothing else changes. The `next` pointer in the
saved frame is still valid.

**The invariant.** A `with` block must not unlink its frame while a resumption that would
search through that frame is outstanding. For the backend this holds for free. For the
interpreter it means not truncating until the last resumption resolves.

The two programs added to `crates/deed-driver/tests/agreement.rs` for this section are the
ratchet. The first checks that after an inner `with` block for a different effect ends, the
outer handler's state is intact and still searchable, which is the same property a scheduler
needs after a resumption completes inside a nested context. The second checks that a handler
operation can perform into a co-installed handler across the frame boundary, which is the
cross-frame search a suspended operation relies on when it is resumed.

## The host enforces the row, and the enforcement is structural rather than checked

The sentence in the first section about WASM's import model making Deed's capability promise
"enforceable by something outside the compiler, not only by it" was true in principle and
unproven in the repository. `run.rs` stopped at `NeedsAHost` when any import was called,
which says which import was missing but proves nothing about whether a real host would have
enforced anything.

#629 adds `Host` to `deed-codegen/src/run.rs`. It is not a third-party runtime and does not
add a dependency: it implements exactly the two properties that make WASM's import model an
enforcement mechanism rather than a naming convention, using the same module representation
the rest of the codebase already has.

**Why not an external runtime.** Adding `wasmtime`, `wasmi`, or similar as a dev-dependency
would confirm that the bytes this backend emits are accepted by a conformant engine, which is
useful but separate from the enforcement claim. The claim is about structure: a module that
does not import an operation has no function index to call it through. That structural fact
is confirmed by inspecting the import section, not by running the bytes. An external engine
would run the bytes and stop at the missing import the same way `Host` does, without adding
information about why the stopping happens. The dependency would be the cost and the
structural argument would be the gain, and the gain is already here without it.

**What `Host` does.** `Host` holds an offer list: a named set of (module, operation) pairs
with implementations. `Host::link` checks every entry in the module's import section against
that list before a single instruction runs. If any import is unsatisfied, `link` returns
`Err(LinkError { ... })` naming the missing entry. Only a module whose entire import section
is covered gets a `Linked` back. `Linked::call` dispatches import calls to the offer list
rather than stopping with `NeedsAHost`.

**The two tests in `deed-driver/tests/host.rs` that prove the enforcement.**

`what_the_row_does_not_name_is_not_reachable`: a component whose row does not mention
writing (`fn answer() -> Int { 2 + 2 }`) is compiled and linked to a host that offers
write. Linking succeeds because the module has no imports. Write is confirmed absent from
the import section. The module runs correctly under the write-offering host, and the host's
write implementation is never called: there is no import index to dispatch through.

`a_component_asking_for_what_the_host_does_not_offer_is_refused_at_load`: a component
whose row does mention writing (the `WRITING` fixture) is linked to an empty host. `link`
returns an error before the module runs, naming the unsatisfied import. The module is
refused at load time rather than failing mid-run.

These two together are the enforcement the introductory section promised: absence from the
row means absence from the import section means the operation is structurally unreachable,
and presence in the row with an unsatisfying host means refusal at load rather than at
runtime.
