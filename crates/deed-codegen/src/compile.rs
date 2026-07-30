//! Compiling [`deed_mir`] to a WebAssembly module.
//!
//! What this handles: `Unit`, `Bool` and `Int`, the operators over them,
//! string literals, lists, records and choices, direct calls, `if`, blocks
//! and slots. Closures, capabilities and effects reach here and are refused
//! by name; see `design/05-backend.md` for the order the rest arrives in.
//!
//! Refusing rather than approximating is the checker's own rule. A backend
//! that quietly compiled something into the wrong shape would produce a
//! program that runs and answers wrongly, which is worse than one that does
//! not build.

use deed_mir::{BinaryOp, Block, Expr, FuncId, Function, Local, Program, Stmt, Ty, UnaryOp};

use crate::layout;
use crate::wasm::{Func, FuncType, Ins, Module, ValType};

/// What a backend can say about a program it will not compile.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Unsupported {
    /// The function it gave up in, by name.
    pub function: String,
    /// What it found, in the words a reader would use.
    pub what: String,
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "`{}` uses {}, which this backend does not compile yet",
            self.function, self.what
        )
    }
}

/// How a MIR type is represented.
///
/// `Unit` has no representation at all rather than a zero: a value nobody
/// can read does not need bits, and giving it some would mean every function
/// returning `()` pushes something its caller then drops.
///
/// Everything that lives in memory is an address, and an address is a
/// number, so all of them come out one width. What tells them apart is the
/// type, which this layer still has.
fn val_type(ty: &Ty) -> Option<ValType> {
    match ty {
        Ty::Unit => None,
        Ty::Bool => Some(ValType::I32),
        _ => Some(ValType::I64),
    }
}

/// Compiles a whole program, or says what stopped it.
pub fn compile(program: &Program) -> Result<Module, Unsupported> {
    let mut module = Module::new();
    module.memory_pages = Some(16);

    // Signatures first, so a body can call anything regardless of where it
    // sits, and so a function's index here is its index in the program.
    for function in &program.functions {
        for ty in &function.params {
            reject(function, ty)?;
        }
        reject(function, &function.ret)?;
        index_of_signature(&mut module, function);
    }

    let mut strings = Strings::new();

    for function in &program.functions {
        let type_index = index_of_signature(&mut module, function);
        let mut body = Builder::new(program, function, &mut strings);
        body.block(&function.body)?;
        body.instructions.push(Ins::Return);

        let compiled = Func {
            type_index,
            locals: body.extra_locals,
            body: body.instructions,
        };
        let func_index = module.add_func(compiled);
        module.export(function.name.clone(), func_index);
    }

    // The bump pointer starts past every literal the data section placed.
    let mut placed = strings.data;
    placed.push((layout::BUMP, (strings.next as i64).to_le_bytes().to_vec()));
    module.data = placed;

    Ok(module)
}

fn index_of_signature(module: &mut Module, function: &Function) -> u32 {
    let params = function.params.iter().filter_map(val_type).collect();
    let results = val_type(&function.ret).into_iter().collect();
    module.intern_type(FuncType { params, results })
}

fn reject(function: &Function, ty: &Ty) -> Result<(), Unsupported> {
    let what = match ty {
        Ty::Capability => "a capability",
        Ty::Closure => "a function value",
        _ => return Ok(()),
    };
    Err(Unsupported {
        function: function.name.clone(),
        what: what.to_string(),
    })
}

/// Every string literal in the program, placed in memory before anything
/// runs.
///
/// A literal never changes, so writing it into the data section costs
/// nothing at runtime and a function that returns one does no work at all.
/// Building it on the heap instead would allocate once per call for a value
/// that is the same every time.
struct Strings {
    next: u32,
    data: Vec<(u32, Vec<u8>)>,
    placed: Vec<(String, u32)>,
}

impl Strings {
    fn new() -> Self {
        Strings {
            next: layout::HEAP_START,
            data: Vec::new(),
            placed: Vec::new(),
        }
    }

