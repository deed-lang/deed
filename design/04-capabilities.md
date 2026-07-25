# Capabilities

Effects say *what kind* of thing a function does. Capabilities say *which particular
resource* it is allowed to do it to.

`uses Net.send` means a function can send over the network. It does not say where. A
capability is the value that answers that, and it can only be obtained by being handed one.

## No ambient authority

In every mainstream language, any code can do anything the process can do. `import os` and
you have the filesystem. A logging library, four levels deep in your dependency tree, has
the same authority as your own code.

This is the root of software supply chain attacks, and it is not a vulnerability in any
particular package. It is the default that every language chose.

Vow has no ambient authority. There is no global `File.open`, no importable network module,
no `System.getenv`. A module gets exactly what it was passed.

```vow
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

```vow
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

```vow
let data = sys.fs.open_dir("/var/lib/app")?
let readonly = data.read_only()
let cache = data.open_dir("cache")?
```

Each derived capability is strictly weaker. There is no widening operation, no way to walk
back up, and `..` does not escape a `Dir`. A function that receives `cache` cannot reach
`/var/lib/app`, and no audit is required to know that.

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
- **Interop.** The moment Vow calls C or WASM, ambient authority comes back with it. A
  foreign function can do whatever the host process can. This is currently unsolved and it
  is the most likely place the whole model leaks.
- **Serialization.** A capability must not be forgeable, so it cannot be a plain value that
  round-trips through a byte stream. What that means for sending work between machines is
  not worked out.
- **Time and memory are also resources.** Nothing here bounds them. A function with an empty
  row can still allocate forever.

## Prior art worth reading

Capability security is old and mostly ignored, which is a bad sign for adoption and a good
sign for the idea being underexplored rather than wrong.

- KeyKOS and EROS, capability operating systems
- The E language, where object references are capabilities
- Joe-E and Caja, capability-safe subsets of Java and JavaScript
- Deno, process-level permissions, which is the same idea at much coarser granularity
- The WASM component model, the closest thing to a mainstream capability boundary today
