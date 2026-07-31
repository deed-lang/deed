# One clause. Two worlds.

Two Deed functions, identical except for one line in the signature.

```
$ cat demo/read_only.deed
```

```deed
module demo/read_only

fn process(files: Dir, name: String) -> Result<String, String>
  uses
    Io.read,
{
    Io.read(files, name)
}
```

```
$ cat demo/read_write.deed
```

```deed
module demo/read_write

fn process(files: Dir, name: String) -> Result<String, String>
  uses
    Io.read,
    Io.save,
{
    match Io.read(files, name) {
        err(why) => err(why),
        ok(text) => match Io.save(files, name, text) {
            err(why) => err(why),
            ok(_) => ok(text),
        },
    }
}
```

The difference is `Io.save` in the `uses` clause.

## The two worlds

Build both:

```
$ deed build demo/read_only.deed demo/read_write.deed
demo/read_only.wasm
demo/read_write.wasm
```

The import section of each module is its world. It lists what the module
asks the host for before a single instruction runs.

```
read_only.wasm imports:
  deed:io  read

read_write.wasm imports:
  deed:io  read
  deed:io  save
```

One extra clause in the signature. One extra line in the import section.

## The host's behaviour

The host inspects the import section before running anything. A host that
provides `Io.read` but not `Io.save` can run `read_only` without
restriction. It refuses `read_write` because the module declared a need the
host cannot meet.

```
$ deed run demo/read_only.deed --dir .
(reads files, does not write)

$ deed run demo/read_write.deed --dir .
error: deed:io.save is not available
```

The refusal happens before any code runs. The module said what it needed.
The host read that. No policy check, no runtime guard, no separate
permission layer. The signature is the permission.

## Verifying it

The test in `crates/deed-driver/tests/demo.rs` pins this mechanically:

```
$ cargo test -p deed-driver --test demo
running 4 tests
test read_only_imports_read_and_nothing_else ... ok
test read_write_imports_read_and_save ... ok
test running_read_only_without_a_host_names_read ... ok
test running_read_write_without_save_names_what_it_wanted ... ok
```

The import sections and the host refusal are not checked by eye. They are
checked on every commit.
