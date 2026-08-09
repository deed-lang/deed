# Changelog

This file is written per change, not assembled at release time. Its sections
follow the compatibility surface in [design/07-versioning.md](design/07-versioning.md),
which says what a Deed version promises.

## Unreleased

Update this section in the same PR as the user-visible change. When cutting a
release, rename it to the version and date, and publish that section as the
release notes.

### Programs that used to compile and no longer do

- None yet.

### Language

- Nothing yet.

### Diagnostics

- Nothing yet.

### Standard library

- Nothing yet.

### Tools

- The embedding guide says what the memory does over time.
  `how-to/embed-a-compiled-program.md` described the layout and how to allocate
  into it, and never said that nothing is ever given back — so a host that
  called an export in a loop grew its memory forever with nothing to warn it.
  The page now says so, with the measurement: the same call five times over
  costs 3,216 bytes then 6,432 then 9,648 then 12,864 then 16,080, and a second
  instance of the same module starts again at 3,216. A fresh instance is the
  only thing that returns it, so a host should decide how long one lives.

  `crates/deed-codegen/smoke.mjs` produces those numbers in the engine Node
  ships, on every commit, and compares them against the page. A guide that
  quotes a compiler two releases old is the failure this is for, and it was
  verified by making one.

- Text crosses the component boundary. `deed build --component` writes a
  component for a module whose exports take and return `String`, and a caller
  passes and receives a `string` without learning anything about this backend's
  layout. Until now those modules were refused by name, which
  `design/decisions/2026-08-09-a-component-for-what-crosses-unchanged.md` said
  was for turning the gap into a failing test; this is that test passing.

  The module inside the component carries a `cabi_realloc`, which is the
  allocator a caller looks for by that spelling, and a wrapper per export that
  takes the boundary's shape: two words in per string, one address out to a
  return area. The character count the layout carries is counted from the bytes,
  because the boundary carries a byte count and nothing else.

  The core module beside the source has none of it. What goes inside the
  component is that module with the adapters appended, so every function keeps
  its index and every export keeps its name, and a host embedding `<name>.wasm`
  is not handed a boundary it did not ask for. An export that carries only
  numbers and booleans still lifts with no options, so every component this
  wrote before is byte for byte what it was.

  A list, a record and a choice are still refused, and the line still names the
  export and what it needs.

  Measured through the Bytecode Alliance's own tooling rather than asserted, and
  in ten checks rather than one, because the ways this can be wrong are not one
  way: an empty string, a string that is not the first parameter, two at once, a
  string going one way only, text outside ASCII, and one long enough to need
  more memory than the module starts with. The last is there because a
  `cabi_realloc` that moved the bump pointer without growing the memory is
  exactly the bug `str_concat` carried for two releases, and it passes
  everything short.

### Measurements

- What a call boundary costs a walk, which was a sentence and is now a table.
  `design/hash-map-requirements.md` said the remaining copying was "`push` at a
  function boundary, where no bound is known". Measured: the same push, moved
  into a two-line function and called from the walk, allocates the answer once
  per element again — 72 bytes becomes 360 at a length of eight, and 1,032
  becomes 67,080 at a hundred and twenty-eight.

  The walk knows its accumulator is unshared and cannot tell the callee; the
  callee cannot see that its caller is finished. So the missing fact is one
  fact, it is interprocedural, and a wrong answer to it is a write into memory
  something else is still reading, which nothing in the language or the runtime
  would notice.
  `design/decisions/2026-08-09-what-a-callee-does-with-its-argument.md` names
  it, fixes the order as the fact before the transformation, and says which way
  the analysis is allowed to be wrong. No compiler behaviour changes.

  Held by `crates/deed-driver/tests/allocation.rs`, by the shape rather than the
  numbers: a push behind a call allocates several times the answer, and doubling
  the length more than doubles that, which a constant per call would not.

## 0.2.10 (2026-08-09)

### Programs that used to compile and no longer do

- None yet.

### Fixed

- A compiled program that joins strings past the memory it started with no
  longer writes them past the end of it. Eleven of the twelve runtime helpers
  that reserve room grow the memory while doing it; `str_concat` moved the bump
  pointer itself and skipped the growth, so text crossing the sixteen pages a
  module starts with went out of bounds and the program stopped with "reached
  past the end of memory".

  That is the sentence a program which has genuinely run out produces, so
  `examples/logs.deed` dying compiled was read for two releases as the limit
  `design/decisions/2026-07-31-compiled-memory-reclamation.md` measures, and
  raising the runner's ceiling not helping was taken as confirmation. It never
  tried to grow. The program now runs compiled and prints the same 69,680 bytes
  the interpreter prints, byte for byte.

  Nothing about reclamation changes: a compiled program still gives nothing
  back, and a keyed update is still quadratic. What changed is that one program
  counted as evidence for that limit was not evidence for anything.

### Language

- A comparison with the name written second now settles as much as the same
  comparison the other way round. `if 0 < n - 5` proved nothing while
  `if n - 5 > 0` proved the refinement below it, because turning the clause
  into a linear form gives the name a count of minus one, and dividing an open
  bound by that overflows: `i64::MIN / -1`. The bound was dropped instead of
  staying open. Two spellings of one claim answering differently is the same
  defect #929 fixed for `!=`, in a different place.

- The checker has one function that bounds a name from a comparison rather than
  two. `narrow_side` and `narrow_scaled` both answered "what does this
  comparison say about this name", and which of them owned it had been written
  down as an open question, with a measurement attached: disabling three of
  `narrow_side`'s arms changed nothing across the suite and disabling the
  fourth broke eighteen tests. The asymmetry was the overflow above, not the
  shape of the corpus. With that fixed, all four ordering arms are dead across
  2394 tests and are gone. What is left of that function is the question only
  it can ask, ruling a single value out at an edge of what is known, and it is
  named for it.

### Diagnostics

- Nothing yet.

### Standard library

- Nothing yet.

### Tools

- `deed build --component` writes a component binary. Until now it wrote a core
  module and a `.wit` world, and handing that core module to
  `wasm-tools component new` produced a component exporting nothing, which
  `design/decisions/2026-08-07-a-wit-world-is-not-a-component.md` measured and
  wrote down as the gap. `<name>.component.wasm` is that gap closed for the
  exports that cross the boundary unchanged: a number is an `s64` on both sides
  and a boolean is a `bool` on both sides, so lifting one needs no adapter.

  Anything wider than a word does not get one. A string crosses as a pointer
  and a length into memory the caller helped allocate through `cabi_realloc`,
  and this backend passes one address in its own layout, so a module carrying
  text gets the core module, the world, and a line naming the export and what
  it needs. A component that answered wrongly would be worse than one that is
  not written.

  The core module and the `.wit` are unchanged and are still written, so
  nothing that reads them has to know about any of this.

  Measured rather than asserted: `crates/deed-codegen/component.mjs` now reads
  the component's world with the Bytecode Alliance's own tooling, transpiles
  it, calls both exports and checks the answers, because reading a world is not
  running one. The encoder is in `crates/deed-codegen/src/component.rs` and has
  no dependency; every constant in it was read out of a component that
  toolchain built.

- `cargo install deed-lang` works. The compiler is on crates.io as of 0.2.9,
  as twenty crates, and the install instructions say so. `deed` on crates.io
  belongs to somebody else, so the package is `deed-lang` and the binary it
  installs is `deed`.

  Verified the way a stranger does it rather than by watching the upload
  succeed: installed from crates.io into an empty root, then `deed explain
  DEED4025`, `deed new`, `deed test`, and a program that imports `std/list`.
  The first two are exactly what this version's two packaging bugs would have
  broken, and one of them would have broken quietly.

### Measurements

- `benchmarks/RESULTS.md`: what five runs of one model against one build said,
  as its own document rather than a section at the bottom of the harness's
  README. Six tasks, five runs, a control arm with the compiler taken away, the
  hundred and two tool calls it made, and a list of what it does not establish.

- The scorer stops reporting `proven` and `guarded` for an answer the compiler
  rejected. `benchmarks/README.md` already said "anything else, and nothing
  further is measured", and the tool measured further: four blind runs came
  back `0 check` with a proven obligation printed beside them, which reads as
  the arm that wrote nothing compilable proving as much as the arm that did.
  Those columns now print `-`, the way the tests column already did, so a zero
  that was measured is distinguishable from one that was not.

- `deed_test` is not one of the tools a benchmark task leaves with nothing to
  do, and the README said it was. Three of the eight `deed_test` calls across
  those runs came back with a property the compiler generated from the contract
  in the answer, a hundred cases each, in tasks whose prompts say "no tests".

## 0.2.9 (2026-08-08)

### Programs that used to compile and no longer do