    /// Where this string lives, placing it if it is new.
    fn place(&mut self, text: &str) -> u32 {
        if let Some((_, at)) = self.placed.iter().find(|(seen, _)| seen == text) {
            return *at;
        }

        let at = self.next;
        let mut bytes = (text.len() as i64).to_le_bytes().to_vec();
        bytes.extend_from_slice(text.as_bytes());
        while bytes.len() % layout::WORD as usize != 0 {
            bytes.push(0);
        }
        self.next += bytes.len() as u32;
        self.data.push((at, bytes));
        self.placed.push((text.to_string(), at));
        at
    }
}

/// Turns one function body into instructions.
struct Builder<'a> {
    program: &'a Program,
    function: &'a Function,
    strings: &'a mut Strings,
    instructions: Vec<Ins>,
    extra_locals: Vec<ValType>,
    /// Where each MIR slot ended up, since a `Unit` slot takes no space and
    /// everything after it shifts down.
    slots: Vec<Option<u32>>,
    /// How many parameters have a representation.
    params: usize,
}

impl<'a> Builder<'a> {
    fn new(program: &'a Program, function: &'a Function, strings: &'a mut Strings) -> Self {
        let mut slots = Vec::new();
        let mut next = 0;
        let mut extra_locals = Vec::new();

        for (index, ty) in function.locals.iter().enumerate() {
            match val_type(ty) {
                None => slots.push(None),
                Some(val) => {
                    slots.push(Some(next));
                    next += 1;
                    if index >= function.params.len() {
                        extra_locals.push(val);
                    }
                }
            }
        }

        Builder {
            program,
            function,
            strings,
            instructions: Vec::new(),
            extra_locals,
            slots,
            params: function.params.iter().filter_map(val_type).count(),
        }
    }

    /// A slot nothing else uses, for holding something mid-expression.
    ///
    /// Handed out fresh each time rather than reused, because two of these
    /// can be live at once: building a record whose field is another record
    /// is exactly that, and sharing one slot would have the inner build
    /// overwrite the outer one's address.
    ///
    /// The index counts the parameters that have a representation and then
    /// everything declared after them, which is the order WebAssembly
    /// numbers a function's slots in.
    fn temporary(&mut self, ty: ValType) -> u32 {
        self.extra_locals.push(ty);
        (self.params + self.extra_locals.len() - 1) as u32
    }

    fn slot(&self, local: Local) -> Option<u32> {
        self.slots[local.0]
    }

    fn fail(&self, what: &str) -> Unsupported {
        Unsupported {
            function: self.function.name.clone(),
            what: what.to_string(),
        }
    }

    fn block(&mut self, block: &Block) -> Result<(), Unsupported> {
        for stmt in &block.stmts {
            self.stmt(stmt)?;
        }
        self.expr(&block.value)
    }

    /// A block's instructions on their own, for an `if` arm, which needs them
    /// nested rather than appended.
    fn nested(&mut self, block: &Block) -> Result<Vec<Ins>, Unsupported> {
        let saved = std::mem::take(&mut self.instructions);
        self.block(block)?;
        Ok(std::mem::replace(&mut self.instructions, saved))
    }

    fn stmt(&mut self, stmt: &Stmt) -> Result<(), Unsupported> {
        match stmt {
            Stmt::Assign { local, value } => {
                self.expr(value)?;
                // A slot with no representation stores nothing, because
                // nothing was pushed for it.
                if let Some(index) = self.slot(*local) {
                    self.instructions.push(Ins::LocalSet(index));
                }
            }
            Stmt::Discard(value) => {
                let produced = self.ty_of(value)?;
                self.expr(value)?;
                if val_type(&produced).is_some() {
                    self.instructions.push(Ins::Drop);
                }
            }
            Stmt::Fail { .. } => {
                // A contract failure ends the program. Carrying the message
                // would need somewhere to print it, and a module compiled by
                // `deed build` has no host to print to yet.
                self.instructions.push(Ins::Unreachable);
            }
        }
        Ok(())
    }

