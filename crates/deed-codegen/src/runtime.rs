//! Functions the backend writes, rather than the program.
//!
//! A string is bytes in memory, so comparing two of them or joining them is a
//! loop, and a loop is not something to write again at every call site. These
//! are compiled into the module like any other function and called by index.
//!
//! They are appended after everything the program declares, so a helper's
//! index is `imports + program functions + its position here` and adding one
//! moves nothing. Only the ones a program reaches are emitted, the way an
//! import is only declared when something calls it, because a module that
//! carries a string comparison it never makes is a module that got bigger for
//! nothing.
//!
//! Nothing here allocates through the same path a lowered expression does.
//! The bump pointer is a word in memory and these read and write it directly,
//! which is the whole of what allocation is in this backend.

use crate::layout;
use crate::wasm::{FuncType, Ins, ValType};

/// A string's bytes start after its two headers.
const TEXT: u32 = 2 * layout::WORD;

/// One of the functions this module writes.
///
/// The order is the order they are emitted in, and it is fixed so that two
/// compilations of the same program produce the same bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) enum Helper {
    /// Whether two strings hold the same characters.
    SameText,
    /// Which of two strings comes first: -1, 0 or 1.
    TextOrder,
    /// A new string holding one after the other.
    JoinedText,
}

impl Helper {
    /// Every helper, in the order they are emitted.
    pub(crate) const ALL: &'static [Helper] =
        &[Helper::SameText, Helper::TextOrder, Helper::JoinedText];

    /// The name a reader of the module sees in a trap.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Helper::SameText => deed_mir::runtime::STR_EQ,
            Helper::TextOrder => deed_mir::runtime::STR_CMP,
            Helper::JoinedText => deed_mir::runtime::STR_CONCAT,
        }
    }

    pub(crate) fn signature(self) -> FuncType {
        match self {
            Helper::SameText => FuncType {
                params: vec![ValType::I64, ValType::I64],
                results: vec![ValType::I32],
            },
            Helper::TextOrder => FuncType {
                params: vec![ValType::I64, ValType::I64],
                results: vec![ValType::I64],
            },
            Helper::JoinedText => FuncType {
                params: vec![ValType::I64, ValType::I64],
                results: vec![ValType::I64],
            },
        }
    }

    /// The helpers this one calls, so emitting it emits them too.
    pub(crate) fn needs(self) -> &'static [Helper] {
        &[]
    }

    /// The locals beyond the parameters, and the body.
    pub(crate) fn compile(self) -> (Vec<ValType>, Vec<Ins>) {
        match self {
            Helper::SameText => str_eq(),
            Helper::TextOrder => str_cmp(),
            Helper::JoinedText => str_concat(),
        }
    }
}

/// The byte length of the string whose address is in `slot`, as an `i32`.
fn byte_length(slot: u32) -> Vec<Ins> {
    vec![
        Ins::LocalGet(slot),
        Ins::I32WrapI64,
        Ins::I64Load(layout::WORD),
        Ins::I32WrapI64,
    ]
}

/// The address of the byte at `index` of the string in `slot`.
fn text_at(slot: u32, index: u32) -> Vec<Ins> {
    vec![
        Ins::LocalGet(slot),
        Ins::I32WrapI64,
        Ins::I32Const(TEXT as i32),
        Ins::I32Add,
        Ins::LocalGet(index),
        Ins::I32Add,
    ]
}

/// Runs `body` with `counter` going from zero up to `limit`.
///
/// The body may leave the loop early by branching two levels out, which is
/// what the comparisons do when they have their answer.
fn count_to(counter: u32, limit: u32, mut body: Vec<Ins>) -> Vec<Ins> {
    let mut inner = vec![
        Ins::LocalGet(counter),
        Ins::LocalGet(limit),
        Ins::I32GeS,
        Ins::BrIf(1),
    ];
    inner.append(&mut body);
    inner.extend([
        Ins::LocalGet(counter),
        Ins::I32Const(1),
        Ins::I32Add,
        Ins::LocalSet(counter),
        Ins::Br(0),
    ]);
    vec![
        Ins::I32Const(0),
        Ins::LocalSet(counter),
        Ins::Block {
            result: None,
            body: vec![Ins::Loop {
                result: None,
                body: inner,
            }],
        },
    ]
}

/// `str_eq(a, b) -> Bool`.
///
/// Length first, because two strings of different lengths are never equal and
/// a length is one load. Then bytes, which is the same answer as characters:
/// two byte sequences are equal exactly when the text they decode to is, and
/// this backend never has to decode either.
fn str_eq() -> (Vec<ValType>, Vec<Ins>) {
    const A: u32 = 0;
    const B: u32 = 1;
    const N: u32 = 2;
    const I: u32 = 3;

    let mut body = byte_length(A);
    body.push(Ins::LocalSet(N));
    body.extend(byte_length(B));
    body.extend([
        Ins::LocalGet(N),
        Ins::I32Ne,
        Ins::If {
            result: None,
            then: vec![Ins::I32Const(0), Ins::Return],
            otherwise: Vec::new(),
        },
    ]);

    let mut differs = text_at(A, I);
    differs.push(Ins::I32Load8U(0));
    differs.extend(text_at(B, I));
    differs.extend([
        Ins::I32Load8U(0),
        Ins::I32Ne,
        Ins::If {
            result: None,
            then: vec![Ins::I32Const(0), Ins::Return],
            otherwise: Vec::new(),
        },
    ]);

    body.extend(count_to(I, N, differs));
    body.extend([Ins::I32Const(1), Ins::Return]);

    (vec![ValType::I32, ValType::I32], body)
}

