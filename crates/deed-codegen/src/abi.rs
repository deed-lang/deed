//! Canonical ABI: how Deed values cross a component boundary.
//!
//! The WebAssembly Component Model canonical ABI defines how values travel
//! between a component and its host.  This module maps each Deed type to its
//! canonical representation and provides [`lower`] and [`lift`] for round-trip
//! testing.
//!
//! Deed types at the component boundary:
//!
//! | Deed type        | Canonical flat type(s)                               |
//! |------------------|------------------------------------------------------|
//! | `Unit`           | (none)                                               |
//! | `Bool`           | `i32` (0 = false, 1 = true)                         |
//! | `Int`            | `i64`                                                |
//! | `String`         | `(i32 ptr, i32 byte_len)` -- UTF-8 in memory         |
//! | `List<T>`        | `(i32 ptr, i32 count)` -- elements in memory         |
//! | record           | fields flattened in declaration order                |
//! | choice / Result  | `i32` discriminant + join of all variant flat types  |
//! | `Closure`        | refused -- closures cannot cross a component boundary|
//! | `Capability`     | refused -- capabilities are scoped to their issuer   |
//!
//! When the flat count exceeds [`MAX_FLAT`] the value is passed by pointer in
//! memory; the flat representation collapses to a single `i32`.
//!
//! [`Memory`] is a simple bump-allocated stand-in used by the round-trip tests
//! for the component linear memory that a real host would provide.
//!
//! The internal eight-byte-word layout the compiled backend already chose
//! (`crates/deed-codegen/src/layout.rs`) is the *module* representation; it
//! does not need to change.  The canonical ABI is a separate marshalling layer
//! that sits at the *component* boundary, translating between the two.
//!
//! References: WebAssembly Component Model canonical ABI specification
//! <https://github.com/WebAssembly/component-model/blob/main/design/mvp/CanonicalABI.md>

use deed_mir::{Layout, LayoutId, Ty, Variant};

// --- unsupported types ---------------------------------------------------

/// A Deed type that cannot cross a component boundary.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Unsupported {
    /// Closures capture state from their enclosing scope and have no stable
    /// calling convention across a component boundary.
    Closure,
    /// Capabilities are unforgeable handles scoped to their issuer.  Allowing
    /// one to cross a boundary would widen authority beyond what the issuer
    /// granted.
    Capability,
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unsupported::Closure => write!(f, "closures cannot cross a component boundary"),
            Unsupported::Capability => write!(
                f,
                "capabilities are scoped to their issuer and cannot cross a component boundary"
            ),
        }
    }
}

// --- flat type representation --------------------------------------------

/// A canonical ABI flat machine type.
///
/// The canonical ABI uses only `i32` and `i64` in flat representations.  All
/// Deed integer types fit in one of these two widths.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlatType {
    I32,
    I64,
}

/// Maximum flat values before a value must be passed by pointer.
///
/// When [`flatten`] returns more than this many entries the canonical ABI
/// collapses the representation to a single `i32` pointer into component
/// memory.
pub const MAX_FLAT: usize = 16;

/// The flat machine types a Deed value of this type presents at the boundary.
///
/// Returns `Ok([])` for `Unit`, which contributes no arguments.  Returns `Err`
/// for `Closure` and `Capability`, which cannot cross.
///
/// When the returned list is longer than [`MAX_FLAT`] the type is passed by
/// pointer; use [`size_of`] and [`align_of`] to compute the layout.
pub fn flatten(ty: &Ty, layouts: &[Layout]) -> Result<Vec<FlatType>, Unsupported> {
    Ok(match ty {
        Ty::Unit => vec![],
        Ty::Bool => vec![FlatType::I32],
        Ty::Int => vec![FlatType::I64],
        Ty::Str => vec![FlatType::I32, FlatType::I32],
        Ty::List(_) => vec![FlatType::I32, FlatType::I32],
        Ty::Aggregate(id) => flatten_aggregate(*id, layouts)?,
        Ty::Closure => return Err(Unsupported::Closure),
        Ty::Capability => return Err(Unsupported::Capability),
    })
}

fn flatten_aggregate(id: LayoutId, layouts: &[Layout]) -> Result<Vec<FlatType>, Unsupported> {
    let layout = &layouts[id.0];
    if !layout.is_tagged() {
        // Record: concatenate flattened fields in declaration order.
        let mut flat = vec![];
        for field in &layout.variants[0].fields {
            flat.extend(flatten(&field.ty, layouts)?);
        }
        Ok(flat)
    } else {
        // Choice / Result: i32 discriminant followed by the element-wise join
        // of every variant's flat types.  The join widens i32 to i64 wherever
        // any variant needs an i64 in the same position.
        let mut payload: Vec<FlatType> = vec![];
        for variant in &layout.variants {
            let mut v_flat = vec![];
            for field in &variant.fields {
                v_flat.extend(flatten(&field.ty, layouts)?);
            }
            payload = join_flat_lists(&payload, &v_flat);
        }
        let mut flat = vec![FlatType::I32]; // discriminant
        flat.extend(payload);
        Ok(flat)
    }
}

/// Element-wise widening join of two flat-type lists.
///
/// Where both lists have an entry at the same position, `i64` wins over `i32`.
/// When one list is shorter its absent positions are taken from the longer one.
fn join_flat_lists(a: &[FlatType], b: &[FlatType]) -> Vec<FlatType> {
    let len = a.len().max(b.len());
    (0..len)
        .map(|i| match (a.get(i), b.get(i)) {
            (Some(&ta), Some(&tb)) => join_one(ta, tb),
            (Some(&ta), None) => ta,
            (None, Some(&tb)) => tb,
            (None, None) => unreachable!(),
        })
        .collect()
}

fn join_one(a: FlatType, b: FlatType) -> FlatType {
    if matches!(a, FlatType::I64) || matches!(b, FlatType::I64) {
        FlatType::I64
    } else {
        FlatType::I32
    }
}

