//! Encoding a WebAssembly module, by hand.
//!
//! No dependency does this, on purpose. Every other crate in this workspace
//! is dependency-free, `deed-lsp` writes its own JSON and its own framing for
//! the same reason, and the WebAssembly binary format is a published
//! specification with a small core. What a compiler backend needs of it is
//! smaller still: numbers, locals, calls, blocks and branches.
//!
//! Only what [`crate::compile`] emits is here. An instruction nobody emits is
//! an instruction nothing tests, so the set below grows when a lowering needs
//! it rather than in anticipation.

use deed_diagnostics::Span;

/// A WebAssembly value type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ValType {
    I32,
    I64,
}

impl ValType {
    fn byte(self) -> u8 {
        match self {
            ValType::I32 => 0x7f,
            ValType::I64 => 0x7e,
        }
    }
}

/// Unsigned LEB128, which is how WebAssembly spells every length and index.
pub fn write_u32(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            return;
        }
    }
}

/// Signed LEB128, which is how it spells a constant.
pub fn write_i64(out: &mut Vec<u8>, mut value: i64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        let sign_bit_set = byte & 0x40 != 0;
        let done = (value == 0 && !sign_bit_set) || (value == -1 && sign_bit_set);
        out.push(if done { byte } else { byte | 0x80 });
        if done {
            return;
        }
    }
}

pub fn write_i32(out: &mut Vec<u8>, value: i32) {
    write_i64(out, value as i64);
}

/// A byte-for-byte name, length first.
fn write_name(out: &mut Vec<u8>, name: &str) {
    write_u32(out, name.len() as u32);
    out.extend_from_slice(name.as_bytes());
}

/// The instructions this backend emits.
///
/// A flat enum rather than a builder, so a lowering can hold a body as data,
/// look at it, and hand it over once. The alternative writes bytes as it goes
/// and cannot answer any question about what it has written.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Ins {
    /// A marker kept in memory, but stripped when encoded.
    ///
    /// The backend uses this to remember which source span gave rise to an
    /// instruction. WebAssembly has no such metadata in the core format, so
    /// the marker is for the compiler and runner here rather than for the
    /// bytes on disk.
    Marked {
        site: u32,
        inner: Box<Ins>,
    },
    Unreachable,
    Nop,
    /// A block that produces one value, or none when `result` is `None`.
    Block {
        result: Option<ValType>,
        body: Vec<Ins>,
    },
    Loop {
        result: Option<ValType>,
        body: Vec<Ins>,
    },
    If {
        result: Option<ValType>,
        then: Vec<Ins>,
        otherwise: Vec<Ins>,
    },
    /// Jump out of the nth enclosing block, counting from the innermost.
    Br(u32),
    BrIf(u32),
    Return,
    Call(u32),
    /// Through the table, by type index. The callee index is on the stack.
    CallIndirect(u32),
    Drop,
    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),
    I32Const(i32),
    I64Const(i64),
    I32Add,
    I32Sub,
    I32Mul,
    I32Eq,
    I32Ne,
    I32LtS,
    I32LeS,
    I32GtS,
    I32GeS,
    I32And,
    I32Or,
    I32Eqz,
    I64Add,
    I64Sub,
    I64Mul,
    I64DivS,
    I64RemS,
    I64Eq,
    I64Ne,
    I64LtS,
    I64LeS,
    I64GtS,
    I64GeS,
    /// Narrow to a boolean-shaped i32, which is what comparisons produce.
    I64Eqz,
    I32WrapI64,
    I64ExtendI32S,
    /// Load and store go through the one memory this backend declares.
    I32Load(u32),
    I64Load(u32),
    I32Store(u32),
    I64Store(u32),
    /// One byte, which is what a string is made of.
    I32Load8U(u32),
    I32Store8(u32),
}

