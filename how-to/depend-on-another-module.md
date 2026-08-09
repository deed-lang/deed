# How do I use somebody else's Deed module?

There is no registry. A dependency is a location and a hash, and the whole of
the mechanism fits on this page.

## Naming one

A `deed.manifest` beside your source says where code that is not in your tree
lives. It has two directives and no others.

```text
# A directory that is another root for module resolution.
component ../shared

# Bytes: where they are, and what they hash to.
module https://example.com/ledger.deed sha256:ba7816bf8f01cfea414140de5dae2ec73b00361bbef0469348423f656b6a3f2c
```

A `component` line adds a place to look, searched only after the roots your
own files imply. A `module` line names bytes. What the module is *called* is
not in the manifest, because the file says that on its own `module` line, so a
location and a hash are the whole of a dependency.

The manifest cannot do anything else. It cannot remap a name, override a
module your files already imply, select features, or configure the compiler.
The format has two directives, so there is no other effect.

## What happens to the bytes

They are fetched, hashed, and refused if the hash does not match. Nothing
demotes that to a warning and nothing retries. The cache is keyed by the bytes
rather than by the URL, so two projects needing the same module store it once,
and a URL that starts serving something else is a hard failure rather than a
silent upgrade.

Nothing in a fetched module runs while it is being fetched. There are no
install hooks, no build scripts, and no post-fetch step, because every
supply-chain incident worth naming went through one.

## Getting the digest you have to write

A `module` line needs a hash you do not have yet, and there is no subcommand
that prints one. Two ways, both of which work today.

Ask the machine you are on:

```
$ sha256sum ledger.deed          # Linux
$ shasum -a 256 ledger.deed      # macOS
> Get-FileHash ledger.deed -Algorithm SHA256   # Windows
```

Or write the line with a hash you know is wrong and read the refusal, which is
the one that works when the bytes are somebody else's and are only on the far
end of a URL:

```
error[DEED7006]: `http://…/ledger.deed` answered with bytes that hash to
5e8206aa39cb8d7afa21c2fef8bbda60894e28e36aad152659e6788cf5332ef4, and the
manifest says 0000000000000000000000000000000000000000000000000000000000000000
```

It names both, so the first number is the one to paste. Nothing was stored and
nothing was compiled while it said so: bytes under a hash that is not theirs
are bytes for a different dependency.

A manifest saved with a byte order mark is read the same as one without,
which is the rule `design/06-grammar.md` gives source files and the reason
a file written by a Windows editor is not a file with a different first
directive.

## Pinning the whole build

```
$ deed build --lock deed.lock app.deed
$ deed build --locked deed.lock app.deed
```

The first writes every input with its SHA-256. The second refuses to proceed
if any of them differs. Both count local files, fetched modules, and the
modules that ship inside the compiler, because "exactly what went into this
build" is not a shorter list than that.

## There is no registry, and here is why

A registry is a name service, and a name service is a place where the meaning
of a name can change without your file changing. `sha256:` is the opposite:
the name *is* the bytes. That gives up discovery, which is the thing a
registry is actually good at, and this is worth saying plainly rather than
pretending it is free.

`design/decisions/2026-08-07-a-dependency-is-a-location-and-a-hash.md` is the
decision, including what would change it.

## A module worth depending on

Playground: [open](https://deed-lang.github.io/)

```deed dependency-ledger
module ledger

record Entry {
    who: String,
    amount: Int,
}

fn total(entries: List<Entry>) -> Int {
    for entry in entries with sum = 0 {
        sum + entry.amount
    }
}
```

Its signatures are the whole of what a caller has to read. A dependency in
this language cannot perform an effect the caller's row does not name, so
"what does this thing do" is answered by the file rather than by trust.

## What is kept green

- [`crates/deed-driver/src/manifest.rs`](../crates/deed-driver/src/manifest.rs) is
  the format, and the five ways of writing it wrong are `DEED7001` to `DEED7005`.
- [`crates/deed-fetch/src/lib.rs`](../crates/deed-fetch/src/lib.rs) is fetch,
  verify and cache, with the five properties it holds written at the top.
- [`crates/deed-cli/src/lock.rs`](../crates/deed-cli/src/lock.rs) is the lock file.
