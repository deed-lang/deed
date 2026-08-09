//! A component binary around a core module.
//!
//! `design/decisions/2026-08-07-a-wit-world-is-not-a-component.md` measured
//! what `--component` used to write: a core module and a `.wit`, which a real
//! toolchain turned into a component exporting nothing. The world derivation
//! was never the gap. The gap was that nothing wrapped the module in the
//! format a component runtime reads.
//!
//! This writes that wrapper. For exports whose values already cross the
//! boundary the way the canonical ABI says they do -- the scalars, where a
//! number is an `s64` on both sides and a boolean is a `bool` on both sides --
//! lifting is a declaration and this is the whole of it.
//!
//! A string is not one of those. It crosses as a pointer and a length into
//! memory the caller asked the callee to allocate, and this backend passes one
//! address to a header and some bytes. [`crate::adapter`] is what stands
//! between the two, and what this adds when it is there: an alias for the
//! memory and one for `cabi_realloc`, and a lift that names both. An export
//! carrying nothing that needs them lifts with no options, exactly as before.
//!
//! The encoding is the component model's binary format, and every constant in
//! it was read out of a component the Bytecode Alliance's own tooling built
//! rather than out of the specification: `crates/deed-codegen/component.mjs`
//! builds one the same way on every commit, reads its world, and calls it.

use crate::adapter::REALLOC;
use crate::wasm::{Module, write_u32};

pub use crate::adapter::Cross;

/// One function the component exports, as it appears on the boundary.
#[derive(Clone, Debug)]
pub struct Lifted {
    /// The name the component exports it under.
    pub name: String,
    /// The core module's export this lifts, which is the same name when
    /// nothing needed an adapter and the wrapper's name when something did.
    pub core: String,
    /// Parameter types in order. The names are generated, because a Deed
    /// parameter name is not part of what the module exports.
    pub params: Vec<Cross>,
    /// The single result, or none. The component model allows a list; this
    /// language returns one value or nothing.
    pub result: Option<Cross>,
}

impl Lifted {
    /// Whether lifting this one has to name the module's memory and
    /// allocator.
    ///
    /// Exactly when something it carries does not cross unchanged. A lift
    /// that named them anyway would be describing a boundary that is not
    /// there, and one that failed to name them where they are needed is a
    /// component a runtime refuses to instantiate.
    fn needs_the_memory(&self) -> bool {
        self.params.iter().any(|one| !one.crosses_unchanged())
            || self.result.is_some_and(|one| !one.crosses_unchanged())
    }
}