/// `str_cmp(a, b) -> Int`, negative, zero or positive.
///
/// Byte order, which for UTF-8 is code point order, which is the order
/// `design/02-syntax.md` says `<` on a string means. A prefix comes first,
/// so when the shared bytes all match the shorter string is the smaller one.
fn str_cmp() -> (Vec<ValType>, Vec<Ins>) {
    const A: u32 = 0;
    const B: u32 = 1;
    const NA: u32 = 2;
    const NB: u32 = 3;
    const N: u32 = 4;
    const I: u32 = 5;

    let mut body = byte_length(A);
    body.push(Ins::LocalSet(NA));
    body.extend(byte_length(B));
    body.push(Ins::LocalSet(NB));

    // n = the shorter of the two
    body.extend([
        Ins::LocalGet(NA),
        Ins::LocalSet(N),
        Ins::LocalGet(NB),
        Ins::LocalGet(NA),
        Ins::I32LtS,
        Ins::If {
            result: None,
            then: vec![Ins::LocalGet(NB), Ins::LocalSet(N)],
            otherwise: Vec::new(),
        },
    ]);

    let mut step = text_at(A, I);
    step.push(Ins::I32Load8U(0));
    step.extend(text_at(B, I));
    step.extend([
        Ins::I32Load8U(0),
        Ins::I32LtS,
        Ins::If {
            result: None,
            then: vec![Ins::I64Const(-1), Ins::Return],
            otherwise: Vec::new(),
        },
    ]);
    step.extend(text_at(A, I));
    step.push(Ins::I32Load8U(0));
    step.extend(text_at(B, I));
    step.extend([
        Ins::I32Load8U(0),
        Ins::I32GtS,
        Ins::If {
            result: None,
            then: vec![Ins::I64Const(1), Ins::Return],
            otherwise: Vec::new(),
        },
    ]);

    body.extend(count_to(I, N, step));

    // Every shared byte matched, so the shorter one comes first.
    body.extend([
        Ins::LocalGet(NA),
        Ins::LocalGet(NB),
        Ins::I32LtS,
        Ins::If {
            result: None,
            then: vec![Ins::I64Const(-1), Ins::Return],
            otherwise: Vec::new(),
        },
        Ins::LocalGet(NA),
        Ins::LocalGet(NB),
        Ins::I32GtS,
        Ins::If {
            result: None,
            then: vec![Ins::I64Const(1), Ins::Return],
            otherwise: Vec::new(),
        },
        Ins::I64Const(0),
        Ins::Return,
    ]);

    (
        vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
        body,
    )
}

/// `str_concat(a, b) -> String`.
///
/// The character count is the sum of the two counts and the byte count is the
/// sum of the two byte counts, so neither has to be worked out from the bytes.
/// The padding a string layout ends with is already zero: memory starts that
/// way and this backend never hands the same bytes out twice.
fn str_concat() -> (Vec<ValType>, Vec<Ins>) {
    const A: u32 = 0;
    const B: u32 = 1;
    const NA: u32 = 2;
    const NB: u32 = 3;
    const TOTAL: u32 = 4;
    const OUT: u32 = 5;
    const I: u32 = 6;

    let mut body = byte_length(A);
    body.push(Ins::LocalSet(NA));
    body.extend(byte_length(B));
    body.push(Ins::LocalSet(NB));
    body.extend([
        Ins::LocalGet(NA),
        Ins::LocalGet(NB),
        Ins::I32Add,
        Ins::LocalSet(TOTAL),
        // out = bump
        Ins::I32Const(layout::BUMP as i32),
        Ins::I64Load(0),
        Ins::I32WrapI64,
        Ins::LocalSet(OUT),
        // bump = out + headers + total, rounded up to a word
        Ins::I32Const(layout::BUMP as i32),
        Ins::LocalGet(OUT),
        Ins::I64ExtendI32S,
        Ins::I64Const(TEXT as i64),
        Ins::I64Add,
        Ins::LocalGet(TOTAL),
        Ins::I64ExtendI32S,
        Ins::I64Add,
        Ins::I64Const(layout::WORD as i64),
        Ins::LocalGet(TOTAL),
        Ins::I64ExtendI32S,
        Ins::I64Const(layout::WORD as i64),
        Ins::I64RemS,
        Ins::I64Sub,
        Ins::I64Const(layout::WORD as i64),
        Ins::I64RemS,
        Ins::I64Add,
        Ins::I64Store(0),
        // characters
        Ins::LocalGet(OUT),
        Ins::LocalGet(A),
        Ins::I32WrapI64,
        Ins::I64Load(0),
        Ins::LocalGet(B),
        Ins::I32WrapI64,
        Ins::I64Load(0),
        Ins::I64Add,
        Ins::I64Store(0),
        // bytes
        Ins::LocalGet(OUT),
        Ins::LocalGet(TOTAL),
        Ins::I64ExtendI32S,
        Ins::I64Store(layout::WORD),
    ]);

    let mut first = vec![
        Ins::LocalGet(OUT),
        Ins::I32Const(TEXT as i32),
        Ins::I32Add,
        Ins::LocalGet(I),
        Ins::I32Add,
    ];
    first.extend(text_at(A, I));
    first.extend([Ins::I32Load8U(0), Ins::I32Store8(0)]);
    body.extend(count_to(I, NA, first));

    let mut second = vec![
        Ins::LocalGet(OUT),
        Ins::I32Const(TEXT as i32),
        Ins::I32Add,
        Ins::LocalGet(NA),
        Ins::I32Add,
        Ins::LocalGet(I),
        Ins::I32Add,
    ];
    second.extend(text_at(B, I));
    second.extend([Ins::I32Load8U(0), Ins::I32Store8(0)]);
    body.extend(count_to(I, NB, second));

    body.extend([Ins::LocalGet(OUT), Ins::I64ExtendI32S, Ins::Return]);

    (
        vec![
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
        ],
        body,
    )
}
