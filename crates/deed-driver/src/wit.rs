//! Emitting a WIT world derived from a program's effect rows.
//!
//! A WIT world is the set of capabilities a compiled Deed program needs from
//! its host. Each effect operation that escapes the program boundary becomes
//! one `import`. Handled effects are already excluded by the effect checker:
//! when a function discharges an effect with a `with` block, that effect does
//! not appear in its declared row, so the world is derived entirely from the
//! rows rather than written by hand.
//!
//! The test that makes this worth anything: take a program, emit its world,
//! then change one function's row and emit again. The world must change. If it
//! does not, the world is not derived from the row and the whole claim is
//! decoration.

use std::collections::BTreeSet;

use deed_ast::Item;
use deed_resolve::{DefKind, ExportKind};

use crate::Checked;

/// Emits the WIT world a program needs from its host.
///
/// The world is derived from `main`'s declared row: every effect operation in
/// that row is an import the host must satisfy. Handled effects do not appear
/// because the effect checker already excludes them from the declared row, so
/// whatever appears here reached the program boundary without being discharged.
///
/// An empty world means the program is self-contained and asks nothing of the
/// host.
///
/// Row variables are not effects and are excluded from the world. They stand
/// for whatever a callback performs, and the caller that provides the callback
/// decides what that is; the world only names effects that reach the program
/// boundary directly.
///
/// The output is deterministic: imports are sorted so that the same program
/// always produces the same world.
pub fn wit_world_for(checked: &Checked) -> String {
    let main_span = checked.module.items.iter().find_map(|item| match item {
        Item::Function(function) if function.sig.name.name == "main" => {
            Some(function.sig.name.span)
        }
        _ => None,
    });

    let Some(main_span) = main_span else {
        return "world program {\n}\n".to_string();
    };

    let Some(main_def) = checked.resolutions.resolution(main_span) else {
        return "world program {\n}\n".to_string();
    };

    let Some(row) = checked.effects.declared(main_def) else {
        return "world program {\n}\n".to_string();
    };

    let imports: BTreeSet<String> = row
        .iter()
        .filter(|item| {
            let kind = checked.resolutions.def(item.effect).kind;
            match kind {
                DefKind::Effect => true,
                DefKind::Import => checked
                    .resolutions
                    .import(item.effect)
                    .is_some_and(|export| export.kind == ExportKind::Effect),
                _ => false,
            }
        })
        .map(|item| {
            let effect_name = checked.resolutions.def(item.effect).name.to_lowercase();
            match &item.operation {
                Some(operation) => format!("  import deed:{effect_name}.{operation};"),
                None => format!("  import deed:{effect_name};"),
            }
        })
        .collect();

    let mut world = "world program {\n".to_string();
    for import in &imports {
        world.push_str(import);
        world.push('\n');
    }
    world.push_str("}\n");
    world
}