impl Ins {
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Ins::Marked { inner, .. } => inner.write(out),
            Ins::Unreachable => out.push(0x00),
            Ins::Nop => out.push(0x01),
            Ins::Block { result, body } => {
                out.push(0x02);
                write_block_type(out, *result);
                for instruction in body {
                    instruction.write(out);
                }
                out.push(0x0b);
            }
            Ins::Loop { result, body } => {
                out.push(0x03);
                write_block_type(out, *result);
                for instruction in body {
                    instruction.write(out);
                }
                out.push(0x0b);
            }
            Ins::If {
                result,
                then,
                otherwise,
            } => {
                out.push(0x04);
                write_block_type(out, *result);
                for instruction in then {
                    instruction.write(out);
                }
                if !otherwise.is_empty() {
                    out.push(0x05);
                    for instruction in otherwise {
                        instruction.write(out);
                    }
                }
                out.push(0x0b);
            }
            Ins::Br(depth) => {
                out.push(0x0c);
                write_u32(out, *depth);
            }
            Ins::BrIf(depth) => {
                out.push(0x0d);
                write_u32(out, *depth);
            }
            Ins::Return => out.push(0x0f),
            Ins::Call(index) => {
                out.push(0x10);
                write_u32(out, *index);
            }
            Ins::CallIndirect(ty) => {
                out.push(0x11);
                write_u32(out, *ty);
                write_u32(out, 0);
            }
            Ins::Drop => out.push(0x1a),
            Ins::LocalGet(index) => {
                out.push(0x20);
                write_u32(out, *index);
            }
            Ins::LocalSet(index) => {
                out.push(0x21);
                write_u32(out, *index);
            }
            Ins::LocalTee(index) => {
                out.push(0x22);
                write_u32(out, *index);
            }
            Ins::I32Load(offset) => {
                out.push(0x28);
                write_u32(out, 2);
                write_u32(out, *offset);
            }
            Ins::I64Load(offset) => {
                out.push(0x29);
                write_u32(out, 3);
                write_u32(out, *offset);
            }
            Ins::I32Store(offset) => {
                out.push(0x36);
                write_u32(out, 2);
                write_u32(out, *offset);
            }
            Ins::I64Store(offset) => {
                out.push(0x37);
                write_u32(out, 3);
                write_u32(out, *offset);
            }
            Ins::I32Load8U(offset) => {
                out.push(0x2d);
                write_u32(out, 0);
                write_u32(out, *offset);
            }
            Ins::I32Store8(offset) => {
                out.push(0x3a);
                write_u32(out, 0);
                write_u32(out, *offset);
            }
            Ins::I32Const(value) => {
                out.push(0x41);
                write_i32(out, *value);
            }
            Ins::I64Const(value) => {
                out.push(0x42);
                write_i64(out, *value);
            }
            Ins::I32Eqz => out.push(0x45),
            Ins::I32Eq => out.push(0x46),
            Ins::I32Ne => out.push(0x47),
            Ins::I32LtS => out.push(0x48),
            Ins::I32GtS => out.push(0x4a),
            Ins::I32LeS => out.push(0x4c),
            Ins::I32GeS => out.push(0x4e),
            Ins::I64Eqz => out.push(0x50),
            Ins::I64Eq => out.push(0x51),
            Ins::I64Ne => out.push(0x52),
            Ins::I64LtS => out.push(0x53),
            Ins::I64GtS => out.push(0x55),
            Ins::I64LeS => out.push(0x57),
            Ins::I64GeS => out.push(0x59),
            Ins::I32Add => out.push(0x6a),
            Ins::I32Sub => out.push(0x6b),
            Ins::I32Mul => out.push(0x6c),
            Ins::I32And => out.push(0x71),
            Ins::I32Or => out.push(0x72),
            Ins::I64Add => out.push(0x7c),
            Ins::I64Sub => out.push(0x7d),
            Ins::I64Mul => out.push(0x7e),
            Ins::I64DivS => out.push(0x7f),
            Ins::I64RemS => out.push(0x81),
            Ins::I32WrapI64 => out.push(0xa7),
            Ins::I64ExtendI32S => out.push(0xac),
        }
    }
}

