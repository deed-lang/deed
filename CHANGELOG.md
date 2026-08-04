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

- `std/ratio` binds `<` to `is_below`, so two ratios compare the way two `Int`s
  do. `is_below` and `is_above` are unchanged and still exported, because a
  comparison is what `sort` takes.

### Tools

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

### Measurements

- `partition` over a list of 256, with the list it walks subtracted off,
  allocated 267304 bytes and now allocates 8224. Over 1024 it ran out of
  memory and now allocates 32800.
- The counts in `design/decisions/2026-08-04-a-walk-that-only-pushes.md` said
  forty-four walks of the shape against thirty-four of every other. The rule
  that shipped answered forty and thirty-eight. The record now says what the
  rule says, and a test holds it there.

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
