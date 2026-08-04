//! Folding a value that lives in memory into one number.
//!
//! The mirror of [`crate::equality`], and deliberately so: everywhere that
//! compares two values reads a word, this absorbs one, in the same order. That
//! correspondence is the whole reason `a == b` implies `hash(a) == hash(b)` —
//! it holds because the two files read a value the same way, which a reader
//! can check, rather than because somebody was careful.
//!
//! One function per shape, emitted like the runtime helpers and numbered after
//! them. Each takes the running hash and a value and answers the hash, so a
//! whole value is one fold rather than a combination of sub-hashes. The
//! interpreter folds once too and the two have to agree exactly.
//!
//! The running hash lives in a local rather than on the stack. A block in
//! WebAssembly cannot see what is under it, so a hash carried on the stack
//! could not cross the `if` that picks a variant without block parameters.
//!
//! The arithmetic is not here. `deed_rt::hashing` has the two constants and
//! both engines read them from it. See
//! `design/decisions/2026-08-05-a-hash-is-the-equality-walk.md`.

use deed_mir::{LayoutId, Program, Ty};

use crate::layout;
use crate::wasm::{FuncType, Ins, ValType};

/// `(hash, value) -> hash`, for a value of one shape.
pub(crate) fn signature() -> FuncType {
    FuncType {
        params: vec![ValType::I64, ValType::I64],
        results: vec![ValType::I64],
    }
}

/// Whether a shape needs a function of its own to be hashed.
///
/// The same question [`crate::equality::walked`] answers, asked of it rather
/// than answered again: a shape this walked and that compared without a walk
/// would be the two disagreeing about what a value is made of.
pub(crate) fn walked(ty: &Ty) -> bool {
    crate::equality::walked(ty)
}

/// Where each shape's fold and the text helper are numbered.
pub(crate) struct Where<'a> {
    pub(crate) hashes: &'a [(Ty, u32)],
    pub(crate) hash_text: u32,
}

impl Where<'_> {
    fn call(&self, ty: &Ty) -> Option<Ins> {
        self.hashes
            .iter()
            .find(|(shape, _)| shape == ty)
            .map(|(_, at)| Ins::Call(*at))
    }
}

const HASH: u32 = 0;
const VALUE: u32 = 1;
const H: u32 = 2;
const AT: u32 = 3;
const COUNT: u32 = 4;
const I: u32 = 5;

/// The locals a shape's fold needs beyond its two parameters.
fn locals() -> Vec<ValType> {
    vec![ValType::I64, ValType::I32, ValType::I32, ValType::I32]
}

/// `H = (H ^ what) * PRIME`, where `what` is whatever the given instructions
/// leave on the stack.
fn absorb(what: Vec<Ins>) -> Vec<Ins> {
    let mut ins = vec![Ins::LocalGet(H)];
    ins.extend(what);
    ins.extend([
        Ins::I64Xor,
        Ins::I64Const(deed_rt::hashing::PRIME as i64),
        Ins::I64Mul,
        Ins::LocalSet(H),
    ]);
    ins
}

/// Absorbs a name known at compile time, a byte at a time.
///
/// The bytes are constants, so this is the fold unrolled. Names are short and
/// there is one per field, which is a size a layout already is.
fn absorb_name(name: &str) -> Vec<Ins> {
    let mut ins = Vec::new();
    for byte in name.as_bytes() {
        ins.extend(absorb(vec![Ins::I64Const(i64::from(*byte))]));
    }
    ins
}

/// Absorbs one value of type `ty`, whose word the given instructions leave on
/// the stack.
fn absorb_value(loaded: Vec<Ins>, ty: &Ty, at: &Where<'_>) -> Option<Vec<Ins>> {
    let call = match ty {
        Ty::Str => Ins::Call(at.hash_text),
        other if walked(other) => at.call(other)?,
        _ => return Some(absorb(loaded)),
    };
    let mut ins = vec![Ins::LocalGet(H)];
    ins.extend(loaded);
    ins.extend([call, Ins::LocalSet(H)]);
    Some(ins)
}

/// The locals beyond the parameters, and the body.
pub(crate) fn compile(program: &Program, ty: &Ty, at: &Where<'_>) -> (Vec<ValType>, Vec<Ins>) {
    let body = match ty {
        Ty::Aggregate(id) => aggregate(program, *id, at),
        Ty::List(element) => list(element, at),
        // Refused before this is reached, the same way comparing one is.
        _ => None,
    };
    (
        locals(),
        body.unwrap_or_else(|| vec![Ins::LocalGet(HASH), Ins::Return]),
    )
}