- None yet.

### Language

- Fixed: a contract that mentions `result` or `old(...)` inside a closure now
  runs. `deed check` accepted it and running it answered `DEED6006`, whose own
  note says either the file was not checked or the check has a hole; it was the
  hole. The interpreter decides whether an obligation needs `result` bound, and
  whether it needs an entry snapshot, by walking the clause, and neither walk
  went into a closure body.

  So `ensures ok => any(numbers, |n: Int| n == result)` — the natural way to
  say the answer came out of the list — was the one shape a contract could not
  be written in. The two walks are now one, matched without a wildcard, so a
  new kind of expression is a build error rather than a contract that quietly
  stops being checked.

### Diagnostics

- Changed: importing a name the language already provides now says so. Five
  benchmark runs of one model against one build all wrote `use
  std/string.{join}`, and the compiler answered with two messages: an error
  that `std/string` declares no `join`, and a warning that `join` hides a
  builtin. Both were true and both pointed at the module, which is the one
  place the answer was not.

  It is now one error saying the name is already in scope, with a
  machine-applicable repair that deletes the import. Nothing is declared for
  the refused name either, so the call in the body binds the builtin and the
  rest of the file goes on being checked instead of cascading. Importing a
  name a module really does declare is unchanged and still warns.

### Standard library

- Nothing yet.

### Tools

- The command line crate is published as `deed-lang`, so `cargo install
  deed-lang` is a thing that will work. `deed` on crates.io belongs to somebody
  else, and the name a reader would guess is the org's. The binary is still
  `deed` and nothing anyone types changes.

  Its directory stays `crates/deed-cli`: every decision record and changelog
  entry that points into this crate points at that path, and moving it would
  rewrite sentences that were true when they were written. `Cargo.toml` says
  so where the two names sit next to each other.

  Nothing in the tree spells a package name, so a rename is invisible until a
  workflow fails on a tag. `the_workflows_build_packages_that_exist` reads
  every `-p` in `.github/workflows` and asks whether that crate is here.

- The workspace can be published. Internal dependencies were paths with no
  version, which `cargo publish` refuses, and putting a version next to each of
  twenty paths would have made the release checklist nineteen lines longer. They
  are declared once in `[workspace.dependencies]` and inherited, and
  `crates/deed-driver/tests/publishing.rs` compares that table to
  `workspace.package.version` so the two cannot drift apart quietly.

  `cargo publish --workspace --dry-run` packages all twenty crates and builds
  each one out of its own archive, and CI runs it on every commit as "it could
  be published". Both of the packaging bugs above were found by hand; this is
  the question being asked automatically instead.

- The shipped library travels inside the crate that carries it. `SHIPPED`
  read the nine `std/*.deed` modules with `include_str!` through three parent
  directories, and `cargo package -p deed-driver --list` carries none of them,
  so a compiler installed from crates.io would not have built at all. The text
  is generated into `crates/deed-driver/generated/shipped_sources.rs` now, and
  held against the files by a test that names the command that rewrites it.

  The order the library is listed in moved into the open while doing it. It was
  a side effect of how the table happened to be written, and three documents
  read in it, so it is a declared list now and the generated part is only text.

- New rule, held for every package: nothing a crate compiles may read above
  its own root. `crates/deed-driver/tests/packaging.rs` walks every `src/` for
  `include!`, `include_str!` and `include_bytes!` and measures how far each
  path climbs, and asks the same of any build script. Both ways this workspace
  had broken it are the two entries above, and they failed differently: one
  would not compile, the other compiled and went silent.

- Fixed, before anybody could hit it: `deed explain` would have printed
  nothing at all, for every code, in a compiler installed from crates.io. The
  pages were produced by a build script that read every `codes.rs` in the
  workspace and the whole test corpus. A published `.crate` carries one
  package directory and no workspace, so that script would have found an empty
  tree, generated zero pages, and **compiled**. Measured, not guessed:
  `cargo package -p deed-explain --list` lists `build.rs` and `src/lib.rs`.

  The pages are generated, committed as `crates/deed-explain/generated/pages.rs`,
  and shipped as source. The reading of the tree happens in a test now, where
  the tree is: one test fails when the committed file has drifted from the
  `codes.rs` files it is made of, and an ignored one rewrites it. A third holds
  the rule the bug broke, which is that whatever `src/lib.rs` includes has to be
  a file the package carries.

- `install.sh` and `install.ps1`: one line to get the binary, and no Rust.
  Four release artifacts have existed for eight versions and nothing pointed at
  them, so the shortest honest instruction was still "clone this and install a
  toolchain". The scripts pick the asset for the machine, refuse it if its hash
  is not the one the release published, and write one file inside your own
  profile, so neither of them ever needs a password.

  Releases now carry `deed-<tag>-checksums.txt`, which is what makes that
  refusal possible. It comes from the same release as the binary, so it catches
  a corrupted download and not a compromised release, and both scripts say so
  rather than implying more.

  `crates/deed-driver/tests/install.rs` builds a release, serves it to the real
  script, and asks it to install a binary and then to refuse the same binary
  with one byte changed. It also holds the platform list in the two scripts
  against the build matrix in `.github/workflows/release.yml`, in both
  directions.

- `deed new <name>` writes a project. Until now the tool could check, test,
  run, compile, format, document, explain and answer an editor, and the one
  thing it could not do was produce a file to point any of that at, so
  somebody trying the language had to work the module header and the file
  layout out of this repository first.

  It writes a directory holding a library module with a contract and its
  tests, and a program that imports it. Two files, because the `module` line
  and the `use` line are the whole of how a project splits in two and seeing
  it once beats reading about it. No manifest: a manifest here says where code
  outside your tree lives, and a new project has none, so writing an empty one
  would be text nothing reads.

  `crates/deed-cli/tests/new.rs` runs the command into a temporary directory
  and then checks, tests, runs and format-checks what came out. The scaffold
  lives in a Rust string literal where none of the corpus ratchets reach it,
  so the claim that it works is held end to end rather than described.

- Fixed: a compiled program called its host with the wrong arguments when one
  of them was computed. `Io.write(sys.console, to_string(n))` declared an
  import taking *one* argument, because the signature was built from a
  function that recognised literals and capabilities and answered nothing for
  anything else, and "nothing" was dropped from the parameter list rather than
  refused. The host was then handed the text where the console belonged.

  Nothing could see it before there was a host: the runner reached the import
  and stopped without reading what it had been passed. The check that a
  capability is one the host handed out is what found it.

  The signature is the argument types the program actually produces now.

- A module's memory grows. The sixteen pages it starts with were a starting
  point and never a decision, and until now they were a ceiling: a compiled
  walk building a list stopped at about sixty-five thousand elements with
  "reached past the end of memory", which says nothing about the program.
  It runs to two million now, and what stops it there is the host.

  This changes only where the ceiling is. Nothing is given back, so total
  allocation is still peak memory: `examples/logs.deed` over one file
  finishes and over two does not, and raising the runner's limit to four
  gigabytes — the whole of a thirty-two bit address space — does not change
  that. Reclamation is still the question, and
  `design/decisions/2026-07-31-compiled-memory-reclamation.md` is still where
  it is asked.

- `deed build --component` says what it writes. It produces a core module and
  the `.wit` world its exports describe, and the help text used to say it
  "produces a component". Handing that core module to `wasm-tools component
  new` produces a component whose world is empty, because the exports cross
  the boundary in this backend's own layout rather than the canonical ABI and
  nothing writes the component-type section.

  Measured on every commit now rather than assumed, with the Bytecode
  Alliance's own tooling. The day the component stops being empty is the day
  that job fails and somebody says why. See
  [design/decisions/2026-08-07-a-wit-world-is-not-a-component.md](design/decisions/2026-08-07-a-wit-world-is-not-a-component.md)
  for the three things that are missing.

- New guide: [how-to/embed-a-compiled-program.md](how-to/embed-a-compiled-program.md),
  which is what a host has to know. The memory layout it reads and writes
  lived in `layout.rs`'s doc comments, so somebody embedding a compiled
  program had to open the compiler's source to find out where a string keeps
  its byte count. `crates/deed-driver/tests/embedding.rs` reads the numbers
  off the page and asks `layout.rs` for the same ones.

### Measurements

- Nothing yet.

## 0.2.8 (2026-08-07)

0.2.7 said a compiled Deed program can act. That was true of the host inside
`deed` and not of anybody else's, and this is the difference.

### Programs that used to compile and no longer do

- None.

### Language

- Nothing.

### Diagnostics

- Nothing.

### Standard library

- Nothing.

### Tools