// --- size and alignment --------------------------------------------------

/// Byte size of a Deed type in a component's linear memory.
///
/// Used when the flat count exceeds [`MAX_FLAT`] and the value must be passed
/// by pointer instead.
pub fn size_of(ty: &Ty, layouts: &[Layout]) -> Result<usize, Unsupported> {
    Ok(match ty {
        Ty::Unit => 0,
        Ty::Bool => 1,
        Ty::Int => 8,
        Ty::Str | Ty::List(_) => 8, // (i32 ptr, i32 len/count)
        Ty::Aggregate(id) => size_aggregate(*id, layouts)?,
        Ty::Closure => return Err(Unsupported::Closure),
        Ty::Capability => return Err(Unsupported::Capability),
    })
}

/// Byte alignment of a Deed type in a component's linear memory.
pub fn align_of(ty: &Ty, layouts: &[Layout]) -> Result<usize, Unsupported> {
    Ok(match ty {
        Ty::Unit => 1,
        Ty::Bool => 1,
        Ty::Int => 8,
        Ty::Str | Ty::List(_) => 4, // pointer and length are both i32
        Ty::Aggregate(id) => align_aggregate(*id, layouts)?,
        Ty::Closure => return Err(Unsupported::Closure),
        Ty::Capability => return Err(Unsupported::Capability),
    })
}

/// Round `n` up to the next multiple of `a` (which must be a power of two).
fn align_up(n: usize, a: usize) -> usize {
    (n + a - 1) & !(a - 1)
}

fn discriminant_size(variants: usize) -> usize {
    if variants <= 0x100 {
        1
    } else if variants <= 0x1_0000 {
        2
    } else {
        4
    }
}

fn size_aggregate(id: LayoutId, layouts: &[Layout]) -> Result<usize, Unsupported> {
    let layout = &layouts[id.0];
    let overall_align = align_aggregate(id, layouts)?;
    if !layout.is_tagged() {
        let mut offset = 0;
        for field in &layout.variants[0].fields {
            offset = align_up(offset, align_of(&field.ty, layouts)?);
            offset += size_of(&field.ty, layouts)?;
        }
        Ok(align_up(offset, overall_align))
    } else {
        let max_ca = max_variant_align(&layout.variants, layouts)?;
        let ds = discriminant_size(layout.variants.len());
        let payload_offset = align_up(ds, max_ca);
        let max_payload = max_variant_payload_size(&layout.variants, layouts)?;
        Ok(align_up(payload_offset + max_payload, overall_align))
    }
}

fn align_aggregate(id: LayoutId, layouts: &[Layout]) -> Result<usize, Unsupported> {
    let layout = &layouts[id.0];
    if !layout.is_tagged() {
        let mut a = 1;
        for field in &layout.variants[0].fields {
            a = a.max(align_of(&field.ty, layouts)?);
        }
        Ok(a)
    } else {
        let max_ca = max_variant_align(&layout.variants, layouts)?;
        Ok(discriminant_size(layout.variants.len()).max(max_ca))
    }
}

fn max_variant_align(variants: &[Variant], layouts: &[Layout]) -> Result<usize, Unsupported> {
    let mut a = 1;
    for v in variants {
        for field in &v.fields {
            a = a.max(align_of(&field.ty, layouts)?);
        }
    }
    Ok(a)
}

fn max_variant_payload_size(
    variants: &[Variant],
    layouts: &[Layout],
) -> Result<usize, Unsupported> {
    let mut max = 0;
    for v in variants {
        let mut offset = 0;
        for field in &v.fields {
            offset = align_up(offset, align_of(&field.ty, layouts)?);
            offset += size_of(&field.ty, layouts)?;
        }
        max = max.max(offset);
    }
    Ok(max)
}

// --- values and errors ---------------------------------------------------

/// A Deed value in its canonical ABI form.
///
/// Mirrors the language value types but excludes `Closure` and `Capability`,
/// which cannot cross a component boundary.  Used by [`lift`] and [`lower`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Val {
    Unit,
    Bool(bool),
    Int(i64),
    Str(String),
    List(Vec<Val>),
    /// A record: field values in declaration order.
    Record(Vec<Val>),
    /// A choice variant or `Result`: discriminant index, payload fields in
    /// declaration order.
    Variant {
        discriminant: usize,
        fields: Vec<Val>,
    },
}

/// An error that prevents a [`lift`] or [`lower`] from completing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Error {
    OutOfBounds,
    InvalidUtf8,
    /// A discriminant read from memory named a variant that does not exist.
    InvalidDiscriminant {
        got: usize,
        max: usize,
    },
    Unsupported(Unsupported),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::OutOfBounds => write!(f, "memory access out of bounds"),
            Error::InvalidUtf8 => write!(f, "string bytes are not valid UTF-8"),
            Error::InvalidDiscriminant { got, max } => {
                write!(f, "discriminant {got} is past the last variant (max {max})")
            }
            Error::Unsupported(u) => write!(f, "{u}"),
        }
    }
}

impl From<Unsupported> for Error {
    fn from(u: Unsupported) -> Self {
        Error::Unsupported(u)
    }
}

// --- flat argument values ------------------------------------------------

/// A flat argument value at the component boundary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlatArg {
    I32(i32),
    I64(i64),
}

impl FlatArg {
    fn as_i64(self) -> i64 {
        match self {
            FlatArg::I32(v) => v as i64,
            FlatArg::I64(v) => v,
        }
    }

    fn as_i32(self) -> i32 {
        match self {
            FlatArg::I32(v) => v,
            FlatArg::I64(v) => v as i32,
        }
    }

    fn zero_for(ty: FlatType) -> Self {
        match ty {
            FlatType::I32 => FlatArg::I32(0),
            FlatType::I64 => FlatArg::I64(0),
        }
    }
}

// --- component memory ----------------------------------------------------