/// Wraps `core` in a component that exports `lifted`.
///
/// The core module goes in whole. Nothing in it is rewritten: what `deed
/// build` writes is inside this, and when an export needed adapters they were
/// appended to it, so every function that was there is there under the name
/// and at the index it had. That is what keeps the two commands describing one
/// compiler rather than two.
pub fn encode(core: &Module, lifted: &[Lifted]) -> Vec<u8> {
    let mut out = Vec::new();
    // The allocator's presence is how this tells an adapted module from one
    // that needed nothing, rather than a flag a caller could get wrong.
    let adapted = core.exports.iter().any(|(name, _)| name == REALLOC);
    let first = u32::from(adapted);

    // `\0asm`, then a version of 13 and a layer of 1. The layer is what tells
    // a reader this is a component rather than a module; a core module writes
    // a version of 1 and a layer of 0 in the same four bytes.
    out.extend_from_slice(&[0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00]);

    // The core module, whole, as section 1's payload.
    section(&mut out, 1, &core.encode());

    // One instance of it, instantiated with nothing, because a component this
    // shape imports nothing.
    section(&mut out, 2, &[0x01, 0x00, 0x00, 0x00]);

    // An alias per export, which is how a core function becomes something the
    // component's own index space can name. `00 00` is the core sort and the
    // core function sort; `01` is "an export of a core instance".
    //
    // When there are adapters, the memory and the allocator come first: a
    // lift's options name them by index, and the indexes are handed out here.
    // The memory has an index space of its own, so it takes nothing from the
    // functions; the allocator takes function zero and moves every export up
    // one, which is what `first` is.
    let mut aliases = Vec::new();
    write_u32(
        &mut aliases,
        lifted.len() as u32 + if adapted { 2 } else { 0 },
    );
    if adapted {
        aliases.extend_from_slice(&[0x00, 0x02, 0x01, 0x00]);
        name(&mut aliases, "memory");
        aliases.extend_from_slice(&[0x00, 0x00, 0x01, 0x00]);
        name(&mut aliases, REALLOC);
    }
    for one in lifted {
        aliases.extend_from_slice(&[0x00, 0x00, 0x01, 0x00]);
        name(&mut aliases, &one.core);
    }
    section(&mut out, 6, &aliases);

    // A component function type per export, and in front of them the types
    // that are not a single byte. A value type is written as a primitive byte
    // or as an index into this section, so anything with a shape of its own
    // is spelled out first and pointed at afterwards.
    let mut defined: Vec<Cross> = Vec::new();
    for one in lifted {
        for carried in one.params.iter().chain(one.result.iter()) {
            if carried.definition().is_some() && !defined.contains(carried) {
                defined.push(*carried);
            }
        }
    }

    let mut types = Vec::new();
    write_u32(&mut types, (defined.len() + lifted.len()) as u32);
    for carried in &defined {
        types.extend_from_slice(carried.definition().expect("it is in this list for that"));
    }
    for one in lifted {
        types.push(0x40);
        write_u32(&mut types, one.params.len() as u32);
        for (at, param) in one.params.iter().enumerate() {
            name(&mut types, &format!("p{at}"));
            valtype(&mut types, param, &defined);
        }
        match &one.result {
            Some(result) => {
                types.push(0x00);
                valtype(&mut types, result, &defined);
            }
            // Not a zero-length list of results: the format spells "none"
            // with its own pair of bytes.
            None => types.extend_from_slice(&[0x01, 0x00]),
        }
    }
    section(&mut out, 7, &types);

    // The lift itself. The options are where the memory and the allocator go;
    // an export carrying nothing that needs either says it has none, which is
    // what every component this wrote before said.
    let mut canon = Vec::new();
    write_u32(&mut canon, lifted.len() as u32);
    for (at, one) in lifted.iter().enumerate() {
        canon.extend_from_slice(&[0x00, 0x00]);
        write_u32(&mut canon, at as u32 + first);
        if one.needs_the_memory() {
            // `03` a memory and `04` a `cabi_realloc`, each followed by the
            // index the alias above gave it. UTF-8 is what a lift is read
            // with when it does not say otherwise, and it is what this
            // backend stores, so it does not say.
            write_u32(&mut canon, 2);
            canon.push(0x03);
            write_u32(&mut canon, 0);
            canon.push(0x04);
            write_u32(&mut canon, 0);
        } else {
            canon.push(0x00);
        }
        write_u32(&mut canon, (defined.len() + at) as u32);
    }
    section(&mut out, 8, &canon);

    // And the exports, which are what a reader of the component sees.
    let mut exports = Vec::new();
    write_u32(&mut exports, lifted.len() as u32);
    for (at, one) in lifted.iter().enumerate() {
        exports.push(0x00);
        name(&mut exports, &one.name);
        exports.push(0x01);
        write_u32(&mut exports, at as u32);
        exports.push(0x00);
    }
    section(&mut out, 11, &exports);

    out
}

/// A section: one byte of identity, a length, and the bytes.
fn section(out: &mut Vec<u8>, id: u8, payload: &[u8]) {
    out.push(id);
    write_u32(out, payload.len() as u32);
    out.extend_from_slice(payload);
}

/// A name: a length and the bytes, the same as a core module spells one.
fn name(out: &mut Vec<u8>, text: &str) {
    write_u32(out, text.len() as u32);
    out.extend_from_slice(text.as_bytes());
}