- `deed build` exports the module's memory, so a host that is not this one can
  read what a compiled program hands it.

  A capability crosses the boundary as a number, but a string does not:
  `Io.write(console, text)` passes an address into the module's own memory.
  The host inside `deed` shares an address space with the module and is handed
  that memory directly, so nothing here noticed that a module which does not
  export it can only be answered from inside this workspace.

  Measured in the engine Node ships, before the export existed: the module
  compiled, it instantiated, its import section was exactly the row,
  `twice(21)` answered `42` — and every operation carrying text was
  unreachable. `crates/deed-codegen/smoke.mjs` is that measurement kept, and
  CI runs it, because an artifact that was built and weighed and never run is
  a mistake this repository has already paid for once.

  This grants a program nothing. A module could always read and write its own
  memory; the export is about what the host may look at.

### Measurements

- Nothing.

## 0.2.7 (2026-08-07)

A compiled Deed program can act now.

The backend has said for months that a program's import section is its
capability requirements, and the tests have held that claim from both sides:
a module that does not name an operation has no index to call it through, and
a module that names one the host cannot answer is refused before it runs.
Nothing crossed the boundary. A compiled `examples/hello.deed` stopped with
``` `deed:sys.console` is the host's to answer, and this is not one ``` while
the interpreted one printed "hello, world".

The obstacle was one line of a type. A host implementation was handed the
call's arguments and nothing else, and a string argument is an address into
the module's own memory, so no host was writable for any operation carrying
something other than a number.

`deed run --compiled` now grants what `deed run` grants, and six of the seven
corpus programs with a `main` print exactly what the interpreted ones print.
The seventh runs out of the module's memory, which is a question about value
reclamation rather than about hosts and is the clearest thing this release
leaves open.

Two smaller things came out of measuring the model against a real transcript:
`deed fix` was writing an import that made a file worse, and
`deed build --component` crashed on the one shape `design/04-capabilities.md`
rests on being impossible.

### Programs that used to compile and no longer do

- None.

### Language

- `Io.env(sys, name)` reads one environment variable, and only the ones the
  run was told to hand over. `deed run --env NAME` grants a name, repeatably,
  and everything else reads as absent.

  That is the `--allow` shape rather than the `--dir` one, and the difference
  is the point. The arguments were typed on the line that started the program;
  the environment is whatever the machine happened to be carrying, which
  routinely includes credentials nobody meant to pass on. So a program sees a
  list of names somebody decided on rather than all of it, and a name that was
  not granted is reported as not granted rather than as unset, because those
  are different facts and only one of them is about the machine.

  It takes the whole `System`, for the reason `Io.args` does: reading the
  outside belongs near `main`, and everything below is handed the value rather
  than the means to read it again. `--env` is refused anywhere but `deed run`,
  because a test whose answer depended on what the machine was carrying would
  be a test of that machine.

- `Io.line(console)` reads what somebody typed, one line at a time. It is the
  `read`/`save` split applied to the console: the same capability, a separate
  entry in the row, so a function handed a `Console` to write to still cannot
  find out what was typed unless its signature says so. The line comes back
  without its terminator and without a carriage return in front of it, because
  which of those a line ends with is a fact about the machine it was typed on.

  Running out of input is `err` rather than an empty string. A program that
  cannot tell "somebody typed nothing" from "there is nothing left" either
  loops forever or stops early, and both of those are silent.

  `deed run` reads standard input when, and only when, `main`'s row says
  `Io.line`. There is no flag and no guess about whether a terminal is
  attached: the row is already the list of what a program does with the
  outside, so it is what decides what the outside hands over, and a program
  that never mentions input cannot be left waiting for it. Everywhere else —
  `deed test`, the playground, the agent server — hands over nothing, for the
  reason they hand over no arguments and no directory: a test whose answer
  depended on what somebody typed would be a test of the typing.

- Ruling a value out at the edge of what is known narrows the other branch.
  `if n == Int.min { .. } else { .. }` and `if n <= Int.min { .. } else { .. }`
  say the same thing about the else branch, and only the second one used to
  prove an obligation below it: `!=` narrowed nothing, on the grounds that a
  range cannot say "not this one". That is true in the middle and false at
  either end, and the end is where the case turned out to be, found by writing
  contracts for `std/ratio` in 0.2.6. A value ruled out in the middle still
  settles nothing, because what is left of the range is two of them.

### Diagnostics

- A walk's accumulator read underneath its walk is told where the value is,
  rather than offered an import. `for n in ns with sum = 0 { .. }` followed by
  `sum` is a name spelled exactly right, declared a statement ago and out of
  scope, and `sum` is also a function in `std/list`, so the answer was a
  machine-applicable fix writing an import for a different function entirely,
  which `deed fix` applied. The diagnostic now names where the accumulator was
  declared, what the walk ended up as, and that what reads it is
  `let sum = for ... { ... }`. The general rule behind it: a name this file
  declares somewhere is a question about scope, so the shipped library stops
  offering to import it.

- `point.x = 1` is `DEED2027` rather than two messages about neither half of
  it. The name form of assignment is one this language has, because a handler's
  state can be written to, so the field form used to arrive as an expression
  statement followed by a stray `=` and the reader was told "expected an
  expression, found `=`". The measured shape is
  `state.entries = state.entries + [entry]`, somebody reaching for a handler's
  state through a receiver, and that one gets a second sentence saying where
  the state is named. No repair either way: a record with one field changed is
  another record literal, which is a rewrite of the line rather than an edit to
  it, and taking `state.` off would be wrong for anybody who bound a record to
  that name.

- `export`, `pub` and `public` in front of a declaration offer to come out. The
  word already carried the reason there is no visibility modifier here, and a
  reason with no repair is something a reader agrees with and then writes
  again: across three benchmark runs one task met that message twenty-four
  times. Taking a word out is not substituting one, which is why this can be
  machine-applicable, and it is offered only when a declaration really follows.

- `perform`, `state`, `append`, `rest`, `update`, `mutate`, `change` and `put`
  are answered rather than run through the edit-distance table. Each is a word
  another language would have had, and a name that is not a typo used to come
  back with whatever short name happened to be nearby, or with nothing at all.

- `text.to_upper()` says there are no methods and names the module that has
  the function. The type checker already said that for prelude names, because
  it can ask whether a name is a builtin; it cannot ask about `std/string`,
  which arrives through imports the file did not write. The sentence is
  finished in the driver, off the same table that writes the `use` line. No
  repair: adding the import leaves the call as broken as it was.

### Tools

- `deed run --compiled` writes lines and reads a clock, because there is a
  host now. A compiled `examples/hello.deed` used to stop with ``
  `deed:sys.console` is the host's to answer, and this is not one``: the
  interpreted program printed "hello, world" and the compiled one could not
  say anything at all.

  What was missing was not the wiring. A host implementation was handed the
  call's arguments and nothing else, and `Io.write(console, text)` passes the
  text as an address into the module's own memory, so no host could be
  written for any operation carrying something other than a number. A host is
  handed the call now — its arguments and the module's memory — and can read
  a string out of it or allocate one back into it.

  A capability crosses the boundary as an index into a table the host keeps,
  and the table records what each handle is. So a number the host never gave
  out is not a capability, and a console passed where a clock belongs is
  refused. Neither is reachable from a checked Deed program; the check is
  there because a host is handed modules rather than programs.

  Each grant offers exactly the imports it can answer, so a program asking
  for one that was not granted is turned down at link time by name — ``the
  host does not offer `wasi:random/random.roll` `` — rather than at the first
  call. `deed run --compiled` grants what `deed run` grants: a console, a
  clock, the directory `--dir` named, the hosts `--allow` named, the
  variables `--env` named, the arguments, and standard input when and only
  when `main`'s row says the program reads it. Six of the seven corpus
  programs with a `main` now print exactly what the interpreted ones print;
  the seventh runs out of the module's memory, which is the reclamation
  question `design/05-backend.md` already names.

  What a `Dir` reaches, what a `Net` reaches, and what an HTTP status outside
  the two hundreds means are `deed-rt`'s in both engines rather than written
  twice. What is left unanswered is an interface a program declared for
  itself with `effect ... from`, because nothing here can know what one
  means. See
  [design/decisions/2026-08-07-what-a-host-hands-a-compiled-program.md](design/decisions/2026-08-07-what-a-host-hands-a-compiled-program.md).

- `deed run --compiled` hands the program its arguments, and no longer
  refuses to start when it was given any.

