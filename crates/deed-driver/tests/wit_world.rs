//! A WIT world derived from a program's effect rows.
//!
//! The world is what a compiled Deed program asks its host for. Each effect
//! operation in `main`'s declared row becomes one `import`. The four properties
//! this file holds to:
//!
//! - A world is emitted from the rows, not authored alongside them.
//! - Handled effects are excluded; escaping ones are included.
//! - Changing the row changes the world.
//! - A function that performs nothing yields an empty world.

use deed_diagnostics::SourceMap;
use deed_driver::{check_all, check_text, wit_world_for};

fn world_for(source: &str) -> String {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.deed", source);
    assert!(
        !checked.has_errors(),
        "this program should check cleanly: {:?}",
        checked.diagnostics
    );
    wit_world_for(&checked)
}

// -- empty world -----------------------------------------------------------

#[test]
fn a_pure_main_yields_an_empty_world() {
    // The other half that makes the non-empty tests below mean something. An
    // emission that listed everything would say the same thing here as there.
    let world = world_for("module main\n\nfn main() -> Int { 1 + 1 }\n");
    assert_eq!(
        world, "world program {\n}\n",
        "a function that performs nothing should produce an empty world"
    );
}

#[test]
fn no_main_yields_an_empty_world() {
    // A library module has no entry point and asks nothing of the host.
    let world = world_for(
        "module lib\n\neffect Log {\n  fn note(message: String) -> Int\n}\n\n\
         fn helper(n: Int) -> Int uses Log.note { n + Log.note(\"x\") }\n",
    );
    assert_eq!(
        world, "world program {\n}\n",
        "a module with no main should produce an empty world"
    );
}

// -- escaping effects ------------------------------------------------------

#[test]
fn an_escaping_effect_appears_in_the_world() {
    // The basic case: main declares an effect and it appears in the world.
    let world = world_for(
        "module main\n\n\
         fn main(sys: System) -> Int\n\
           uses\n\
             Io.write,\n\
         {\n\
             Io.write(sys.console, \"hi\")\n\
             0\n\
         }\n",
    );
    assert!(
        world.contains("import deed:io.write;"),
        "an escaping Io.write should appear in the world, got:\n{world}"
    );
}

#[test]
fn multiple_escaping_effects_all_appear() {
    let world = world_for(
        "module main\n\n\
         fn main(sys: System) -> Int\n\
           uses\n\
             Io.write,\n\
             Io.now,\n\
         {\n\
             let _ = Io.now(sys.clock)\n\
             Io.write(sys.console, \"hi\")\n\
             0\n\
         }\n",
    );
    assert!(
        world.contains("import deed:io.write;"),
        "Io.write should appear, got:\n{world}"
    );
    assert!(
        world.contains("import deed:io.now;"),
        "Io.now should appear, got:\n{world}"
    );
}

#[test]
fn a_user_effect_in_the_row_appears_in_the_world() {
    // A locally declared effect that escapes main is in the world the same way
    // a builtin one is.
    let world = world_for(
        "module main\n\n\
         effect Log {\n\
           fn note(message: String) -> Int\n\
         }\n\n\
         fn main(n: Int) -> Int\n\
           uses\n\
             Log.note,\n\
         {\n\
             n + Log.note(\"hi\")\n\
         }\n",
    );
    assert!(
        world.contains("import deed:log.note;"),
        "an escaping user-defined effect should appear in the world, got:\n{world}"
    );
}

#[test]
fn an_imported_effect_in_the_row_appears_in_the_world() {
    let mut sources = SourceMap::new();
    let effect = sources.add(
        "effect.deed".to_string(),
        "module dep\n\neffect Log {\n  fn note(message: String) -> Int\n}\n".to_string(),
    );
    let main = sources.add(
        "main.deed".to_string(),
        "module main\n\n\
         use dep.{Log}\n\n\
         fn main(n: Int) -> Int\n\
           uses\n\
             Log.note,\n\
         {\n\
             n + Log.note(\"hi\")\n\
         }\n"
        .to_string(),
    );
    let checked = check_all(&sources, &[effect, main]);
    assert!(!checked[1].has_errors(), "main should check cleanly");

    assert_eq!(
        wit_world_for(&checked[1]),
        "world program {\n  import deed:log.note;\n}\n"
    );
}

// -- handled effects excluded ----------------------------------------------

#[test]
fn a_handled_effect_is_excluded_from_the_world() {
    // A `with` block inside main discharges the effect. The checker already
    // excludes it from main's declared row, so the world does not list it.
    // This is the "handled effects are excluded" requirement.
    let world = world_for(
        "module main\n\n\
         effect Log {\n\
           fn note(message: String) -> Int\n\
         }\n\n\
         handler Counted implements Log {\n\
           state seen: Int\n\n\
           fn note(message) -> Int {\n\
             seen = seen + 1\n\
             seen\n\
           }\n\
         }\n\n\
         fn logs(n: Int) -> Int uses Log.note { n + Log.note(\"x\") }\n\n\
         fn main() -> Int {\n\
           with Counted { seen: 0 } {\n\
             logs(1)\n\
           }\n\
         }\n",
    );
    assert_eq!(
        world, "world program {\n}\n",
        "a handled effect should not appear in the world, got:\n{world}"
    );
}

