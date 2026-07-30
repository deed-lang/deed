//! Running a WebAssembly module, so that what this backend emits can be
//! checked against what the interpreter does with the same program.
//!
//! A compiler whose output nothing runs is a compiler nothing tests. Every
//! runtime that could run it is a dependency, and this workspace has none, so
//! this is a small one over exactly the instructions [`crate::compile`]
//! emits. It is not a WebAssembly implementation: it does not validate, it
//! does not implement the instructions nothing here produces, and it is not
//! the thing a released `deed build` would ship. It is a test oracle that
//! happens to run.

use crate::wasm::{Ins, Module, ValType};

/// A value on the stack.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Value {
    I32(i32),
    I64(i64),
}

impl Value {
    pub fn as_i64(self) -> i64 {
        match self {
            Value::I32(value) => value as i64,
            Value::I64(value) => value,
        }
    }

    pub fn as_bool(self) -> bool {
        self.as_i64() != 0
    }

    fn zero(ty: ValType) -> Value {
        match ty {
            ValType::I32 => Value::I32(0),
            ValType::I64 => Value::I64(0),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Trap {
    /// `unreachable` with nothing left behind saying why.
    Unreachable,
    /// A contract, assertion or match that did not hold, with the code and
    /// sentence the compiled program left in memory before it stopped.
    ///
    /// The same two things the interpreter files a diagnostic with, so the
    /// two engines can be asked whether they stopped for the same reason
    /// rather than only whether they both stopped.
    Failed {
        code: String,
        message: String,
    },
    DivideByZero,
    /// Reached past the end of memory.
    OutOfBounds,
    /// Ran longer than the budget below allows.
    TooLong,
    /// A module that would not validate: an instruction was handed a value
    /// of the wrong width.
    ///
    /// Not a trap a real engine raises, because a real engine would have
    /// refused the module before running a byte of it. This runner does not
    /// validate, so without the check it would happily run something no
    /// engine would load, and the backend would have a bug nothing here
    /// could see. That is not hypothetical: an `i32` stored through an
    /// `i64.store` is what a missing sign extension looks like.
    Mistyped(String),
    /// Something this small runner does not implement, named rather than
    /// silently skipped.
    Unimplemented(String),
}

impl std::fmt::Display for Trap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Trap::Unreachable => write!(f, "the program stopped"),
            Trap::Failed { code, message } => write!(f, "{code}: {message}"),
            Trap::DivideByZero => write!(f, "divided by zero"),
            Trap::OutOfBounds => write!(f, "reached past the end of memory"),
            Trap::TooLong => write!(f, "ran too long"),
            Trap::Mistyped(what) => write!(f, "{what}, which no engine would load"),
            Trap::Unimplemented(what) => write!(f, "`{what}` is not implemented here"),
        }
    }
}

/// How many instructions one call may run before this gives up.
///
/// A budget rather than a timer: a test that fails when the machine is busy
/// is a test people learn to rerun, which is the reasoning `design/01-principles.md`
/// already applies to the scaling test.
const BUDGET: u64 = 5_000_000;

/// Calls an exported function.
pub fn call(module: &Module, name: &str, args: &[Value]) -> Result<Option<Value>, Trap> {
    let index = module
        .exports
        .iter()
        .find(|(exported, _)| exported == name)
        .map(|(_, index)| *index)
        .ok_or_else(|| Trap::Unimplemented(format!("no export named {name}")))?;

    let mut run = Run {
        module,
        fuel: BUDGET,
        memory: memory_of(module),
    };
    match run.call(index, args) {
        // A trap that left something behind says what it was. Read here
        // rather than where the trap is raised, because the two words are
        // in memory and memory is what this owns.
        Err(Trap::Unreachable) => Err(run.why().unwrap_or(Trap::Unreachable)),
        other => other,
    }
}

/// Linear memory, with whatever the data section placed already in it.
fn memory_of(module: &Module) -> Vec<u8> {
    let pages = module.memory_pages.unwrap_or(0) as usize;
    let mut memory = vec![0u8; pages * 64 * 1024];
    for (at, bytes) in &module.data {
        let at = *at as usize;
        if at + bytes.len() <= memory.len() {
            memory[at..at + bytes.len()].copy_from_slice(bytes);
        }
    }
    memory
}