- Fixed: `deed build --component` panicked on a signature holding a capability
  or a function value one field down. `record Holder { dir: Dir }` walked past
  the refusal and reached an `unreachable!` while the WIT world was being
  written, so the compiler crashed on the one shape
  `design/04-capabilities.md` rests on being impossible. The refusal was a
  list written beside the WIT printer and it did not look inside a record or a
  choice.

  It asks the canonical ABI now. `crates/deed-codegen/src/abi.rs` is this
  workspace's transcription of the component model's own rules for how a value
  crosses a boundary, it walks fields and variants because a host has to read
  them, and until now nothing in the compiler called it. One rule instead of
  two, and the weaker of the two is gone.

  The canonical ABI grew the half it was missing in return: a `list<T>` is
  refused when `T` is, which the list beside the printer did check.

## 0.2.6 (2026-08-07)

This release is about the edges of what the language can say and what the
backend can compile, and both were found by asking a question the corpus could
not answer.

The smallest `Int` has a literal now. It never had one, because negation is an
operator and the digits of that number are one past the largest, so a clause
about the edge had to be spelled `0 - 9223372036854775807 - 1`.

The backend compiles two shapes it had always been handed and never lowered.
Nothing had noticed, because a program the backend cannot lower and a program
with no tests in it produce the same silence. The suite that said the backend
refuses nothing was measuring `examples/`, which is the shapes one author
happened to write; every case in `conformance/` is now held to being lowerable,
because those exist to cover the language, which is the question that was
actually being asked.

`std/ratio` writes contracts, which is the threshold
`design/fractional-values.md` had been watching for. Writing them found an
overflow nothing else had.

### Programs that used to compile and no longer do

- None.

### Language

- The smallest `Int` can be written down: `-9223372036854775808` is one
  literal. Negation is an operator everywhere else, and the digits of the
  smallest `Int` are one past the largest, so a literal alone could never say
  it and a clause about the edge had to spell it `0 - 9223372036854775807 - 1`.
  The lexer now hands those digits over without judging them, because whether
  the minus in front is the unary one is a question about the grammar, and the
  parser reads the pair as one number. Anywhere else the digits are still the
  literal that does not fit, reported once, by the pass that knows.

- The backend compiles two shapes the checker had always accepted: a pattern
  inside a record pattern, `Box { size: Inner { depth } }`, and a `let` that
  takes a value apart, `let Point { x, y } = point`. Both checked, both ran
  under the interpreter, and neither could be lowered, so `deed build` and
  `deed test --compiled` quietly had nothing to say about a file that used
  one. Nothing had noticed because a program the backend cannot lower and a
  program with no tests in it produce the same silence.

  Every case in `conformance/` that the checker accepts is now held to being
  lowerable. The suite that already said the backend refuses nothing was
  measuring `examples/`, which is the shapes one author happened to write; the
  conformance cases exist to cover the language, which is the question that
  was actually being asked.

### Standard library

- `std/ratio` writes contracts. `absolute` promises a number that is not
  negative and asks for one that is not the smallest `Int`, because that one
  has no positive counterpart and `0 - n` overflowed for it; `ratio` turns it
  away at the door so everything below is proven rather than checked again.
  `simplified` promises a positive denominator. Ninety-two of that module's
  obligations are proven, three are tested by generated inputs and eleven are
  guarded, and every guarded one is a call whose arguments are arithmetic.
  This is the threshold `design/fractional-values.md` was watching for: the
  page has been reread and the answer did not change, because a clause about a
  fraction made of two `Int`s is a clause about integers.
- Fixed: `ratio(Int.min, 1)` stopped the program instead of answering. Nothing
  had noticed, because nothing had been asked to say what `absolute` promises.

### Measurements

- The corpus and the shipped library carry 167 proven obligations, 11 tested
  and 23 guarded, against 75, 8 and 12 before `std/ratio` had contracts. An
  unattempted `ensures` clause is no longer the majority of what is guarded,
  which was never what the page rested on; that nothing settles one ahead of
  time is, and it still holds.
- Of the twenty-nine calls to `at` in the library and the corpus, twelve pay
  for a failure with a `match` or a `?`, and exactly one of those is a bound
  the checker could discharge today. Every other one indexes a list that came
  back from a call, and `length(f(x))` is not a term the prover holds, so the
  caller could not prove the bound even if there were somewhere to spend it.
  That moves the open question about a total indexing form rather than
  answering it: what is in the way is not the prelude name.

## 0.2.5 (2026-08-05)

This release is about what a compiled program does with memory, and about the
last things a reader could not see.

A walk that builds a list now builds one block rather than one a turn in three
more shapes than it did: one that reads its own length, one that stops on a
`while` clause, and one that starts from a list it was handed. That last one is
`concat` and `prepend`, so joining two lists of 512 went from running out of
memory to allocating twenty-four kilobytes. A compiled hash map reached two to
three hundred keys and now reaches three to four hundred, which is the ceiling
`design/hash-map-requirements.md` has been watching move.

One of those changes closed a hole rather than opening a door. The rule read a
walk's body and a `while` clause is not in the body, so a walk that handed its
accumulator to something that kept it there was compiled as though nothing
could reach the list. `crates/deed-driver/tests/agreement.rs` carries the
program: it answered 4 interpreted and 404 compiled.

Two questions that had been open were answered by building the thing and
measuring it, and one of the answers is no. Property tests still do not cover
effectful functions, and the reason is now written down rather than assumed: a
handler with state is a model with a starting point, and generated arguments
take it outside the world it models.

The library gained a set, and the calendar gained the years before 1970. The
language server answers semantic tokens, which is the last capability on its
advertised list and the one an editor cannot work out for itself.

### Programs that used to compile and no longer do

- None.

### Language

- `hash(value) -> Int` is a prelude function. Structural, with no trait bound,
  which is what #617 decided. It is the equality walk with a different
  accumulator: whatever `==` compares, `hash` absorbs, in the same order, which
  is why `a == b` implies `hash(a) == hash(b)` by construction rather than by
  care. One algorithm, written down once in `deed_rt::hashing` and read by both
  engines, because a hash is an `Int` a program can assert on and two engines
  computing it differently would be two engines disagreeing about what a
  program means. It refuses a function value and a capability with `DEED4032`:
  both are equal to another when they are the same one, so there is nothing to
  read. Not cryptographic and not seeded, with the reasons in
  `design/decisions/2026-08-05-a-hash-is-the-equality-walk.md`.
- `<` can be bound to a function, and one binding answers all four comparisons:
  `a > b` is `b < a`, and `a <= b` is not `b < a`. Binding four separately would
  let them disagree about the same two values with nothing to say so. An order
  answers with a `Bool` rather than with the type it was given, which is the
  one place a bound operator's shape differs from the arithmetic three. This is
  notation and not dispatch: `<` on a type parameter is still refused and
  `sort` still takes a comparison, so the trait threshold has not moved.
  Decision record: `design/decisions/2026-08-04-one-binding-for-an-order.md`.
- An effect may name the interface its operations come from:
  `effect Random from "wasi:random/random" { .. }`. Without the clause an
  effect is its own interface and an unhandled operation is imported as
  `deed:<effect>`, which is the right default for one a program invented and
  useless for one that already exists somewhere else. `from` is a soft keyword,
  so `fn from(from: Int)` still compiles.

### Diagnostics

- `DEED7003`, `DEED7004` and `DEED7005`: a `module` directive with nothing
  after it, one with a location and no hash, and a hash that is not sixty-four
  lowercase hexadecimal digits after `sha256:`. `DEED7006`: bytes that could
  not be retrieved, or that did not hash to what the manifest said.
- `DEED2026`: an interface name with nothing in it. Leaving the clause off is
  how an effect says it is its own interface, and that is a different thing
  from writing one and leaving it blank.

### Standard library

- `std/set` is a set, written as the hash map with the values hidden. `none`,
  `one`, `including`, `has`, `count`, `items`, `without`, `union`,
  `intersection`, `difference`, `within` and `entries_of`. Not `with`, because
  that is how a handler is installed and the grammar has the word already, and
  no `Empty`, because an empty list takes its element type from where it is
  used and there is nowhere here to take one from, so every constructor takes a
  sample the way `std/hashmap`'s `empty` does.
- `std/date` answers for a clock set before 1970. The only thing in the way was
  that `/` rounds toward zero, so the millisecond before the epoch landed on
  day zero and read as the first of January 1970; `days_since_epoch` floors
  now. `date_of` still refuses, but only before year zero, which is where the
  shift `civil_from_days` applies runs out rather than where the calendar
  starts.
- `std/ratio` binds `<` to `is_below`, so two ratios compare the way two `Int`s
  do. `is_below` and `is_above` are unchanged and still exported, because a
  comparison is what `sort` takes.

### Tools

- `deed lsp` answers `textDocument/semanticTokens/full`, so an editor can
  colour from what the compiler concluded rather than from a grammar that
  guesses at spelling. The split between a keyword and a name is the lexer's
  own, and what a name stands for is the resolver's: a name that resolves to a
  function is a function, one that resolves to a record is a type, a variant is
  an enum member and a parameter is a parameter. Punctuation carries no colour,
  because an editor already draws brackets. A file that does not check is still
  painted, since that is when a reader most wants it.
