# Decision: a component asks for what it needs, and an effect can say where it comes from

- Status: Accepted
- Date: 2026-08-04
- Supersedes: None
- Superseded by: None

## Context

`deed build --component` derived a world from a program and wrote only its exports. A
component that performed an effect nothing in the module handled compiled cleanly, declared
no WebAssembly import at all, and trapped when the export was called. The world said the
component was self-contained. It was not.

That is worse than a missing feature, because
`design/decisions/2026-07-31-row-to-wit-world-mapping.md` is the one thing here nobody else
does, and `crates/deed-driver/tests/wit_world.rs` holds it up honestly for `wit_world_for`,
which is about `main`. The `--component` path used a second emitter, `generate_wit`, and
the two never met. So the claim was tested on the path that does not ship a component, and
the path that does could not be wrong because it never said anything.

The second half is `design/04-capabilities.md`'s oldest open question, which lists interop
as unsolved and calls it "the most likely place the whole model leaks", on the grounds that
calling C or WASM brings ambient authority back with it.

## Decision

**A component's imports are the operations it can only get an answer to from its host, and
they are computed rather than written.**

The rule: an effect the program performs and never installs a handler for cannot be
answered anywhere inside the program, under any call path, so every performance of it
reaches the boundary. That is exact rather than an approximation. `escaping_operations` in
`deed-codegen` is where it lives, the world lists one `import` per entry, and the module
declares and calls the same set.

**An effect may say which interface its operations come from.**

```deed
effect Random from "wasi:random/random" {
  fn roll(sides: Int) -> Int
}
```

Without the clause an effect is its own interface and the import is `deed:<effect>`, which
is the right default for one this program invented and useless for one that already exists.
`from` is a soft keyword: between an effect's name and its brace there is nowhere else a
word can go, so nothing needs reserving and `fn from(from: Int)` still compiles.

**The interop question is answered, and the answer is that this is not the leak the
document expected.** A WebAssembly component cannot communicate except through its imports.
An import is therefore the opposite of ambient authority: it is a capability the host
decided to hand over, named in the world, refusable by not offering it. A foreign function
in Deed is an effect operation the module does not handle, it appears in the row, the row is
in the signature, and the signature is what a reviewer reads. Nothing new had to be invented
to keep the guarantee; what was missing was that the module never asked.

## Drawbacks (required)

**The rule is exact but not complete.** An effect with an `Install` somewhere may still have
performs that escape, on a path that does not run under the `with`. Those keep the behaviour
they had, which is a trap. Making that case work needs to know which performs are under
which handler at runtime, and the honest version of that question is the checker's declared
rows rather than a walk of the MIR. See the open questions.

**The compiler does not check the interface name.** `from "not an interface"` is accepted
and becomes an import under that name. Whoever links the component finds out. Checking it
would mean the compiler holding an opinion about WIT's grammar, which is a second grammar to
keep in step with a specification that is still moving.

**A host is handed the operation's own arguments and nothing else.** An installed handler
gets its state cell first; there is no cell here because there is no handler. That is right,
and it means an effect's operations have one shape when handled and one when imported, which
is a thing a reader has to know.

**Nothing type-checks the boundary.** The operation's Deed signature becomes a WebAssembly
signature, and whether the host's idea of `wasi:random/random.roll` matches is not a question
this compiler can answer.

## Rejected Ideas (required)

- Option: a separate `extern` declaration form for foreign functions.
  - Rejected because: an effect already carries everything one needs, which is a name, a
    signature, and a row entry that puts it in the caller's signature. A second form would
    say the same things in a second spelling, and it would be the only declaration in the
    language whose calls do not appear in a row.

- Option: infer the interface from the effect's name without a clause, always.
  - Rejected because: `deed:random` is a fine name for an effect this program invented and
    a wrong one for `wasi:random/random`. Inference cannot tell the two apart, and the one
    it would get wrong is the one that has to link against something real.

- Option: treat every effect a module mentions as an import.
  - Rejected because: an effect a `with` answers never reaches the host, and asking for it
    anyway makes the world a list of what the module talks about rather than what it needs.
    A host would have to satisfy imports the component never calls.

- Option: refuse to build a component that performs an unhandled effect, and stop there.
  - Rejected because: it is the honest version of doing nothing. It turns a trapping
    artifact into a clear refusal, which is better, and it leaves the component model story
    where it was: components that can only compute.

- Option: emit the imports in the world without emitting them in the module.
  - Rejected because: that is a world that disagrees with its own artifact, which is worse
    than one that says too little.

- Option: check the interface name against a list of known WASI interfaces.
  - Rejected because: the list changes without this compiler, and a component may import
    an interface that is nobody's standard. The name is the host's business.

## Open Questions (required)

- The escape rule is a walk of the MIR, and the exact answer is the checker's declared rows:
  an operation escapes a function when it is in that function's row. The lowerer is not
  handed `Effects` today. What would change this: a program in the corpus that installs a
  handler for an effect and also performs it somewhere the handler does not cover.

- Whether a component should be able to export an interface as well as a set of loose
  functions. Every function a module declares is an export today, which is what "there are
  no visibility modifiers" means, and grouping them under a named interface is a second
  decision about what a module is.

- Whether an imported effect should be handleable at all. It is today, and that is what
  makes one testable: a `with` in a test answers for it and the test does not need a host.
  The cost is that reading `uses Random.roll` does not by itself say the call leaves the
  component, which is true of every effect and is what the `with` is for.

## References

- Issue [#912](https://github.com/deed-lang/deed/issues/912), with the measurement: a
  170-byte module with no section 2.
- `crates/deed-codegen/src/compile.rs`, `escaping_operations` and `Builder::host_answer`.
- `crates/deed-driver/tests/host.rs`, the three tests at the end.
- `design/decisions/2026-07-31-row-to-wit-world-mapping.md`, which this makes true of the
  path that ships a component.