/// Starts the fold from what was handed in, and narrows the address.
fn opened() -> Vec<Ins> {
    vec![
        Ins::LocalGet(HASH),
        Ins::LocalSet(H),
        Ins::LocalGet(VALUE),
        Ins::I32WrapI64,
        Ins::LocalSet(AT),
    ]
}

/// A record or a choice: the variant's name, then the fields it holds.
///
/// The name and not the tag, because a tag is a position in a layout and the
/// interpreter has no layout to count in. Two same-named variants declared in
/// two modules therefore hash alike and compare unequal, which is a collision
/// rather than a mistake.
///
/// Fields are absorbed in the order of their names rather than of their
/// declaration. Two records built in different orders are equal, and the
/// interpreter holds fields in a `BTreeMap`, so the order the two can agree on
/// is the one neither of them chose.
fn aggregate(program: &Program, id: LayoutId, at: &Where<'_>) -> Option<Vec<Ins>> {
    let held = program.layout(id);
    let tagged = held.is_tagged();

    let mut body = opened();

    for (index, variant) in held.variants.iter().enumerate() {
        // A choice's variant carries its name into the hash and a record's
        // does not, because the interpreter holds a variant with a name and
        // holds a record as bare fields. `choice` and not `is_tagged`: a
        // one-variant choice has nothing to tell apart and still has a name.
        let mut arm = if held.choice {
            absorb_name(&variant.name)
        } else {
            Vec::new()
        };
        arm.extend(absorb(vec![Ins::I64Const(variant.fields.len() as i64)]));

        let mut ordered: Vec<(usize, &deed_mir::Field)> =
            variant.fields.iter().enumerate().collect();
        ordered.sort_by(|(_, one), (_, other)| one.name.cmp(&other.name));

        for (position, one) in ordered {
            arm.extend(absorb_name(&one.name));
            arm.extend(absorb_value(
                vec![
                    Ins::LocalGet(AT),
                    Ins::I64Load(layout::field_offset(tagged, position)),
                ],
                &one.ty,
                at,
            )?);
        }

        if !tagged {
            body.extend(arm);
            continue;
        }
        body.extend([
            Ins::LocalGet(AT),
            Ins::I64Load(0),
            Ins::I64Const(index as i64),
            Ins::I64Eq,
        ]);
        body.push(Ins::If {
            result: None,
            then: arm,
            otherwise: Vec::new(),
        });
    }

    body.extend([Ins::LocalGet(H), Ins::Return]);
    Some(body)
}

/// A list: its length, then every element.
fn list(element: &Ty, at: &Where<'_>) -> Option<Vec<Ins>> {
    let mut body = opened();
    body.extend(absorb(vec![Ins::LocalGet(AT), Ins::I64Load(0)]));
    body.extend([
        Ins::LocalGet(AT),
        Ins::I64Load(0),
        Ins::I32WrapI64,
        Ins::LocalSet(COUNT),
        Ins::I32Const(0),
        Ins::LocalSet(I),
    ]);

    let mut turn = vec![
        Ins::LocalGet(I),
        Ins::LocalGet(COUNT),
        Ins::I32GeS,
        Ins::BrIf(1),
    ];
    let mut read = element_address();
    read.push(Ins::I64Load(0));
    turn.extend(absorb_value(read, element, at)?);
    turn.extend([
        Ins::LocalGet(I),
        Ins::I32Const(1),
        Ins::I32Add,
        Ins::LocalSet(I),
        Ins::Br(0),
    ]);

    body.push(Ins::Block {
        result: None,
        body: vec![Ins::Loop {
            result: None,
            body: turn,
        }],
    });
    body.extend([Ins::LocalGet(H), Ins::Return]);
    Some(body)
}

/// The address of element `I` of the list in `AT`.
fn element_address() -> Vec<Ins> {
    vec![
        Ins::LocalGet(AT),
        Ins::I32Const(layout::element_offset(0) as i32),
        Ins::I32Add,
        Ins::LocalGet(I),
        Ins::I32Const(layout::WORD as i32),
        Ins::I32Mul,
        Ins::I32Add,
    ]
}
