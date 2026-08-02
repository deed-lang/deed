//! Comparing two values that live in memory.
//!
//! Two addresses being equal is not two records being equal, and equality in
//! this language is structural, so the backend refused rather than answering
//! the wrong question. What it needed was a comparison per shape: a record
//! knows its fields, a choice knows its variants, and a list knows what it
//! holds, and none of that survives into a value at runtime.
//!
//! So one function per shape, written here and emitted like the runtime
//! helpers, numbered after them. A shape that holds another shape calls that
//! shape's function, which is why they are collected transitively before any
//! of them is numbered.

use deed_mir::{LayoutId, Program, Ty};

use crate::layout;
use crate::wasm::{FuncType, Ins, ValType};

/// `(a, b) -> Bool`, for a value of one shape.
pub(crate) fn signature() -> FuncType {
    FuncType {
        params: vec![ValType::I64, ValType::I64],
        results: vec![ValType::I32],
    }
}

/// Whether a shape needs a function of its own to be compared.
///
/// A number, a boolean and a capability are a word each and compare as one.
/// Text has a helper already. Everything else is a walk.
pub(crate) fn walked(ty: &Ty) -> bool {
    matches!(ty, Ty::Aggregate(_) | Ty::List(_))
}

/// Every shape a program compares two of, and every shape those hold.
///
/// In a fixed order, so the same program compiles to the same bytes: the
/// order shapes were first reached, which is the order the walk finds them.
pub(crate) fn close_over(program: &Program, found: &mut Vec<Ty>) {
    let mut at = 0;
    while at < found.len() {
        for ty in held_by(program, &found[at]) {
            if walked(&ty) && !found.contains(&ty) {
                found.push(ty);
            }
        }
        at += 1;
    }
}

