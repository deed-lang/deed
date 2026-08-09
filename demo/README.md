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
the host here is `Host` in `crates/deed-codegen`, which `deed-rt` supplies the
behaviour for.

So the demonstration is a test, and it is written as one:

```rust
// crates/deed-driver/tests/demo.rs
let mut host = Host::new();
host.offer("deed:io", "read", |_| Ok(Some(Value::I64(0))));

let refused = host
    .link(&module_for("read_write.deed"))
    .expect_err("a host that cannot save should refuse the module that saves");

assert_eq!(refused.name, "save");
```

`Host::link` reads the whole import section and answers before an instruction
runs. The same host links `read_only` and hands it back ready to call, which is
the other half of the claim: a host that refused both would be a host with no
filesystem, and that says nothing about the clause.

Running with no host at all is a weaker thing, and this file used to show that
instead. It stops at whichever import execution reaches first, and for both of
these modules that is `deed:io.read` — the operation they share. A host that
worked that way would have read the file before noticing it could not save.

The refusal happens before any code runs. The module said what it needed.
The host read that. No policy check, no runtime guard, no separate
permission layer. The signature is the permission.

## Verifying it

The test in `crates/deed-driver/tests/demo.rs` pins this mechanically:

```
$ cargo test -p deed-driver --test demo
running 7 tests
test read_only_imports_read_and_nothing_else ... ok
test read_write_imports_read_and_save ... ok
test a_host_without_save_refuses_read_write_and_names_save ... ok
test the_same_host_links_read_only_without_restriction ... ok
test running_without_a_host_stops_at_the_first_import_either_way ... ok
test the_readme_lists_the_imports_the_modules_actually_have ... ok
test the_readme_lists_the_tests_this_file_declares ... ok
```

The import sections and the host refusal are not checked by eye. They are
checked on every commit, and so are the two listings printed further up this
file: one test reads the import listing back out of these paragraphs and
compares it to what the modules import, and another reads this list of test
names back and compares it to the tests the file declares.