/// One value type: a primitive's byte, or the index of a type defined above.
fn valtype(out: &mut Vec<u8>, carried: &Cross, defined: &[Cross]) {
    match carried.byte() {
        Some(byte) => out.push(byte),
        None => {
            let at = defined
                .iter()
                .position(|one| one == carried)
                .expect("a type with no byte of its own was written out above");
            write_u32(out, at as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::{Func, FuncType, Ins, Module, ValType};

    /// A module with one function, so a test has something to wrap.
    fn adder() -> Module {
        let mut module = Module::new();
        let ty = module.intern_type(FuncType {
            params: vec![ValType::I64, ValType::I64],
            results: vec![ValType::I64],
        });
        let index = module.add_func(Func {
            type_index: ty,
            locals: vec![],
            body: vec![Ins::LocalGet(0), Ins::LocalGet(1), Ins::I64Add],
        });
        module.export("add", index);
        module
    }

    /// A second function, so a test can watch every index move.
    fn with_nothing(module: &mut Module) {
        let ty = module.intern_type(FuncType {
            params: vec![],
            results: vec![],
        });
        let index = module.add_func(Func {
            type_index: ty,
            locals: vec![],
            body: vec![],
        });
        module.export("nothing", index);
    }

    fn lifted() -> Vec<Lifted> {
        vec![Lifted {
            name: "add".to_string(),
            core: "add".to_string(),
            params: vec![Cross::Signed64, Cross::Signed64],
            result: Some(Cross::Signed64),
        }]
    }

    /// The four bytes that say this is a component and not a module.
    ///
    /// A core module writes `01 00 00 00` here, so a reader that gets this
    /// wrong does not get a wrong component, it gets a module that fails to
    /// load. Worth a test of its own for that reason.
    #[test]
    fn it_starts_with_the_component_preamble() {
        let bytes = encode(&adder(), &lifted());
        assert_eq!(&bytes[0..4], b"\0asm");
        assert_eq!(&bytes[4..8], &[0x0d, 0x00, 0x01, 0x00]);
    }

    /// The core module goes in unchanged, which is the claim that makes this
    /// one compiler rather than two.
    #[test]
    fn it_carries_the_core_module_verbatim() {
        let core = adder();
        let inside = core.encode();
        let bytes = encode(&core, &lifted());

        let at = bytes
            .windows(inside.len())
            .position(|window| window == inside)
            .expect("the core module is in there");
        // Immediately after the preamble and section 1's header, rather than
        // anywhere: a component reads its sections in order.
        assert_eq!(bytes[8], 1, "the first section is the core module");
        assert!(at > 8 && at < 8 + 6, "at {at}");
    }

    /// Every section this writes, in the order the format reads them.
    ///
    /// Order is not a style question here. Aliases make core functions
    /// nameable, types have to exist before a lift names one, and an export
    /// names a lift, so a section out of place is a component that does not
    /// load.
    #[test]
    fn the_sections_come_in_the_order_the_indexes_need() {
        let bytes = encode(&adder(), &lifted());
        let mut seen = Vec::new();
        let mut at = 8;
        while at < bytes.len() {
            seen.push(bytes[at]);
            at += 1;
            let (size, width) = read_u32(&bytes[at..]);
            at += width + size as usize;
        }
        assert_eq!(seen, vec![1, 2, 6, 7, 8, 11]);
    }

    /// A second export moves every index, so one that is written as a constant
    /// passes the test above and produces a component wired to the wrong
    /// function.
    #[test]
    fn a_second_export_lifts_the_second_function() {
        let mut core = adder();
        with_nothing(&mut core);

        let mut two = lifted();
        two.push(Lifted {
            name: "nothing".to_string(),
            core: "nothing".to_string(),
            params: vec![],
            result: None,
        });
        let bytes = encode(&core, &two);

        // The canon section, read back: two lifts, the second of core function
        // one with type one.
        let canon = payload(&bytes, 8);
        assert_eq!(
            canon,
            vec![
                0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01
            ]
        );
    }

    /// Nothing to give back is spelled with its own bytes rather than an empty
    /// list, which is the one place this format does not do what a core module
    /// does.
    #[test]
    fn a_function_with_no_result_says_so_the_way_the_format_does() {
        let mut core = adder();
        with_nothing(&mut core);

        let bytes = encode(
            &core,
            &[Lifted {
                name: "nothing".to_string(),
                core: "nothing".to_string(),
                params: vec![],
                result: None,
            }],
        );
        let types = payload(&bytes, 7);
        assert_eq!(types, vec![0x01, 0x40, 0x00, 0x01, 0x00]);
    }

    /// A module with adapters in it: the memory and the allocator are aliased
    /// before anything else, and the allocator taking core function zero is
    /// what moves every export up one.
    ///
    /// A lift left pointing at the function it pointed at before the allocator
    /// arrived calls the allocator instead, which is a component that answers
    /// with an address where a caller expects text.
    #[test]
    fn the_allocator_takes_the_first_core_function_and_moves_the_exports_up() {
        let mut core = adder();
        core.export(REALLOC, 0);

        let bytes = encode(
            &core,
            &[Lifted {
                name: "greet".to_string(),
                core: "greet.lift".to_string(),
                params: vec![Cross::Text],
                result: Some(Cross::Text),
            }],
        );

        let aliases = payload(&bytes, 6);
        assert_eq!(
            &aliases[..1],
            &[0x03],
            "three aliases: the memory, the allocator, and the export"
        );
        assert_eq!(
            &aliases[1..5],
            &[0x00, 0x02, 0x01, 0x00],
            "the memory is a core memory alias"
        );

        let canon = payload(&bytes, 8);
        assert_eq!(
            canon,
            vec![0x01, 0x00, 0x00, 0x01, 0x02, 0x03, 0x00, 0x04, 0x00, 0x00],
            "one lift, of core function one, with a memory and an allocator"
        );
    }

    /// An export that carries nothing needing them still lifts with no
    /// options, even in a module that has them. Options naming a boundary an
    /// export does not cross are a description of something that is not there.
    #[test]
    fn an_export_that_crosses_unchanged_lifts_without_options() {
        let mut core = adder();
        core.export(REALLOC, 0);

        let bytes = encode(&core, &lifted());
        let canon = payload(&bytes, 8);
        assert_eq!(canon, vec![0x01, 0x00, 0x00, 0x01, 0x00, 0x00]);
    }

    /// A string in either direction is what asks for them, and nothing else
    /// does.
    #[test]
    fn text_is_what_makes_a_lift_name_the_memory() {
        let one = |params: Vec<Cross>, result: Option<Cross>| {
            Lifted {
                name: "f".to_string(),
                core: "f".to_string(),
                params,
                result,
            }
            .needs_the_memory()
        };
        assert!(!one(vec![Cross::Signed64], Some(Cross::Bool)));
        assert!(one(vec![Cross::Text], Some(Cross::Bool)));
        assert!(one(vec![Cross::Signed64], Some(Cross::Text)));
        assert!(one(vec![Cross::Bool, Cross::Text], None));
        assert!(one(vec![Cross::Numbers], Some(Cross::Bool)));
        assert!(one(vec![Cross::Signed64], Some(Cross::Numbers)));
    }

    /// A type with a shape of its own is written out and pointed at.
    ///
    /// A primitive is one byte in place; anything else is an entry in this
    /// section and an index afterwards. So the entries come first and every
    /// function type moves along by as many, including the index the lift
    /// names — which is the byte that decides whether a runtime reads the
    /// right signature or a list as though it were a function.
    #[test]
    fn a_list_is_written_out_and_the_types_after_it_move_along() {
        let mut core = adder();
        core.export(REALLOC, 0);
        let carrying = vec![Lifted {
            name: "doubled".to_string(),
            core: "doubled.lift".to_string(),
            params: vec![Cross::Numbers],
            result: Some(Cross::Numbers),
        }];

        let types = payload(&encode(&core, &carrying), 7);
        assert_eq!(
            types,
            vec![
                // Two types: the list, and the function that carries it.
                0x02, //
                // `70` a list, `78` the `s64` in it.
                0x70, 0x78, //
                // `40` a function, one parameter named `p0`, and both its
                // type and the result's written as index 0 rather than as a
                // byte.
                0x40, 0x01, 0x02, b'p', b'0', 0x00, 0x00, 0x00,
            ]
        );

        // And the lift names the function type, which is now the second.
        let canon = payload(&encode(&core, &carrying), 8);
        assert_eq!(canon.last(), Some(&0x01));
    }

    /// Nothing changes for a component carrying only primitives, which is
    /// what keeps the ones that worked before byte for byte what they were.
    #[test]
    fn a_component_carrying_only_primitives_writes_the_types_it_used_to() {
        let types = payload(&encode(&adder(), &lifted()), 7);
        assert_eq!(
            types,
            vec![
                0x01, 0x40, 0x02, 0x02, b'p', b'0', 0x78, 0x02, b'p', b'1', 0x78, 0x00, 0x78
            ]
        );
    }

    /// The payload of the first section with this id.
    fn payload(bytes: &[u8], id: u8) -> Vec<u8> {
        let mut at = 8;
        while at < bytes.len() {
            let here = bytes[at];
            at += 1;
            let (size, width) = read_u32(&bytes[at..]);
            at += width;
            if here == id {
                return bytes[at..at + size as usize].to_vec();
            }
            at += size as usize;
        }
        panic!("no section {id}");
    }

    /// Unsigned LEB128, and how many bytes it took.
    fn read_u32(bytes: &[u8]) -> (u32, usize) {
        let mut value = 0u32;
        let mut shift = 0;
        for (at, byte) in bytes.iter().enumerate() {
            value |= u32::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return (value, at + 1);
            }
            shift += 7;
        }
        panic!("unterminated leb128");
    }
}
