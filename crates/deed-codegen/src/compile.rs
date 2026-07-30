//! Compiling [`deed_mir`] to a WebAssembly module.
//!
//! What this handles today: `Unit`, `Bool` and `Int` values, the operators
//! over them, direct calls, `if`, blocks and slots. Strings, lists,
//! aggregates and closures reach [`Ty::is_boxed`] and are refused by name
//! rather than compiled into something that would be wrong; see
//! `design/05-backend.md` for the order the rest arrives in.
//!
//! Refusing rather than approximating is the same rule the checker follows.
//! A backend that quietly compiled a string into a number would produce a
//! program that runs and answers wrongly, which is worse than one that does
//! not build.

use deed_mir::{BinaryOp, Block, Expr, FuncId, Function, Local, Program, Stmt, Ty, UnaryOp};

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

/// How a MIR type is represented in a WebAssembly module.
///
/// `Unit` has no representation at all rather than a zero: a value nobody can
/// read does not need bits, and giving it some would mean every function
/// returning `()` pushes something its caller then drops.
fn val_type(ty: &Ty) -> Option<ValType> {
    match ty {
        Ty::Unit => None,
        Ty::Bool => Some(ValType::I32),
        Ty::Int => Some(ValType::I64),
        // A reference, once there is a heap to point into.
        _ => Some(ValType::I32),
    }
}

fn describe(ty: &Ty) -> &'static str {
    match ty {
        Ty::Str => "a string",
        Ty::List(_) => "a list",
        Ty::Aggregate(_) => "a record or a choice",
        Ty::Capability => "a capability",
        Ty::Closure => "a function value",
        _ => "a value",
    }
}

/// Compiles a whole program, or says what stopped it.
pub fn compile(program: &Program) -> Result<Module, Unsupported> {
    let mut module = Module::new();

    for function in &program.functions {
        let mut params = Vec::new();
        for ty in &function.params {
            reject(function, ty)?;
            if let Some(val) = val_type(ty) {
                params.push(val);
            }
        }
        reject(function, &function.ret)?;
        let results = val_type(&function.ret).into_iter().collect();
        module.intern_type(FuncType { params, results });
    }

    for function in &program.functions {
        let type_index = index_of_signature(&mut module, function);
        let mut body = Builder::new(program, function);
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

    Ok(module)
}

fn index_of_signature(module: &mut Module, function: &Function) -> u32 {
    let params = function.params.iter().filter_map(val_type).collect();
    let results = val_type(&function.ret).into_iter().collect();
    module.intern_type(FuncType { params, results })
}

fn reject(function: &Function, ty: &Ty) -> Result<(), Unsupported> {
    if ty.is_boxed() {
        return Err(Unsupported {
            function: function.name.clone(),
            what: describe(ty).to_string(),
        });
    }
    Ok(())
}

/// Turns one function body into instructions.
struct Builder<'a> {
    program: &'a Program,
    function: &'a Function,
    instructions: Vec<Ins>,
    extra_locals: Vec<ValType>,
    /// Where each MIR slot ended up, since a `Unit` slot takes no space and
    /// everything after it shifts down.
    slots: Vec<Option<u32>>,
}

impl<'a> Builder<'a> {
    fn new(program: &'a Program, function: &'a Function) -> Self {
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
            instructions: Vec::new(),
            extra_locals,
            slots,
        }
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
                // A contract failure ends the program. Reaching a call into
                // the runtime needs a heap for the message, so until strings
                // compile this traps, which is the same outcome with a worse
                // explanation and is written down as such.
                self.instructions.push(Ins::Unreachable);
            }
        }
        Ok(())
    }

    fn expr(&mut self, expr: &Expr) -> Result<(), Unsupported> {
        match expr {
            Expr::Unit => {}
            Expr::Bool(value) => self
                .instructions
                .push(Ins::I32Const(if *value { 1 } else { 0 })),
            Expr::Int(value) => self.instructions.push(Ins::I64Const(*value)),
            Expr::Str(_) => return Err(self.fail("a string")),
            Expr::Local(local) => {
                if let Some(index) = self.slot(*local) {
                    self.instructions.push(Ins::LocalGet(index));
                }
            }
            Expr::Unary { op, operand } => {
                self.expr(operand)?;
                match op {
                    UnaryOp::Not => self.instructions.push(Ins::I32Eqz),
                    UnaryOp::Negate => {
                        // No negate instruction: subtract from zero, with the
                        // operand already on the stack, so swap the order by
                        // building it the other way round.
                        let operand_instructions =
                            self.instructions.split_off(self.instructions.len() - 1);
                        self.instructions.push(Ins::I64Const(0));
                        self.instructions.extend(operand_instructions);
                        self.instructions.push(Ins::I64Sub);
                    }
                }
            }
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
            Expr::Make { .. } | Expr::Field { .. } | Expr::Discriminant { .. } => {
                return Err(self.fail("a record or a choice"));
            }
            Expr::List { .. } => return Err(self.fail("a list")),
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
            Expr::Runtime { .. } => return Err(self.fail("the runtime library")),
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
            BinaryOp::ConcatStr
            | BinaryOp::LtStr
            | BinaryOp::LeStr
            | BinaryOp::GtStr
            | BinaryOp::GeStr => return Err(self.fail("a string")),
            // Equality is structural and works on every type, so which
            // instruction it is depends on what was compared rather than on
            // the operator.
            BinaryOp::Eq | BinaryOp::Ne => {
                let operand = self.ty_of(left)?;
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
    /// Worked out from the IR rather than recorded on every node, because
    /// this is only asked where an instruction depends on it: equality, and
    /// discarding a value.
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
        let mut function = Function::new("greet", vec![], Ty::Str);
        function.body = Block::of(Expr::Str("hi".to_string()));
        let mut program = Program::new();
        program.add_function(function);

        let refused = compile(&program).expect_err("a string is not compiled yet");
        assert_eq!(refused.function, "greet");
        assert!(refused.to_string().contains("a string"), "{refused}");
    }
}