/// What one shape holds, once.
fn held_by(program: &Program, ty: &Ty) -> Vec<Ty> {
    match ty {
        Ty::List(element) => vec![(**element).clone()],
        Ty::Aggregate(id) => program
            .layout(*id)
            .variants
            .iter()
            .flat_map(|variant| variant.fields.iter().map(|field| field.ty.clone()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Where each shape's comparison and each helper is numbered.
pub(crate) struct Where<'a> {
    pub(crate) equals: &'a [(Ty, u32)],
    pub(crate) same_text: u32,
}

impl Where<'_> {
    fn call(&self, ty: &Ty) -> Option<Ins> {
        self.equals
            .iter()
            .find(|(shape, _)| shape == ty)
            .map(|(_, at)| Ins::Call(*at))
    }
}

/// The locals beyond the parameters, and the body.
///
/// Two values of a shape this cannot compare answer `false`, which never
/// happens: `helpers_used` refuses a program holding one before any of this
/// is reached.
pub(crate) fn compile(program: &Program, ty: &Ty, at: &Where<'_>) -> (Vec<ValType>, Vec<Ins>) {
    match ty {
        Ty::Aggregate(id) => aggregate(program, *id, at),
        Ty::List(element) => list(element, at),
        _ => (Vec::new(), vec![Ins::I32Const(0), Ins::Return]),
    }
}

/// Both parameters as narrow addresses, which is what every read needs.
const LEFT: u32 = 0;
const RIGHT: u32 = 1;
const A: u32 = 2;
const B: u32 = 3;
const COUNT: u32 = 4;
const I: u32 = 5;

fn narrowed() -> Vec<Ins> {
    vec![
        Ins::LocalGet(LEFT),
        Ins::I32WrapI64,
        Ins::LocalSet(A),
        Ins::LocalGet(RIGHT),
        Ins::I32WrapI64,
        Ins::LocalSet(B),
    ]
}

/// Reads one word out of each side, at the same offset, and leaves the two
/// on the stack.
fn both(offset: u32) -> Vec<Ins> {
    vec![
        Ins::LocalGet(A),
        Ins::I64Load(offset),
        Ins::LocalGet(B),
        Ins::I64Load(offset),
    ]
}

/// Answers `false` unless what `test` pushes holds.
fn unless(test: Vec<Ins>) -> Vec<Ins> {
    let mut ins = test;
    ins.push(Ins::I32Eqz);
    ins.push(Ins::If {
        result: None,
        then: vec![Ins::I32Const(0), Ins::Return],
        otherwise: Vec::new(),
    });
    ins
}

/// Whether the two words at `offset` are equal, for a field of type `ty`.
fn field(offset: u32, ty: &Ty, at: &Where<'_>) -> Vec<Ins> {
    match ty {
        Ty::Unit => Vec::new(),
        Ty::Str => {
            let mut ins = both(offset);
            ins.push(Ins::Call(at.same_text));
            unless(ins)
        }
        other if walked(other) => {
            let mut ins = both(offset);
            match at.call(other) {
                Some(call) => ins.push(call),
                None => return vec![Ins::I32Const(0), Ins::Return],
            }
            unless(ins)
        }
        _ => {
            let mut ins = both(offset);
            ins.push(Ins::I64Eq);
            unless(ins)
        }
    }
}

/// A record or a choice: the tag first, then every field of the variant it
/// turned out to hold.
fn aggregate(program: &Program, id: LayoutId, at: &Where<'_>) -> (Vec<ValType>, Vec<Ins>) {
    let held = program.layout(id);
    let tagged = held.is_tagged();

    let mut body = narrowed();
    if tagged {
        let mut tag = both(0);
        tag.push(Ins::I64Eq);
        body.extend(unless(tag));
    }

    for (index, variant) in held.variants.iter().enumerate() {
        let mut fields = Vec::new();
        for (position, one) in variant.fields.iter().enumerate() {
            fields.extend(field(layout::field_offset(tagged, position), &one.ty, at));
        }
        if !tagged {
            body.extend(fields);
            continue;
        }
        // Which variant it is decides which fields there are to compare, and
        // both sides hold the same one because the tag already agreed.
        body.extend([
            Ins::LocalGet(A),
            Ins::I64Load(0),
            Ins::I64Const(index as i64),
            Ins::I64Eq,
        ]);
        body.push(Ins::If {
            result: None,
            then: fields,
            otherwise: Vec::new(),
        });
    }

    body.extend([Ins::I32Const(1), Ins::Return]);
    (vec![ValType::I32, ValType::I32], body)
}

/// A list: the same length, then every element.
fn list(element: &Ty, at: &Where<'_>) -> (Vec<ValType>, Vec<Ins>) {
    let locals = vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32];
    let mut body = narrowed();
    let mut length = both(0);
    length.push(Ins::I64Eq);
    body.extend(unless(length));

    // A list of nothing is as equal as its length says. There is no element
    // to read and nothing a read would find.
    if matches!(element, Ty::Unit) {
        body.extend([Ins::I32Const(1), Ins::Return]);
        return (locals, body);
    }

    body.extend([
        Ins::LocalGet(A),
        Ins::I64Load(0),
        Ins::I32WrapI64,
        Ins::LocalSet(COUNT),
        Ins::I32Const(0),
        Ins::LocalSet(I),
    ]);

    // The address of the element the counter is at, on both sides, moved by
    // the counter rather than read at a fixed offset.
    let step = |slot: u32| {
        vec![
            Ins::LocalGet(slot),
            Ins::I32Const(layout::element_offset(0) as i32),
            Ins::I32Add,
            Ins::LocalGet(I),
            Ins::I32Const(layout::WORD as i32),
            Ins::I32Mul,
            Ins::I32Add,
        ]
    };

    let mut turn = step(A);
    turn.push(Ins::I64Load(0));
    turn.extend(step(B));
    turn.push(Ins::I64Load(0));
    let compare = match element {
        Ty::Str => vec![Ins::Call(at.same_text)],
        other if walked(other) => match at.call(other) {
            Some(call) => vec![call],
            None => return (locals, vec![Ins::I32Const(0), Ins::Return]),
        },
        _ => vec![Ins::I64Eq],
    };
    turn.extend(compare);
    let mut inner = unless(turn);
    inner.extend([
        Ins::LocalGet(I),
        Ins::I32Const(1),
        Ins::I32Add,
        Ins::LocalSet(I),
    ]);

    let mut walk = vec![Ins::LocalGet(I), Ins::LocalGet(COUNT), Ins::I32GeS];
    walk.push(Ins::BrIf(1));
    walk.extend(inner);
    walk.push(Ins::Br(0));
    body.push(Ins::Block {
        result: None,
        body: vec![Ins::Loop {
            result: None,
            body: walk,
        }],
    });

    body.extend([Ins::I32Const(1), Ins::Return]);
    (locals, body)
}

/// Whether comparing these shapes needs the text helper anywhere inside.
pub(crate) fn needs_text(program: &Program, shapes: &[Ty]) -> bool {
    shapes.iter().any(|ty| {
        held_by(program, ty)
            .iter()
            .any(|held| matches!(held, Ty::Str))
    })
}