- `deed debug` speaks the Debug Adapter Protocol on stdin and stdout, so an
  editor can set breakpoints, step in, over and out, and read the stack and the
  bindings of every active call. The program is held still by a hook the
  interpreter calls before each statement: a watcher stops by not returning, so
  the host stack is the program's stack and nothing is re-run or simulated.
  What a breakpoint is and what a step means live in `deed-dap` and not in the
  interpreter, which knows nothing about lines. There is no `pause` and no
  `evaluate`, both stated with their reasons in
  `design/decisions/2026-08-04-a-place-to-stand.md`.
- A `deed.manifest` can declare a module by where its bytes are and what they
  hash to: `module https://example.com/list.deed sha256:9f2c...`. The bytes are
  fetched when the cache does not already hold them, refused outright when they
  do not hash to what the manifest says, and never stored when they do not. A
  location may also be a path, because a dependency being developed alongside
  its user is the ordinary case and the hash is checked either way.
  The directive does not say what the module is called: a fetched module is
  named by its own `module` line, so two projects fetching the same bytes from
  two mirrors get one module. This makes `deed-fetch` reachable for the first
  time; only its SHA-256 helper had a caller before. Decision record:
  `design/decisions/2026-08-04-a-dependency-is-a-location-and-a-hash.md`.
- `deed build --component` now writes what a component needs as well as what
  it offers. An effect the program performs and never handles becomes an
  `import` in the world and a WebAssembly import in the module, and the call
  goes to the host instead of walking off the end of the handler list. Before
  this a component that performed an unhandled effect produced a world claiming
  it was self-contained and a module with no import section at all, and it
  trapped when the export was called. Decision record:
  `design/decisions/2026-08-04-a-component-asks-for-what-it-needs.md`.
- Fixed: a compiled walk that pushed onto its accumulator twice in a turn, with
  the second push away from the value the turn hands back, was read as the
  shape that builds one list. It grew the list by two a turn into room reserved
  for one, so it wrote past the end and answered with a list of a length the
  interpreter never produced. The same gap admitted a branch that kept the
  accumulator somewhere else in the body. Both are refused now, and neither
  shape appears in the shipped library or the corpus, so nothing that used to
  take the fast path stopped taking it.
- Fixed: the same rule never looked at a walk's `while` clause, which is read
  before each turn with the accumulator in scope. A walk that handed its
  accumulator to something that kept it there was still compiled as though
  nothing could reach the list, and went on writing into a block the program
  was holding. `crates/deed-driver/tests/agreement.rs` carries the program:
  it answered 4 interpreted and 404 compiled.
- A walk may start from a list it was handed and still build one block.
  `concat` and `prepend` in `std/list` are that shape, and they used to copy
  the whole accumulator every turn. The block is reserved as long as what the
  walk started from plus the list it walks, and the walk copies what it started
  from into it once, because whoever handed that list over is still holding it.
  Joining two lists of 128 allocated 202272 bytes and now allocates 6184; two
  of 512 ran out of memory and now allocate 24616. Decision record:
  `design/decisions/2026-08-05-a-walk-may-start-from-a-list.md`.
- A walk may read its accumulator's own length. `push(out, length(out))` builds
  one list rather than one a turn now, so a walk can number what it is building
  without carrying the position beside it. Reading a length keeps nothing, and
  the length of a reserved block is written as the walk goes, so every read
  answers what a walk that copied would have answered. `std/hashmap`'s `range`
  is written the obvious way again and the record it carried to work around the
  refusal is gone; a compiled map now stops between three and four hundred keys
  rather than between two and three. Decision record:
  `design/decisions/2026-08-05-a-walk-may-read-its-own-length.md`.

### Measurements

- `partition` over a list of 256, with the list it walks subtracted off,
  allocated 267304 bytes and now allocates 8224. Over 1024 it ran out of
  memory and now allocates 32800.
- The counts in `design/decisions/2026-08-04-a-walk-that-only-pushes.md` said
  forty-four walks of the shape against thirty-four of every other. The rule
  that shipped answered forty and thirty-eight. The record now says what the
  rule says, and a test holds it there.
- Those corrected counts were wrong too, in two ways that cancelled out about
  half of each other. One condition of the rule lived at the call site rather
  than with the other two, so the measurement counted eight walks the compiler
  never built in one list, and the denominator counted walks whose accumulator
  is a number and which allocate nothing. Of the walks that build a list,
  thirty-nine are built in one and twenty-one are not. Both decision records
  print the numbers and both are read off disk by the test.
- A compiled `range` of 64 allocated 2080 bytes and now allocates 1040, which
  is the record it used to build a turn going away.
- Of the walks in the library and the corpus that build a list, thirty-nine
  were built in one block and forty-seven are now, which is `concat`, `prepend`
  and six more like them joining the set.

## 0.2.4 (2026-08-04)

Most of this release is one question asked properly: what does a compiled
program do with memory. The answer was that building a list of 256 allocated
129 times what the answer was worth, and that a list of 1024 could not be
built at all, because every turn of a walk copied the accumulator to append
one item. Nothing about that was visible from a clock, which is why it went
unnoticed for four releases: what makes it visible is counting instructions
and bytes rather than timing them, and a count is the same on every machine
and can therefore be held to a ratchet.

Two things came out of the counting. A handler's state was allocated where a
handler was installed and never given back, so a program that installs one in
a loop grew without bound; it goes on the frame stack now and installing one
is free. And a walk whose accumulator is only ever pushed onto builds one
list, which is not reuse analysis and does not stand in for it: it asks
nothing about whether a value is unshared, it arranges for there to be nothing
to share. That works because `for` is the only loop and a `for` is a fold, so
the intermediate lists exist only as values of one name.

The rest is notation. `1/2 + 1/3` is written the way arithmetic is written,
`Int.max` is a number rather than nineteen digits, and an effect takes row
variables, which is what a scheduler needed before it could be shipped.

### Programs that used to compile and no longer do

- None yet.

### Language

- An operator can be bound to a function a module declares: `operator + = added`
  says that `+`, written between two values of the type that function takes,
  means it. `+`, `-` and `*`, and nothing else: an operator answers with the
  type it was handed, and `/` is partial, which this language spells with a
  `Result`. A binding rather than a definition, so the function keeps its name
  and can still be passed. The binding travels with the type, so a module that
  imports `Ratio` imports what `+` means on one. `std/ratio` binds all three,
  and `1/2 + 1/3` is now written the way arithmetic is written rather than as
  `added(half, third)`. Ordering is deliberately left out: `<` on a user type
  meets `sort`, which is the trait question rather than a question about
  notation.
- `Int.max` and `Int.min` are numbers. `Int` is a signed 64-bit integer and a
  program had no way to say so: a `where` clause keeping a sum inside the type
  had to carry nineteen digits, and the smallest `Int` had to be written
  `0 - 9223372036854775807 - 1` because negation is an operator rather than
  part of a literal. Both are folded where they are written, so a clause
  bounded by one is settled at check time rather than guarded.
- An effect takes row variables: `effect Task<uses r>`. They are in scope in
  its operation signatures and in the state of any handler implementing it, so
  a handler can hold a `List<Fn() uses r -> ()>`, and each call to an operation
  fills the variable in from the value it was passed. A program that forks a
  task which logs is charged with logging. Before this a scheduler's queue had
  to name every effect its tasks might perform, which only the program knows,
  so a scheduler could be written and not shipped.
- A function value written into a handler's state at a `with` is charged to
  whoever wrote it there. The state is the one way into a handler that does
  not go through an operation, so `with RoundRobin { queue: [noisy] }` used to
  put a task in that nobody answered for.
- A record pattern binds what it names. `err(OverLimit { limit })` bound
  nothing when `OverLimit` was a record rather than a variant of a choice, and
  the name it did not bind was reported missing where it was used rather than
  where it was written. The same pattern on a variant has always worked, and
  the two are one shape.

### Diagnostics

- A digit run one past the largest `Int` says what to write instead. It is the
  number somebody reaches for when they want the smallest one, and the answer
  is `Int.min` rather than digits. DEED4031, which used to answer `Int.max`
  with the digits and ask for them to be written out, is gone: the name is a
  value now.
- A refinement that fails at runtime says which refinement in the sentence and
  which value in the label under the source line, rather than putting the value
  in the sentence. Both engines stop on one now and only one of them can write
  an arbitrary value into a sentence, so the value in it would have been a
  second dialect of the same failure.