    /// Reserves `size` bytes, leaving the address in `into` and nothing on
    /// the stack.
    fn allocate(&mut self, size: u32, into: u32) {
        self.instructions.push(Ins::I32Const(layout::BUMP as i32));
        self.instructions.push(Ins::I64Load(0));
        self.instructions.push(Ins::LocalSet(into));

        self.instructions.push(Ins::I32Const(layout::BUMP as i32));
        self.instructions.push(Ins::LocalGet(into));
        self.instructions.push(Ins::I64Const(size as i64));
        self.instructions.push(Ins::I64Add);
        self.instructions.push(Ins::I64Store(0));
    }

    /// Writes one word at `address + offset`, where `address` is a slot.
    fn store_word(&mut self, address: u32, offset: u32, value: &Expr) -> Result<(), Unsupported> {
        let ty = self.ty_of(value)?;
        let Some(width) = val_type(&ty) else {
            // Nothing to store, and nothing to evaluate it onto the stack
            // for either, so this leaves a hole a reader of the memory would
            // never look at.
            return Ok(());
        };

        self.instructions.push(Ins::LocalGet(address));
        self.instructions.push(Ins::I32WrapI64);
        self.expr(value)?;
        if width == ValType::I32 {
            self.instructions.push(Ins::I64ExtendI32S);
        }
        self.instructions.push(Ins::I64Store(offset));
        Ok(())
    }

    /// Builds something in memory: allocate, fill, leave the address behind.
    fn build(
        &mut self,
        size: u32,
        header: Option<i64>,
        fields: &[(u32, &Expr)],
    ) -> Result<(), Unsupported> {
        let address = self.temporary(ValType::I64);
        self.allocate(size, address);

        if let Some(value) = header {
            self.instructions.push(Ins::LocalGet(address));
            self.instructions.push(Ins::I32WrapI64);
            self.instructions.push(Ins::I64Const(value));
            self.instructions.push(Ins::I64Store(0));
        }

        for (offset, value) in fields {
            self.store_word(address, *offset, value)?;
        }

        self.instructions.push(Ins::LocalGet(address));
        Ok(())
    }