struct Run<'a> {
    module: &'a Module,
    fuel: u64,
    memory: Vec<u8>,
}

/// What a block ended with: fell off the end, or jumped out of one.
enum Flow {
    Normal,
    /// Branch out of this many enclosing blocks, still counting down.
    Break(u32),
    Return,
}

impl Run<'_> {
    /// What the program left in memory about why it stopped, if anything.
    fn why(&self) -> Option<Trap> {
        Some(Trap::Failed {
            code: self.string_at(crate::layout::FAILURE_CODE)?,
            message: self.string_at(crate::layout::FAILURE_MESSAGE)?,
        })
    }

    /// The string whose address is in this word, read the way every string
    /// in a compiled module is laid out: a length, then the bytes.
    fn string_at(&self, word: u32) -> Option<String> {
        let address = u64::from_le_bytes(
            self.memory
                .get(word as usize..word as usize + 8)?
                .try_into()
                .ok()?,
        ) as usize;
        if address == 0 {
            return None;
        }
        let length =
            u64::from_le_bytes(self.memory.get(address..address + 8)?.try_into().ok()?) as usize;
        let bytes = self.memory.get(address + 8..address + 8 + length)?;
        String::from_utf8(bytes.to_vec()).ok()
    }

    fn call(&mut self, index: u32, args: &[Value]) -> Result<Option<Value>, Trap> {
        let func = &self.module.funcs[index as usize];
        let signature = &self.module.types[func.type_index as usize];

        let mut locals: Vec<Value> = args.to_vec();
        for ty in &func.locals {
            locals.push(Value::zero(*ty));
        }

        let mut stack = Vec::new();
        self.run(&func.body, &mut locals, &mut stack)?;

        Ok(match signature.results.first() {
            None => None,
            Some(_) => stack.pop(),
        })
    }

    fn run(
        &mut self,
        body: &[Ins],
        locals: &mut Vec<Value>,
        stack: &mut Vec<Value>,
    ) -> Result<Flow, Trap> {
        for instruction in body {
            self.fuel = self.fuel.checked_sub(1).ok_or(Trap::TooLong)?;

            match instruction {
                Ins::Unreachable => return Err(Trap::Unreachable),
                Ins::Nop | Ins::Drop => {
                    if matches!(instruction, Ins::Drop) {
                        stack.pop();
                    }
                }
                Ins::Return => return Ok(Flow::Return),
                Ins::Br(depth) => return Ok(Flow::Break(*depth)),
                Ins::BrIf(depth) => {
                    if pop(stack)?.as_bool() {
                        return Ok(Flow::Break(*depth));
                    }
                }
                Ins::Block { body, .. } | Ins::Loop { body, .. } => {
                    let repeats = matches!(instruction, Ins::Loop { .. });
                    loop {
                        match self.run(body, locals, stack)? {
                            Flow::Normal => break,
                            Flow::Return => return Ok(Flow::Return),
                            Flow::Break(0) if repeats => continue,
                            Flow::Break(0) => break,
                            Flow::Break(depth) => return Ok(Flow::Break(depth - 1)),
                        }
                    }
                }
                Ins::If {
                    then, otherwise, ..
                } => {
                    let taken = if pop(stack)?.as_bool() {
                        then
                    } else {
                        otherwise
                    };
                    match self.run(taken, locals, stack)? {
                        Flow::Normal => {}
                        Flow::Return => return Ok(Flow::Return),
                        Flow::Break(0) => {}
                        Flow::Break(depth) => return Ok(Flow::Break(depth - 1)),
                    }
                }
                Ins::Call(index) => {
                    let callee = &self.module.funcs[*index as usize];
                    let signature = &self.module.types[callee.type_index as usize];
                    let count = signature.params.len();
                    let at = stack.len() - count;
                    let args: Vec<Value> = stack.split_off(at);
                    if let Some(result) = self.call(*index, &args)? {
                        stack.push(result);
                    }
                }
                Ins::CallIndirect(_) => {
                    let slot = pop(stack)?.as_i64() as usize;
                    let index = *self.module.table.get(slot).ok_or(Trap::OutOfBounds)?;
                    let callee = &self.module.funcs[index as usize];
                    let signature = &self.module.types[callee.type_index as usize];
                    let count = signature.params.len();
                    let at = stack.len() - count;
                    let args: Vec<Value> = stack.split_off(at);
                    if let Some(result) = self.call(index, &args)? {
                        stack.push(result);
                    }
                }
                Ins::LocalGet(index) => stack.push(locals[*index as usize]),
                Ins::LocalSet(index) => {
                    let value = pop(stack)?;
                    locals[*index as usize] = value;
                }
                Ins::LocalTee(index) => {
                    let value = *stack.last().ok_or(Trap::Unreachable)?;
                    locals[*index as usize] = value;
                }
                Ins::I32Const(value) => stack.push(Value::I32(*value)),
                Ins::I64Const(value) => stack.push(Value::I64(*value)),
                Ins::I32Eqz => {
                    let value = pop(stack)?.as_i64();
                    stack.push(Value::I32(if value == 0 { 1 } else { 0 }));
                }
                Ins::I64Eqz => {
                    let value = pop(stack)?.as_i64();
                    stack.push(Value::I32(if value == 0 { 1 } else { 0 }));
                }
                Ins::I32WrapI64 => {
                    let value = pop(stack)?.as_i64();
                    stack.push(Value::I32(value as i32));
                }
                Ins::I64ExtendI32S => {
                    let value = pop(stack)?.as_i64();
                    stack.push(Value::I64(value));
                }
                Ins::I64Load(offset) => {
                    let at = pop(stack)?.as_i64() as usize + *offset as usize;
                    stack.push(Value::I64(self.load(at)?));
                }
                Ins::I32Load(offset) => {
                    let at = pop(stack)?.as_i64() as usize + *offset as usize;
                    stack.push(Value::I32(self.load(at)? as i32));
                }
                Ins::I64Store(offset) => {
                    let value = pop(stack)?;
                    if !matches!(value, Value::I64(_)) {
                        return Err(Trap::Mistyped("an i64.store was handed an i32".to_string()));
                    }
                    let at = pop(stack)?.as_i64() as usize + *offset as usize;
                    self.store(at, value.as_i64())?;
                }
                Ins::I32Store(offset) => {
                    let value = pop(stack)?;
                    if !matches!(value, Value::I32(_)) {
                        return Err(Trap::Mistyped("an i32.store was handed an i64".to_string()));
                    }
                    let at = pop(stack)?.as_i64() as usize + *offset as usize;
                    self.store(at, value.as_i64())?;
                }
                other => self.arithmetic(other, stack)?,
            }
        }
        Ok(Flow::Normal)
    }

    fn load(&self, at: usize) -> Result<i64, Trap> {
        let bytes = self
            .memory
            .get(at..at + 8)
            .ok_or(Trap::OutOfBounds)?
            .try_into()
            .expect("eight bytes");
        Ok(i64::from_le_bytes(bytes))
    }

    fn store(&mut self, at: usize, value: i64) -> Result<(), Trap> {
        let place = self.memory.get_mut(at..at + 8).ok_or(Trap::OutOfBounds)?;
        place.copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    fn arithmetic(&mut self, instruction: &Ins, stack: &mut Vec<Value>) -> Result<(), Trap> {
        let right = pop(stack)?.as_i64();
        let left = pop(stack)?.as_i64();

        let value = match instruction {
            Ins::I32Add | Ins::I64Add => Value::I64(left.wrapping_add(right)),
            Ins::I32Sub | Ins::I64Sub => Value::I64(left.wrapping_sub(right)),
            Ins::I32Mul | Ins::I64Mul => Value::I64(left.wrapping_mul(right)),
            Ins::I64DivS => {
                if right == 0 {
                    return Err(Trap::DivideByZero);
                }
                Value::I64(left.wrapping_div(right))
            }
            Ins::I64RemS => {
                if right == 0 {
                    return Err(Trap::DivideByZero);
                }
                Value::I64(left.wrapping_rem(right))
            }
            Ins::I32Eq | Ins::I64Eq => boolean(left == right),
            Ins::I32Ne | Ins::I64Ne => boolean(left != right),
            Ins::I32LtS | Ins::I64LtS => boolean(left < right),
            Ins::I32LeS | Ins::I64LeS => boolean(left <= right),
            Ins::I32GtS | Ins::I64GtS => boolean(left > right),
            Ins::I32GeS | Ins::I64GeS => boolean(left >= right),
            Ins::I32And => boolean(left != 0 && right != 0),
            Ins::I32Or => boolean(left != 0 || right != 0),
            other => return Err(Trap::Unimplemented(format!("{other:?}"))),
        };

        // An i32 operation on i64 operands only happens where the compiler
        // put booleans in, and those stay in range, so the width is decided
        // by what the instruction produces rather than by the operands.
        stack.push(match instruction {
            Ins::I32Add | Ins::I32Sub | Ins::I32Mul => Value::I32(value.as_i64() as i32),
            _ => value,
        });
        Ok(())
    }
}