- An outcome written on a `where` clause gets DEED2023 and the condition under
  it is still read. `where ok => n + n <= 10` is what somebody writes after
  reading the `ensures` beside it, and it used to end the contract at the `=>`,
  so the answer was a block that was expected and, separately, that `ok` is a
  builtin rather than a value.
- A number or a string the lexer cannot read is one message rather than two.
  It used to hand the parser an invalid token, which then said an expression
  was expected in the same column the lexer had just written in. What was
  written stands in for it instead, the way a decimal point already did: the
  digits before the bad one, the largest `Int` for a number too big, and what
  was read before the line ended for a string with no closing quote.
- `a and b` and `a or b` are read as `&&` and `||`, with DEED2020 naming the
  operator and `deed fix` writing it. The words are ordinary names here, so a
  `where` clause holding one used to stop at the word and answer with the block
  that did not follow it. The resolver has had the answer since #213 and never
  got to give it: across eighteen recorded model runs the word reached the
  parser seventeen times and the resolver none.
- A function body written without braces gets DEED2021, which says a body is a
  block and offers the braces, and the body is read to the next declaration so
  the function is still a function. It used to be a message about a brace, then
  a message about the first line of the body not being a declaration, then a
  return type that matched nothing. Thirty-two of the six hundred programs in
  the recorded model runs were written this way.
- A contract written before the return type gets DEED2022, and the return type
  is read where it was written. `fn f(n: Int) where n > 0 -> Int` used to put
  the arrow where the body should have begun, so the message was about a brace
  and the function went on to have no return type and a body that did not match
  the one it was missing.
- `Int.max` and `Int.min` get DEED4031, which names the number and writes it.
  The answer used to be that `Int` is a type and not a value, which is true and
  is not the question. Ten of the recorded model turns reached for one of the
  two, all of them writing a `where` clause that had to keep a sum inside the
  type.
### Standard library

- `std/task` ships: a cooperative scheduler with `Task.fork`, `Task.more`,
  `Task.step`, and `run` and `run_up_to` over them. Tasks are function values
  and run to completion in the order they were forked; a task that wants to
  leave room for another forks the rest of itself. There are no resumptions,
  so nothing suspends in the middle. `examples/tasks.deed` uses it and
  `examples/scheduler.deed` is the same scheduler written by hand, kept for
  the comparison.

### Tools

- The compiled backend runs the whole shipped library. It ran 59 of the 91
  tests those modules carry, and `std/map` was twenty of which two ran. A
  module's generic functions are lowered once per set of type arguments, so a
  module full of them builds cleanly with no generic body ever put through the
  backend at all, and "the backend compiles the corpus" said nothing about
  them. `crates/deed-driver/tests/shipped.rs` runs every one now, and
  `examples/tree.deed` joined the corpus files whose test blocks run compiled.
- Two copies of a generic function that differ only in the order of their type
  arguments are two copies. The name a copy was cached under sorted them, so
  `keys<Int, Str>` and `keys<Str, Int>` were one entry and the second call
  reached the first call's body.
- A `use` that asks for a function gets the types in its signature too.
  `use std/table.{set}` is the whole of what a program writes, and `set` hands
  back a `Table<K, V>` over an `Entry<K, V>`; neither name appears on the
  importing side, so the backend refused the program over a type it had never
  been told about.
- A function from another module can be named as a value. The keyed libraries
  take a comparator and the one a program passes is one of theirs, so
  `insert(m, k, v, cmp_string)` was refused.
- A type parameter no argument says anything about, and a variant of a generic
  choice written where nothing says which one, stand in as numbers.
  `holds([], key)` over a `Table<K, V>` has no example of `V`, and `Empty`
  carries none of either half of a `Map<K, V>`; both still need a layout for a
  value that holds nothing.
- A closure whose body lifts anything points at itself. The compiled backend
  reserved the closure's place before lowering its body, and a body that
  lifted a function of its own, another closure, a copy of a generic function,
  a wrapper for a function named as a value, took that place first. So
  `|| Task.fork(step)` compiled to a value pointing at the wrapper for `step`,
  and calling it ran the wrong code with no diagnostic anywhere.
- A handler declared in another module is lowered against that module's
  tables. Installing one only needs the declaration, so a `with` naming an
  imported handler got that far and then read its operation bodies with the
  wrong resolutions, where every name resolved to nothing.
- A call inside an imported module reaches the function it names. A `DefId` is
  an index into one module's table, and the table of functions in the module
  being compiled was consulted for definitions from anywhere, so a recursive
  function in a library reached whatever happened to have its number in the
  program that imported it.
- `deed fix` checks a file with the modules it imports beside it. On its own,
  a call into another module has no row, so a function performing an effect
  only through such a call looked like one declaring an effect it never
  performs, and the fix on offer was to delete the row.
- `deed build` compiles every program in the corpus. It compiled seven of the
  thirty-five when #877 was opened; the rest were refused for a dozen separate
  reasons and each one is closed. What `deed run` interprets and what
  `deed build` produces are the same language now, which is the difference
  between a program you can write and a program you can hand to somebody.
- Two values that live in memory compare by what they hold. Equality is
  structural in this language and two addresses being equal is not two records
  being equal, so the backend refused rather than answering the wrong
  question. It writes a comparison per shape now: a record knows its fields, a
  choice knows its variants, a list knows what it holds, and a shape holding
  another calls that shape's comparison.
- A handler whose first operation lifts a function of its own dispatches the
  rest of its operations to the right bodies. Each operation was told where it
  would be before any of them was lowered, and a body that added a function
  after itself moved every operation after it, so `Queue.more()` reached
  whatever the operation before it had lifted.
- A field with no representation is not read. `ok(nothing)` on a call that
  answers with `()` bound a name to a word that was never a value, and left it
  behind on the stack.
- An empty list's element stands in as a number rather than as `()`. A `()`
  takes no room, so a walk over a list of them counted elements of nothing and
  the function disagreed with itself about how much of the stack it had.
- A pattern that reaches one level in compiles.
  `err(OverLimit { limit: reached })` names a field of the record the failure
  holds, and what it reads is the same read twice over.
- A generic function whose type parameter appears only inside a type somebody
  declared, or only inside an alias for one, compiles. `Option<Int>` and
  `Option<String>` become two layouts and neither says what it holds, so what
  the parameter stood for cannot be read off the value the way it can off a
  `List`. What the checker recorded says it, and an alias is followed to
  whatever it is written over.
- A type a module borrowed brings whatever it is written over with it.
  `use std/table.{Table}` is enough to need `Entry`, which nobody writes down
  on the borrowing side and the backend had no way to find.
- A `for` whose accumulator only the surrounding type says the shape of is
  checked against that shape. `with seen = Nothing` says nothing about what
  the choice holds, and the `while` above the body reads the accumulator
  before anything has settled it, so every read of it was a value the compiler
  knew nothing about and the backend could not compile.
- A record, a choice, an alias, an effect or a handler declared in one module
  and used in another compiles. A type crosses a boundary the same way a
  function does, and what it comes out as is what the module that declared it
  built, so a value of one record fits everywhere it is named rather than
  fitting one layout in one file and another elsewhere.
- An `if` or a `match` written where a type is expected of it compiles.
  The checker works out what one comes to and did not write it down, because
  the pass that records a type per expression is the one that infers rather
  than the one that checks against something. Five more corpus programs
  compile, and every one of them was a `match` arm or a branch whose value was
  a branch.
- `deed build` compiles a call into another module. The lowering was handed one
  file and the interpreter was handed all of them, so anything with a `use` in
  it was refused, which is most programs that do real work. What is lowered is
  what is reached: a module that ships thirty functions and is imported for one
  contributes one, along with whatever that one calls, and a contract on the
  other side is checked here because the call that could break it was answered
  for here.
- A declared function written where a value belongs compiles. `map(step, xs)`
  with `step` a function rather than a closure used to be refused, because a
  call through a value passes an environment and a call by name does not, so
  the value points at a wrapper that takes the environment and calls the real
  one. The contract stays on the function it belongs to.
- `perform` compiles in a module that installs no handler for the effect. The
  shape of the call was interned from whatever bodies happened to be nearby,
  and a handler installed only inside a test block is not one of them, so a
  program that performed an effect it did not also handle was refused.
- `deed build` compiles `?`. A function that propagates a failure used to be
  refused with "this expression is not lowered yet", which is four of the
  thirty-four programs in the corpus and most of the ones that do any real
  work with `Result`.
- A refinement the checker could not prove is checked in the compiled program
  as well as the interpreted one. `deed check` said "so it becomes a runtime
  check" and `deed build` emitted nothing, so a value that violated its own
  type went through. Both engines stop on the same code, in the same place,
  with the same sentence.
