//! The canonical ABI adapters, for the values that do not cross unchanged.
//!
//! [`crate::component`] wraps a core module in a component, and until now it
//! could only do that for exports whose values are the same value on both
//! sides: a number is an `s64` and a boolean is a `bool`, so lifting one is a
//! declaration. Anything wider than a word was refused, and
//! `design/decisions/2026-08-09-a-component-for-what-crosses-unchanged.md`
//! named the missing piece: `cabi_realloc` and the two halves of a string.
//!
//! This is that piece. A component's caller passes a string as a pointer and
//! a length into memory it asked the callee to allocate, and takes one back
//! through a return area the callee allocates. This backend passes a single
//! address to a header of two words followed by the bytes. Neither side is
//! going to change, so something has to stand between them, and that is what
//! is written here: a wrapper per export that takes the boundary's shape,
//! converts, calls the function the compiler already emitted, and converts
//! back.
//!
//! Two things this does not do, both on purpose.
//!
//! It does not touch the module the compiler produced. The wrappers are
//! appended, so every function keeps the index it had and every export keeps
//! the name it had; `deed build` and `deed build --component` still describe
//! one compiler. What goes inside the component is that module with more
//! functions in it, not a different one.
//!
//! And it does not free anything. `cabi_realloc` is the bump pointer every
//! other allocation in a compiled module already uses, which means a host that
//! calls a component in a loop grows its memory in a loop.
//! `design/decisions/2026-07-31-compiled-memory-reclamation.md` is the open
//! question that covers it, and it covers the whole backend rather than this
//! corner of it.

use crate::layout;
use crate::runtime::{TEXT, allocate, count_to, string_room};
use crate::wasm::{Func, FuncType, Ins, Module, ValType};

/// What the canonical ABI calls the allocator a component's caller uses.
///
/// The name is not a choice. A host lowering a string into a component looks
/// for this export by this spelling, so a module that spells it differently
/// gets a caller that cannot hand it anything.
pub const REALLOC: &str = "cabi_realloc";

/// What one exported value looks like on the boundary.
///
/// The first two need no adapter and were the whole of what could be lifted
/// before. [`Cross::Text`] is the one that does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cross {
    /// `bool`. One core `i32`, zero or one, the same value on both sides.
    Bool,
    /// `s64`. One core `i64`, the same value on both sides.
    Signed64,
    /// `string`. Two core `i32`s going in, a pointer to two more coming back,
    /// and UTF-8 bytes in the module's own memory either way.
    Text,
}

impl Cross {
    /// The component model's byte for this type.
    pub(crate) fn byte(self) -> u8 {
        match self {
            Cross::Bool => 0x7f,
            Cross::Signed64 => 0x78,
            Cross::Text => 0x73,
        }
    }

    /// Whether a value of this type is the same value on both sides.
    ///
    /// The whole of what "needs an adapter" means. An export where every
    /// answer here is `true` is lifted with no options and no wrapper, which
    /// is what keeps the components that worked before byte for byte what
    /// they were.
    pub fn crosses_unchanged(self) -> bool {
        !matches!(self, Cross::Text)
    }

    /// The core types this one arrives as, in order.
    fn flat(self) -> &'static [ValType] {
        match self {
            Cross::Bool => &[ValType::I32],
            Cross::Signed64 => &[ValType::I64],
            Cross::Text => &[ValType::I32, ValType::I32],
        }
    }
}

/// One export the adapters have to stand in front of.
pub struct Crossing<'a> {
    /// The name the core module exports the compiled function under.
    pub name: &'a str,
    pub params: &'a [Cross],
    pub result: Option<Cross>,
}

/// What [`adapt`] added, so the component encoder can name it.
pub struct Added {
    /// The export name of each wrapper, in the order the crossings were given,
    /// or `None` where the export needed no wrapper.
    pub wrappers: Vec<Option<String>>,
}