fn boolean(value: bool) -> Value {
    Value::I32(if value { 1 } else { 0 })
}

fn pop(stack: &mut Vec<Value>) -> Result<Value, Trap> {
    stack.pop().ok_or(Trap::Unreachable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::{Func, FuncType};

    fn module_with(body: Vec<Ins>, params: Vec<ValType>, results: Vec<ValType>) -> Module {
        let mut module = Module::new();
        let type_index = module.intern_type(FuncType { params, results });
        let func = module.add_func(Func {
            type_index,
            locals: Vec::new(),
            body,
        });
        module.export("f", func);
        module
    }

    #[test]
    fn arithmetic_comes_back_with_an_answer() {
        let module = module_with(
            vec![Ins::LocalGet(0), Ins::LocalGet(1), Ins::I64Add],
            vec![ValType::I64, ValType::I64],
            vec![ValType::I64],
        );
        let answer = call(&module, "f", &[Value::I64(2), Value::I64(3)]).expect("this runs");
        assert_eq!(answer, Some(Value::I64(5)));
    }

    #[test]
    fn a_branch_picks_one_arm_and_not_the_other() {
        let module = module_with(
            vec![
                Ins::LocalGet(0),
                Ins::If {
                    result: Some(ValType::I64),
                    then: vec![Ins::I64Const(10)],
                    otherwise: vec![Ins::I64Const(20)],
                },
            ],
            vec![ValType::I32],
            vec![ValType::I64],
        );
        assert_eq!(
            call(&module, "f", &[Value::I32(1)]),
            Ok(Some(Value::I64(10)))
        );
        assert_eq!(
            call(&module, "f", &[Value::I32(0)]),
            Ok(Some(Value::I64(20)))
        );
    }

    #[test]
    fn dividing_by_zero_says_so_rather_than_answering() {
        let module = module_with(
            vec![Ins::I64Const(1), Ins::I64Const(0), Ins::I64DivS],
            vec![],
            vec![ValType::I64],
        );
        assert_eq!(call(&module, "f", &[]), Err(Trap::DivideByZero));
    }

    /// A loop with no way out has to end the test rather than the machine.
    #[test]
    fn a_walk_that_never_ends_runs_out_of_budget() {
        let module = module_with(
            vec![Ins::Loop {
                result: None,
                body: vec![Ins::Br(0)],
            }],
            vec![],
            vec![],
        );
        assert_eq!(call(&module, "f", &[]), Err(Trap::TooLong));
    }

    #[test]
    fn an_export_nobody_declared_is_an_error_rather_than_a_panic() {
        let module = module_with(vec![], vec![], vec![]);
        assert!(matches!(
            call(&module, "nonesuch", &[]),
            Err(Trap::Unimplemented(_))
        ));
    }

    /// This does not validate modules, so the one thing it can usefully be
    /// strict about is the width of what an instruction is handed.
    ///
    /// Being lenient here is not a shortcut, it is a hole: a store that took
    /// whatever was on the stack would run a module no engine would load,
    /// and the backend bug behind it would look like a passing test.
    #[test]
    fn a_store_handed_the_wrong_width_says_so_rather_than_running() {
        let mut module = module_with(
            vec![Ins::I32Const(0), Ins::I32Const(7), Ins::I64Store(0)],
            vec![],
            vec![],
        );
        module.memory_pages = Some(1);
        assert!(
            matches!(call(&module, "f", &[]), Err(Trap::Mistyped(_))),
            "an i32 through an i64.store should be refused"
        );

        let mut widened = module_with(
            vec![
                Ins::I32Const(0),
                Ins::I32Const(7),
                Ins::I64ExtendI32S,
                Ins::I64Store(0),
            ],
            vec![],
            vec![],
        );
        widened.memory_pages = Some(1);
        assert_eq!(call(&widened, "f", &[]), Ok(None));
    }
}