    fn expr(&mut self, expr: &Expr) -> Result<(), Unsupported> {
        match expr {
            Expr::Unit => {}
            Expr::Bool(value) => self
                .instructions
                .push(Ins::I32Const(if *value { 1 } else { 0 })),
            Expr::Int(value) => self.instructions.push(Ins::I64Const(*value)),
            Expr::Str(text) => {
                let at = self.strings.place(text);
                self.instructions.push(Ins::I64Const(at as i64));
            }
            Expr::Local(local) => {
                if let Some(index) = self.slot(*local) {
                    self.instructions.push(Ins::LocalGet(index));
                }
            }
            Expr::Unary { op, operand } => match op {
                UnaryOp::Not => {
                    self.expr(operand)?;
                    self.instructions.push(Ins::I32Eqz);
                }
                UnaryOp::Negate => {
                    // No negate instruction: subtract from zero, so the zero
                    // is pushed before the operand rather than after it.
                    self.instructions.push(Ins::I64Const(0));
                    self.expr(operand)?;
                    self.instructions.push(Ins::I64Sub);
                }
            },
            Expr::Binary { op, left, right } => {
                self.expr(left)?;
                self.expr(right)?;
                self.binary(*op, left)?;
            }
            Expr::Call { func, args } => {
                for arg in args {
                    self.expr(arg)?;
                }
                self.instructions.push(Ins::Call(func.0 as u32));
            }
            Expr::CallIndirect { .. } => return Err(self.fail("a function value")),
            Expr::Make {
                layout: id,
                variant,
                fields,
            } => {
                let tagged = self.program.layout(*id).is_tagged();
                let places: Vec<(u32, &Expr)> = fields
                    .iter()
                    .enumerate()
                    .map(|(index, field)| (layout::field_offset(tagged, index), field))
                    .collect();
                self.build(
                    layout::aggregate_size(tagged, fields.len()),
                    tagged.then_some(*variant as i64),
                    &places,
                )?;
            }
            Expr::Field {
                value,
                layout: id,
                field,
                ..
            } => {
                let tagged = self.program.layout(*id).is_tagged();
                let ty = self.ty_of(expr)?;
                self.expr(value)?;
                self.instructions.push(Ins::I32WrapI64);
                self.instructions
                    .push(Ins::I64Load(layout::field_offset(tagged, *field)));
                if matches!(val_type(&ty), Some(ValType::I32)) {
                    self.instructions.push(Ins::I32WrapI64);
                }
            }
            Expr::Discriminant { value, layout: id } => {
                if self.program.layout(*id).is_tagged() {
                    self.expr(value)?;
                    self.instructions.push(Ins::I32WrapI64);
                    self.instructions.push(Ins::I64Load(0));
                } else {
                    // One variant, so there is nothing stored to read and the
                    // answer is the only one it could be.
                    self.expr(value)?;
                    self.instructions.push(Ins::Drop);
                    self.instructions.push(Ins::I64Const(0));
                }
            }
            Expr::List { items, .. } => {
                let places: Vec<(u32, &Expr)> = items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| (layout::element_offset(index), item))
                    .collect();
                self.build(
                    layout::list_size(items.len()),
                    Some(items.len() as i64),
                    &places,
                )?;
            }
            Expr::If {
                condition,
                then,
                otherwise,
                ty,
            } => {
                self.expr(condition)?;
                let then_body = self.nested(then)?;
                let else_body = self.nested(otherwise)?;
                self.instructions.push(Ins::If {
                    result: val_type(ty),
                    then: then_body,
                    otherwise: else_body,
                });
            }
            Expr::Block(block) => self.block(block)?,
            Expr::Runtime { name, args, .. } => {
                use deed_mir::runtime;
                match *name {
                    // Both of these are the first word of what they point at,
                    // which is the whole reason the two layouts agree about
                    // where a length goes.
                    runtime::STR_LEN | runtime::LIST_LEN => {
                        self.expr(&args[0])?;
                        self.instructions.push(Ins::I32WrapI64);
                        self.instructions.push(Ins::I64Load(0));
                    }
                    other => return Err(self.fail(&format!("the runtime function `{other}`"))),
                }
            }
        }
        Ok(())
    }

    fn binary(&mut self, op: BinaryOp, left: &Expr) -> Result<(), Unsupported> {
        let instruction = match op {
            BinaryOp::AddInt => Ins::I64Add,
            BinaryOp::SubInt => Ins::I64Sub,
            BinaryOp::MulInt => Ins::I64Mul,
            BinaryOp::DivInt => Ins::I64DivS,
            BinaryOp::RemInt => Ins::I64RemS,
            BinaryOp::LtInt => Ins::I64LtS,
            BinaryOp::LeInt => Ins::I64LeS,
            BinaryOp::GtInt => Ins::I64GtS,
            BinaryOp::GeInt => Ins::I64GeS,
            BinaryOp::And => Ins::I32And,
            BinaryOp::Or => Ins::I32Or,
            BinaryOp::ConcatStr => return Err(self.fail("joining two strings")),
            BinaryOp::LtStr | BinaryOp::LeStr | BinaryOp::GtStr | BinaryOp::GeStr => {
                return Err(self.fail("ordering two strings"));
            }
            // Equality is structural and works on every type, so which
            // instruction it is depends on what was compared rather than on
            // the operator. Two addresses being equal is not two values being
            // equal, so anything living in memory is refused rather than
            // compared the wrong way.
            BinaryOp::Eq | BinaryOp::Ne => {
                let operand = self.ty_of(left)?;
                if operand.is_boxed() {
                    return Err(self.fail("comparing two values that live in memory"));
                }
                match (val_type(&operand), op) {
                    (Some(ValType::I64), BinaryOp::Eq) => Ins::I64Eq,
                    (Some(ValType::I64), BinaryOp::Ne) => Ins::I64Ne,
                    (Some(ValType::I32), BinaryOp::Eq) => Ins::I32Eq,
                    (Some(ValType::I32), BinaryOp::Ne) => Ins::I32Ne,
                    // Two values of a type with no representation are equal,
                    // and there is nothing on the stack to compare.
                    (None, BinaryOp::Eq) => Ins::I32Const(1),
                    (None, _) => Ins::I32Const(0),
                    _ => unreachable!("only Eq and Ne reach here"),
                }
            }
        };
        self.instructions.push(instruction);
        Ok(())
    }

    /// What an expression produces.
    ///
    /// Worked out from the IR rather than recorded on every node, because it
    /// is only asked where an instruction depends on it.
    fn ty_of(&self, expr: &Expr) -> Result<Ty, Unsupported> {
        Ok(match expr {
            Expr::Unit => Ty::Unit,
            Expr::Bool(_) => Ty::Bool,
            Expr::Int(_) => Ty::Int,
            Expr::Str(_) => Ty::Str,
            Expr::Local(local) => self.function.local_ty(*local).clone(),
            Expr::Unary { op, .. } => match op {
                UnaryOp::Not => Ty::Bool,
                UnaryOp::Negate => Ty::Int,
            },
            Expr::Binary { op, .. } => op.result_ty(),
            Expr::Call { func, .. } => self.callee(*func).ret.clone(),
            Expr::CallIndirect { ret, .. } => (**ret).clone(),
            Expr::Make { layout, .. } => Ty::Aggregate(*layout),
            Expr::Field {
                layout,
                variant,
                field,
                ..
            } => self.program.layout(*layout).variants[*variant].fields[*field]
                .ty
                .clone(),
            Expr::Discriminant { .. } => Ty::Int,
            Expr::List { element, .. } => Ty::List(element.clone()),
            Expr::If { ty, .. } => (**ty).clone(),
            Expr::Block(block) => self.ty_of(&block.value)?,
            Expr::Runtime { ret, .. } => (**ret).clone(),
        })
    }

    fn callee(&self, func: FuncId) -> &Function {
        self.program.function(func)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deed_mir::{Field, Layout, Variant};

    fn adding() -> Program {
        let mut program = Program::new();
        let mut function = Function::new("add", vec![Ty::Int, Ty::Int], Ty::Int);
        function.body = Block::of(Expr::Binary {
            op: BinaryOp::AddInt,
            left: Box::new(Expr::Local(Local(0))),
            right: Box::new(Expr::Local(Local(1))),
        });
        program.add_function(function);
        program
    }

    #[test]
    fn a_function_of_two_numbers_compiles_and_is_exported() {
        let module = compile(&adding()).expect("this compiles");
        assert_eq!(module.funcs.len(), 1);
        assert_eq!(module.exports, vec![("add".to_string(), 0)]);
        assert_eq!(
            module.types[0],
            FuncType {
                params: vec![ValType::I64, ValType::I64],
                results: vec![ValType::I64],
            }
        );
        assert!(module.encode().starts_with(b"\0asm"));
    }

    /// The half of the type mapping that is easy to get wrong by treating
    /// `Unit` as a number: a slot with no representation takes no index, so
    /// everything after it shifts down.
    #[test]
    fn a_unit_slot_takes_no_index_and_the_next_one_moves_up() {
        let mut function = Function::new("f", vec![Ty::Unit, Ty::Int], Ty::Int);
        function.body = Block::of(Expr::Local(Local(1)));
        let mut program = Program::new();
        program.add_function(function);

        let module = compile(&program).expect("this compiles");
        assert_eq!(
            module.types[0],
            FuncType {
                params: vec![ValType::I64],
                results: vec![ValType::I64],
            }
        );
        assert_eq!(module.funcs[0].body[0], Ins::LocalGet(0));
    }

    #[test]
    fn what_is_not_compiled_yet_is_named_rather_than_approximated() {
        let mut function = Function::new("hand", vec![], Ty::Closure);
        function.body = Block::of(Expr::Unit);
        let mut program = Program::new();
        program.add_function(function);

        let refused = compile(&program).expect_err("a function value is not compiled yet");
        assert_eq!(refused.function, "hand");
        assert!(
            refused.to_string().contains("a function value"),
            "{refused}"
        );
    }

    /// A literal is placed once however many times it is written, and the
    /// bump pointer starts past everything placed.
    #[test]
    fn a_string_literal_is_placed_once_and_the_heap_starts_after_it() {
        let mut function = Function::new("f", vec![], Ty::Str);
        function.body = Block {
            stmts: vec![Stmt::Discard(Expr::Str("hi".to_string()))],
            value: Expr::Str("hi".to_string()),
        };
        let mut program = Program::new();
        program.add_function(function);

        let module = compile(&program).expect("this compiles");
        let placed = module
            .data
            .iter()
            .filter(|(at, _)| *at != layout::BUMP)
            .count();
        assert_eq!(placed, 1, "one literal, placed once");

        let (_, bump) = module
            .data
            .iter()
            .find(|(at, _)| *at == layout::BUMP)
            .expect("the bump pointer is initialised");
        let start = i64::from_le_bytes(bump[..8].try_into().unwrap());
        assert_eq!(start as u32, layout::HEAP_START + layout::string_size(2));
    }

    /// Two things at once: a record has no tag, and its second field sits one
    /// word in rather than two.
    #[test]
    fn a_record_reads_its_second_field_one_word_in() {
        let mut program = Program::new();
        let record = program.add_layout(Layout {
            name: "Pair".to_string(),
            variants: vec![Variant {
                name: "Pair".to_string(),
                fields: vec![
                    Field {
                        name: "left".to_string(),
                        ty: Ty::Int,
                    },
                    Field {
                        name: "right".to_string(),
                        ty: Ty::Int,
                    },
                ],
            }],
        });

        let mut function = Function::new("f", vec![], Ty::Int);
        function.body = Block::of(Expr::Field {
            value: Box::new(Expr::Make {
                layout: record,
                variant: 0,
                fields: vec![Expr::Int(1), Expr::Int(2)],
            }),
            layout: record,
            variant: 0,
            field: 1,
        });
        program.add_function(function);

        let module = compile(&program).expect("this compiles");
        assert!(
            module.funcs[0].body.contains(&Ins::I64Load(layout::WORD)),
            "the second field of an untagged aggregate is one word in"
        );
    }

    /// Building a record inside a record needs two addresses live at once,
    /// which is what a shared scratch slot would get wrong.
    #[test]
    fn one_aggregate_inside_another_keeps_two_addresses_apart() {
        let mut program = Program::new();
        let inner = program.add_layout(Layout {
            name: "Inner".to_string(),
            variants: vec![Variant {
                name: "Inner".to_string(),
                fields: vec![Field {
                    name: "n".to_string(),
                    ty: Ty::Int,
                }],
            }],
        });
        let outer = program.add_layout(Layout {
            name: "Outer".to_string(),
            variants: vec![Variant {
                name: "Outer".to_string(),
                fields: vec![Field {
                    name: "held".to_string(),
                    ty: Ty::Aggregate(inner),
                }],
            }],
        });

        let mut function = Function::new("f", vec![], Ty::Int);
        function.body = Block::of(Expr::Field {
            value: Box::new(Expr::Field {
                value: Box::new(Expr::Make {
                    layout: outer,
                    variant: 0,
                    fields: vec![Expr::Make {
                        layout: inner,
                        variant: 0,
                        fields: vec![Expr::Int(7)],
                    }],
                }),
                layout: outer,
                variant: 0,
                field: 0,
            }),
            layout: inner,
            variant: 0,
            field: 0,
        });
        program.add_function(function);

        let module = compile(&program).expect("this compiles");
        let sets: Vec<&Ins> = module.funcs[0]
            .body
            .iter()
            .filter(|instruction| matches!(instruction, Ins::LocalSet(_)))
            .collect();
        assert_eq!(
            sets.len(),
            2,
            "two allocations, two slots, and they should not be the same one"
        );
        assert_ne!(
            sets[0], sets[1],
            "the inner build must not reuse the outer slot"
        );
    }
}
