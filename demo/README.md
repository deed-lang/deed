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

There is no command to type for this, and this file used to show two. A
component has no `main` (`deed build --component` refuses programs that
declare one), so `deed run` on either of these answers ``no `main` found, so
there is nothing to run`` and stops. Running a component is a host's job, and
the host here is the runner in `crates/deed-rt`.

So the demonstration is a test, and it is written as one:

```rust
// crates/deed-driver/tests/demo.rs
let stopped = call(&module, "process", &[Value::I64(0), Value::I64(0)])
    .expect_err("read_write needs a host to provide save");

let Trap::NeedsAHost(what) = stopped else {
    panic!("it should say what it wanted, not {stopped}");
};
```

`NeedsAHost` carries the name of the operation the module asked for, and it
is reached before any instruction of `process` executes. The read-only module
stops the same way for `deed:io.read` alone.

The refusal happens before any code runs. The module said what it needed.
The host read that. No policy check, no runtime guard, no separate
permission layer. The signature is the permission.

## Verifying it

The test in `crates/deed-driver/tests/demo.rs` pins this mechanically:

```
$ cargo test -p deed-driver --test demo
running 5 tests
test read_only_imports_read_and_nothing_else ... ok
test running_read_only_without_a_host_names_read ... ok
test read_write_imports_read_and_save ... ok
test running_read_write_without_save_names_what_it_wanted ... ok
test the_readme_lists_the_imports_the_modules_actually_have ... ok
```

The import sections and the host refusal are not checked by eye. They are
checked on every commit, and so is the listing printed further up this file:
the last test reads it back out of these paragraphs and compares it to what
the modules import.