/// Bump-allocated component linear memory for round-trip testing.
///
/// [`lower`] writes string bytes and list elements here; [`lift`] reads them
/// back.  Not for production use: nothing is freed.
pub struct Memory {
    bytes: Vec<u8>,
    next: usize,
}

impl Memory {
    /// A fresh 64 KiB component memory.
    pub fn new() -> Self {
        Self {
            bytes: vec![0u8; 64 * 1024],
            next: 0,
        }
    }

    fn alloc(&mut self, size: usize, align: usize) -> Result<usize, Error> {
        let ptr = align_up(self.next, align.max(1));
        let end = ptr.checked_add(size).ok_or(Error::OutOfBounds)?;
        if end > self.bytes.len() {
            return Err(Error::OutOfBounds);
        }
        self.next = end;
        Ok(ptr)
    }

    fn write_bytes(&mut self, at: usize, data: &[u8]) -> Result<(), Error> {
        let dest = self
            .bytes
            .get_mut(at..at + data.len())
            .ok_or(Error::OutOfBounds)?;
        dest.copy_from_slice(data);
        Ok(())
    }

    fn read_bytes(&self, at: usize, len: usize) -> Result<&[u8], Error> {
        self.bytes.get(at..at + len).ok_or(Error::OutOfBounds)
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

// --- lower ---------------------------------------------------------------

/// Lower a Deed value to its canonical flat representation.
///
/// Heap data (string bytes, list elements) is written into `memory`; the
/// returned `FlatArg` list carries pointers and lengths for those.  The number
/// and types of flat arguments match [`flatten`] for the same type.
///
/// When the flat count would exceed [`MAX_FLAT`] the value is written directly
/// into `memory` and only a single `i32` pointer is returned.
///
/// Returns `Err` for `Closure` and `Capability` (see [`Unsupported`]).
pub fn lower(
    val: &Val,
    ty: &Ty,
    layouts: &[Layout],
    memory: &mut Memory,
) -> Result<Vec<FlatArg>, Error> {
    // If the type needs indirect passing, store in memory and return a pointer.
    let flat_types = flatten(ty, layouts)?;
    if flat_types.len() > MAX_FLAT {
        let size = size_of(ty, layouts)?;
        let align = align_of(ty, layouts)?;
        let ptr = memory.alloc(size, align)?;
        lower_to_memory(val, ty, layouts, memory, ptr)?;
        return Ok(vec![FlatArg::I32(ptr as i32)]);
    }
    lower_flat(val, ty, &flat_types, layouts, memory)
}

fn lower_flat(
    val: &Val,
    ty: &Ty,
    flat_types: &[FlatType],
    layouts: &[Layout],
    memory: &mut Memory,
) -> Result<Vec<FlatArg>, Error> {
    match (val, ty) {
        (Val::Unit, Ty::Unit) => Ok(vec![]),
        (Val::Bool(b), Ty::Bool) => Ok(vec![FlatArg::I32(*b as i32)]),
        (Val::Int(n), Ty::Int) => Ok(vec![FlatArg::I64(*n)]),
        (Val::Str(s), Ty::Str) => lower_str(s, memory),
        (Val::List(elements), Ty::List(element_ty)) => {
            lower_list(elements, element_ty, layouts, memory)
        }
        (Val::Record(fields), Ty::Aggregate(id)) if !layouts[id.0].is_tagged() => {
            lower_record_flat(fields, *id, layouts, memory)
        }
        (
            Val::Variant {
                discriminant,
                fields,
            },
            Ty::Aggregate(id),
        ) if layouts[id.0].is_tagged() => {
            lower_variant_flat(*discriminant, fields, *id, flat_types, layouts, memory)
        }
        (_, Ty::Closure) => Err(Error::from(Unsupported::Closure)),
        (_, Ty::Capability) => Err(Error::from(Unsupported::Capability)),
        // Any other combination is a programmer error in the test; return empty
        // rather than panic so the mismatch surfaces as an assertion failure.
        _ => Ok(vec![]),
    }
}

fn lower_str(s: &str, memory: &mut Memory) -> Result<Vec<FlatArg>, Error> {
    let bytes = s.as_bytes();
    let ptr = if bytes.is_empty() {
        0
    } else {
        let p = memory.alloc(bytes.len(), 1)?;
        memory.write_bytes(p, bytes)?;
        p
    };
    Ok(vec![
        FlatArg::I32(ptr as i32),
        FlatArg::I32(bytes.len() as i32),
    ])
}

fn lower_list(
    elements: &[Val],
    element_ty: &Ty,
    layouts: &[Layout],
    memory: &mut Memory,
) -> Result<Vec<FlatArg>, Error> {
    let stride = element_stride(element_ty, layouts)?;
    let ptr = if elements.is_empty() {
        0
    } else {
        let total = stride * elements.len();
        let elem_align = align_of(element_ty, layouts)?;
        let p = memory.alloc(total, elem_align.max(1))?;
        for (i, element) in elements.iter().enumerate() {
            lower_to_memory(element, element_ty, layouts, memory, p + i * stride)?;
        }
        p
    };
    Ok(vec![
        FlatArg::I32(ptr as i32),
        FlatArg::I32(elements.len() as i32),
    ])
}

fn lower_record_flat(
    fields: &[Val],
    id: LayoutId,
    layouts: &[Layout],
    memory: &mut Memory,
) -> Result<Vec<FlatArg>, Error> {
    let layout = &layouts[id.0];
    let field_defs = &layout.variants[0].fields;
    let mut args = vec![];
    for (fval, fdef) in fields.iter().zip(field_defs.iter()) {
        let ft = flatten(&fdef.ty, layouts)?;
        args.extend(lower_flat(fval, &fdef.ty, &ft, layouts, memory)?);
    }
    Ok(args)
}

fn lower_variant_flat(
    discriminant: usize,
    fields: &[Val],
    id: LayoutId,
    flat_types: &[FlatType],
    layouts: &[Layout],
    memory: &mut Memory,
) -> Result<Vec<FlatArg>, Error> {
    let layout = &layouts[id.0];
    let variant_def = &layout.variants[discriminant];

    // Lower the actual payload for this variant.
    let mut payload = vec![];
    for (fval, fdef) in fields.iter().zip(variant_def.fields.iter()) {
        let ft = flatten(&fdef.ty, layouts)?;
        payload.extend(lower_flat(fval, &fdef.ty, &ft, layouts, memory)?);
    }

    // The join may have widened some i32 slots to i64.  Match flat_types[1+i].
    for (i, arg) in payload.iter_mut().enumerate() {
        if let Some(&FlatType::I64) = flat_types.get(i + 1) {
            *arg = FlatArg::I64(arg.as_i64());
        }
    }

    // Pad remaining slots (from wider variants) with zeros.
    let needed = flat_types.len().saturating_sub(1);
    while payload.len() < needed {
        payload.push(FlatArg::zero_for(flat_types[payload.len() + 1]));
    }

    let mut args = vec![FlatArg::I32(discriminant as i32)];
    args.extend(payload);
    Ok(args)
}

/// Write a value into component memory at `offset`, following the canonical
/// ABI memory layout (sizes, alignments, field padding).
fn lower_to_memory(
    val: &Val,
    ty: &Ty,
    layouts: &[Layout],
    memory: &mut Memory,
    offset: usize,
) -> Result<(), Error> {
    match (val, ty) {
        (Val::Unit, Ty::Unit) => {}
        (Val::Bool(b), Ty::Bool) => {
            memory.write_bytes(offset, &[*b as u8])?;
        }
        (Val::Int(n), Ty::Int) => {
            memory.write_bytes(offset, &n.to_le_bytes())?;
        }
        (Val::Str(s), Ty::Str) => {
            let bytes = s.as_bytes();
            let ptr = if bytes.is_empty() {
                0u32
            } else {
                let p = memory.alloc(bytes.len(), 1)?;
                memory.write_bytes(p, bytes)?;
                p as u32
            };
            memory.write_bytes(offset, &ptr.to_le_bytes())?;
            memory.write_bytes(offset + 4, &(bytes.len() as u32).to_le_bytes())?;
        }
        (Val::List(elements), Ty::List(element_ty)) => {
            let stride = element_stride(element_ty, layouts)?;
            let ptr = if elements.is_empty() {
                0u32
            } else {
                let total = stride * elements.len();
                let elem_align = align_of(element_ty, layouts)?;
                let p = memory.alloc(total, elem_align.max(1))?;
                for (i, element) in elements.iter().enumerate() {
                    lower_to_memory(element, element_ty, layouts, memory, p + i * stride)?;
                }
                p as u32
            };
            memory.write_bytes(offset, &ptr.to_le_bytes())?;
            memory.write_bytes(offset + 4, &(elements.len() as u32).to_le_bytes())?;
        }
        (Val::Record(fields), Ty::Aggregate(id)) if !layouts[id.0].is_tagged() => {
            let layout = &layouts[id.0];
            let field_defs = &layout.variants[0].fields;
            let mut off = offset;
            for (fval, fdef) in fields.iter().zip(field_defs.iter()) {
                off = align_up(off, align_of(&fdef.ty, layouts)?);
                lower_to_memory(fval, &fdef.ty, layouts, memory, off)?;
                off += size_of(&fdef.ty, layouts)?;
            }
        }
        (
            Val::Variant {
                discriminant,
                fields,
            },
            Ty::Aggregate(id),
        ) if layouts[id.0].is_tagged() => {
            let layout = &layouts[id.0];
            let variant_def = &layout.variants[*discriminant];
            let ds = discriminant_size(layout.variants.len());
            // Write the discriminant (little-endian, truncated to ds bytes).
            let disc_bytes = (*discriminant as u32).to_le_bytes();
            memory.write_bytes(offset, &disc_bytes[..ds])?;
            // Payload starts at offset + disc rounded up to max case alignment.
            let max_ca = max_variant_align(&layout.variants, layouts)?;
            let payload_base = offset + align_up(ds, max_ca);
            let mut poff = payload_base;
            for (fval, fdef) in fields.iter().zip(variant_def.fields.iter()) {
                poff = align_up(poff, align_of(&fdef.ty, layouts)?);
                lower_to_memory(fval, &fdef.ty, layouts, memory, poff)?;
                poff += size_of(&fdef.ty, layouts)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// The stride between consecutive elements of a list, in bytes.
fn element_stride(ty: &Ty, layouts: &[Layout]) -> Result<usize, Unsupported> {
    let s = size_of(ty, layouts)?;
    let a = align_of(ty, layouts)?;
    Ok(align_up(s, a.max(1)))
}

// --- lift ----------------------------------------------------------------

/// Lift a Deed value from its canonical flat representation.
///
/// The `flat` slice must contain exactly as many entries as [`flatten`]
/// returns for `ty`.  Heap data (string bytes, list elements) is read from
/// `memory`.
///
/// Returns `Err` for `Closure` and `Capability`, for out-of-bounds memory
/// accesses, for non-UTF-8 strings, and for discriminants past the last
/// variant.
pub fn lift(flat: &[FlatArg], ty: &Ty, layouts: &[Layout], memory: &Memory) -> Result<Val, Error> {
    match ty {
        Ty::Unit => Ok(Val::Unit),
        Ty::Bool => Ok(Val::Bool(flat.first().is_some_and(|a| a.as_i32() != 0))),
        Ty::Int => Ok(Val::Int(flat.first().map_or(0, |a| a.as_i64()))),
        Ty::Str => lift_str(flat, memory),
        Ty::List(element_ty) => lift_list(flat, element_ty, layouts, memory),
        Ty::Aggregate(id) => lift_aggregate_flat(flat, *id, layouts, memory),
        Ty::Closure => Err(Error::from(Unsupported::Closure)),
        Ty::Capability => Err(Error::from(Unsupported::Capability)),
    }
}

fn lift_str(flat: &[FlatArg], memory: &Memory) -> Result<Val, Error> {
    let ptr = flat.first().map_or(0, |a| a.as_i32()) as usize;
    let len = flat.get(1).map_or(0, |a| a.as_i32()) as usize;
    if len == 0 {
        return Ok(Val::Str(String::new()));
    }
    let bytes = memory.read_bytes(ptr, len)?;
    let s = std::str::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)?;
    Ok(Val::Str(s.to_string()))
}

fn lift_list(
    flat: &[FlatArg],
    element_ty: &Ty,
    layouts: &[Layout],
    memory: &Memory,
) -> Result<Val, Error> {
    let ptr = flat.first().map_or(0, |a| a.as_i32()) as usize;
    let count = flat.get(1).map_or(0, |a| a.as_i32()) as usize;
    let stride = element_stride(element_ty, layouts)?;
    let mut elements = Vec::with_capacity(count);
    for i in 0..count {
        elements.push(lift_from_memory(
            element_ty,
            layouts,
            memory,
            ptr + i * stride,
        )?);
    }
    Ok(Val::List(elements))
}

fn lift_aggregate_flat(
    flat: &[FlatArg],
    id: LayoutId,
    layouts: &[Layout],
    memory: &Memory,
) -> Result<Val, Error> {
    let layout = &layouts[id.0];
    if !layout.is_tagged() {
        // Record: consume flat args for each field in declaration order.
        let field_defs = &layout.variants[0].fields;
        let mut cursor = 0;
        let mut fields = Vec::with_capacity(field_defs.len());
        for fdef in field_defs {
            let ft = flatten(&fdef.ty, layouts)?;
            let slice = &flat[cursor..cursor + ft.len()];
            // Coerce: the join may have widened i32 to i64; narrow back.
            let coerced = coerce_flat(slice, &ft);
            fields.push(lift(&coerced, &fdef.ty, layouts, memory)?);
            cursor += ft.len();
        }
        Ok(Val::Record(fields))
    } else {
        // Choice / Result: read discriminant, then lift the matching payload.
        let disc = flat.first().map_or(0, |a| a.as_i32() as usize);
        if disc >= layout.variants.len() {
            return Err(Error::InvalidDiscriminant {
                got: disc,
                max: layout.variants.len() - 1,
            });
        }
        let variant_def = &layout.variants[disc];
        let mut cursor = 1; // skip the discriminant slot
        let mut fields = Vec::with_capacity(variant_def.fields.len());
        for fdef in &variant_def.fields {
            let ft = flatten(&fdef.ty, layouts)?;
            let slice = &flat[cursor..cursor + ft.len()];
            // The join may have widened this variant's i32 slots to i64.
            let coerced = coerce_flat(slice, &ft);
            fields.push(lift(&coerced, &fdef.ty, layouts, memory)?);
            cursor += ft.len();
        }
        Ok(Val::Variant {
            discriminant: disc,
            fields,
        })
    }
}

/// Coerce a slice of [`FlatArg`]s to match the expected [`FlatType`]s.
///
/// The canonical ABI widens `i32` payload slots to `i64` when another variant
/// needs `i64` at the same position.  Lifting has to narrow them back for the
/// variant that placed an `i32` there.
fn coerce_flat(have: &[FlatArg], want: &[FlatType]) -> Vec<FlatArg> {
    have.iter()
        .zip(want.iter())
        .map(|(arg, ty)| match ty {
            FlatType::I32 => FlatArg::I32(arg.as_i32()),
            FlatType::I64 => FlatArg::I64(arg.as_i64()),
        })
        .collect()
}

/// Lift a value from a block of component memory at `offset`.
fn lift_from_memory(
    ty: &Ty,
    layouts: &[Layout],
    memory: &Memory,
    offset: usize,
) -> Result<Val, Error> {
    match ty {
        Ty::Unit => Ok(Val::Unit),
        Ty::Bool => {
            let b = memory.read_bytes(offset, 1)?[0] != 0;
            Ok(Val::Bool(b))
        }
        Ty::Int => {
            let bytes: [u8; 8] = memory
                .read_bytes(offset, 8)?
                .try_into()
                .expect("eight bytes");
            Ok(Val::Int(i64::from_le_bytes(bytes)))
        }
        Ty::Str => {
            let ptr_bytes: [u8; 4] = memory
                .read_bytes(offset, 4)?
                .try_into()
                .expect("four bytes");
            let len_bytes: [u8; 4] = memory
                .read_bytes(offset + 4, 4)?
                .try_into()
                .expect("four bytes");
            let ptr = u32::from_le_bytes(ptr_bytes) as usize;
            let len = u32::from_le_bytes(len_bytes) as usize;
            if len == 0 {
                return Ok(Val::Str(String::new()));
            }
            let bytes = memory.read_bytes(ptr, len)?;
            let s = std::str::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)?;
            Ok(Val::Str(s.to_string()))
        }
        Ty::List(element_ty) => {
            let ptr_bytes: [u8; 4] = memory
                .read_bytes(offset, 4)?
                .try_into()
                .expect("four bytes");
            let count_bytes: [u8; 4] = memory
                .read_bytes(offset + 4, 4)?
                .try_into()
                .expect("four bytes");
            let ptr = u32::from_le_bytes(ptr_bytes) as usize;
            let count = u32::from_le_bytes(count_bytes) as usize;
            let stride = element_stride(element_ty, layouts)?;
            let mut elements = Vec::with_capacity(count);
            for i in 0..count {
                elements.push(lift_from_memory(
                    element_ty,
                    layouts,
                    memory,
                    ptr + i * stride,
                )?);
            }
            Ok(Val::List(elements))
        }
        Ty::Aggregate(id) => lift_aggregate_from_memory(*id, layouts, memory, offset),
        Ty::Closure => Err(Error::from(Unsupported::Closure)),
        Ty::Capability => Err(Error::from(Unsupported::Capability)),
    }
}

fn lift_aggregate_from_memory(
    id: LayoutId,
    layouts: &[Layout],
    memory: &Memory,
    offset: usize,
) -> Result<Val, Error> {
    let layout = &layouts[id.0];
    if !layout.is_tagged() {
        let field_defs = &layout.variants[0].fields;
        let mut off = offset;
        let mut fields = Vec::with_capacity(field_defs.len());
        for fdef in field_defs {
            off = align_up(off, align_of(&fdef.ty, layouts)?);
            fields.push(lift_from_memory(&fdef.ty, layouts, memory, off)?);
            off += size_of(&fdef.ty, layouts)?;
        }
        Ok(Val::Record(fields))
    } else {
        let ds = discriminant_size(layout.variants.len());
        let disc_bytes = memory.read_bytes(offset, ds)?;
        let disc = match ds {
            1 => disc_bytes[0] as usize,
            2 => u16::from_le_bytes(disc_bytes.try_into().expect("two bytes")) as usize,
            _ => u32::from_le_bytes(disc_bytes.try_into().expect("four bytes")) as usize,
        };
        if disc >= layout.variants.len() {
            return Err(Error::InvalidDiscriminant {
                got: disc,
                max: layout.variants.len() - 1,
            });
        }
        let variant_def = &layout.variants[disc];
        let max_ca = max_variant_align(&layout.variants, layouts)?;
        let payload_base = offset + align_up(ds, max_ca);
        let mut poff = payload_base;
        let mut fields = Vec::with_capacity(variant_def.fields.len());
        for fdef in &variant_def.fields {
            poff = align_up(poff, align_of(&fdef.ty, layouts)?);
            fields.push(lift_from_memory(&fdef.ty, layouts, memory, poff)?);
            poff += size_of(&fdef.ty, layouts)?;
        }
        Ok(Val::Variant {
            discriminant: disc,
            fields,
        })
    }
}

// --- tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use deed_mir::{Field, Layout, LayoutId, Ty, Variant};

    use super::*;

    // ---- helpers ----

    fn record_layout(name: &str, fields: Vec<(&str, Ty)>) -> Layout {
        Layout {
            name: name.to_string(),
            variants: vec![Variant {
                name: name.to_string(),
                fields: fields
                    .into_iter()
                    .map(|(n, ty)| Field {
                        name: n.to_string(),
                        ty,
                    })
                    .collect(),
            }],
        }
    }

    fn choice_layout(name: &str, variants: Vec<(&str, Vec<(&str, Ty)>)>) -> Layout {
        Layout {
            name: name.to_string(),
            variants: variants
                .into_iter()
                .map(|(vname, fields)| Variant {
                    name: vname.to_string(),
                    fields: fields
                        .into_iter()
                        .map(|(fname, ty)| Field {
                            name: fname.to_string(),
                            ty,
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// Round-trip a value through flat args + memory.
    fn round_trip_flat(val: Val, ty: Ty, layouts: &[Layout]) -> Val {
        let mut mem = Memory::new();
        let flat = lower(&val, &ty, layouts, &mut mem).expect("lower should succeed");
        lift(&flat, &ty, layouts, &mem).expect("lift should succeed")
    }

    /// Round-trip a value through component memory at a reserved offset.
    ///
    /// Reserves space for the value first so heap allocations (strings, list
    /// elements) start after the value's own bytes and do not overwrite them.
    fn round_trip_memory(val: Val, ty: Ty, layouts: &[Layout]) -> Val {
        let mut mem = Memory::new();
        // Reserve the value's memory block before writing so that heap
        // allocations for strings/lists start past the value's region.
        let size = size_of(&ty, layouts).unwrap_or(0);
        let align = align_of(&ty, layouts).unwrap_or(1).max(1);
        let offset = if size == 0 {
            0
        } else {
            mem.alloc(size, align).expect("alloc should succeed")
        };
        lower_to_memory(&val, &ty, layouts, &mut mem, offset)
            .expect("lower_to_memory should succeed");
        lift_from_memory(&ty, layouts, &mem, offset).expect("lift_from_memory should succeed")
    }

    // ---- Unit ----

    #[test]
    fn unit_has_no_flat_args() {
        assert_eq!(flatten(&Ty::Unit, &[]), Ok(vec![]));
        assert_eq!(size_of(&Ty::Unit, &[]), Ok(0));
    }

    #[test]
    fn unit_round_trips() {
        assert_eq!(round_trip_flat(Val::Unit, Ty::Unit, &[]), Val::Unit);
    }

    // ---- Bool ----

    #[test]
    fn bool_is_one_i32() {
        assert_eq!(flatten(&Ty::Bool, &[]), Ok(vec![FlatType::I32]));
        assert_eq!(size_of(&Ty::Bool, &[]), Ok(1));
        assert_eq!(align_of(&Ty::Bool, &[]), Ok(1));
    }

    #[test]
    fn bool_true_and_false_round_trip() {
        assert_eq!(
            round_trip_flat(Val::Bool(true), Ty::Bool, &[]),
            Val::Bool(true)
        );
        assert_eq!(
            round_trip_flat(Val::Bool(false), Ty::Bool, &[]),
            Val::Bool(false)
        );
    }

    // ---- Int ----

    #[test]
    fn int_is_one_i64() {
        assert_eq!(flatten(&Ty::Int, &[]), Ok(vec![FlatType::I64]));
        assert_eq!(size_of(&Ty::Int, &[]), Ok(8));
        assert_eq!(align_of(&Ty::Int, &[]), Ok(8));
    }

    #[test]
    fn int_values_round_trip() {
        for n in [0i64, 1, -1, i64::MAX, i64::MIN, 42] {
            assert_eq!(
                round_trip_flat(Val::Int(n), Ty::Int, &[]),
                Val::Int(n),
                "failed for {n}"
            );
        }
    }

    // ---- String ----

    #[test]
    fn string_is_ptr_and_byte_len() {
        assert_eq!(
            flatten(&Ty::Str, &[]),
            Ok(vec![FlatType::I32, FlatType::I32])
        );
        assert_eq!(size_of(&Ty::Str, &[]), Ok(8));
        assert_eq!(align_of(&Ty::Str, &[]), Ok(4));
    }

    #[test]
    fn empty_string_round_trips() {
        assert_eq!(
            round_trip_flat(Val::Str(String::new()), Ty::Str, &[]),
            Val::Str(String::new())
        );
    }

    #[test]
    fn ascii_string_round_trips() {
        let val = Val::Str("hello, world".to_string());
        assert_eq!(round_trip_flat(val.clone(), Ty::Str, &[]), val);
    }

    #[test]
    fn unicode_string_round_trips() {
        let val = Val::Str("\u{1F4A1} light bulb".to_string());
        assert_eq!(round_trip_flat(val.clone(), Ty::Str, &[]), val);
    }

    // ---- List ----

    #[test]
    fn list_of_int_is_ptr_and_count() {
        let ty = Ty::List(Box::new(Ty::Int));
        assert_eq!(flatten(&ty, &[]), Ok(vec![FlatType::I32, FlatType::I32]));
        assert_eq!(size_of(&ty, &[]), Ok(8));
        assert_eq!(align_of(&ty, &[]), Ok(4));
    }

    #[test]
    fn empty_list_round_trips() {
        let ty = Ty::List(Box::new(Ty::Int));
        assert_eq!(
            round_trip_flat(Val::List(vec![]), ty, &[]),
            Val::List(vec![])
        );
    }

    #[test]
    fn list_of_ints_round_trips() {
        let ty = Ty::List(Box::new(Ty::Int));
        let val = Val::List(vec![Val::Int(1), Val::Int(2), Val::Int(3)]);
        assert_eq!(round_trip_flat(val.clone(), ty, &[]), val);
    }

    #[test]
    fn list_of_strings_round_trips() {
        let ty = Ty::List(Box::new(Ty::Str));
        let val = Val::List(vec![
            Val::Str("one".to_string()),
            Val::Str("two".to_string()),
            Val::Str("three".to_string()),
        ]);
        assert_eq!(round_trip_flat(val.clone(), ty, &[]), val);
    }

    #[test]
    fn list_of_bools_round_trips() {
        let ty = Ty::List(Box::new(Ty::Bool));
        let val = Val::List(vec![Val::Bool(true), Val::Bool(false), Val::Bool(true)]);
        assert_eq!(round_trip_flat(val.clone(), ty, &[]), val);
    }

    // ---- Record ----

    #[test]
    fn record_flattens_fields_in_order() {
        // Point { x: Int, y: Int } -> [i64, i64]
        let layout = record_layout("Point", vec![("x", Ty::Int), ("y", Ty::Int)]);
        let ty = Ty::Aggregate(LayoutId(0));
        assert_eq!(
            flatten(&ty, &[layout]),
            Ok(vec![FlatType::I64, FlatType::I64])
        );
    }

    #[test]
    fn record_with_int_fields_round_trips() {
        let layout = record_layout("Point", vec![("x", Ty::Int), ("y", Ty::Int)]);
        let ty = Ty::Aggregate(LayoutId(0));
        let val = Val::Record(vec![Val::Int(10), Val::Int(20)]);
        assert_eq!(round_trip_flat(val.clone(), ty, &[layout]), val);
    }

    #[test]
    fn record_with_mixed_types_round_trips() {
        // Tagged { name: String, active: Bool }
        let layout = record_layout("Tagged", vec![("name", Ty::Str), ("active", Ty::Bool)]);
        let ty = Ty::Aggregate(LayoutId(0));
        let val = Val::Record(vec![Val::Str("deed".to_string()), Val::Bool(true)]);
        assert_eq!(round_trip_flat(val.clone(), ty, &[layout]), val);
    }

    #[test]
    fn record_in_memory_has_canonical_size_and_alignment() {
        // { x: Int, y: Int }: size = 16, align = 8
        let layout = record_layout("Point", vec![("x", Ty::Int), ("y", Ty::Int)]);
        let id = LayoutId(0);
        let layouts = vec![layout];
        assert_eq!(size_of(&Ty::Aggregate(id), &layouts), Ok(16));
        assert_eq!(align_of(&Ty::Aggregate(id), &layouts), Ok(8));
    }

    #[test]
    fn record_round_trips_through_memory() {
        let layout = record_layout("Point", vec![("x", Ty::Int), ("y", Ty::Int)]);
        let ty = Ty::Aggregate(LayoutId(0));
        let val = Val::Record(vec![Val::Int(3), Val::Int(7)]);
        assert_eq!(round_trip_memory(val.clone(), ty, &[layout]), val);
    }

    // ---- Choice ----

    #[test]
    fn choice_flattens_to_discriminant_plus_join() {
        // Color { Red, Blue } -> [i32] (discriminant only, no payloads)
        let layout = choice_layout("Color", vec![("Red", vec![]), ("Blue", vec![])]);
        let ty = Ty::Aggregate(LayoutId(0));
        assert_eq!(flatten(&ty, &[layout]), Ok(vec![FlatType::I32]));
    }

    #[test]
    fn choice_with_payload_flattens_correctly() {
        // Shape { Circle { radius: Int }, Square { side: Int } }
        // -> [i32, i64] (discriminant + Int payload)
        let layout = choice_layout(
            "Shape",
            vec![
                ("Circle", vec![("radius", Ty::Int)]),
                ("Square", vec![("side", Ty::Int)]),
            ],
        );
        let ty = Ty::Aggregate(LayoutId(0));
        assert_eq!(
            flatten(&ty, &[layout]),
            Ok(vec![FlatType::I32, FlatType::I64])
        );
    }

    #[test]
    fn choice_without_payload_round_trips() {
        let layout = choice_layout("Color", vec![("Red", vec![]), ("Green", vec![])]);
        let ty = Ty::Aggregate(LayoutId(0));
        let layouts = vec![layout];
        let red = Val::Variant {
            discriminant: 0,
            fields: vec![],
        };
        let green = Val::Variant {
            discriminant: 1,
            fields: vec![],
        };
        assert_eq!(round_trip_flat(red.clone(), ty.clone(), &layouts), red);
        assert_eq!(round_trip_flat(green.clone(), ty, &layouts), green);
    }

    #[test]
    fn choice_with_payload_round_trips() {
        let layout = choice_layout(
            "Shape",
            vec![
                ("Circle", vec![("radius", Ty::Int)]),
                ("Square", vec![("side", Ty::Int)]),
            ],
        );
        let ty = Ty::Aggregate(LayoutId(0));
        let layouts = vec![layout];
        let circle = Val::Variant {
            discriminant: 0,
            fields: vec![Val::Int(5)],
        };
        let square = Val::Variant {
            discriminant: 1,
            fields: vec![Val::Int(3)],
        };
        assert_eq!(
            round_trip_flat(circle.clone(), ty.clone(), &layouts),
            circle
        );
        assert_eq!(round_trip_flat(square.clone(), ty, &layouts), square);
    }

    // ---- Result ----

    /// Build a `Result<T, E>` layout the way MIR's lower does: two variants
    /// named "ok" and "err", each with a single "value" field.
    fn result_layout(ok: Ty, err: Ty) -> Layout {
        choice_layout(
            &format!("Result<{ok:?}, {err:?}>"),
            vec![("ok", vec![("value", ok)]), ("err", vec![("value", err)])],
        )
    }

    #[test]
    fn result_int_string_flattens_with_join() {
        // Result<Int, String>:
        //   ok  payload: [i64]
        //   err payload: [i32, i32]
        //   join: [i64, i32]
        //   total: [i32, i64, i32]
        let layout = result_layout(Ty::Int, Ty::Str);
        let ty = Ty::Aggregate(LayoutId(0));
        assert_eq!(
            flatten(&ty, &[layout]),
            Ok(vec![FlatType::I32, FlatType::I64, FlatType::I32])
        );
    }

    #[test]
    fn result_ok_round_trips() {
        let layout = result_layout(Ty::Int, Ty::Str);
        let ty = Ty::Aggregate(LayoutId(0));
        let ok = Val::Variant {
            discriminant: 0,
            fields: vec![Val::Int(42)],
        };
        assert_eq!(round_trip_flat(ok.clone(), ty, &[layout]), ok);
    }

    #[test]
    fn result_err_round_trips() {
        let layout = result_layout(Ty::Int, Ty::Str);
        let ty = Ty::Aggregate(LayoutId(0));
        let err = Val::Variant {
            discriminant: 1,
            fields: vec![Val::Str("something went wrong".to_string())],
        };
        assert_eq!(round_trip_flat(err.clone(), ty, &[layout]), err);
    }

    #[test]
    fn result_with_unit_ok_round_trips() {
        // Result<Unit, String>: ok() has no payload flat args, err has (ptr, len).
        let layout = result_layout(Ty::Unit, Ty::Str);
        let ty = Ty::Aggregate(LayoutId(0));
        let layouts = vec![layout];
        let ok = Val::Variant {
            discriminant: 0,
            fields: vec![Val::Unit],
        };
        let err = Val::Variant {
            discriminant: 1,
            fields: vec![Val::Str("oops".to_string())],
        };
        assert_eq!(round_trip_flat(ok.clone(), ty.clone(), &layouts), ok);
        assert_eq!(round_trip_flat(err.clone(), ty, &layouts), err);
    }

    #[test]
    fn result_round_trips_through_memory() {
        let layout = result_layout(Ty::Int, Ty::Str);
        let ty = Ty::Aggregate(LayoutId(0));
        let layouts = vec![layout];
        let ok = Val::Variant {
            discriminant: 0,
            fields: vec![Val::Int(99)],
        };
        let err = Val::Variant {
            discriminant: 1,
            fields: vec![Val::Str("error text".to_string())],
        };
        assert_eq!(round_trip_memory(ok.clone(), ty.clone(), &layouts), ok);
        assert_eq!(round_trip_memory(err.clone(), ty, &layouts), err);
    }

    // ---- Unsupported ----

    #[test]
    fn closure_is_refused_at_the_boundary() {
        assert_eq!(flatten(&Ty::Closure, &[]), Err(Unsupported::Closure));
        assert_eq!(size_of(&Ty::Closure, &[]), Err(Unsupported::Closure));
        let mut mem = Memory::new();
        assert_eq!(
            lower(&Val::Unit, &Ty::Closure, &[], &mut mem),
            Err(Error::Unsupported(Unsupported::Closure))
        );
    }

    #[test]
    fn capability_is_refused_at_the_boundary() {
        assert_eq!(flatten(&Ty::Capability, &[]), Err(Unsupported::Capability));
        assert_eq!(size_of(&Ty::Capability, &[]), Err(Unsupported::Capability));
        let mut mem = Memory::new();
        assert_eq!(
            lower(&Val::Unit, &Ty::Capability, &[], &mut mem),
            Err(Error::Unsupported(Unsupported::Capability))
        );
    }

    // ---- Invalid discriminant ----

    #[test]
    fn lifting_an_out_of_range_discriminant_is_an_error() {
        let layout = choice_layout("Color", vec![("Red", vec![]), ("Green", vec![])]);
        let ty = Ty::Aggregate(LayoutId(0));
        let flat = vec![FlatArg::I32(99)]; // discriminant 99 does not exist
        let mem = Memory::new();
        assert!(matches!(
            lift(&flat, &ty, &[layout], &mem),
            Err(Error::InvalidDiscriminant { got: 99, max: 1 })
        ));
    }
}