fn write_block_type(out: &mut Vec<u8>, result: Option<ValType>) {
    match result {
        None => out.push(0x40),
        Some(ty) => out.push(ty.byte()),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpanRole {
    Trap,
    Call,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InstructionSpan {
    pub offset: u32,
    pub span: Span,
    pub role: SpanRole,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FunctionSpans {
    pub function: u32,
    pub sites: Vec<InstructionSpan>,
}

/// One function's signature, which WebAssembly stores once and shares.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct FuncType {
    pub params: Vec<ValType>,
    pub results: Vec<ValType>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Func {
    pub type_index: u32,
    /// Slots beyond the parameters, which the type already named.
    pub locals: Vec<ValType>,
    pub body: Vec<Ins>,
}

/// Something the module needs from whoever runs it.
///
/// A WebAssembly module cannot open a file, read a clock or write a line. It
/// says what it wants and the host decides. That is not a limitation being
/// worked around here, it is the same shape a `Dir` capability already has,
/// and it is most of why this backend targets WASM.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Import {
    /// Which namespace the host publishes it under.
    pub module: String,
    pub name: String,
    pub type_index: u32,
}

/// A module under construction.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Module {
    pub types: Vec<FuncType>,
    /// What the host has to supply, and what the function index space starts
    /// with: an import is numbered before every function the module defines.
    pub imports: Vec<Import>,
    pub funcs: Vec<Func>,
    /// Name and function index, for everything callable from outside.
    pub exports: Vec<(String, u32)>,
    /// How many 64KiB pages of memory the module wants, when it wants any.
    pub memory_pages: Option<u32>,
    /// Bytes placed in memory before anything runs, by offset.
    pub data: Vec<(u32, Vec<u8>)>,
    /// Functions reachable through `call_indirect`, in table order.
    ///
    /// A closure is a code pointer and an environment, and a code pointer in
    /// WebAssembly is an index into this. Only what a closure could name goes
    /// in, so the table is the set of function bodies a value can carry
    /// rather than everything the module declares.
    pub table: Vec<u32>,
    /// Debug names for functions, by function index.
    ///
    /// These go in the WebAssembly name custom section. A name section costs
    /// nothing at runtime, and it is the reason a trap says "unreachable in
    /// `answer`" rather than "unreachable in function 3": without it, a host
    /// runtime has no way to name the frame.
    pub names: Vec<(u32, String)>,
    /// Source spans for selected instructions, by function and byte offset.
    pub spans: Vec<FunctionSpans>,
}

impl Module {
    pub fn new() -> Self {
        Self::default()
    }

    /// Interns a signature and hands back its index.
    pub fn intern_type(&mut self, ty: FuncType) -> u32 {
        if let Some(found) = self.types.iter().position(|existing| *existing == ty) {
            return found as u32;
        }
        self.types.push(ty);
        (self.types.len() - 1) as u32
    }

    /// Declares something the host has to supply, handing back the index it
    /// is callable at.
    ///
    /// Interned, since two calls to the same operation are one import.
    /// Imports have to be declared before any function is added, because
    /// they are numbered first and adding one afterwards would move every
    /// function already placed.
    pub fn add_import(&mut self, module: &str, name: &str, type_index: u32) -> u32 {
        assert!(
            self.funcs.is_empty(),
            "imports are numbered before defined functions, so they cannot be added after one"
        );
        if let Some(at) = self
            .imports
            .iter()
            .position(|found| found.module == module && found.name == name)
        {
            return at as u32;
        }
        self.imports.push(Import {
            module: module.to_string(),
            name: name.to_string(),
            type_index,
        });
        (self.imports.len() - 1) as u32
    }

    pub fn add_func(&mut self, func: Func) -> u32 {
        self.funcs.push(func);
        (self.imports.len() + self.funcs.len() - 1) as u32
    }

    pub fn export(&mut self, name: impl Into<String>, func: u32) {
        self.exports.push((name.into(), func));
    }

    /// Where this function sits in the table, putting it there if it is new.
    pub fn intern_table(&mut self, func: u32) -> u32 {
        if let Some(at) = self.table.iter().position(|found| *found == func) {
            return at as u32;
        }
        self.table.push(func);
        (self.table.len() - 1) as u32
    }

    /// The module as bytes, in the order the specification lays the sections
    /// out. Section order is not advisory: a runtime reads them in it.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"\0asm");
        out.extend_from_slice(&1u32.to_le_bytes());

        if !self.types.is_empty() {
            let mut section = Vec::new();
            write_u32(&mut section, self.types.len() as u32);
            for ty in &self.types {
                section.push(0x60);
                write_u32(&mut section, ty.params.len() as u32);
                for param in &ty.params {
                    section.push(param.byte());
                }
                write_u32(&mut section, ty.results.len() as u32);
                for result in &ty.results {
                    section.push(result.byte());
                }
            }
            write_section(&mut out, 1, &section);
        }

        // The import section comes before the function section, and what it
        // declares is numbered before what the function section does.
        if !self.imports.is_empty() {
            let mut section = Vec::new();
            write_u32(&mut section, self.imports.len() as u32);
            for import in &self.imports {
                write_name(&mut section, &import.module);
                write_name(&mut section, &import.name);
                section.push(0x00);
                write_u32(&mut section, import.type_index);
            }
            write_section(&mut out, 2, &section);
        }

        if !self.funcs.is_empty() {
            let mut section = Vec::new();
            write_u32(&mut section, self.funcs.len() as u32);
            for func in &self.funcs {
                write_u32(&mut section, func.type_index);
            }
            write_section(&mut out, 3, &section);
        }

        if !self.table.is_empty() {
            let mut section = Vec::new();
            write_u32(&mut section, 1);
            // A table of function references, exactly as big as what went in.
            section.push(0x70);
            section.push(0x01);
            write_u32(&mut section, self.table.len() as u32);
            write_u32(&mut section, self.table.len() as u32);
            write_section(&mut out, 4, &section);
        }

        if let Some(pages) = self.memory_pages {
            let mut section = Vec::new();
            write_u32(&mut section, 1);
            section.push(0x00);
            write_u32(&mut section, pages);
            write_section(&mut out, 5, &section);
        }
        if !self.exports.is_empty() {
            let mut section = Vec::new();
            write_u32(&mut section, self.exports.len() as u32);
            for (name, index) in &self.exports {
                write_name(&mut section, name);
                section.push(0x00);
                write_u32(&mut section, *index);
            }
            write_section(&mut out, 7, &section);
        }

        // The element section fills the table, and comes before the code
        // that calls through it.
        if !self.table.is_empty() {
            let mut section = Vec::new();
            write_u32(&mut section, 1);
            write_u32(&mut section, 0);
            section.push(0x41);
            write_i32(&mut section, 0);
            section.push(0x0b);
            write_u32(&mut section, self.table.len() as u32);
            for func in &self.table {
                write_u32(&mut section, *func);
            }
            write_section(&mut out, 9, &section);
        }

        if !self.funcs.is_empty() {
            let mut section = Vec::new();
            write_u32(&mut section, self.funcs.len() as u32);
            for func in &self.funcs {
                let mut body = Vec::new();
                let runs = compress_locals(&func.locals);
                write_u32(&mut body, runs.len() as u32);
                for (count, ty) in runs {
                    write_u32(&mut body, count);
                    body.push(ty.byte());
                }
                for instruction in &func.body {
                    instruction.write(&mut body);
                }
                body.push(0x0b);
                write_u32(&mut section, body.len() as u32);
                section.extend_from_slice(&body);
            }
            write_section(&mut out, 10, &section);
        }

        if !self.data.is_empty() {
            let mut section = Vec::new();
            write_u32(&mut section, self.data.len() as u32);
            for (offset, bytes) in &self.data {
                write_u32(&mut section, 0);
                section.push(0x41);
                write_i32(&mut section, *offset as i32);
                section.push(0x0b);
                write_u32(&mut section, bytes.len() as u32);
                section.extend_from_slice(bytes);
            }
            write_section(&mut out, 11, &section);
        }

        // The name section is a custom section (id 0) placed after all
        // standard sections. It maps function indices to human-readable
        // names, which a host runtime uses when formatting a trap: the
        // difference between "unreachable in `answer`" and "unreachable in
        // function 3" is the whole of what this section costs.
        if !self.names.is_empty() {
            // Sort by function index so the subsection is in order.
            let mut sorted = self.names.clone();
            sorted.sort_by_key(|(index, _)| *index);

            // Encode the function names subsection (id = 1).
            let mut funcnames = Vec::new();
            write_u32(&mut funcnames, sorted.len() as u32);
            for (index, name) in &sorted {
                write_u32(&mut funcnames, *index);
                write_name(&mut funcnames, name);
            }

            // The custom section body is the section name followed by any
            // number of subsections, each with their own id and size.
            let mut body = Vec::new();
            write_name(&mut body, "name");
            body.push(1); // subsection id: function names
            write_u32(&mut body, funcnames.len() as u32);
            body.extend_from_slice(&funcnames);

            write_section(&mut out, 0, &body);
        }

        out
    }
}

/// Runs of the same type, which is how a function's locals are stored.
fn compress_locals(locals: &[ValType]) -> Vec<(u32, ValType)> {
    let mut runs: Vec<(u32, ValType)> = Vec::new();
    for ty in locals {
        match runs.last_mut() {
            Some((count, seen)) if seen == ty => *count += 1,
            _ => runs.push((1, *ty)),
        }
    }
    runs
}

pub fn instruction_size(instruction: &Ins) -> u32 {
    let mut bytes = Vec::new();
    instruction.write(&mut bytes);
    bytes.len() as u32
}

fn write_section(out: &mut Vec<u8>, id: u8, body: &[u8]) {
    out.push(id);
    write_u32(out, body.len() as u32);
    out.extend_from_slice(body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_module_starts_with_the_magic_number_and_a_version() {
        let bytes = Module::new().encode();
        assert_eq!(&bytes[..4], b"\0asm");
        assert_eq!(&bytes[4..8], &1u32.to_le_bytes());
    }

    #[test]
    fn an_unsigned_number_is_seven_bits_at_a_time() {
        let mut out = Vec::new();
        write_u32(&mut out, 0);
        assert_eq!(out, vec![0x00]);

        out.clear();
        write_u32(&mut out, 127);
        assert_eq!(out, vec![0x7f]);

        out.clear();
        write_u32(&mut out, 128);
        assert_eq!(out, vec![0x80, 0x01]);

        out.clear();
        write_u32(&mut out, 624485);
        assert_eq!(out, vec![0xe5, 0x8e, 0x26]);
    }

    /// The case that catches an unsigned encoder wearing a signed name: a
    /// small negative number has to come back as itself, not as a huge
    /// positive one.
    #[test]
    fn a_signed_number_keeps_its_sign() {
        let mut out = Vec::new();
        write_i64(&mut out, -1);
        assert_eq!(out, vec![0x7f]);

        out.clear();
        write_i64(&mut out, -123456);
        assert_eq!(out, vec![0xc0, 0xbb, 0x78]);

        out.clear();
        write_i64(&mut out, 64);
        assert_eq!(out, vec![0xc0, 0x00]);
    }

    #[test]
    fn a_signature_is_interned_rather_than_repeated() {
        let mut module = Module::new();
        let one = module.intern_type(FuncType {
            params: vec![ValType::I64],
            results: vec![ValType::I64],
        });
        let same = module.intern_type(FuncType {
            params: vec![ValType::I64],
            results: vec![ValType::I64],
        });
        let other = module.intern_type(FuncType {
            params: vec![],
            results: vec![],
        });
        assert_eq!(one, same);
        assert_ne!(one, other);
        assert_eq!(module.types.len(), 2);
    }

    #[test]
    fn locals_of_the_same_type_are_stored_as_one_run() {
        let runs = compress_locals(&[ValType::I64, ValType::I64, ValType::I32, ValType::I64]);
        assert_eq!(
            runs,
            vec![(2, ValType::I64), (1, ValType::I32), (1, ValType::I64)]
        );
        assert!(compress_locals(&[]).is_empty());
    }

    /// A name section is a custom section (id 0) that carries the string
    /// "name" and a function-names subsection (id 1). Without it a runtime
    /// has no human-readable label for a trapped frame.
    #[test]
    fn a_name_section_is_emitted_when_names_are_present() {
        let mut module = Module::new();
        module.names.push((0, "answer".to_string()));
        let bytes = module.encode();

        // Find the custom section in the encoded bytes (after the 8-byte header).
        // A custom section starts with id byte 0, followed by the LEB128 size.
        let mut pos = 8usize;
        let mut found = false;
        while pos < bytes.len() {
            let section_id = bytes[pos];
            pos += 1;
            // Read the LEB128 size.
            let mut size = 0u32;
            let mut shift = 0;
            loop {
                let byte = bytes[pos];
                pos += 1;
                size |= ((byte & 0x7f) as u32) << shift;
                shift += 7;
                if byte & 0x80 == 0 {
                    break;
                }
            }
            if section_id == 0 {
                // The body starts with the section name "name".
                let name_len = bytes[pos] as usize;
                let name_bytes = &bytes[pos + 1..pos + 1 + name_len];
                assert_eq!(name_bytes, b"name", "custom section should be named 'name'");
                found = true;
                break;
            }
            pos += size as usize;
        }
        assert!(found, "encoded module should contain a name custom section");
    }

    /// A module with no names does not emit the custom section at all.
    #[test]
    fn no_name_section_when_no_names_are_given() {
        let bytes = Module::new().encode();
        // After the 8-byte header, all section ids should be non-zero.
        let mut pos = 8usize;
        while pos < bytes.len() {
            let section_id = bytes[pos];
            assert_ne!(
                section_id, 0,
                "should not emit a custom section without names"
            );
            pos += 1;
            let mut size = 0u32;
            let mut shift = 0;
            loop {
                let byte = bytes[pos];
                pos += 1;
                size |= ((byte & 0x7f) as u32) << shift;
                shift += 7;
                if byte & 0x80 == 0 {
                    break;
                }
            }
            pos += size as usize;
        }
    }
}
