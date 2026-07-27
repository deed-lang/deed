# Capabilities

Effects say *what kind* of thing a function does. Capabilities say *which particular
resource* it is allowed to do it to.

`uses Net.send` means a function can send over the network. It does not say where. A
capability is the value that answers that, and it can only be obtained by being handed one.

That sentence, and the `Fs` and `Net` effects in the illustrations further down, are the
shape of the argument rather than the state of the compiler. What exists is one built-in
effect, `Io`, with `write`, `now`, `epoch`, `open`, `read`, `save`, `remove`, `make`, `list`
and `args`, and a `System` carrying `console`,
`clock` and `files`. The next section is the part that runs.

## What actually exists

Enough to run a program, and not much more. `deed run` calls `main`, hands it the one
`System` there is, and everything below it holds whatever it was passed.

```deed
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

`System` carries `console`, `clock` and `files`. All three are opaque: there is no field to
read, no constructor, nothing to know about one except that you were handed it. `Io.write`
takes the `Console` it writes to and `Io.now` takes the `Clock` it reads, so the operation
cannot be performed without naming the thing it acts on.

The parts that are real:

- `greet` cannot read a clock. It holds no `Clock`, and there is nowhere to get one.
- Passing a `Clock` where a `Console` is wanted is a type error, not a runtime surprise.
- Holding a `Console` is not enough on its own. `uses Io.write` still has to be there, and
  declaring it without performing it is also an error.
- Writing `Io.write(Console, "hi")` does not work. `Console` is a type, and a type in
  expression position is `DEED4019`, not a value.

That last one is the whole thing in one rule, and it was not true when this was first
written. The type checker gave a type name in expression position no type at all, and a
type-less expression is compatible with everything, so authority could be conjured by
spelling it. The test that was supposed to prove capabilities work found it.

`Dir` is real too, and it is the interesting one, so it gets its own section below.

## No ambient authority

In every mainstream language, any code can do anything the process can do. `import os` and
you have the filesystem. A logging library, four levels deep in your dependency tree, has
the same authority as your own code.

This is the root of software supply chain attacks, and it is not a vulnerability in any
particular package. It is the default that every language chose.

Deed has no ambient authority. There is no global `File.open`, no importable network module,
no `System.getenv`. A module gets exactly what it was passed.

```deed
fn read_config(fs: Dir, name: String) -> Result<Config, ConfigError>
  uses Fs.read
{
    let text = fs.read(name)?
    Config.parse(text)
}
```

`fs: Dir` is a capability, and it is scoped. Given a `Dir` rooted at `/etc/myapp`, this
function cannot read `/etc/shadow`. Not because it is forbidden by a policy engine but
because it has no way to name that path.

## Authority enters at `main` and only there

```deed
fn main(sys: System) -> Result<(), Error>
  uses sys.*
{
    let config_dir = sys.fs.open_dir("/etc/myapp")?
    let db = sys.net.connect(DATABASE_HOST)?

    let config = read_config(config_dir, "server.toml")?
    let ledger = Ledger.postgres(db)

    payments.serve(ledger, sys.clock, config)
}
```

`System` is the root of all authority and the runtime supplies exactly one, to `main`.
Everything after that is delegation, and delegation only ever narrows.

Reading a `main` like the one above tells you the entire attack surface of the program.
There is nothing else it could touch, because there is nowhere else authority could come
from.

## Attenuation

Capabilities narrow as they are passed down.

```deed
fn describe(files: Dir, name: String) -> String
  uses
    Io.read,
{
    match Io.read(files, name) {
        ok(text) => "found it",
        err(why) => why,
    }
}
```

`describe` reads inside whatever directory it was handed and has no way to tell which one
that is. Hand it `sys.files` and it reads the working directory. Hand it the result of
`Io.open(sys.files, "cache")` and it reads `cache`, and there is no argument it could be
given and no string it could build that would get it back out.

That holds because narrowing is the only operation. `Io.open` goes down, nothing goes up,
and a `Dir` carries a canonical path the program can neither read nor compare.

### Reading, writing, listing, deleting and making are different authorities over the same capability
`Io.save(files, name, contents)` writes a file, and it takes the same `Dir` `Io.read` does.
What separates them is not the type but the row: a function that declares `uses Io.read` and
is handed `sys.files` cannot write to it, because performing an operation it did not declare
is an error. So there are two authorities over one capability, and which one a caller is
handing over is written in the signature it is handing it to.

`Io.list(files)` is the third, and it is the one that tests the claim rather than
illustrating it. Reading means: you may read the file I told you about. Listing means: you
may find out what is here. That is strictly more, and in the shapes people actually write it
is a lot more, because a directory handed over for one purpose usually has other things in
it. It needed nothing new: it is a third entry in the row over the same `Dir`, and a function
that did not declare it cannot enumerate whatever it happens to be holding.

It takes no name, which is the whole operation, so there is nothing for the sandbox to refuse
and nothing to refuse it with. Authority still only shrinks on the way down: listing a `Dir`
that came out of `Io.open` sees inside that one and nothing above it.

The answer is files only, and sorted. Sorted because a caller that depends on the order the
filesystem felt like today is a caller with a bug that appears on somebody else's machine.
Files only because a list holding two kinds of thing with no way to tell them apart is the
sort of thing that turns into a bug in the caller, and `Io.open` already needs a name from
somewhere.

That is the split this document keeps claiming: the row says what kind of operation, the
argument says which resource. It is also why there is no separate write capability type. A
second type would say the same thing twice and would have to be threaded through every
signature that already carries a `Dir`.

`Io.remove(files, name)` is the fourth, and it is the one where a second type was most
tempting. Reading, listing and writing all leave what was there. Deleting does not, and the
difference is not one of degree: a program that writes the wrong bytes can be put back from
what it overwrote, and one that deletes the wrong file cannot be put back from anything. So
the obvious move is a `Dir` that may destroy and a `Dir` that may not.

It is the same move that was rejected for writing, and it is rejected here for the same
reason and with the same result: `uses Io.remove` cannot be reached from `uses Io.save`,
holding a `Dir` says nothing about which of the four a function may do, and nothing new had
to be built. That is three tests of the claim now, and the third was the one designed to
break it.

Files only, like listing. Removing a directory is a different operation with a different
blast radius and nothing written here has wanted it. A name that is not there comes back as
`err` rather than as a success, because "it was already gone" and "I removed it" are
different answers and a program that cannot tell them apart has a bug waiting. It goes
through the same name check as everything else, so a symlink pointing out of the directory is
refused rather than followed out of it.

`Io.make(files, name)` is the fifth, and it is the one that looks like it breaks the rule
above. It hands back a `Dir`, and a `Dir` is authority, so it reads like authority being made
rather than narrowed. This document said exactly that for as long as the operation was
missing, and it was wrong.

Work out what a caller holds. A `Dir` reaches everything under its root, and which of those
paths happen to exist is not part of what it grants: `Io.save` already writes files that were
not there a moment ago and nobody calls that authority creation. `Io.make(files, "notes")`
hands back a `Dir` rooted at `<root>/notes`, which is inside the one that went in. The set of
reachable paths did not grow; the set of things that exist inside a place the caller could
already reach did. Narrowing is still the only operation, and there is a test that makes a
directory and then fails to climb out of the result.

Nothing may already be at the name, file or directory. "I made it" and "it was already
there" are different answers, which is the same reason a missing file is an error for
removing. It is not recursive and there is no "make it if it is not there", because both hide
which of the two answers you got.

By that argument `Io.save` and `Io.write` should be one name, since writing to a console and
writing to a file are the same kind of operation on different resources. They are two names
because a signature is one list of types per name and there is no overloading. That is a
limitation showing through the design rather than a decision, and it is worth writing down
as one.

Writing goes through the same name check reading does, in the same function, so `..`, an
absolute path, a separator and a symlink pointing out are refused for writing exactly as
they are for reading. The one difference is that a file being written does not have to exist
yet, which means the check cannot lean on canonicalising an existing path and has to be
correct by construction instead: the name has already been established to be one ordinary
component, and the root is canonical, so there is no traversal left and nothing to follow.

### The part that has to be right

Refusing `..` is the obvious half and the useless half on its own. The rules, in
`crates/deed-interp/src/sandbox.rs`:

- a name is one component, so no separator of either flavour
- `.` and `..` are refused by name, and so is the empty string
- anything absolute, or carrying a drive prefix, is refused
- the result is canonicalized and checked to still be under the root

The last one is what decides. A check that refuses `../etc` textually and then follows a
symlink to `/etc` is not a check, it is a message. Canonicalizing first and comparing after
is the only version that survives `ln -s`, and there is a test that makes the symlink and
asserts the escape fails.

Backslashes are refused textually as well as structurally, because on Unix a backslash is an
ordinary character in a filename and `Path` would call `a\b` one component. A name that
means two different things on two platforms is not something to hand to a security check.

### What a refusal is

`Io.open` and `Io.read` return `Result<_, String>`, and every refusal above comes back as
`err` naming the rule it hit. That is deliberate: the name usually comes from data, and a
path that arrived in a request is not a bug in the program that received it. It does mean a
program can write `Io.read(files, "..")` and get a runtime answer to a question that could
have been settled earlier, which is a real cost and the alternative was worse. `Io.save`,
`Io.remove` and `Io.make` answer the same way, for the same reason.

The error being a `String` is a placeholder. A real `IoError` in the prelude is its own
argument about how much of the P2 budget capabilities get to spend.

### Still missing

`read_only()`. It is less obviously needed than it looks, because a function that cannot
write is a function that did not declare `Io.save`, and that is a row rather than a type. It
starts to matter the moment a row can be a wildcard, and `sys.*` already is one.

Recursive listing is not there, and does not need to be: it is a third amount of authority
that a caller can build out of `Io.list` and `Io.open` if it declares both, which is the
model working.

`examples/journal.deed` is the file to argue with. It reads a journal, appends a line and
saves it, and everything it is refused is refused in front of you.

### Where the root comes from

`deed run --dir <path>` decides, defaulting to the working directory. The runtime does not
pick one on its own: a program given no directory gets no `sys.files` rather than a quiet
fallback to wherever the process happens to be. Defaulting to the working directory at all
is not obviously right, and the reason it is not `--dir` or nothing is that a flag people
have to type every time is a flag people type without reading.

Arguments arrive the same way. Everything after `--` on the command line goes to the program
and nothing else does, so the runtime never reads its own invocation on the program's behalf.

### Arguments are input, not authority

`Io.args(sys)` hands back a `List<String>`. It is the odd operation of the ten: it does
nothing, it returns data rather than something opaque, and it takes the whole `System` rather
than a narrower capability. It reads like it does not belong.

It is in the row anyway, for a reason that is not about authority. How a program was invoked
is input from outside, every other way of getting input from outside says so in a signature,
and a program that behaves differently depending on its arguments should not be able to hide
that in a body. `uses Io.args` in a row is a claim worth being able to read.

Taking the root capability rather than a narrower one is deliberate too, and it is the part
that keeps the shape honest. Only code that was handed everything can read the arguments, so
they are read near `main` and passed down as ordinary values. A function three levels in that
wanted them would have to be handed a `System`, which its own signature would then admit to.

## What this changes

**Dependency risk becomes visible and bounded.** A left-pad style package that takes no
capability arguments cannot exfiltrate anything, no matter what its code says. It is not a
matter of trusting the maintainer, or of scanning for known bad packages. It has no reach.
The interesting review question stops being "what does this dependency do" and becomes "what
did we hand it", which is a question you can answer by reading one line.

**Generated code can run without a container.** If the untrusted part is a function whose
signature is `uses Ledger.balance` and whose parameters carry no capabilities, running it is
safe by construction. That matters when the loop is write, run, check, thousands of times,
and a container boot is a full second of it.

**Least privilege is the default rather than an audit finding.** You have to actively pass
authority for code to have any. Most functions end up with none, and that is not discipline,
it is just what happens.

## Relationship to effects

They are two halves of one statement and neither is sufficient alone.

| | answers | mechanism |
| --- | --- | --- |
| Effect row | what kind of operation | static, in the signature |
| Capability | which resource | dynamic, a value passed in |

`uses Fs.read` with a `Dir` parameter reads as: this function reads files, and only inside
that directory. Either half without the other leaves a hole. Effects alone cannot say
*which* file. Capabilities alone cannot be checked without running the program.

## Open questions

The list is longer than I would like, and this is the least settled document here.

- **`uses sys.*` in `main` is doing a lot of work and may be a cheat.** If it becomes the
  idiom, authority stops being narrow at exactly the place the argument depends on. The
  compiler now warns when a row grants everything a capability carries, on the grounds that
  granting everything is the same as promising nothing, and that saying so is better than
  reporting a clean check it never performed. The warning is not a fix.
- **Ergonomics.** Threading capabilities through deep call stacks is exactly the argument
  that killed explicit dependency passing before, and "just use a container" won that
  argument for a reason. Implicit parameters would fix the plumbing and would put a hole
  straight through P1.
- **Interop.** The moment Deed calls C or WASM, ambient authority comes back with it. A
  foreign function can do whatever the host process can. This is currently unsolved and it
  is the most likely place the whole model leaks.
- **Serialization.** A capability must not be forgeable, so it cannot be a plain value that
  round-trips through a byte stream. What that means for sending work between machines is
  not worked out.
- **Time and memory are also resources.** Nothing here bounds them. A function with an empty
  row can still allocate forever.
- **The clock is a lie, and now it says which one.** `Io.now` counts calls rather than
  reading a wall clock, because P8 says the default is deterministic and a real clock would
  make every run different. That is the right default for testing and the wrong answer for a
  program that needs the actual time, so `Io.epoch` reads the machine's clock and is a
  separate entry in the row. Holding a `Clock` says nothing about which of the two a function
  may do, which is the same split `read` and `save` get about the same `Dir`, applied to
  something other than authority: a signature saying `uses Io.epoch` is a function whose
  output can change between two runs of the same program. What is still missing is anything
  above milliseconds since 1970. A calendar is a library rather than a capability, and the
  language has no way to write one yet.

## Prior art worth reading

Capability security is old and mostly ignored, which is a bad sign for adoption and a good
sign for the idea being underexplored rather than wrong.

- KeyKOS and EROS, capability operating systems
- The E language, where object references are capabilities
- Joe-E and Caja, capability-safe subsets of Java and JavaScript
- Deno, process-level permissions, which is the same idea at much coarser granularity
- The WASM component model, the closest thing to a mainstream capability boundary today