- A `where` clause an `assert refuses` is aiming at keeps its runtime check in
  the compiled program. The check is dropped when every recorded call proved
  the clause, and a call written to break it was recorded nowhere, so the one
  caller that needed the check was the one caller nothing knew about.
- The compiled backend has all thirteen prelude functions. `at`, `push`,
  `repeat`, `split`, `join`, `trim`, `upper`, `lower`, `to_string` and `to_int`
  used to exist only in the interpreter, so a program that called one ran under
  `deed run` and refused under `deed build`. Each is now written in
  WebAssembly directly and emitted only when reached. Thirteen of the
  thirty-four programs in the corpus compile, up from seven, and every shape
  that started compiling is answered the same way by both engines first.
- The compiled backend joins two strings, compares them and orders them.
  `deed build` used to refuse any program that put two pieces of text together
  or asked which came first, which is most programs that touch text at all.
  These are functions the backend writes into the module and calls, numbered
  after everything the program declares, and only the ones a program reaches
  are emitted.
- `deed check` no longer panics on a function whose parameter list never
  closes. A declaration used to end at the token sitting where its closing
  token should have been, so a signature could reach past the start of its own
  body, and `deed fix` builds the region a `uses` clause goes in by subtracting
  one from the other. Every construct now ends where the parser actually read
  to. Found by the nightly fuzzer.
- The scheduled fuzz run reports what it finds as an issue rather than as a
  pull request. Opening a pull request from a workflow needs a repository
  setting that also lets workflows approve pull requests, and the first run
  that found anything got told so and said nothing to anybody. The branch is
  pushed either way, so the issue names it.

### Measurements

- A walk that only pushes builds one list. `for` is the only loop and a `for`
  is a fold, so the lists a walk builds on the way exist only as values of its
  accumulator: when every mention of that name is `push`'s first argument or a
  branch handing it on, and the value of every path is the accumulator or one
  `push` onto it, nothing can reach an intermediate list and the walk builds a
  single one at the length of the list it walks. Building a list of 256 used to
  allocate 129 times what the answer is worth and now allocates the answer, and
  at 1024 the answer was always eight kilobytes while building it exhausted a
  megabyte, so what changed there is not that it got cheaper but that it runs.
  This is not reuse analysis and does not stand in for it: it asks nothing
  about whether a value is unshared, it arranges for there to be nothing to
  share. `std/table` is untouched, because its cost is across calls to `set`
  rather than inside a walk.
- Most of what a compiled program wastes is one shape. `for` is the only loop
  and a `for` is a fold, so the lists a walk builds on the way exist only as
  values of its accumulator, and whether any of them can be observed is a
  question about what the body does with that one name. Counted over the
  shipped library and the corpus: 44 walks mention their accumulator only as
  `push`'s first argument or as the value of a branch handing it on, against
  34 of every other shape. Those 44 have nothing holding an intermediate list,
  so there is no reason for them to be separate lists.
  `design/decisions/2026-08-04-a-walk-that-only-pushes.md` proposes what to do
  about it; nothing is written yet.
- Installing a handler is free. A handler's state is reserved from the frame
  stack now rather than the value heap, so it is given back when the block
  ends the way the frame already was, and a walk that installs one every turn
  allocates exactly what the same walk without one does. What made it safe is
  the rule that made the frame safe: nothing in a program can hold the state
  itself, since `DEED4030` refuses a closure over it and an operation hands
  back the value in a field rather than the block holding it.
- A walk over numbers allocates a word a turn on its own, and nothing in it is
  a value that lives in memory. Found while measuring the line above, which
  had been credited to the handler.
- What a compiled program allocates is what its memory reached, because
  nothing gives any of it back except a handler frame. So the number worth
  having is not the total but how much of the total is still worth anything,
  and building a list by folding allocates the whole answer once per element:
  129 times over at 256, and at 1024 the answer is eight kilobytes and
  building it exhausts a megabyte. Every one of those copies dies the moment
  the next is made and nothing else points at it, which is the case reuse
  analysis answers and a collector would only clean up afterwards. It also
  answers the first open question of the reclamation decision, which asked
  what workload would make the limit unacceptable: a keyed structure of a few
  hundred entries.
- The tree-versus-table crossover, compiled. The decision in
  `design/decisions/2026-07-31-tree-vs-table-decision.md` was measured on the
  interpreter and predicted that a compiled backend would move the crossover
  toward smaller N without reversing it. Both halves held: `std/map` is ahead
  of `std/table` from sixteen keys for lookup and sixty-four for insert, where
  interpreted it took a few hundred, and neither growth rate changed. The
  compiled half is counted in instructions rather than seconds, because what
  runs a module here is an interpreter over the instructions the compiler
  emits: its clock is a fact about that runner, its instruction count is a
  fact about the compiled program, and the second one is the same on every
  machine.
- A compiled program cannot build a thousand-key structure. It gets one
  megabyte and a handler frame is the only thing it gives back, so the list
  runs out of memory copying itself and the tree runs out two hundred inserts
  later. Above a few hundred keys the module a program picks is not what stops
  it; value reclamation is.

## 0.2.3 (2026-08-01)

Most of this release was found by pointing three models at the compiler
through `deed mcp` and reading what they were told. A diagnostic that names a
mistake and then throws the rest of the file away costs the reader more than
the mistake did, and nine of these turned out to be that. The one that is not
about wording is the checker: it could not use a `where` clause about a sum,
so it answered Guarded to a function whose precondition was word for word its
own obligation, and the transcript shows a model taking that answer and
writing a worse contract because of it.

### Programs that used to compile and no longer do

- Naming a generic prelude function instead of calling it is refused. Five
  names work on any type, so there is no one signature to hand back, and they
  were typed `Unknown` instead. `Unknown` absorbs, so `at == n` compared clean
  against an `Int` and reached an interpreter that had no value for it either.
  DEED4019 now, with a note saying to call it.
- `with H { ... }` is checked where the handler is installed. The form with
  braces was already checked as a literal; the form without one parsed as the
  handler followed by a block, so missing state waited for the interpreter and
  arrived as DEED6006 attached to whichever test ran first. Whether a value
  was written is a fact about the source, so it is answered there.

### Language

- A `where` clause about two names added together is a fact the checker keeps.
  It held a range per name and a range for the difference of a pair, because
  `low < high` is about neither name on its own, and a sum had nowhere to go.
  So `where count + delivered > 0` was read and dropped, and returning
  `count + delivered` into a `value > 0` refinement came back Guarded while
  the strictly stronger `count > 0, delivered > 0` proved. Two names and only
  two: three is a shape this does not hold, and Guarded stays the answer.
- A handler frame is given back when its block ends. Frames were never freed,
  so a `with` inside a walk allocated once per turn and a program installing a
  handler in an ordinary loop ran out of linear memory. A frame's lifetime is
  exactly its `with` block, which is what `with` means rather than something
  inferred, so they get their own region and the block rewinds it. Values do
  not follow: a block's value outlives the block.

### Diagnostics

- A list written one to a line with no commas says so, once per comma, and
  goes on reading the list. Match arms used to cost nine diagnostics and none
  of them said comma; a `choice` said "insert `}`", which is an answer to a
  question nobody asked and a repair that would have made it worse. DEED2015,
  with three sentences, one each for arms, variants and fields.
- `->` where `=>` belongs is named rather than expected. DEED2016, with the
  right arrow offered and the arm read as an arm afterwards.
- An `ensures` clause that names no outcome says so once. The condition
  standing where `ok =>` belongs used to cost two diagnostics, the second one
  about the `=>` that never came.
- A constraint written on a parameter says which clause it belongs in, and the
  expression is read into that clause rather than thrown away, so the names in
  it resolve. DEED2017. A `type` carries its refinement inline, so writing the
  same thing on a parameter is a fair guess.
- `xs ++ ys` and `x :: xs` are named, with the call from `std/list` that does
  it and which argument the list is. DEED2018.
- A value on a handler's state says where the value goes instead of ending the
  handler. DEED2019, and the operations after it are still operations.
- `cannot find X in this scope` names the shipped module that declares the
  name and writes the `use`. `fold` is in `std/list`, compiled into the binary
  that just said it could not find it. A name two shipped modules declare gets
  the sentence and no repair.
- Assigning to a name inside a `for` says what to write. DEED4015 said handler
  state is the only mutable thing here, which is right and does not say what
  the shape is, so it now spells the accumulator out with the reader's own
  names.
- A handler's `state` declaration says what one looks like, both halves of it,
  rather than naming the token it wanted.
- An unhandled effect that reached `main` says it is the program's boundary.
  `deed run` used to advise a `with` block, which would discharge the effect
  and take the import back out of the component's world.