#[test]
fn only_unhandled_effects_appear_when_some_are_handled() {
    // main handles Log internally but lets Io.write escape. Only the escaping
    // one should appear in the world.
    let world = world_for(
        "module main\n\n\
         effect Log {\n\
           fn note(message: String) -> Int\n\
         }\n\n\
         handler Silent implements Log {\n\
           fn note(message) -> Int { 0 }\n\
         }\n\n\
         fn logs(n: Int) -> Int uses Log.note { n + Log.note(\"x\") }\n\n\
         fn main(sys: System) -> Int\n\
           uses\n\
             Io.write,\n\
         {\n\
           let result = with Silent {\n\
             logs(1)\n\
           }\n\
           Io.write(sys.console, \"done\")\n\
           result\n\
         }\n",
    );
    assert!(
        world.contains("import deed:io.write;"),
        "the escaping Io.write should appear, got:\n{world}"
    );
    assert!(
        !world.contains("log"),
        "the handled Log should not appear, got:\n{world}"
    );
}

// -- changing the row changes the world ------------------------------------

#[test]
fn changing_the_row_changes_the_world() {
    // The test that makes the whole claim worth anything. If the world were
    // authored alongside the rows rather than derived from them, it would not
    // change when the row changed. It must change.
    let with_write = world_for(
        "module main\n\n\
         fn main(sys: System) -> Int\n\
           uses\n\
             Io.write,\n\
         {\n\
             Io.write(sys.console, \"hi\")\n\
             0\n\
         }\n",
    );

    let pure = world_for("module main\n\nfn main() -> Int { 0 }\n");

    assert_ne!(
        with_write, pure,
        "adding Io.write to the row must change the world: \
         before={pure:?} after={with_write:?}"
    );
    assert!(
        with_write.contains("import deed:io.write;"),
        "the world after adding Io.write should import it, got:\n{with_write}"
    );
    assert_eq!(
        pure, "world program {\n}\n",
        "removing all effects from the row must leave an empty world"
    );
}

#[test]
fn adding_an_operation_to_the_row_adds_it_to_the_world() {
    // A second check on derivation: one extra operation in the row means one
    // extra import in the world.
    let mut sources_one = SourceMap::new();
    let ids_one = vec![
        sources_one.add(
            "test.deed".to_string(),
            "module main\n\n\
         fn main(sys: System) -> Int\n\
           uses\n\
             Io.write,\n\
         {\n\
             Io.write(sys.console, \"hi\")\n\
             0\n\
         }\n"
            .to_string(),
        ),
    ];
    let checked_one = check_all(&sources_one, &ids_one);
    let world_one = wit_world_for(&checked_one[0]);

    let mut sources_two = SourceMap::new();
    let ids_two = vec![
        sources_two.add(
            "test.deed".to_string(),
            "module main\n\n\
         fn main(sys: System) -> Int\n\
           uses\n\
             Io.write,\n\
             Io.now,\n\
         {\n\
             let _ = Io.now(sys.clock)\n\
             Io.write(sys.console, \"hi\")\n\
             0\n\
         }\n"
            .to_string(),
        ),
    ];
    let checked_two = check_all(&sources_two, &ids_two);
    let world_two = wit_world_for(&checked_two[0]);

    assert_ne!(
        world_one, world_two,
        "adding Io.now to the row must change the world"
    );
    assert!(
        world_two.contains("import deed:io.now;"),
        "the expanded world should include Io.now, got:\n{world_two}"
    );
    assert!(
        !world_one.contains("import deed:io.now;"),
        "the original world should not include Io.now, got:\n{world_one}"
    );
}

// -- output properties ------------------------------------------------------

#[test]
fn the_world_is_deterministic() {
    // The same program always produces the same world. Sorted output means no
    // hash-map nondeterminism can sneak in.
    let source = "module main\n\n\
         fn main(sys: System) -> Int\n\
           uses\n\
             Io.write,\n\
             Io.now,\n\
         {\n\
             let _ = Io.now(sys.clock)\n\
             Io.write(sys.console, \"hi\")\n\
             0\n\
         }\n";

    let first = world_for(source);
    let second = world_for(source);
    assert_eq!(
        first, second,
        "the same program must always produce the same world"
    );
}

#[test]
fn imports_are_in_sorted_order() {
    // Sorted output means the world is reproducible and diff-friendly.
    let world = world_for(
        "module main\n\n\
         fn main(sys: System) -> Int\n\
           uses\n\
             Io.write,\n\
             Io.now,\n\
         {\n\
             let _ = Io.now(sys.clock)\n\
             Io.write(sys.console, \"hi\")\n\
             0\n\
         }\n",
    );

    let lines: Vec<&str> = world
        .lines()
        .filter(|line| line.contains("import"))
        .collect();
    let mut sorted = lines.clone();
    sorted.sort();
    assert_eq!(
        lines, sorted,
        "imports should appear in sorted order, got:\n{world}"
    );
}
