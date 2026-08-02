//! Functions the backend writes, rather than the program.
//!
//! A string is bytes in memory and a list is words in memory, so most of what
//! the prelude offers is a loop, and a loop is not something to write again at
//! every call site. These are compiled into the module like any other function
//! and called by index.
//!
//! They are appended after everything the program declares, so a helper's
//! index is `imports + program functions + its position here` and adding one
//! moves nothing. Only the ones a program reaches are emitted, together with
//! whatever those call, the way an import is only declared when something
//! calls it: a module that carries a string comparison it never makes is a
//! module that got bigger for nothing.
//!
//! Nothing here allocates through the same path a lowered expression does.
//! The bump pointer is a word in memory and these read and write it directly,
//! which is the whole of what allocation is in this backend.
//!
//! What each one does is the interpreter's answer rather than a second reading
//! of the prelude's documentation, because two engines answering the same call
//! differently is a difference a program can see.

use std::collections::HashMap;

use crate::compile::Strings;
use crate::layout;
use crate::wasm::{FuncType, Ins, ValType};

/// A string's bytes start after its two headers.
const TEXT: u32 = 2 * layout::WORD;

/// A list's elements start after its length.
const ELEMENTS: u32 = layout::WORD;

/// Where a `Result` keeps what it holds, and which tag is which.
///
/// `ok` and `err` are the only two variants and lowering builds them in that
/// order, so a helper handing one back writes the tag a lowered `ok` would.
const OK: i64 = 0;
const ERR: i64 = 1;
const PAYLOAD: u32 = layout::WORD;

/// Cutting a piece out of a string, which `split` and `trim` both do.
const STR_SLICE: &str = "deed_rt_str_slice";
/// Where one string sits inside another, which `split` walks with.
const STR_FIND: &str = "deed_rt_str_find";

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
    /// The bytes of a string between two offsets, as a string of its own.
    TextSlice,
    /// Where one string sits inside another at or after an offset, or -1.
    TextSearch,
    /// A number written out in decimal.
    NumberText,
    /// A number read out of text, or a `Result` saying it was not one.
    TextNumber,
    /// The pieces of a string, separated.
    TextPieces,
    /// A list of strings, put back together with something between them.
    PiecesText,
    /// A string with the four whitespace characters taken off each end.
    TrimmedText,
    /// The twenty-six letters, raised.
    RaisedText,
    /// The twenty-six letters, lowered.
    LoweredText,
    /// The element of a list at an index, or a `Result` saying it is not there.
    ListElement,
    /// A list with one more element on the end.
    ListExtended,
    /// A list holding the same value a number of times.
    ListFilled,
}

impl Helper {
    /// Every helper, in the order they are emitted.
    pub(crate) const ALL: &'static [Helper] = &[
        Helper::SameText,
        Helper::TextOrder,
        Helper::JoinedText,
        Helper::TextSlice,
        Helper::TextSearch,
        Helper::NumberText,
        Helper::TextNumber,
        Helper::TextPieces,
        Helper::PiecesText,
        Helper::TrimmedText,
        Helper::RaisedText,
        Helper::LoweredText,
        Helper::ListElement,
        Helper::ListExtended,
        Helper::ListFilled,
    ];

    /// Which helper answers for a runtime name the IR asked for.
    pub(crate) fn named(name: &str) -> Option<Helper> {
        use deed_mir::runtime;
        Some(match name {
            runtime::STR_EQ => Helper::SameText,
            runtime::STR_CMP => Helper::TextOrder,
            runtime::STR_CONCAT => Helper::JoinedText,
            runtime::STR_SPLIT => Helper::TextPieces,
            runtime::STR_JOIN => Helper::PiecesText,
            runtime::STR_TRIM => Helper::TrimmedText,
            runtime::STR_UPPER => Helper::RaisedText,
            runtime::STR_LOWER => Helper::LoweredText,
            runtime::INT_TO_STR => Helper::NumberText,
            runtime::STR_TO_INT => Helper::TextNumber,
            runtime::LIST_AT => Helper::ListElement,
            runtime::LIST_PUSH => Helper::ListExtended,
            runtime::LIST_REPEAT => Helper::ListFilled,
            _ => return None,
        })
    }

    /// The name a reader of the module sees in a trap.
    pub(crate) fn name(self) -> &'static str {
        use deed_mir::runtime;
        match self {
            Helper::SameText => runtime::STR_EQ,
            Helper::TextOrder => runtime::STR_CMP,
            Helper::JoinedText => runtime::STR_CONCAT,
            Helper::TextSlice => STR_SLICE,
            Helper::TextSearch => STR_FIND,
            Helper::NumberText => runtime::INT_TO_STR,
            Helper::TextNumber => runtime::STR_TO_INT,
            Helper::TextPieces => runtime::STR_SPLIT,
            Helper::PiecesText => runtime::STR_JOIN,
            Helper::TrimmedText => runtime::STR_TRIM,
            Helper::RaisedText => runtime::STR_UPPER,
            Helper::LoweredText => runtime::STR_LOWER,
            Helper::ListElement => runtime::LIST_AT,
            Helper::ListExtended => runtime::LIST_PUSH,
            Helper::ListFilled => runtime::LIST_REPEAT,
        }
    }

    pub(crate) fn signature(self) -> FuncType {
        let two = |results: Vec<ValType>| FuncType {
            params: vec![ValType::I64, ValType::I64],
            results,
        };
        match self {
            Helper::SameText => two(vec![ValType::I32]),
            Helper::TextOrder
            | Helper::JoinedText
            | Helper::TextPieces
            | Helper::PiecesText
            | Helper::ListElement
            | Helper::ListExtended
            | Helper::ListFilled => two(vec![ValType::I64]),
            Helper::TextSlice | Helper::TextSearch => FuncType {
                params: vec![ValType::I64, ValType::I64, ValType::I64],
                results: vec![ValType::I64],
            },
            Helper::NumberText
            | Helper::TextNumber
            | Helper::TrimmedText
            | Helper::RaisedText
            | Helper::LoweredText => FuncType {
                params: vec![ValType::I64],
                results: vec![ValType::I64],
            },
        }
    }

    /// The helpers this one calls, so emitting it emits them too.
    pub(crate) fn needs(self) -> &'static [Helper] {
        match self {
            Helper::ListElement => &[Helper::NumberText, Helper::JoinedText],
            Helper::TextNumber => &[Helper::JoinedText],
            Helper::TextPieces => &[Helper::TextSlice, Helper::TextSearch],
            Helper::TrimmedText => &[Helper::TextSlice],
            _ => &[],
        }
    }

    /// The locals beyond the parameters, and the body.
    pub(crate) fn compile(self, at: &mut Where<'_>) -> (Vec<ValType>, Vec<Ins>) {
        match self {
            Helper::SameText => str_eq(),
            Helper::TextOrder => str_cmp(),
            Helper::JoinedText => str_concat(),
            Helper::TextSlice => text_slice(),
            Helper::TextSearch => text_search(),
            Helper::NumberText => number_text(),
            Helper::TextNumber => text_number(at),
            Helper::TextPieces => text_pieces(at),
            Helper::PiecesText => pieces_text(),
            Helper::TrimmedText => trimmed_text(at),
            Helper::RaisedText => cased_text(true),
            Helper::LoweredText => cased_text(false),
            Helper::ListElement => list_element(at),
            Helper::ListExtended => list_extended(),
            Helper::ListFilled => list_filled(),
        }
    }
}