- A guarded obligation says why it is guarded, including when nothing tried to
  prove it, which used to be said by saying nothing at all.

### Standard library

- `std/date`, a calendar. `date_of` refuses a clock set before 1970 rather
  than answering, `is_leap_year` and `days_in_month` for a caller walking a
  year, and `text` as `YYYY-MM-DD`, ordered so that sorting the text sorts the
  dates.
- `std/ratio`, exact fractions, as a library rather than a number type.
- `std/list` gains `sum` and `largest`. `fold`'s own comment said a library
  with `map` and `filter` and no `fold` stops working the moment somebody
  wants a sum, and then the file did not have the sum. `largest` takes the
  comparator `sort` takes, so one function covers both ends.

### Tools

- `deed doc <path>...` writes the API page any shipped module gets, for any
  module, as Markdown on standard output. The generator lived in a test file.
- `deed mcp`, a server that hands an agent the compiler's six verbs. Its
  handshake names every tool it offers and says why checking is not the last
  step.
- The wasm artifact runs the tests a contract generates, not only the ones a
  person wrote, and reports each property with its seed. It also refuses a
  module that does not check, and ends a test run with a summary line, because
  silence already meant "well formed" on that surface and could not also mean
  "nothing ran".
- The wasm artifact says why there is nothing to run. It answered "no `main`
  found" where the CLI says "no `main` found, so there is nothing to run", and
  most of the corpus is libraries, so that is the answer a reader is most
  likely to meet first. One constant now, used by both.

### Measurements

- The edit loop was re-measured. 82us a file and 42ms at 512 files, against
  the 59us and 30.1ms the design document stated, so the trigger for writing
  an incremental cache is about 1,200 files rather than about 1,700. The
  conclusion does not change and a third of the claimed headroom was not
  there. The arithmetic on top of the table is now a test.
- Something other than the author writes Deed, and it is measured.
  `benchmarks/` runs six tasks through a model twice, once with the compiler
  and once without. Without it, three models answered 0 of 6 across every run.
- The conformance suite has twenty-eight cases in three groups rather than
  four, which was an example of a format rather than a suite.

## 0.2.2 (2026-07-31)

Nothing about the language moved. What changed is what `deed explain` prints
and what the WebAssembly artifact will answer, both of them for the benefit of
something outside this repository reading them.

### Programs that used to compile and no longer do

- None.

### Language

- None.

### Diagnostics

- `deed explain` prints Deed. Thirty of the ninety-seven pages showed a Rust
  `format!` template instead of a program, doubled braces and placeholders and
  all, because the example is copied out of a test and a test is Rust. Every
  page that had an example still has one.

### Standard library

- None.

### Tools

- The wasm artifact answers two more questions. `deed_tokens` says how the
  compiler's own lexer classified each byte range, so a page can colour Deed
  without a second grammar to keep in step, and `deed_explain` hands over
  every diagnostic code with its page, so a site can host an error index
  rather than write one.

### Measurements

- The wasm artifact is 858,097 bytes uncompressed and 280,748 gzipped, up
  from 816,258 and 265,841, inside the same ceiling.

## 0.2.1 (2026-07-31)

One fix, and it is the whole release: 0.2.0's WebAssembly artifact did not
work. Nothing else changed, so there is nothing here to be careful about.

### Programs that used to compile and no longer do

- None.

### Language

- None.

### Diagnostics

- None.

### Standard library

- None.

### Tools

- The wasm artifact answers instead of trapping. Every verb went through
  `check_all`, which read a clock, and `wasm32-unknown-unknown` has none, so
  0.2.0's artifact aborted on any input at all. CI now runs the artifact
  rather than only building and weighing it.

### Measurements

- None.

## 0.2.0 (2026-07-31)

Written after the fact rather than as the changes landed, which is the thing
this file asks for and did not get: 231 commits reached `main` between 0.1.0
and here, and one of them touched this file. That one added the policy.
Reconstructed from the merged work rather than from memory, and the counts in
it are measured.

### Programs that used to compile and no longer do

- A handler must implement every operation of the effect it names, or none of
  them. Half a handler used to check cleanly and then fail at run time.
- A contract may only reach an effect its signature already mentions. Reaching
  further used to check cleanly and then die at run time, inside the callee
  that had declared everything correctly.
- A closure written over a handler's `state` is refused. It used to compile,
  and read a different handler's state than the one it was written under.
- A task cannot outlive the block that started it.
- Two files claiming the same module path is an error, reported on both.
- A name imported into a `uses` clause that is not an effect is an error
  rather than a warning. As a warning it also silenced the rest of that
  function's row checking.

### Language

- Effect handlers are one-shot, and a resumption is a one-shot value. That is
  what decides the dispatch rather than the other way round.
- `finally` on a handler, for the cleanup a block owes when it is left.
- `abandon`, which unwinds a suspended computation and runs the `finally`
  clauses on the way out.
- The generator pattern, which turns out to be a `Yield` effect and no new
  syntax at all.
- Alternative patterns in a match arm. They bind nothing, which is what keeps
  them free.
- An alias may name a shape with a hole in it, and it expands on its way out
  of a module, however many modules it came from.
- `String` length is Unicode scalar values, stated rather than implied.
- Identifiers are UTF-8, with the grammar written down.

### Diagnostics

- 73 codes to 85. New: `AMBIGUOUS_MODULE`, `NO_DETACHED_SPAWN`, `ABANDONED`,
  `CLOSURE_OVER_STATE`, `HANDLER_MISSING_OPERATION`,
  `CONTRACT_EFFECT_NOT_DECLARED`, `BINDING_IN_AN_ALTERNATIVE`,
  `REFINEMENT_TYPE_PARAM`, `DEPRECATED_DECLARATION`, `UNKNOWN_EDITION`,
  `UNKNOWN_DIRECTIVE`, `MISSING_COMPONENT_PATH`. None removed or renumbered.
- Every message the lexer, parser, resolver, effect checker, type checker and
  interpreter can produce was read back in a test. A code having one test does
  not mean its sentences have been read: several codes carried messages that
  nothing had ever rendered, and two of those carried repairs that made the
  program worse.
- A secondary label can name a file of its own, so a diagnostic about two
  modules stops drawing its caret into the wrong one.

### Standard library

- `std/list` and `std/table` ship with the compiler rather than sitting in
  `examples/`.
- `std/map`, a red-black tree, for the case an ordered table was answering
  badly.
- `std/list` gains `find`, `take`, `drop`, `concat`, `flatten`, `partition`,
  `zip`, `unzip`, `enumerate`, `windows`, `chunks`, `intersperse`, `scan`,
  `transpose`, `group_by`, `sort`, `flat_map`, `filter_at` and `fold_at`.
- `std/string` gains `contains`, `replace`, `trim_start`, `trim_end`,
  `to_upper` and `to_lower`.
- `std/table` gains `remove`.

### Tools

- `deed build` writes a WebAssembly module beside the file it was given, and
  `deed build --component` writes a component whose WIT world is derived from
  the program's effect rows rather than written by hand beside them.
- `deed run --compiled`, which maps a trap back to the span that caused it.
- `deed run --profile-runtime`, attributing cost to functions, contracts and
  handlers separately.
- `--lock` and `--locked`, for builds that are offline and say what they used.
- A wasm entry point a page can call: `check`, `run`, `test` and `fmt` over
  linear memory, with the CLI's own JSON and no binding generator.
- The language server gains document highlight, folding ranges, inlay hints,
  go to implementation, type definition, document links and selection ranges,
  and the VS Code extension now starts it.
- A tree-sitter grammar, which is what Helix and Neovim colour with.
- An external conformance suite and a harness that runs it.
- Fuzzing: a corpus replayed on every build, and a scheduled run that adds to
  it.

### Measurements

- The wasm artifact is 816,258 bytes, 265,841 gzipped, against a ceiling of
  1,500,000 and 550,000.
- A handler operation costs roughly 128ns over a plain call when the handler
  holds no state, 142ns when it reads some. Not enough to justify a
  tail-resuming fast path before a compiled backend can measure its own.
- `std/map` against `std/table`, benchmarked, with the decision recorded.
- Compiled memory growth, and the slopes of the edit loop and the runtime, are
  both ratcheted in CI rather than hoped for.

## 0.1.0 (2026-07-27)

Initial public release.

### Programs that used to compile and no longer do

- Not applicable in the initial public release.

### Language

- Initial public release of the language and compiler.

### Diagnostics

- Initial public release of the compiler's user-facing diagnostics.

### Standard library

- Initial public release of the shipped `std/` modules.

### Tools

- Initial public release of the `deed` CLI and editor tooling.

### Measurements

- The generated `v0.1.0` release notes included measurement work; future
  releases record the measurements that changed a decision here.
