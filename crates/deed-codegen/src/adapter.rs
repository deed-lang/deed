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
    /// `list<s64>`. Two core `i32`s going in, a pointer to two more coming
    /// back, and the elements in the module's own memory either way.
    ///
    /// The one aggregate needing no element loop on the way out. A list here
    /// is a length and then its elements, eight bytes each, and a canonical
    /// `list<s64>` is those elements with nothing in front of them, so what
    /// goes back is the list's own address plus one word.
    Numbers,
}

impl Cross {
    /// The component model's byte for this type, when it has one.
    ///
    /// `None` for a type the format writes out in its type section and refers
    /// to by index afterwards. A value type is one byte for a primitive or an
    /// index for anything else, and the two cannot be confused: the
    /// primitives start at `0x73` and an index is written smallest first.
    pub(crate) fn byte(self) -> Option<u8> {
        match self {
            Cross::Bool => Some(0x7f),
            Cross::Signed64 => Some(0x78),
            Cross::Text => Some(0x73),
            Cross::Numbers => None,
        }
    }

    /// What defines this type, for one with no byte of its own.
    pub(crate) fn definition(self) -> Option<&'static [u8]> {
        match self {
            // `70` a list, then what it holds.
            Cross::Numbers => Some(&[0x70, 0x78]),
            Cross::Bool | Cross::Signed64 | Cross::Text => None,
        }
    }

    /// Whether a value of this type is the same value on both sides.
    ///
    /// The whole of what "needs an adapter" means. An export where every
    /// answer here is `true` is lifted with no options and no wrapper, which
    /// is what keeps the components that worked before byte for byte what
    /// they were.
    pub fn crosses_unchanged(self) -> bool {
        !matches!(self, Cross::Text | Cross::Numbers)
    }

    /// The core types this one arrives as, in order.
    fn flat(self) -> &'static [ValType] {
        match self {
            Cross::Bool => &[ValType::I32],
            Cross::Signed64 => &[ValType::I64],
            Cross::Text | Cross::Numbers => &[ValType::I32, ValType::I32],
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

    // A local per adapted parameter, for the address of the value built from
    // it, and then the four every wrapper uses.
    let adapted = one
        .params
        .iter()
        .filter(|cross| !cross.crosses_unchanged())
        .count();
    let base = params.len() as u32;
    let counter = base + adapted as u32;
    let characters = counter + 1;
    let answer = counter + 2;
    let area = counter + 3;
    let locals = vec![ValType::I32; adapted + 4];

    let mut body = Vec::new();
    let mut built = Vec::new();
    for (at, cross) in one.params.iter().enumerate() {
        if cross.crosses_unchanged() {
            continue;
        }
        let into = base + built.len() as u32;
        body.extend(match cross {
            Cross::Text => lower_text(starts[at], starts[at] + 1, into, counter, characters),
            _ => lower_numbers(starts[at], starts[at] + 1, into, counter),
        });
        built.push(into);
    }

    let mut next = 0;
    for (at, cross) in one.params.iter().enumerate() {
        if cross.crosses_unchanged() {
            body.push(Ins::LocalGet(starts[at]));
        } else {
            body.extend([Ins::LocalGet(built[next]), Ins::I64ExtendI32S]);
            next += 1;
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
        Some(Cross::Text) | Some(Cross::Numbers) => vec![ValType::I32],
    };
    // Where the elements start, and where the count is read from, are the
    // whole difference between handing back text and handing back numbers.
    // Text keeps two words in front of its bytes and the second is the byte
    // count; a list keeps one and it is the length.
    if let Some(carried @ (Cross::Text | Cross::Numbers)) = one.result {
        let (elements, count) = match carried {
            Cross::Text => (TEXT, layout::WORD),
            _ => (layout::WORD, 0),
        };
        body.extend([Ins::I32WrapI64, Ins::LocalSet(answer)]);
        // A word: the return area is a pointer and a length, and both are
        // four bytes wide on this boundary whatever else here is eight.
        body.extend(allocate(area, |ins| {
            ins.push(Ins::I64Const(layout::WORD as i64))
        }));
        body.extend([
            Ins::LocalGet(area),
            Ins::LocalGet(answer),
            Ins::I32Const(elements as i32),
            Ins::I32Add,
            Ins::I32Store(0),
            Ins::LocalGet(area),
            Ins::LocalGet(answer),
            Ins::I64Load(count),
            Ins::I32WrapI64,
            Ins::I32Store(4),
            Ins::LocalGet(area),
        ]);
    }
    body.push(Ins::Return);

    (FuncType { params, results }, locals, body)
}

/// Builds the list the module uses out of the pointer and length a caller
/// passed, leaving its address in `into`.
///
/// Element for element rather than byte for byte, because both sides store an
/// `s64` in eight bytes and the only difference is the length this backend
/// keeps in front. A caller's elements are already in this module's memory,
/// because a list arriving here was allocated through `cabi_realloc`.
fn lower_numbers(pointer: u32, length: u32, into: u32, counter: u32) -> Vec<Ins> {
    let mut body = allocate(into, |ins| {
        ins.extend([
            Ins::LocalGet(length),
            Ins::I64ExtendI32S,
            Ins::I64Const(1),
            Ins::I64Add,
            Ins::I64Const(layout::WORD as i64),
            Ins::I64Mul,
        ]);
    });
    body.extend([
        Ins::LocalGet(into),
        Ins::LocalGet(length),
        Ins::I64ExtendI32S,
        Ins::I64Store(0),
    ]);
    body.extend(count_to(
        counter,
        length,
        vec![
            Ins::LocalGet(into),
            Ins::I32Const(layout::WORD as i32),
            Ins::I32Add,
            Ins::LocalGet(counter),
            Ins::I32Const(layout::WORD as i32),
            Ins::I32Mul,
            Ins::I32Add,
            Ins::LocalGet(pointer),
            Ins::LocalGet(counter),
            Ins::I32Const(layout::WORD as i32),
            Ins::I32Mul,
            Ins::I32Add,
            Ins::I64Load(0),
            Ins::I64Store(0),
        ],
    ));
    body
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

    /// Text in either direction is enough on its own.
    ///
    /// An export taking a number and giving text back needs the allocator as
    /// much as one that takes text, and a check that asked for both would
    /// leave it without one.
    #[test]
    fn text_in_either_direction_is_enough_to_need_the_adapters() {
        for (params, result) in [
            (vec![Cross::Text], Some(Cross::Bool)),
            (vec![Cross::Signed64], Some(Cross::Text)),
            (vec![Cross::Text], None),
        ] {
            let mut module = greeter();
            let added = adapt(
                &mut module,
                &[Crossing {
                    name: "greet",
                    params: &params,
                    result,
                }],
            );
            assert!(
                added.is_some_and(|added| added.wrappers == vec![Some("greet.lift".to_string())]),
                "{params:?} -> {result:?}"
            );
        }
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

    /// The bytes the format gives the primitives, and the shape it gives the
    /// one that has none. Swapping them is a component that lies about what
    /// it takes.
    #[test]
    fn the_crossings_are_the_bytes_and_shapes_the_format_gives_them() {
        assert_eq!(Cross::Bool.byte(), Some(0x7f));
        assert_eq!(Cross::Signed64.byte(), Some(0x78));
        assert_eq!(Cross::Text.byte(), Some(0x73));

        // A list is written out and referred to by index, so it has no byte
        // of its own: `70` a list, `78` the `s64` in it.
        assert_eq!(Cross::Numbers.byte(), None);
        assert_eq!(Cross::Numbers.definition(), Some(&[0x70, 0x78][..]));
        for primitive in [Cross::Bool, Cross::Signed64, Cross::Text] {
            assert_eq!(primitive.definition(), None, "{primitive:?}");
        }
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

    // -- what the wrappers do when they run ---------------------------------
    //
    // Everything above reads the module the adapters produced. None of it
    // notices a wrapper that reads its second parameter from the first slot,
    // and every one of those is a component that answers with somebody else's
    // text. So the rest of this runs them, through this backend's own runner,
    // by adding a function to the module that calls the wrapper and reports
    // what came back.

    /// Where a caller's bytes are put, and where the second string starts.
    const SOURCE: u32 = layout::HEAP_START;
    const OTHER: u32 = layout::HEAP_START + 64;
    /// Where the bump pointer starts, above both of them.
    const START: u32 = layout::HEAP_START + 128;

    /// A module holding two strings' bytes and one function standing in for a
    /// compiled one, under the name the crossings below use.
    fn standing_in(params: Vec<ValType>, results: Vec<ValType>, body: Vec<Ins>) -> Module {
        let mut module = Module::new();
        module.memory_pages = Some(16);
        module.exported_memory = Some("memory".to_string());
        module
            .data
            .push((layout::BUMP, i64::from(START).to_le_bytes().to_vec()));
        module.data.push((SOURCE, "dünya".as_bytes().to_vec()));
        module.data.push((OTHER, "second".as_bytes().to_vec()));
        let type_index = module.intern_type(FuncType { params, results });
        let index = module.add_func(Func {
            type_index,
            locals: vec![],
            body,
        });
        module.export("under_test", index);
        module
    }

    /// Adapts `module`, calls the wrapper with `args`, and hands back what
    /// `read` makes of the answer.
    fn answered(mut module: Module, crossing: Crossing<'_>, args: Vec<Ins>, read: Vec<Ins>) -> i64 {
        adapt(&mut module, &[crossing]).expect("something needed adapters");
        let lift = module
            .exports
            .iter()
            .find(|(name, _)| name == "under_test.lift")
            .expect("a wrapper was exported")
            .1;

        let type_index = module.intern_type(FuncType {
            params: vec![],
            results: vec![ValType::I64],
        });
        let mut body = args;
        body.push(Ins::Call(lift));
        body.extend(read);
        body.push(Ins::Return);
        let index = module.add_func(Func {
            type_index,
            locals: vec![ValType::I32],
            body,
        });
        module.export("ask", index);

        match crate::run::call(&module, "ask", &[]) {
            Ok(Some(crate::run::Value::I64(value))) => value,
            other => panic!("the wrapper did not answer: {other:?}"),
        }
    }

    /// What a caller passes for the first string. Five characters in six
    /// bytes, so a count taken from the wrong one is a different number.
    fn first() -> Vec<Ins> {
        vec![
            Ins::I32Const(SOURCE as i32),
            Ins::I32Const("dünya".len() as i32),
        ]
    }

    /// And for the second, which is six of each.
    fn second() -> Vec<Ins> {
        vec![Ins::I32Const(OTHER as i32), Ins::I32Const(6)]
    }

    /// How many bytes the return area says came back.
    fn bytes_back() -> Vec<Ins> {
        vec![
            Ins::LocalSet(0),
            Ins::LocalGet(0),
            Ins::I32Load(4),
            Ins::I64ExtendI32S,
        ]
    }

    /// How many characters the value behind the return area carries.
    fn characters_back() -> Vec<Ins> {
        vec![
            Ins::LocalSet(0),
            Ins::LocalGet(0),
            Ins::I32Load(0),
            Ins::I32Const(TEXT as i32),
            Ins::I32Sub,
            Ins::I64Load(0),
        ]
    }

    /// The byte at `at` of what came back.
    fn byte_back(at: i32) -> Vec<Ins> {
        vec![
            Ins::LocalSet(0),
            Ins::LocalGet(0),
            Ins::I32Load(0),
            Ins::I32Const(at),
            Ins::I32Add,
            Ins::I32Load8U(0),
            Ins::I64ExtendI32S,
        ]
    }

    /// A function taking one address and giving it straight back, which is
    /// what a compiled `fn greet(name: String) -> String` that changed nothing
    /// would be.
    fn echoing() -> Module {
        standing_in(
            vec![ValType::I64],
            vec![ValType::I64],
            vec![Ins::LocalGet(0), Ins::Return],
        )
    }

    fn one_string() -> Crossing<'static> {
        Crossing {
            name: "under_test",
            params: &[Cross::Text],
            result: Some(Cross::Text),
        }
    }

    /// The byte count crosses, the character count is worked out, and the
    /// bytes are copied rather than pointed at.
    ///
    /// `dünya` is five characters in six bytes, so a wrapper that carried the
    /// byte count into the character slot answers six here and is right about
    /// every ASCII string ever tested.
    #[test]
    fn a_string_arrives_with_its_bytes_and_its_character_count() {
        assert_eq!(
            answered(echoing(), one_string(), first(), bytes_back()),
            6,
            "six bytes"
        );
        assert_eq!(
            answered(echoing(), one_string(), first(), characters_back()),
            5,
            "five characters"
        );
        assert_eq!(
            answered(echoing(), one_string(), first(), byte_back(0)),
            i64::from(b'd')
        );
        assert_eq!(
            answered(echoing(), one_string(), first(), byte_back(5)),
            i64::from(b'a'),
            "the last byte is copied too"
        );
    }

    /// An empty string is a length of zero and a pointer nothing reads, and
    /// the loops that count and copy have to run zero times rather than once.
    #[test]
    fn an_empty_string_crosses_as_an_empty_string() {
        let none = vec![Ins::I32Const(SOURCE as i32), Ins::I32Const(0)];
        assert_eq!(
            answered(echoing(), one_string(), none.clone(), bytes_back()),
            0
        );
        assert_eq!(
            answered(echoing(), one_string(), none, characters_back()),
            0
        );
    }

    /// A string that is not the first parameter is read from the slots it is
    /// actually in.
    ///
    /// A wrapper reading fixed slots is right about `greet(name)` and hands
    /// the compiled function a number where it wants an address here.
    #[test]
    fn a_string_among_other_parameters_is_read_from_its_own_slots() {
        let mixed = Crossing {
            name: "under_test",
            params: &[Cross::Signed64, Cross::Text, Cross::Bool],
            result: Some(Cross::Text),
        };
        // Takes a number, an address and a boolean, and gives back the
        // address, so what comes out is the string if it landed in slot one.
        let module = standing_in(
            vec![ValType::I64, ValType::I64, ValType::I32],
            vec![ValType::I64],
            vec![Ins::LocalGet(1), Ins::Return],
        );
        let mut args = vec![Ins::I64Const(11)];
        args.extend(first());
        args.push(Ins::I32Const(1));

        assert_eq!(
            answered(module, mixed, args, characters_back()),
            5,
            "the string came back, so it went in where the function looks"
        );
    }

    /// And the values around it land where they belong too.
    ///
    /// The same shape, with the compiled function reporting the number and the
    /// boolean instead of the text, because a wrapper that got the string
    /// right can still have pushed the other two in the wrong order.
    #[test]
    fn the_values_around_a_string_land_where_they_belong() {
        let mixed = Crossing {
            name: "under_test",
            params: &[Cross::Signed64, Cross::Text, Cross::Bool],
            result: Some(Cross::Signed64),
        };
        let module = standing_in(
            vec![ValType::I64, ValType::I64, ValType::I32],
            vec![ValType::I64],
            vec![
                Ins::LocalGet(0),
                Ins::I64Const(10),
                Ins::I64Mul,
                Ins::LocalGet(2),
                Ins::I64ExtendI32S,
                Ins::I64Add,
                Ins::Return,
            ],
        );
        let mut args = vec![Ins::I64Const(11)];
        args.extend(first());
        args.push(Ins::I32Const(1));

        assert_eq!(answered(module, mixed, args, Vec::new()), 111);
    }

    /// Two strings at once are two values, not one written over the other.
    #[test]
    fn two_strings_do_not_overwrite_each_other() {
        let both = || Crossing {
            name: "under_test",
            params: &[Cross::Text, Cross::Text],
            result: Some(Cross::Text),
        };
        let giving_back = |slot: u32| {
            standing_in(
                vec![ValType::I64, ValType::I64],
                vec![ValType::I64],
                vec![Ins::LocalGet(slot), Ins::Return],
            )
        };
        let mut args = first();
        args.extend(second());

        assert_eq!(
            answered(giving_back(0), both(), args.clone(), characters_back()),
            5,
            "the first is still the first"
        );
        assert_eq!(
            answered(giving_back(1), both(), args.clone(), characters_back()),
            6,
            "and the second is still the second"
        );
        assert_eq!(
            answered(giving_back(1), both(), args.clone(), byte_back(0)),
            i64::from(b's'),
            "the second one's bytes are the second one's"
        );
        // The second value built has to go somewhere no parameter is, and a
        // slot chosen by counting the wrong way lands on the length the caller
        // passed. The character count survives that, because it is counted
        // before the slot is written; the byte count is the length itself, and
        // is what comes back wrong.
        assert_eq!(
            answered(giving_back(1), both(), args, bytes_back()),
            6,
            "and its length is its own"
        );
    }

    /// A result that is not text needs no return area, and the string going
    /// the other way still crosses.
    #[test]
    fn a_string_going_only_one_way_still_crosses() {
        let counting = Crossing {
            name: "under_test",
            params: &[Cross::Text],
            result: Some(Cross::Signed64),
        };
        // Reads the character count out of the value the wrapper built.
        let module = standing_in(
            vec![ValType::I64],
            vec![ValType::I64],
            vec![
                Ins::LocalGet(0),
                Ins::I32WrapI64,
                Ins::I64Load(0),
                Ins::Return,
            ],
        );
        assert_eq!(answered(module, counting, first(), Vec::new()), 5);
    }

    /// The allocator hands out room and moves on, which is the whole of what a
    /// caller lowering a string needs from it.
    #[test]
    fn the_allocator_hands_out_room_and_moves_on() {
        let mut module = echoing();
        adapt(&mut module, &[one_string()]).expect("adapters");
        let realloc = module
            .exports
            .iter()
            .find(|(name, _)| name == REALLOC)
            .expect("the allocator is exported")
            .1;

        let type_index = module.intern_type(FuncType {
            params: vec![],
            results: vec![ValType::I64],
        });
        let index = module.add_func(Func {
            type_index,
            locals: vec![],
            body: vec![
                // Ask twice, and give back the distance between the answers.
                Ins::I32Const(0),
                Ins::I32Const(0),
                Ins::I32Const(1),
                Ins::I32Const(9),
                Ins::Call(realloc),
                Ins::I32Const(0),
                Ins::I32Const(0),
                Ins::I32Const(1),
                Ins::I32Const(9),
                Ins::Call(realloc),
                Ins::I32Sub,
                Ins::I64ExtendI32S,
                Ins::Return,
            ],
        });
        module.export("twice", index);

        assert_eq!(
            crate::run::call(&module, "twice", &[]),
            Ok(Some(crate::run::Value::I64(-16))),
            "nine bytes rounded up to two words, and the second answer is above the first"
        );
    }

    /// And it keeps what was already there when a caller asks it to move
    /// something, which is the half of `cabi_realloc` that is not allocation.
    #[test]
    fn the_allocator_keeps_what_was_there() {
        let mut module = echoing();
        adapt(&mut module, &[one_string()]).expect("adapters");
        let realloc = module
            .exports
            .iter()
            .find(|(name, _)| name == REALLOC)
            .expect("the allocator is exported")
            .1;

        let type_index = module.intern_type(FuncType {
            params: vec![],
            results: vec![ValType::I64],
        });
        let index = module.add_func(Func {
            type_index,
            locals: vec![ValType::I32],
            body: vec![
                // Room for one byte, written into, then asked to grow.
                Ins::I32Const(0),
                Ins::I32Const(0),
                Ins::I32Const(1),
                Ins::I32Const(1),
                Ins::Call(realloc),
                Ins::LocalTee(0),
                Ins::I32Const(42),
                Ins::I32Store8(0),
                Ins::LocalGet(0),
                Ins::I32Const(1),
                Ins::I32Const(1),
                Ins::I32Const(32),
                Ins::Call(realloc),
                Ins::I32Load8U(0),
                Ins::I64ExtendI32S,
                Ins::Return,
            ],
        });
        module.export("moved", index);

        assert_eq!(
            crate::run::call(&module, "moved", &[]),
            Ok(Some(crate::run::Value::I64(42)))
        );
    }

    /// Well clear of where the bump pointer reaches in these tests, so a list
    /// written here is not something an allocation walks over.
    const AWAY: u32 = layout::HEAP_START + 4096;

    /// Three numbers as a caller passes them: the elements, and nothing in
    /// front of them.
    fn passed(count: i32) -> Vec<Ins> {
        let mut body = Vec::new();
        for at in 0..count {
            body.extend([
                Ins::I32Const(AWAY as i32 + at * layout::WORD as i32),
                Ins::I64Const(i64::from(at) + 1),
                Ins::I64Store(0),
            ]);
        }
        body.extend([Ins::I32Const(AWAY as i32), Ins::I32Const(count)]);
        body
    }

    /// One list parameter and a number back, so the answer can be anything
    /// the wrapper built.
    fn one_list() -> Crossing<'static> {
        Crossing {
            name: "under_test",
            params: &[Cross::Numbers],
            result: Some(Cross::Signed64),
        }
    }

    /// A list arrives with its length in front of the elements the caller
    /// passed, which is the layout everything else in this backend reads.
    #[test]
    fn a_list_arrives_with_its_length_and_its_elements() {
        let reads = |offset: u32| {
            standing_in(
                vec![ValType::I64],
                vec![ValType::I64],
                vec![
                    Ins::LocalGet(0),
                    Ins::I32WrapI64,
                    Ins::I64Load(offset),
                    Ins::Return,
                ],
            )
        };

        assert_eq!(answered(reads(0), one_list(), passed(3), vec![]), 3);
        for (at, expected) in [(1, 1), (2, 2), (3, 3)] {
            assert_eq!(
                answered(reads(at * layout::WORD), one_list(), passed(3), vec![]),
                expected,
                "element {at}"
            );
        }
    }

    /// An empty one is the case where the pointer points at no elements, and
    /// the length in front of them is the only thing written.
    #[test]
    fn an_empty_list_crosses_as_an_empty_list() {
        let module = standing_in(
            vec![ValType::I64],
            vec![ValType::I64],
            vec![
                Ins::LocalGet(0),
                Ins::I32WrapI64,
                Ins::I64Load(0),
                Ins::Return,
            ],
        );
        assert_eq!(answered(module, one_list(), passed(0), vec![]), 0);
    }

    /// Coming back, the boundary reads a pointer and a count out of a return
    /// area. The pointer is the list's own address a word along, because the
    /// word in front of the elements is the length and the caller is not
    /// expecting one.
    #[test]
    fn a_list_going_back_is_its_elements_and_a_count() {
        let stored = |count: i64| {
            let mut body = vec![
                Ins::I32Const(AWAY as i32),
                Ins::I64Const(count),
                Ins::I64Store(0),
            ];
            for at in 0..count {
                body.extend([
                    Ins::I32Const(AWAY as i32 + (at as i32 + 1) * layout::WORD as i32),
                    Ins::I64Const((at + 1) * 10),
                    Ins::I64Store(0),
                ]);
            }
            body.extend([Ins::I64Const(i64::from(AWAY)), Ins::Return]);
            standing_in(vec![], vec![ValType::I64], body)
        };

        let giving = || Crossing {
            name: "under_test",
            params: &[],
            result: Some(Cross::Numbers),
        };

        // The count, read out of the second half of the return area.
        assert_eq!(
            answered(
                stored(3),
                giving(),
                vec![],
                vec![Ins::I32Load(4), Ins::I64ExtendI32S]
            ),
            3
        );

        // The pointer, which is the list's address and a word.
        assert_eq!(
            answered(
                stored(3),
                giving(),
                vec![],
                vec![Ins::I32Load(0), Ins::I64ExtendI32S]
            ),
            i64::from(AWAY + layout::WORD)
        );

        // And what is actually at that pointer, which is the first element
        // rather than the length.
        assert_eq!(
            answered(
                stored(3),
                giving(),
                vec![],
                vec![Ins::I32Load(0), Ins::I64Load(0)]
            ),
            10
        );

        assert_eq!(
            answered(
                stored(0),
                giving(),
                vec![],
                vec![Ins::I32Load(4), Ins::I64ExtendI32S]
            ),
            0
        );
    }
}
