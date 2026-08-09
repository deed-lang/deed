//! A component binary around a core module.
//!
//! `design/decisions/2026-08-07-a-wit-world-is-not-a-component.md` measured
//! what `--component` used to write: a core module and a `.wit`, which a real
//! toolchain turned into a component exporting nothing. The world derivation
//! was never the gap. The gap was that nothing wrapped the module in the
//! format a component runtime reads.
//!
//! This writes that wrapper. It is the whole of a component for exports whose
//! values already cross the boundary the way the canonical ABI says they do,
//! which is the scalars: a number is an `s64` on both sides and a boolean is a
//! `bool` on both sides, so lifting one is a declaration rather than a
//! conversion.
//!
//! It is deliberately not the whole of a component for anything else. A string
//! or a list crosses as a pointer and a length into memory the caller helped
//! allocate, through `cabi_realloc`, and this backend passes one address in
//! its own layout instead. Lifting those without the adapters would produce a
//! component that answers wrongly, and the record this replaces says in as
//! many words that answering wrongly is worse than not answering. So the
//! caller checks first, and what arrives here is already only what fits.
//!
//! The encoding is the component model's binary format, and every constant in
//! it was read out of a component the Bytecode Alliance's own tooling built
//! rather than out of the specification: `crates/deed-codegen/component.mjs`
//! builds one the same way on every commit and compares the worlds.

use crate::wasm::{Module, write_u32};

/// What a lifted export carries across, on both sides.
///
/// Only the types that need no adapter. The name is the whole argument: these
/// are the ones where the core value and the component value are the same
/// value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Flat {
    /// `bool`. One core `i32`, zero or one.
    Bool,
    /// `s64`. One core `i64`.
    Signed64,
}

impl Flat {
    /// The component model's byte for this type.
    fn byte(self) -> u8 {
        match self {
            Flat::Bool => 0x7f,
            Flat::Signed64 => 0x78,
        }
    }
}

/// One function the component exports, as it appears on the boundary.
#[derive(Clone, Debug)]
pub struct Lifted {
    /// The name the component exports it under, and the name the core module
    /// exports it under. They are the same name; a component that renamed
    /// them would be describing a module other than the one it holds.
    pub name: String,
    /// Parameter types in order. The names are generated, because a Deed
    /// parameter name is not part of what the module exports.
    pub params: Vec<Flat>,
    /// The single result, or none. The component model allows a list; this
    /// language returns one value or nothing.
    pub result: Option<Flat>,
}

/// Wraps `core` in a component that exports `lifted`.
///
/// The core module goes in verbatim. Nothing about it changes: the same bytes
/// `deed build` writes are the bytes inside this, which is what keeps the two
/// commands describing one compiler rather than two.
pub fn encode(core: &Module, lifted: &[Lifted]) -> Vec<u8> {
    let mut out = Vec::new();

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
    let mut aliases = Vec::new();
    write_u32(&mut aliases, lifted.len() as u32);
    for one in lifted {
        aliases.extend_from_slice(&[0x00, 0x00, 0x01, 0x00]);
        name(&mut aliases, &one.name);
    }
    section(&mut out, 6, &aliases);

    // A component function type per export. Parameters are named because the
    // format names them; `p0`, `p1` and so on, which is what the `.wit` beside
    // the file already says, for the same reason.
    let mut types = Vec::new();
    write_u32(&mut types, lifted.len() as u32);
    for one in lifted {
        types.push(0x40);
        write_u32(&mut types, one.params.len() as u32);
        for (at, param) in one.params.iter().enumerate() {
            name(&mut types, &format!("p{at}"));
            types.push(param.byte());
        }
        match one.result {
            Some(result) => {
                types.push(0x00);
                types.push(result.byte());
            }
            // Not a zero-length list of results: the format spells "none" with
            // its own pair of bytes.
            None => types.extend_from_slice(&[0x01, 0x00]),
        }
    }
    section(&mut out, 7, &types);

    // The lift itself. No options, which is the whole reason only these types
    // are here: options are where a memory and a `cabi_realloc` would go.
    let mut canon = Vec::new();
    write_u32(&mut canon, lifted.len() as u32);
    for at in 0..lifted.len() as u32 {
        canon.extend_from_slice(&[0x00, 0x00]);
        write_u32(&mut canon, at);
        canon.push(0x00);
        write_u32(&mut canon, at);
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
            params: vec![Flat::Signed64, Flat::Signed64],
            result: Some(Flat::Signed64),
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
                params: vec![],
                result: None,
            }],
        );
        let types = payload(&bytes, 7);
        assert_eq!(types, vec![0x01, 0x40, 0x00, 0x01, 0x00]);
    }

    /// The two types this can carry are two different bytes, and swapping them
    /// is a component that lies about what it takes.
    #[test]
    fn the_two_flat_types_are_the_bytes_the_format_gives_them() {
        assert_eq!(Flat::Bool.byte(), 0x7f);
        assert_eq!(Flat::Signed64.byte(), 0x78);
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