/// Appends the adapters `crossings` need, and says what it added.
///
/// Returns `None` when nothing needed one, which is the case every component
/// this backend could already write is in. That is not an optimisation: a
/// component with no adapters lifts with no options, and a lift with options
/// naming a memory it does not use would be describing a boundary that is not
/// there.
pub fn adapt(module: &mut Module, crossings: &[Crossing<'_>]) -> Option<Added> {
    let needed: Vec<bool> = crossings
        .iter()
        .map(|one| {
            one.params.iter().any(|p| !p.crosses_unchanged())
                || one.result.is_some_and(|r| !r.crosses_unchanged())
        })
        .collect();
    if !needed.iter().any(|one| *one) {
        return None;
    }

    let (ty, locals, body) = realloc();
    let type_index = module.intern_type(ty);
    let index = module.add_func(Func {
        type_index,
        locals,
        body,
    });
    module.export(REALLOC, index);
    module.names.push((index, REALLOC.to_string()));

    let mut wrappers = Vec::new();
    for (one, needed) in crossings.iter().zip(needed) {
        if !needed {
            wrappers.push(None);
            continue;
        }
        let called = module
            .exports
            .iter()
            .find(|(name, _)| name == one.name)
            .map(|(_, index)| *index)
            .expect("a crossing names an export of the module it is adapting");
        let (ty, locals, body) = wrapper(called, one);
        let type_index = module.intern_type(ty);
        let index = module.add_func(Func {
            type_index,
            locals,
            body,
        });
        // A Deed name cannot hold a full stop, so this collides with nothing a
        // program could have exported.
        let name = format!("{}.lift", one.name);
        module.export(name.clone(), index);
        module.names.push((index, name.clone()));
        wrappers.push(Some(name));
    }

    Some(Added { wrappers })
}

/// `cabi_realloc(original, held, align, wanted) -> address`.
///
/// The four arguments the canonical ABI gives an allocator: what was there,
/// how big it was, what alignment the caller needs, and how much it wants
/// now. A host calls it with a null and a zero to ask for room, which is the
/// only shape a string parameter uses.
///
/// The bump pointer every other allocation in a compiled module uses answers
/// all four, and hands back a word-aligned address, so nothing here has to
/// know about alignment beyond refusing to promise more than it has.
fn realloc() -> (FuncType, Vec<ValType>, Vec<Ins>) {
    const ORIGINAL: u32 = 0;
    const HELD: u32 = 1;
    const ALIGN: u32 = 2;
    const WANTED: u32 = 3;
    const OUT: u32 = 4;
    const I: u32 = 5;

    let mut body = vec![
        // An alignment this cannot promise is a wrong answer rather than a
        // slow one, and a wrong answer here is a value read at the wrong
        // address rather than an error anybody sees.
        Ins::LocalGet(ALIGN),
        Ins::I32Const(layout::WORD as i32),
        Ins::I32GtS,
        Ins::If {
            result: None,
            then: vec![Ins::Unreachable],
            otherwise: Vec::new(),
        },
        // Nothing was there, so there is nothing to keep, whatever the caller
        // said about how much of it there was.
        Ins::LocalGet(ORIGINAL),
        Ins::I32Eqz,
        Ins::If {
            result: None,
            then: vec![Ins::I32Const(0), Ins::LocalSet(HELD)],
            otherwise: Vec::new(),
        },
    ];
    body.extend(allocate(OUT, |ins| ins.extend(word_room(WANTED))));
    // What survives a move is the smaller of the two sizes. Growing keeps all
    // of it; shrinking keeps as much as still fits.
    body.extend([
        Ins::LocalGet(HELD),
        Ins::LocalGet(WANTED),
        Ins::I32GtS,
        Ins::If {
            result: None,
            then: vec![Ins::LocalGet(WANTED), Ins::LocalSet(HELD)],
            otherwise: Vec::new(),
        },
    ]);
    body.extend(count_to(
        I,
        HELD,
        vec![
            Ins::LocalGet(OUT),
            Ins::LocalGet(I),
            Ins::I32Add,
            Ins::LocalGet(ORIGINAL),
            Ins::LocalGet(I),
            Ins::I32Add,
            Ins::I32Load8U(0),
            Ins::I32Store8(0),
        ],
    ));
    body.extend([Ins::LocalGet(OUT), Ins::Return]);

    (
        FuncType {
            params: vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            results: vec![ValType::I32],
        },
        vec![ValType::I32, ValType::I32],
        body,
    )
}

/// The wrapper that stands in front of one export.
///
/// It is the compiled function's signature with the strings turned inside
/// out: each one arrives as a pointer and a length and is built into the
/// layout the module already uses, and a string coming back is written into a
/// return area as the pointer and the length the caller reads.
fn wrapper(called: u32, one: &Crossing<'_>) -> (FuncType, Vec<ValType>, Vec<Ins>) {
    let mut params = Vec::new();
    // Where each parameter's core values start, by parameter.
    let mut starts = Vec::new();
    for cross in one.params {
        starts.push(params.len() as u32);
        params.extend_from_slice(cross.flat());
    }

    // A local per string parameter, for the address of the value built from
    // it, and then the four every wrapper uses.
    let texts = one
        .params
        .iter()
        .filter(|cross| **cross == Cross::Text)
        .count();
    let base = params.len() as u32;
    let counter = base + texts as u32;
    let characters = counter + 1;
    let answer = counter + 2;
    let area = counter + 3;
    let locals = vec![ValType::I32; texts + 4];

    let mut body = Vec::new();
    let mut built = Vec::new();
    for (at, cross) in one.params.iter().enumerate() {
        if *cross != Cross::Text {
            continue;
        }
        let into = base + built.len() as u32;
        body.extend(lower_text(
            starts[at],
            starts[at] + 1,
            into,
            counter,
            characters,
        ));
        built.push(into);
    }

    let mut next = 0;
    for (at, cross) in one.params.iter().enumerate() {
        match cross {
            Cross::Text => {
                body.extend([Ins::LocalGet(built[next]), Ins::I64ExtendI32S]);
                next += 1;
            }
            _ => body.push(Ins::LocalGet(starts[at])),
        }
    }
    body.push(Ins::Call(called));

    let results = match one.result {
        None => Vec::new(),
        Some(Cross::Bool) => vec![ValType::I32],
        Some(Cross::Signed64) => vec![ValType::I64],
        // Two values do not fit in one core result, so the canonical ABI has
        // the callee write them somewhere and give back where. Not a choice
        // this backend makes: a caller reads the answer at that address
        // whether or not anything here would have preferred a pair.
        Some(Cross::Text) => vec![ValType::I32],
    };
    if one.result == Some(Cross::Text) {
        body.extend([Ins::I32WrapI64, Ins::LocalSet(answer)]);
        body.extend(allocate(area, |ins| {
            ins.push(Ins::I64Const(2 * layout::WORD as i64))
        }));
        body.extend([
            Ins::LocalGet(area),
            Ins::LocalGet(answer),
            Ins::I32Const(TEXT as i32),
            Ins::I32Add,
            Ins::I32Store(0),
            Ins::LocalGet(area),
            Ins::LocalGet(answer),
            Ins::I64Load(layout::WORD),
            Ins::I32WrapI64,
            Ins::I32Store(4),
            Ins::LocalGet(area),
        ]);
    }
    body.push(Ins::Return);

    (FuncType { params, results }, locals, body)
}

/// Builds the value the module uses out of the pointer and length a caller
/// passed, leaving its address in `into`.
///
/// The character count is counted rather than carried, because the boundary
/// does not carry one: a caller says how many bytes there are and the layout
/// this backend uses says how many characters, and the only way from one to
/// the other is to look at the bytes. A byte begins a character unless it is
/// a UTF-8 continuation byte, which is the range from `0x80` to `0xbf`, so
/// the two comparisons below are exclusive and their sum is one or zero.
///
/// Two comparisons and an add rather than a mask: this backend's own runner
/// reads `i32.and` as the boolean operator the language has rather than the
/// bitwise one WebAssembly has, and nothing here may depend on which.
fn lower_text(pointer: u32, length: u32, into: u32, counter: u32, characters: u32) -> Vec<Ins> {
    let mut body = vec![Ins::I32Const(0), Ins::LocalSet(characters)];
    body.extend(count_to(
        counter,
        length,
        vec![
            Ins::LocalGet(characters),
            Ins::LocalGet(pointer),
            Ins::LocalGet(counter),
            Ins::I32Add,
            Ins::I32Load8U(0),
            Ins::I32Const(0x80),
            Ins::I32LtS,
            Ins::I32Add,
            Ins::LocalGet(pointer),
            Ins::LocalGet(counter),
            Ins::I32Add,
            Ins::I32Load8U(0),
            Ins::I32Const(0xc0),
            Ins::I32GeS,
            Ins::I32Add,
            Ins::LocalSet(characters),
        ],
    ));
    body.extend(allocate(into, |ins| ins.extend(string_room(length))));
    body.extend([
        Ins::LocalGet(into),
        Ins::LocalGet(characters),
        Ins::I64ExtendI32S,
        Ins::I64Store(0),
        Ins::LocalGet(into),
        Ins::LocalGet(length),
        Ins::I64ExtendI32S,
        Ins::I64Store(layout::WORD),
    ]);
    body.extend(count_to(
        counter,
        length,
        vec![
            Ins::LocalGet(into),
            Ins::I32Const(TEXT as i32),
            Ins::I32Add,
            Ins::LocalGet(counter),
            Ins::I32Add,
            Ins::LocalGet(pointer),
            Ins::LocalGet(counter),
            Ins::I32Add,
            Ins::I32Load8U(0),
            Ins::I32Store8(0),
        ],
    ));
    body
}

/// Pushes what `bytes` holds, rounded up so the next allocation starts on a
/// word.
///
/// A remainder rather than a mask, for the reason [`lower_text`] gives.
fn word_room(bytes: u32) -> Vec<Ins> {
    vec![
        Ins::LocalGet(bytes),
        Ins::I64ExtendI32S,
        Ins::I64Const(layout::WORD as i64),
        Ins::LocalGet(bytes),
        Ins::I64ExtendI32S,
        Ins::I64Const(layout::WORD as i64),
        Ins::I64RemS,
        Ins::I64Sub,
        Ins::I64Const(layout::WORD as i64),
        Ins::I64RemS,
        Ins::I64Add,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::{Func, FuncType, Module, ValType};

    /// A module exporting one function that takes an address and gives one
    /// back, which is what a compiled `fn greet(name: String) -> String` is.
    fn greeter() -> Module {
        let mut module = Module::new();
        module.memory_pages = Some(16);
        module.exported_memory = Some("memory".to_string());
        let ty = module.intern_type(FuncType {
            params: vec![ValType::I64],
            results: vec![ValType::I64],
        });
        let index = module.add_func(Func {
            type_index: ty,
            locals: vec![],
            body: vec![Ins::LocalGet(0)],
        });
        module.export("greet", index);
        module
    }

    fn text() -> Vec<Crossing<'static>> {
        vec![Crossing {
            name: "greet",
            params: &[Cross::Text],
            result: Some(Cross::Text),
        }]
    }

    /// The case that used to be everything: nothing needs an adapter, so
    /// nothing is added and the module is the one that came in.
    #[test]
    fn a_module_whose_values_all_cross_unchanged_is_left_alone() {
        let mut module = greeter();
        let before = module.clone();
        let added = adapt(
            &mut module,
            &[Crossing {
                name: "greet",
                params: &[Cross::Signed64],
                result: Some(Cross::Bool),
            }],
        );
        assert!(added.is_none());
        assert_eq!(module, before);
    }

    /// Every function the compiler emitted keeps its index, which is what
    /// makes the module inside a component the module beside it with more in
    /// it rather than a different one.
    #[test]
    fn the_wrappers_are_appended_and_move_nothing() {
        let mut module = greeter();
        let before = module.clone();
        adapt(&mut module, &text()).expect("text needs adapters");

        assert_eq!(module.funcs[..before.funcs.len()], before.funcs[..]);
        assert_eq!(module.exports[..before.exports.len()], before.exports[..]);
    }

    /// The allocator is exported under the name a caller looks for, and a
    /// wrapper under one no program could have taken.
    #[test]
    fn it_exports_the_allocator_and_a_wrapper_per_export_that_needs_one() {
        let mut module = greeter();
        let added = adapt(&mut module, &text()).expect("text needs adapters");

        let names: Vec<&str> = module
            .exports
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, vec!["greet", "cabi_realloc", "greet.lift"]);
        assert_eq!(added.wrappers, vec![Some("greet.lift".to_string())]);
    }

    /// An export that needs no wrapper does not get one even when another in
    /// the same module does, because a wrapper it does not need is a second
    /// path to the same answer for nothing.
    #[test]
    fn an_export_that_needs_no_wrapper_is_not_given_one() {
        let mut module = greeter();
        let ty = module.intern_type(FuncType {
            params: vec![ValType::I64],
            results: vec![ValType::I64],
        });
        let index = module.add_func(Func {
            type_index: ty,
            locals: vec![],
            body: vec![Ins::LocalGet(0)],
        });
        module.export("twice", index);

        let added = adapt(
            &mut module,
            &[
                Crossing {
                    name: "greet",
                    params: &[Cross::Text],
                    result: Some(Cross::Text),
                },
                Crossing {
                    name: "twice",
                    params: &[Cross::Signed64],
                    result: Some(Cross::Signed64),
                },
            ],
        )
        .expect("one of them needs adapters");

        assert_eq!(
            added.wrappers,
            vec![Some("greet.lift".to_string()), None],
            "the second export crosses unchanged"
        );
    }

    /// The wrapper's signature is the boundary's shape rather than the
    /// compiled function's: a string arrives as two values and comes back
    /// through one address.
    #[test]
    fn a_wrapper_takes_the_shape_the_boundary_has() {
        let mut module = greeter();
        adapt(&mut module, &text()).expect("text needs adapters");

        let wrapper = module.funcs.last().expect("a wrapper was added");
        let signature = &module.types[wrapper.type_index as usize];
        assert_eq!(signature.params, vec![ValType::I32, ValType::I32]);
        assert_eq!(signature.results, vec![ValType::I32]);
    }

    /// The two bytes that used to be the whole of this, and the one that is
    /// new. Swapping them is a component that lies about what it takes.
    #[test]
    fn the_three_crossings_are_the_bytes_the_format_gives_them() {
        assert_eq!(Cross::Bool.byte(), 0x7f);
        assert_eq!(Cross::Signed64.byte(), 0x78);
        assert_eq!(Cross::Text.byte(), 0x73);
    }

    /// Which of them needs standing in front of, which is the question the
    /// whole module turns on.
    #[test]
    fn only_text_needs_an_adapter() {
        assert!(Cross::Bool.crosses_unchanged());
        assert!(Cross::Signed64.crosses_unchanged());
        assert!(!Cross::Text.crosses_unchanged());
    }

    /// The allocator grows the memory. A `cabi_realloc` that moved the bump
    /// pointer without growing is the bug `str_concat` had for two releases,
    /// and the caller writing into what it hands back is exactly the shape
    /// that hit it.
    #[test]
    fn the_allocator_grows_the_memory() {
        let (_, _, body) = realloc();
        let mut flat = Vec::new();
        flatten(&body, &mut flat);
        assert!(
            flat.contains(&Ins::MemoryGrow),
            "the allocator has to grow the memory it hands out"
        );
    }

    fn flatten(body: &[Ins], out: &mut Vec<Ins>) {
        for instruction in body {
            out.push(instruction.clone());
            match instruction {
                Ins::Block { body, .. } | Ins::Loop { body, .. } => flatten(body, out),
                Ins::If {
                    then, otherwise, ..
                } => {
                    flatten(then, out);
                    flatten(otherwise, out);
                }
                _ => {}
            }
        }
    }
}