/// What a helper needs from the module around it.
pub(crate) struct Where<'a> {
    index: &'a HashMap<Helper, u32>,
    strings: &'a mut Strings,
}

impl<'a> Where<'a> {
    pub(crate) fn new(index: &'a HashMap<Helper, u32>, strings: &'a mut Strings) -> Self {
        Where { index, strings }
    }

    /// Calls another helper. Every caller says so in [`Helper::needs`], which
    /// is what puts the callee in the module.
    fn call(&self, helper: Helper) -> Ins {
        Ins::Call(
            *self
                .index
                .get(&helper)
                .expect("a helper calls only what it says it needs"),
        )
    }

    /// The address of a string literal, placed in the data section.
    fn text(&mut self, literal: &str) -> Ins {
        Ins::I64Const(self.strings.place(literal) as i64)
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

/// Reserves as many bytes as `size` pushes, leaving the address in `out`.
fn allocate(out: u32, size: impl FnOnce(&mut Vec<Ins>)) -> Vec<Ins> {
    let mut ins = vec![
        Ins::I32Const(layout::BUMP as i32),
        Ins::I64Load(0),
        Ins::I32WrapI64,
        Ins::LocalSet(out),
        Ins::I32Const(layout::BUMP as i32),
        Ins::LocalGet(out),
        Ins::I64ExtendI32S,
    ];
    size(&mut ins);
    ins.push(Ins::I64Add);
    ins.push(Ins::I64Store(0));
    ins
}

/// Pushes the room a string of as many bytes as `bytes` holds takes, headers
/// and padding both.
///
/// The padding is a remainder rather than a mask because this backend's own
/// runner reads `i32.and` as the boolean operator the language has and not as
/// the bitwise one WebAssembly has, and nothing here may depend on which.
fn string_room(bytes: u32) -> Vec<Ins> {
    vec![
        Ins::I64Const(TEXT as i64),
        Ins::LocalGet(bytes),
        Ins::I64ExtendI32S,
        Ins::I64Add,
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

/// Pushes the room a list of as many elements as `count` holds takes.
fn list_room(count: u32) -> Vec<Ins> {
    vec![
        Ins::I64Const(layout::WORD as i64),
        Ins::LocalGet(count),
        Ins::I64ExtendI32S,
        Ins::I64Const(layout::WORD as i64),
        Ins::I64Mul,
        Ins::I64Add,
    ]
}

/// The address of the element at `index` of the list whose address is in the
/// `i32` local `base`.
///
/// A list arrives as a parameter and is built into a local, so the address is
/// asked for as the narrow thing an address is rather than as the word a
/// value travels in.
fn element_at(base: u32, index: u32) -> Vec<Ins> {
    vec![
        Ins::LocalGet(base),
        Ins::I32Const(ELEMENTS as i32),
        Ins::I32Add,
        Ins::LocalGet(index),
        Ins::I32Const(layout::WORD as i32),
        Ins::I32Mul,
        Ins::I32Add,
    ]
}

/// `if` with nothing after it, which is how a helper answers early.
fn when(condition: Vec<Ins>, then: Vec<Ins>) -> Vec<Ins> {
    let mut ins = condition;
    ins.push(Ins::If {
        result: None,
        then,
        otherwise: Vec::new(),
    });
    ins
}

/// Whether the byte `byte` pushes carries on a character rather than starting
/// one.
///
/// UTF-8 marks a continuation byte with its top two bits, and this backend's
/// own runner reads `i32.and` as the boolean operator the language has rather
/// than the bitwise one WebAssembly has, so the range those bits describe is
/// written out instead of masked for.
fn is_continuation(byte: Vec<Ins>) -> Vec<Ins> {
    let mut ins = byte.clone();
    ins.extend([Ins::I32Const(128), Ins::I32GeS]);
    ins.extend(byte);
    ins.extend([Ins::I32Const(192), Ins::I32LtS, Ins::I32And]);
    ins
}

/// Walks `counter` from zero while `keep` holds, with `body` in between.
fn walk_while(counter: u32, keep: Vec<Ins>, body: Vec<Ins>) -> Vec<Ins> {
    let mut inner = keep;
    inner.push(Ins::I32Eqz);
    inner.push(Ins::BrIf(1));
    inner.extend(body);
    inner.extend([
        Ins::LocalGet(counter),
        Ins::I32Const(1),
        Ins::I32Add,
        Ins::LocalSet(counter),
        Ins::Br(0),
    ]);
    vec![Ins::Block {
        result: None,
        body: vec![Ins::Loop {
            result: None,
            body: inner,
        }],
    }]
}

/// Builds a `Result` around what `value` pushes, and answers with it.
fn result(out: u32, tag: i64, value: impl FnOnce(&mut Vec<Ins>)) -> Vec<Ins> {
    let mut ins = allocate(out, |ins| {
        ins.push(Ins::I64Const(layout::aggregate_size(true, 1) as i64));
    });
    ins.extend([
        Ins::LocalGet(out),
        Ins::I64Const(tag),
        Ins::I64Store(0),
        Ins::LocalGet(out),
    ]);
    value(&mut ins);
    ins.extend([
        Ins::I64Store(PAYLOAD),
        Ins::LocalGet(out),
        Ins::I64ExtendI32S,
        Ins::Return,
    ]);
    ins
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

/// `to_string(n) -> String`.
///
/// The digits come out of a negative accumulator, because the smallest `Int`
/// has no positive counterpart and taking its magnitude would be the one
/// number this gets wrong.
fn number_text() -> (Vec<ValType>, Vec<Ins>) {
    const N: u32 = 0;
    const M: u32 = 1;
    const T: u32 = 2;
    const DIGITS: u32 = 3;
    const NEGATIVE: u32 = 4;
    const OUT: u32 = 5;
    const I: u32 = 6;
    const TOTAL: u32 = 7;

    let mut body = vec![
        Ins::LocalGet(N),
        Ins::I64Const(0),
        Ins::I64LtS,
        Ins::LocalSet(NEGATIVE),
        Ins::LocalGet(N),
        Ins::LocalSet(M),
    ];
    body.extend(when(
        vec![Ins::LocalGet(NEGATIVE), Ins::I32Eqz],
        vec![
            Ins::I64Const(0),
            Ins::LocalGet(N),
            Ins::I64Sub,
            Ins::LocalSet(M),
        ],
    ));

    // How many digits, counted before anything is written, because the first
    // one written is the last one produced.
    body.extend([
        Ins::I32Const(0),
        Ins::LocalSet(DIGITS),
        Ins::LocalGet(M),
        Ins::LocalSet(T),
        Ins::Block {
            result: None,
            body: vec![Ins::Loop {
                result: None,
                body: vec![
                    Ins::LocalGet(DIGITS),
                    Ins::I32Const(1),
                    Ins::I32Add,
                    Ins::LocalSet(DIGITS),
                    Ins::LocalGet(T),
                    Ins::I64Const(10),
                    Ins::I64DivS,
                    Ins::LocalSet(T),
                    Ins::LocalGet(T),
                    Ins::I64Eqz,
                    Ins::BrIf(1),
                    Ins::Br(0),
                ],
            }],
        },
        Ins::LocalGet(DIGITS),
        Ins::LocalGet(NEGATIVE),
        Ins::I32Add,
        Ins::LocalSet(TOTAL),
    ]);

    body.extend(allocate(OUT, |ins| ins.extend(string_room(TOTAL))));
    body.extend([
        Ins::LocalGet(OUT),
        Ins::LocalGet(TOTAL),
        Ins::I64ExtendI32S,
        Ins::I64Store(0),
        Ins::LocalGet(OUT),
        Ins::LocalGet(TOTAL),
        Ins::I64ExtendI32S,
        Ins::I64Store(layout::WORD),
    ]);

    body.extend(when(
        vec![Ins::LocalGet(NEGATIVE)],
        vec![
            Ins::LocalGet(OUT),
            Ins::I32Const(TEXT as i32),
            Ins::I32Add,
            Ins::I32Const(b'-' as i32),
            Ins::I32Store8(0),
        ],
    ));

    // From the end back, one digit at a time.
    body.extend([
        Ins::LocalGet(TOTAL),
        Ins::I32Const(1),
        Ins::I32Sub,
        Ins::LocalSet(I),
        Ins::LocalGet(M),
        Ins::LocalSet(T),
        Ins::Block {
            result: None,
            body: vec![Ins::Loop {
                result: None,
                body: vec![
                    Ins::LocalGet(OUT),
                    Ins::I32Const(TEXT as i32),
                    Ins::I32Add,
                    Ins::LocalGet(I),
                    Ins::I32Add,
                    Ins::I32Const(b'0' as i32),
                    Ins::I64Const(0),
                    Ins::LocalGet(T),
                    Ins::I64Const(10),
                    Ins::I64RemS,
                    Ins::I64Sub,
                    Ins::I32WrapI64,
                    Ins::I32Add,
                    Ins::I32Store8(0),
                    Ins::LocalGet(T),
                    Ins::I64Const(10),
                    Ins::I64DivS,
                    Ins::LocalSet(T),
                    Ins::LocalGet(I),
                    Ins::I32Const(1),
                    Ins::I32Sub,
                    Ins::LocalSet(I),
                    Ins::LocalGet(T),
                    Ins::I64Eqz,
                    Ins::BrIf(1),
                    Ins::Br(0),
                ],
            }],
        },
        Ins::LocalGet(OUT),
        Ins::I64ExtendI32S,
        Ins::Return,
    ]);

    (
        vec![
            ValType::I64,
            ValType::I64,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
        ],
        body,
    )
}

/// `at(xs, i) -> Result<T, String>`.
///
/// An index nobody promised is there is not a mistake in the caller, so it
/// comes back as an error value. The sentence is the interpreter's word for
/// word: two engines answering the same call with two different strings is a
/// difference a program can see.
fn list_element(at: &mut Where<'_>) -> (Vec<ValType>, Vec<Ins>) {
    const XS: u32 = 0;
    const INDEX: u32 = 1;
    const FROM: u32 = 2;
    const N: u32 = 3;
    const OUT: u32 = 4;
    const MESSAGE: u32 = 5;
    const I: u32 = 6;

    let opening = at.text("index ");
    let middle = at.text(" is outside a list of ");
    let join = at.call(Helper::JoinedText);
    let number = at.call(Helper::NumberText);

    let mut body = vec![
        Ins::LocalGet(XS),
        Ins::I32WrapI64,
        Ins::LocalSet(FROM),
        Ins::LocalGet(FROM),
        Ins::I64Load(0),
        Ins::LocalSet(N),
    ];

    let mut refuse = vec![
        opening,
        Ins::LocalGet(INDEX),
        number.clone(),
        join.clone(),
        middle,
        join.clone(),
        Ins::LocalGet(N),
        number,
        join,
        Ins::LocalSet(MESSAGE),
    ];
    refuse.extend(result(OUT, ERR, |ins| ins.push(Ins::LocalGet(MESSAGE))));

    body.extend(when(
        vec![
            Ins::LocalGet(INDEX),
            Ins::I64Const(0),
            Ins::I64LtS,
            Ins::LocalGet(INDEX),
            Ins::LocalGet(N),
            Ins::I64GeS,
            Ins::I32Or,
        ],
        refuse,
    ));

    body.extend([Ins::LocalGet(INDEX), Ins::I32WrapI64, Ins::LocalSet(I)]);
    body.extend(result(OUT, OK, |ins| {
        ins.extend(element_at(FROM, I));
        ins.push(Ins::I64Load(0));
    }));

    (
        vec![
            ValType::I32,
            ValType::I64,
            ValType::I32,
            ValType::I64,
            ValType::I32,
        ],
        body,
    )
}

/// `push(xs, x) -> List<T>`.
///
/// A new list rather than a longer one: values in this language do not
/// change, and the list that went in is still the list it was.
fn list_extended() -> (Vec<ValType>, Vec<Ins>) {
    const XS: u32 = 0;
    const VALUE: u32 = 1;
    const FROM: u32 = 2;
    const N: u32 = 3;
    const OUT: u32 = 4;
    const I: u32 = 5;
    const ROOM: u32 = 6;

    let mut body = vec![
        Ins::LocalGet(XS),
        Ins::I32WrapI64,
        Ins::LocalSet(FROM),
        Ins::LocalGet(FROM),
        Ins::I64Load(0),
        Ins::I32WrapI64,
        Ins::LocalSet(N),
        Ins::LocalGet(N),
        Ins::I32Const(1),
        Ins::I32Add,
        Ins::LocalSet(ROOM),
    ];
    body.extend(allocate(OUT, |ins| ins.extend(list_room(ROOM))));
    body.extend([
        Ins::LocalGet(OUT),
        Ins::LocalGet(ROOM),
        Ins::I64ExtendI32S,
        Ins::I64Store(0),
    ]);

    let mut step = element_at(OUT, I);
    step.extend(element_at(FROM, I));
    step.extend([Ins::I64Load(0), Ins::I64Store(0)]);
    body.extend(count_to(I, N, step));

    let mut last = element_at(OUT, N);
    last.extend([
        Ins::LocalGet(VALUE),
        Ins::I64Store(0),
        Ins::LocalGet(OUT),
        Ins::I64ExtendI32S,
        Ins::Return,
    ]);
    body.extend(last);

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

/// `repeat(value, count) -> List<T>`.
///
/// A count of zero or less is the empty list rather than a refusal. The call
/// this exists for is `repeat(" ", width - length(text))`, which goes negative
/// exactly when the text is already wider than the column, and what it means
/// there is no padding.
fn list_filled() -> (Vec<ValType>, Vec<Ins>) {
    const VALUE: u32 = 0;
    const COUNT: u32 = 1;
    const N: u32 = 2;
    const OUT: u32 = 3;
    const I: u32 = 4;

    let mut body = vec![Ins::LocalGet(COUNT), Ins::I32WrapI64, Ins::LocalSet(N)];
    body.extend(when(
        vec![Ins::LocalGet(COUNT), Ins::I64Const(0), Ins::I64LtS],
        vec![Ins::I32Const(0), Ins::LocalSet(N)],
    ));
    body.extend(allocate(OUT, |ins| ins.extend(list_room(N))));
    body.extend([
        Ins::LocalGet(OUT),
        Ins::LocalGet(N),
        Ins::I64ExtendI32S,
        Ins::I64Store(0),
    ]);

    let mut step = element_at(OUT, I);
    step.extend([Ins::LocalGet(VALUE), Ins::I64Store(0)]);
    body.extend(count_to(I, N, step));
    body.extend([Ins::LocalGet(OUT), Ins::I64ExtendI32S, Ins::Return]);

    (vec![ValType::I32, ValType::I32, ValType::I32], body)
}

/// `str_slice(s, from, to) -> String`, in bytes.
///
/// The character count is counted here rather than carried, because a piece
/// of a string knows how many bytes it took and not how many characters they
/// were. Every byte that is not a continuation byte starts one.
fn text_slice() -> (Vec<ValType>, Vec<Ins>) {
    const S: u32 = 0;
    const FROM: u32 = 1;
    const TO: u32 = 2;
    const START: u32 = 3;
    const N: u32 = 4;
    const OUT: u32 = 5;
    const I: u32 = 6;
    const CHARS: u32 = 7;

    let source = |index: u32| {
        vec![
            Ins::LocalGet(S),
            Ins::I32WrapI64,
            Ins::I32Const(TEXT as i32),
            Ins::I32Add,
            Ins::LocalGet(START),
            Ins::I32Add,
            Ins::LocalGet(index),
            Ins::I32Add,
            Ins::I32Load8U(0),
        ]
    };

    let mut body = vec![
        Ins::LocalGet(FROM),
        Ins::I32WrapI64,
        Ins::LocalSet(START),
        Ins::LocalGet(TO),
        Ins::I32WrapI64,
        Ins::LocalGet(START),
        Ins::I32Sub,
        Ins::LocalSet(N),
        Ins::I32Const(0),
        Ins::LocalSet(CHARS),
    ];
    body.extend(allocate(OUT, |ins| ins.extend(string_room(N))));
    body.extend([
        Ins::LocalGet(OUT),
        Ins::LocalGet(N),
        Ins::I64ExtendI32S,
        Ins::I64Store(layout::WORD),
    ]);

    let mut step = vec![
        Ins::LocalGet(OUT),
        Ins::I32Const(TEXT as i32),
        Ins::I32Add,
        Ins::LocalGet(I),
        Ins::I32Add,
    ];
    step.extend(source(I));
    step.push(Ins::I32Store8(0));
    let mut counts = is_continuation(source(I));
    counts.push(Ins::I32Eqz);
    step.extend(when(
        counts,
        vec![
            Ins::LocalGet(CHARS),
            Ins::I32Const(1),
            Ins::I32Add,
            Ins::LocalSet(CHARS),
        ],
    ));
    body.extend(count_to(I, N, step));

    body.extend([
        Ins::LocalGet(OUT),
        Ins::LocalGet(CHARS),
        Ins::I64ExtendI32S,
        Ins::I64Store(0),
        Ins::LocalGet(OUT),
        Ins::I64ExtendI32S,
        Ins::Return,
    ]);

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

/// `str_find(s, needle, from) -> Int`, or -1.
///
/// The needle is never empty here: the one caller answers that case without
/// searching, because an empty separator means the characters.
fn text_search() -> (Vec<ValType>, Vec<Ins>) {
    const S: u32 = 0;
    const NEEDLE: u32 = 1;
    const FROM: u32 = 2;
    const NS: u32 = 3;
    const NN: u32 = 4;
    const AT: u32 = 5;
    const I: u32 = 6;
    const SAME: u32 = 7;

    let mut body = byte_length(S);
    body.push(Ins::LocalSet(NS));
    body.extend(byte_length(NEEDLE));
    body.push(Ins::LocalSet(NN));
    body.extend([Ins::LocalGet(FROM), Ins::I32WrapI64, Ins::LocalSet(AT)]);

    let mut compare = vec![
        Ins::LocalGet(S),
        Ins::I32WrapI64,
        Ins::I32Const(TEXT as i32),
        Ins::I32Add,
        Ins::LocalGet(AT),
        Ins::I32Add,
        Ins::LocalGet(I),
        Ins::I32Add,
        Ins::I32Load8U(0),
    ];
    compare.extend(text_at(NEEDLE, I));
    compare.push(Ins::I32Load8U(0));
    compare.push(Ins::I32Ne);
    compare.extend(when(
        Vec::new(),
        vec![Ins::I32Const(0), Ins::LocalSet(SAME)],
    ));

    let mut attempt = vec![Ins::I32Const(1), Ins::LocalSet(SAME)];
    attempt.extend(count_to(I, NN, compare));
    attempt.extend(when(
        vec![Ins::LocalGet(SAME)],
        vec![Ins::LocalGet(AT), Ins::I64ExtendI32S, Ins::Return],
    ));

    body.extend(walk_while(
        AT,
        vec![
            Ins::LocalGet(AT),
            Ins::LocalGet(NS),
            Ins::LocalGet(NN),
            Ins::I32Sub,
            Ins::I32LeS,
        ],
        attempt,
    ));
    body.extend([Ins::I64Const(-1), Ins::Return]);

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

/// `split(text, separator) -> List<String>`.
///
/// An empty separator gives the characters, which is the prelude's answer and
/// the reason walking a string needs no second name. Otherwise the pieces come
/// out either side of every occurrence, so a separator at an edge leaves an
/// empty piece and `split` and `join` stay inverses.
///
/// Two walks: one to count the pieces and one to cut them. The list has to be
/// allocated whole, and how many pieces there are is not known until the last
/// one is found.
fn text_pieces(at: &mut Where<'_>) -> (Vec<ValType>, Vec<Ins>) {
    const S: u32 = 0;
    const SEP: u32 = 1;
    const N: u32 = 2;
    const NSEP: u32 = 3;
    const COUNT: u32 = 4;
    const OUT: u32 = 5;
    const FROM: u32 = 6;
    const FOUND: u32 = 7;
    const WRITTEN: u32 = 8;
    const I: u32 = 9;
    const NEXT: u32 = 10;

    let slice = at.call(Helper::TextSlice);
    let find = at.call(Helper::TextSearch);

    let mut body = byte_length(S);
    body.push(Ins::LocalSet(N));
    body.extend(byte_length(SEP));
    body.push(Ins::LocalSet(NSEP));

    // -- the characters, one string each ------------------------------------
    let mut characters = vec![Ins::I32Const(0), Ins::LocalSet(COUNT)];
    let mut counting = is_continuation({
        let mut ins = text_at(S, I);
        ins.push(Ins::I32Load8U(0));
        ins
    });
    counting.push(Ins::I32Eqz);
    characters.extend(count_to(
        I,
        N,
        when(
            counting,
            vec![
                Ins::LocalGet(COUNT),
                Ins::I32Const(1),
                Ins::I32Add,
                Ins::LocalSet(COUNT),
            ],
        ),
    ));
    characters.extend(allocate(OUT, |ins| ins.extend(list_room(COUNT))));
    characters.extend([
        Ins::LocalGet(OUT),
        Ins::LocalGet(COUNT),
        Ins::I64ExtendI32S,
        Ins::I64Store(0),
        Ins::I32Const(0),
        Ins::LocalSet(WRITTEN),
    ]);

    // Each character runs from its own first byte to the next one.
    let mut cut = vec![
        Ins::LocalGet(I),
        Ins::I32Const(1),
        Ins::I32Add,
        Ins::LocalSet(NEXT),
    ];
    let mut carries = is_continuation({
        let mut ins = text_at(S, NEXT);
        ins.push(Ins::I32Load8U(0));
        ins
    });
    carries.insert(0, Ins::I32LtS);
    carries.insert(0, Ins::LocalGet(N));
    carries.insert(0, Ins::LocalGet(NEXT));
    carries.push(Ins::I32And);
    cut.extend(walk_while(NEXT, carries, Vec::new()));
    cut.extend(element_at(OUT, WRITTEN));
    cut.extend([
        Ins::LocalGet(S),
        Ins::LocalGet(I),
        Ins::I64ExtendI32S,
        Ins::LocalGet(NEXT),
        Ins::I64ExtendI32S,
        slice.clone(),
        Ins::I64Store(0),
        Ins::LocalGet(WRITTEN),
        Ins::I32Const(1),
        Ins::I32Add,
        Ins::LocalSet(WRITTEN),
    ]);
    let mut starts = is_continuation({
        let mut ins = text_at(S, I);
        ins.push(Ins::I32Load8U(0));
        ins
    });
    starts.push(Ins::I32Eqz);
    characters.extend(count_to(I, N, when(starts, cut)));
    characters.extend([Ins::LocalGet(OUT), Ins::I64ExtendI32S, Ins::Return]);

    body.extend(when(vec![Ins::LocalGet(NSEP), Ins::I32Eqz], characters));

    // -- one piece more than the separator appears --------------------------
    body.extend([
        Ins::I32Const(1),
        Ins::LocalSet(COUNT),
        Ins::I32Const(0),
        Ins::LocalSet(FROM),
        Ins::Block {
            result: None,
            body: vec![Ins::Loop {
                result: None,
                body: vec![
                    Ins::LocalGet(S),
                    Ins::LocalGet(SEP),
                    Ins::LocalGet(FROM),
                    Ins::I64ExtendI32S,
                    find.clone(),
                    Ins::I32WrapI64,
                    Ins::LocalTee(FOUND),
                    Ins::I32Const(0),
                    Ins::I32LtS,
                    Ins::BrIf(1),
                    Ins::LocalGet(COUNT),
                    Ins::I32Const(1),
                    Ins::I32Add,
                    Ins::LocalSet(COUNT),
                    Ins::LocalGet(FOUND),
                    Ins::LocalGet(NSEP),
                    Ins::I32Add,
                    Ins::LocalSet(FROM),
                    Ins::Br(0),
                ],
            }],
        },
    ]);

    body.extend(allocate(OUT, |ins| ins.extend(list_room(COUNT))));
    body.extend([
        Ins::LocalGet(OUT),
        Ins::LocalGet(COUNT),
        Ins::I64ExtendI32S,
        Ins::I64Store(0),
        Ins::I32Const(0),
        Ins::LocalSet(WRITTEN),
        Ins::I32Const(0),
        Ins::LocalSet(FROM),
    ]);

    let mut writing = vec![
        Ins::LocalGet(S),
        Ins::LocalGet(SEP),
        Ins::LocalGet(FROM),
        Ins::I64ExtendI32S,
        find,
        Ins::I32WrapI64,
        Ins::LocalSet(FOUND),
    ];
    writing.extend(when(
        vec![Ins::LocalGet(FOUND), Ins::I32Const(0), Ins::I32LtS],
        vec![Ins::LocalGet(N), Ins::LocalSet(FOUND)],
    ));
    writing.extend(element_at(OUT, WRITTEN));
    writing.extend([
        Ins::LocalGet(S),
        Ins::LocalGet(FROM),
        Ins::I64ExtendI32S,
        Ins::LocalGet(FOUND),
        Ins::I64ExtendI32S,
        slice,
        Ins::I64Store(0),
        Ins::LocalGet(FOUND),
        Ins::LocalGet(NSEP),
        Ins::I32Add,
        Ins::LocalSet(FROM),
    ]);
    body.extend(count_to(WRITTEN, COUNT, writing));
    body.extend([Ins::LocalGet(OUT), Ins::I64ExtendI32S, Ins::Return]);

    (
        vec![
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
        ],
        body,
    )
}

/// `join(pieces, separator) -> String`.
///
/// Two walks again, and for the same reason: the room the answer takes is the
/// sum of what goes in it, and that is not known until everything has been
/// measured.
fn pieces_text() -> (Vec<ValType>, Vec<Ins>) {
    const XS: u32 = 0;
    const SEP: u32 = 1;
    const FROM: u32 = 2;
    const COUNT: u32 = 3;
    const BYTES: u32 = 4;
    const CHARS: u32 = 5;
    const OUT: u32 = 6;
    const I: u32 = 7;
    const PIECE: u32 = 8;
    const WROTE: u32 = 9;
    const J: u32 = 10;
    const NP: u32 = 11;

    let mut body = vec![
        Ins::LocalGet(XS),
        Ins::I32WrapI64,
        Ins::LocalSet(FROM),
        Ins::LocalGet(FROM),
        Ins::I64Load(0),
        Ins::I32WrapI64,
        Ins::LocalSet(COUNT),
        Ins::I32Const(0),
        Ins::LocalSet(BYTES),
        Ins::I32Const(0),
        Ins::LocalSet(CHARS),
    ];

    let mut measure = element_at(FROM, I);
    measure.extend([Ins::I64Load(0), Ins::LocalSet(PIECE), Ins::LocalGet(BYTES)]);
    measure.extend(byte_length(PIECE));
    measure.extend([
        Ins::I32Add,
        Ins::LocalSet(BYTES),
        Ins::LocalGet(CHARS),
        Ins::LocalGet(PIECE),
        Ins::I32WrapI64,
        Ins::I64Load(0),
        Ins::I32WrapI64,
        Ins::I32Add,
        Ins::LocalSet(CHARS),
    ]);
    body.extend(count_to(I, COUNT, measure));

    // The separator goes between, so one fewer than there are pieces.
    body.extend(when(
        vec![Ins::LocalGet(COUNT), Ins::I32Const(0), Ins::I32GtS],
        {
            let mut ins = vec![Ins::LocalGet(BYTES)];
            ins.extend(byte_length(SEP));
            ins.extend([
                Ins::LocalGet(COUNT),
                Ins::I32Const(1),
                Ins::I32Sub,
                Ins::I32Mul,
                Ins::I32Add,
                Ins::LocalSet(BYTES),
                Ins::LocalGet(CHARS),
                Ins::LocalGet(SEP),
                Ins::I32WrapI64,
                Ins::I64Load(0),
                Ins::I32WrapI64,
                Ins::LocalGet(COUNT),
                Ins::I32Const(1),
                Ins::I32Sub,
                Ins::I32Mul,
                Ins::I32Add,
                Ins::LocalSet(CHARS),
            ]);
            ins
        },
    ));

    body.extend(allocate(OUT, |ins| ins.extend(string_room(BYTES))));
    body.extend([
        Ins::LocalGet(OUT),
        Ins::LocalGet(CHARS),
        Ins::I64ExtendI32S,
        Ins::I64Store(0),
        Ins::LocalGet(OUT),
        Ins::LocalGet(BYTES),
        Ins::I64ExtendI32S,
        Ins::I64Store(layout::WORD),
        Ins::I32Const(0),
        Ins::LocalSet(WROTE),
    ]);

    // One piece, preceded by the separator unless it is the first.
    let copy = |from: u32, counter: u32| {
        let mut step = vec![
            Ins::LocalGet(OUT),
            Ins::I32Const(TEXT as i32),
            Ins::I32Add,
            Ins::LocalGet(WROTE),
            Ins::I32Add,
            Ins::LocalGet(counter),
            Ins::I32Add,
        ];
        step.extend(text_at(from, counter));
        step.extend([Ins::I32Load8U(0), Ins::I32Store8(0)]);
        step
    };

    let mut write = Vec::new();
    write.extend(when(
        vec![Ins::LocalGet(I), Ins::I32Const(0), Ins::I32GtS],
        {
            let mut ins = byte_length(SEP);
            ins.push(Ins::LocalSet(NP));
            ins.extend(count_to(J, NP, copy(SEP, J)));
            ins.extend([
                Ins::LocalGet(WROTE),
                Ins::LocalGet(NP),
                Ins::I32Add,
                Ins::LocalSet(WROTE),
            ]);
            ins
        },
    ));
    write.extend(element_at(FROM, I));
    write.extend([Ins::I64Load(0), Ins::LocalSet(PIECE)]);
    write.extend(byte_length(PIECE));
    write.push(Ins::LocalSet(NP));
    write.extend(count_to(J, NP, copy(PIECE, J)));
    write.extend([
        Ins::LocalGet(WROTE),
        Ins::LocalGet(NP),
        Ins::I32Add,
        Ins::LocalSet(WROTE),
    ]);
    body.extend(count_to(I, COUNT, write));

    body.extend([Ins::LocalGet(OUT), Ins::I64ExtendI32S, Ins::Return]);

    (
        vec![
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I64,
            ValType::I32,
            ValType::I32,
            ValType::I32,
        ],
        body,
    )
}

/// `trim(s) -> String`, the four characters and not the Unicode table.
///
/// A large table is a large amount of behaviour to hide behind a four letter
/// name, which is the interpreter's reason and so is this one's.
fn trimmed_text(at: &mut Where<'_>) -> (Vec<ValType>, Vec<Ins>) {
    const S: u32 = 0;
    const N: u32 = 1;
    const FROM: u32 = 2;
    const TO: u32 = 3;

    let slice = at.call(Helper::TextSlice);

    let white = |index: u32| {
        let byte = || {
            let mut ins = text_at(S, index);
            ins.push(Ins::I32Load8U(0));
            ins
        };
        let mut ins = byte();
        ins.extend([Ins::I32Const(b' ' as i32), Ins::I32Eq]);
        for &character in b"\t\r\n" {
            ins.extend(byte());
            ins.extend([Ins::I32Const(character as i32), Ins::I32Eq, Ins::I32Or]);
        }
        ins
    };

    let mut body = byte_length(S);
    body.push(Ins::LocalSet(N));
    body.extend([
        Ins::I32Const(0),
        Ins::LocalSet(FROM),
        Ins::LocalGet(N),
        Ins::LocalSet(TO),
    ]);

    body.extend(walk_while(
        FROM,
        {
            let mut ins = vec![Ins::LocalGet(FROM), Ins::LocalGet(TO), Ins::I32LtS];
            ins.extend(white(FROM));
            ins.push(Ins::I32And);
            ins
        },
        Vec::new(),
    ));

    // From the other end, which counts down, so the loop is written out.
    body.push(Ins::Block {
        result: None,
        body: vec![Ins::Loop {
            result: None,
            body: {
                let mut inner = vec![
                    Ins::LocalGet(TO),
                    Ins::LocalGet(FROM),
                    Ins::I32LeS,
                    Ins::BrIf(1),
                ];
                let mut last = white({
                    // The byte before `TO`, which is the last one in range.
                    const BEFORE: u32 = 4;
                    BEFORE
                });
                last.push(Ins::I32Eqz);
                inner.extend([
                    Ins::LocalGet(TO),
                    Ins::I32Const(1),
                    Ins::I32Sub,
                    Ins::LocalSet(4),
                ]);
                inner.extend(last);
                inner.push(Ins::BrIf(1));
                inner.extend([Ins::LocalGet(4), Ins::LocalSet(TO), Ins::Br(0)]);
                inner
            },
        }],
    });

    body.extend([
        Ins::LocalGet(S),
        Ins::LocalGet(FROM),
        Ins::I64ExtendI32S,
        Ins::LocalGet(TO),
        Ins::I64ExtendI32S,
        slice,
        Ins::Return,
    ]);

    (
        vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
        body,
    )
}

/// `upper(s)` and `lower(s)`, the twenty-six letters and nothing else.
///
/// Every other byte comes back as it went in, so text in a script with no case
/// survives rather than being mangled by a rule that was not written for it.
/// Neither count changes, because the letters this touches are one byte each.
fn cased_text(raise: bool) -> (Vec<ValType>, Vec<Ins>) {
    const S: u32 = 0;
    const N: u32 = 1;
    const OUT: u32 = 2;
    const I: u32 = 3;
    const BYTE: u32 = 4;

    let (low, high, shift) = if raise {
        (b'a' as i32, b'z' as i32, -32)
    } else {
        (b'A' as i32, b'Z' as i32, 32)
    };

    let mut body = byte_length(S);
    body.push(Ins::LocalSet(N));
    body.extend(allocate(OUT, |ins| ins.extend(string_room(N))));
    body.extend([
        Ins::LocalGet(OUT),
        Ins::LocalGet(S),
        Ins::I32WrapI64,
        Ins::I64Load(0),
        Ins::I64Store(0),
        Ins::LocalGet(OUT),
        Ins::LocalGet(N),
        Ins::I64ExtendI32S,
        Ins::I64Store(layout::WORD),
    ]);

    let mut step = text_at(S, I);
    step.extend([Ins::I32Load8U(0), Ins::LocalSet(BYTE)]);
    step.extend(when(
        vec![
            Ins::LocalGet(BYTE),
            Ins::I32Const(low),
            Ins::I32GeS,
            Ins::LocalGet(BYTE),
            Ins::I32Const(high),
            Ins::I32LeS,
            Ins::I32And,
        ],
        vec![
            Ins::LocalGet(BYTE),
            Ins::I32Const(shift),
            Ins::I32Add,
            Ins::LocalSet(BYTE),
        ],
    ));
    step.extend([
        Ins::LocalGet(OUT),
        Ins::I32Const(TEXT as i32),
        Ins::I32Add,
        Ins::LocalGet(I),
        Ins::I32Add,
        Ins::LocalGet(BYTE),
        Ins::I32Store8(0),
    ]);
    body.extend(count_to(I, N, step));
    body.extend([Ins::LocalGet(OUT), Ins::I64ExtendI32S, Ins::Return]);

    (
        vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
        body,
    )
}

/// `to_int(s) -> Result<Int, String>`.
///
/// The same text the interpreter accepts: an optional sign, then at least one
/// digit, then nothing else. Accumulated negatively so that the smallest `Int`
/// is a number this can read, and checked before each step rather than after,
/// because after is too late.
fn text_number(at: &mut Where<'_>) -> (Vec<ValType>, Vec<Ins>) {
    const S: u32 = 0;
    const N: u32 = 1;
    const I: u32 = 2;
    const NEGATIVE: u32 = 3;
    const ACC: u32 = 4;
    const DIGIT: u32 = 5;
    const OUT: u32 = 6;
    const MESSAGE: u32 = 7;

    let quote = at.text("`");
    let tail = at.text("` is not a number");
    let join = at.call(Helper::JoinedText);

    let mut refuse = vec![
        quote,
        Ins::LocalGet(S),
        join.clone(),
        tail,
        join,
        Ins::LocalSet(MESSAGE),
    ];
    refuse.extend(result(OUT, ERR, |ins| ins.push(Ins::LocalGet(MESSAGE))));

    let mut body = byte_length(S);
    body.push(Ins::LocalSet(N));
    body.extend([
        Ins::I32Const(0),
        Ins::LocalSet(I),
        Ins::I32Const(0),
        Ins::LocalSet(NEGATIVE),
        Ins::I64Const(0),
        Ins::LocalSet(ACC),
    ]);

    for (character, mark) in [(b'-', true), (b'+', false)] {
        let mut sign = vec![Ins::LocalGet(I), Ins::LocalGet(N), Ins::I32LtS];
        sign.extend(text_at(S, I));
        sign.extend([
            Ins::I32Load8U(0),
            Ins::I32Const(character as i32),
            Ins::I32Eq,
            Ins::I32And,
        ]);
        let mut then = Vec::new();
        if mark {
            then.extend([Ins::I32Const(1), Ins::LocalSet(NEGATIVE)]);
        }
        then.extend([
            Ins::LocalGet(I),
            Ins::I32Const(1),
            Ins::I32Add,
            Ins::LocalSet(I),
        ]);
        body.extend(when(sign, then));
    }

    // A sign and nothing else is not a number.
    body.extend(when(
        vec![Ins::LocalGet(I), Ins::LocalGet(N), Ins::I32GeS],
        refuse.clone(),
    ));

    let mut digit = text_at(S, I);
    digit.extend([Ins::I32Load8U(0), Ins::LocalSet(DIGIT)]);
    digit.extend(when(
        vec![
            Ins::LocalGet(DIGIT),
            Ins::I32Const(b'0' as i32),
            Ins::I32LtS,
            Ins::LocalGet(DIGIT),
            Ins::I32Const(b'9' as i32),
            Ins::I32GtS,
            Ins::I32Or,
        ],
        refuse.clone(),
    ));
    // Room for another digit, asked before the multiply rather than after.
    digit.extend(when(
        vec![
            Ins::LocalGet(ACC),
            Ins::I64Const(i64::MIN / 10),
            Ins::I64LtS,
        ],
        refuse.clone(),
    ));
    digit.extend([
        Ins::LocalGet(ACC),
        Ins::I64Const(10),
        Ins::I64Mul,
        Ins::LocalSet(ACC),
    ]);
    digit.extend(when(
        vec![
            Ins::LocalGet(ACC),
            Ins::I64Const(i64::MIN),
            Ins::LocalGet(DIGIT),
            Ins::I32Const(b'0' as i32),
            Ins::I32Sub,
            Ins::I64ExtendI32S,
            Ins::I64Add,
            Ins::I64LtS,
        ],
        refuse.clone(),
    ));
    digit.extend([
        Ins::LocalGet(ACC),
        Ins::LocalGet(DIGIT),
        Ins::I32Const(b'0' as i32),
        Ins::I32Sub,
        Ins::I64ExtendI32S,
        Ins::I64Sub,
        Ins::LocalSet(ACC),
    ]);
    body.extend(count_to(I, N, digit));

    // A positive answer is the accumulator turned round, and the smallest
    // `Int` has no positive counterpart to turn into.
    body.extend(when(
        vec![
            Ins::LocalGet(NEGATIVE),
            Ins::I32Eqz,
            Ins::LocalGet(ACC),
            Ins::I64Const(i64::MIN),
            Ins::I64Eq,
            Ins::I32And,
        ],
        refuse,
    ));
    body.extend(when(
        vec![Ins::LocalGet(NEGATIVE), Ins::I32Eqz],
        vec![
            Ins::I64Const(0),
            Ins::LocalGet(ACC),
            Ins::I64Sub,
            Ins::LocalSet(ACC),
        ],
    ));
    body.extend(result(OUT, OK, |ins| ins.push(Ins::LocalGet(ACC))));

    (
        vec![
            ValType::I32,
            ValType::I32,
            ValType::I32,
            ValType::I64,
            ValType::I32,
            ValType::I32,
            ValType::I64,
        ],
        body,
    )
}
