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
//!
//! [`Host`] is the part that proves the capability model is enforced by
//! structure rather than by check. It offers a specific set of imports and
//! refuses, at link time, any module whose import section asks for something
//! the host does not offer. What the module does not import it cannot call,
//! because the function index for an unimported operation does not exist in
//! the module's index space.

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
    /// The module asked the host for something, and this is not a host.
    ///
    /// Nothing missing here is going to be filled in: writing a line or
    /// reading a file are the host's to decide about, and a test oracle that
    /// decided them would be a program with authority nobody granted it. So
    /// this says which operation was wanted and stops, which is also the
    /// most useful thing it could say about a module that is going to be
    /// handed to a real embedder.
    NeedsAHost(String),
    /// Something this small runner does not implement, named rather than
    /// silently skipped.
    Unimplemented(String),
    /// The module would not validate, the way a real engine would refuse it
    /// before running a byte: see [`crate::validate`].
    Invalid(String),
}

/// The result of running an exported function, with memory usage.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Outcome {
    /// What the function answered with, if it returns a value.
    pub value: Option<Value>,
    /// How many bytes were allocated while this call was running.
    pub allocated: u64,
}

impl std::fmt::Display for Trap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Trap::Unreachable => write!(f, "the program stopped"),
            Trap::Failed { code, message } => write!(f, "{code}: {message}"),
            Trap::DivideByZero => write!(f, "divided by zero"),
            Trap::OutOfBounds => write!(f, "reached past the end of memory"),
            Trap::TooLong => write!(f, "ran too long"),
            Trap::NeedsAHost(what) => {
                write!(f, "`{what}` is the host's to answer, and this is not one")
            }
            Trap::Unimplemented(what) => write!(f, "`{what}` is not implemented here"),
            Trap::Invalid(reason) => write!(f, "the module does not validate: {reason}"),
        }
    }
}

/// How many instructions one call may run before this gives up.
///
/// A budget rather than a timer: a test that fails when the machine is busy
/// is a test people learn to rerun, which is the reasoning `design/01-principles.md`
/// already applies to the scaling test.
const BUDGET: u64 = 5_000_000;

/// The type of a host-provided import implementation.
type HostFn = Box<dyn Fn(&[Value]) -> Option<Value>>;

/// A host that provides implementations for a specific, named set of imports.
///
/// Where [`call`] stops with [`Trap::NeedsAHost`] when it reaches an import,
/// a `Host` actually answers those calls -- with exactly the set it declares
/// and no wider. [`Host::link`] checks that the module's import section is
/// fully covered before anything runs. A module that imports something the
/// host does not offer is refused at link time, the way a real engine refuses
/// to instantiate rather than running until the first missing call.
///
/// # What this proves
///
/// Deed's capability row names the operations a function may use. The compiler
/// turns that row into the module's import section. A host that offers a
/// narrow set enforces the row by structure:
///
/// - A component whose row does not mention an operation does not import it.
///   The operation's function index does not exist in the module's index space,
///   so the module cannot call it regardless of what the host offers.
///
/// - A component whose row does mention an operation that the host cannot
///   satisfy is refused at link time, before a single byte of the module runs.
pub struct Host {
    offers: Vec<HostOffer>,
}

struct HostOffer {
    module: String,
    name: String,
    func: HostFn,
}

/// An import the module declares that this host does not offer.
///
/// Returned by [`Host::link`]. The module is refused before it runs, which is
/// the "refused at load time" property the capability model requires.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LinkError {
    /// The WASM namespace the module asked for (`"deed:io"`, for example).
    pub module: String,
    /// The operation name within that namespace (`"write"`, for example).
    pub name: String,
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the host does not offer `{}.{}`", self.module, self.name)
    }
}

/// A module that has been linked to a [`Host`]: every import is satisfied.
///
/// Running this does not stop at a [`Trap::NeedsAHost`]. The host answers
/// every import the module declares, and the module cannot reach anything the
/// host did not include in its offer list.
pub struct Linked<'a> {
    host: &'a Host,
    module: &'a Module,
}

impl std::fmt::Debug for Linked<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Linked")
            .field("imports", &self.module.imports.len())
            .field("offers", &self.host.offers.len())
            .finish()
    }
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}

impl Host {
    /// A host that offers nothing.
    pub fn new() -> Self {
        Self { offers: Vec::new() }
    }

    /// Offer an implementation for one import.
    ///
    /// The module and name match what the WASM binary section declares:
    /// `"deed:io"` and `"write"` for `Io.write`, for example.
    pub fn offer(
        &mut self,
        module: &str,
        name: &str,
        func: impl Fn(&[Value]) -> Option<Value> + 'static,
    ) -> &mut Self {
        self.offers.push(HostOffer {
            module: module.to_string(),
            name: name.to_string(),
            func: Box::new(func),
        });
        self
    }

    fn implementation_for(&self, module: &str, name: &str) -> Option<&HostFn> {
        self.offers
            .iter()
            .find(|o| o.module == module && o.name == name)
            .map(|o| &o.func)
    }

    /// Try to link a module against this host.
    ///
    /// Every import in the module's section must be on this host's offer list.
    /// A module that imports something the host does not offer is refused here,
    /// before a single instruction runs.
    pub fn link<'a>(&'a self, module: &'a Module) -> Result<Linked<'a>, LinkError> {
        for import in &module.imports {
            if self
                .implementation_for(&import.module, &import.name)
                .is_none()
            {
                return Err(LinkError {
                    module: import.module.clone(),
                    name: import.name.clone(),
                });
            }
        }
        Ok(Linked { host: self, module })
    }
}

impl Linked<'_> {
    /// Call an exported function, dispatching host imports to the offer list.
    pub fn call(&self, name: &str, args: &[Value]) -> Result<Option<Value>, Trap> {
        if let Err(crate::validate::Invalid(reason)) = crate::validate::validate(self.module) {
            return Err(Trap::Invalid(reason));
        }
        let index = self
            .module
            .exports
            .iter()
            .find(|(exported, _)| exported == name)
            .map(|(_, index)| *index)
            .ok_or_else(|| Trap::Unimplemented(format!("no export named {name}")))?;
        let mut run = Run {
            module: self.module,
            fuel: BUDGET,
            memory: memory_of(self.module),
            host: Some(self.host),
        };
        match run.call(index, args) {
            Err(Trap::Unreachable) => Err(run.why().unwrap_or(Trap::Unreachable)),
            other => other,
        }
    }
}

/// Calls an exported function.
///
/// Validates the whole module first, the way a real engine would refuse to
/// load it rather than run it: a module only this permissive runner accepts
/// is a module every other engine rejects, and that gap should be found
/// here rather than by somebody outside this workspace (see
/// [`crate::validate`], and #567 for the shape of the gap this closes).
pub fn call(module: &Module, name: &str, args: &[Value]) -> Result<Option<Value>, Trap> {
    Ok(call_measured(module, name, args)?.value)
}

/// Calls an exported function and reports what it allocated.
pub fn call_measured(module: &Module, name: &str, args: &[Value]) -> Result<Outcome, Trap> {
    if let Err(crate::validate::Invalid(reason)) = crate::validate::validate(module) {
        return Err(Trap::Invalid(reason));
    }

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
        host: None,
    };
    let before = run.bump();
    match run.call(index, args) {
        // A trap that left something behind says what it was. Read here
        // rather than where the trap is raised, because the two words are
        // in memory and memory is what this owns.
        Err(Trap::Unreachable) => Err(run.why().unwrap_or(Trap::Unreachable)),
        Err(other) => Err(other),
        Ok(value) => Ok(Outcome {
            value,
            allocated: run.bump().saturating_sub(before),
        }),
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
    host: Option<&'a Host>,
}

/// What a block ended with: fell off the end, or jumped out of one.
enum Flow {
    Normal,
    /// Branch out of this many enclosing blocks, still counting down.
    Break(u32),
    Return,
}

impl Run<'_> {
    /// Where the bump pointer sits now, or zero when no memory is declared.
    fn bump(&self) -> u64 {
        self.memory
            .get(crate::layout::BUMP as usize..crate::layout::BUMP as usize + 8)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u64::from_le_bytes)
            .unwrap_or(0)
    }

    /// What the program left in memory about why it stopped, if anything.
    fn why(&self) -> Option<Trap> {
        Some(Trap::Failed {
            code: self.string_at(crate::layout::FAILURE_CODE)?,
            message: self.string_at(crate::layout::FAILURE_MESSAGE)?,
        })
    }

    /// The string whose address is in this word, read the way every string
    /// in a compiled module is laid out: a character count, a byte count,
    /// then the bytes.
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
        let length = u64::from_le_bytes(
            self.memory
                .get(address + 8..address + 16)?
                .try_into()
                .ok()?,
        ) as usize;
        let bytes = self.memory.get(address + 16..address + 16 + length)?;
        String::from_utf8(bytes.to_vec()).ok()
    }

    /// How many arguments the function at this index takes, whether it is
    /// imported or defined here.
    fn arity(&self, index: u32) -> Result<usize, Trap> {
        let type_index = match (index as usize).checked_sub(self.module.imports.len()) {
            None => self.module.imports[index as usize].type_index,
            Some(at) => {
                self.module
                    .funcs
                    .get(at)
                    .ok_or(Trap::OutOfBounds)?
                    .type_index
            }
        };
        Ok(self.module.types[type_index as usize].params.len())
    }

    fn call(&mut self, index: u32, args: &[Value]) -> Result<Option<Value>, Trap> {
        let Some(at) = (index as usize).checked_sub(self.module.imports.len()) else {
            let import = &self.module.imports[index as usize];
            if let Some(host) = self.host {
                if let Some(func) = host.implementation_for(&import.module, &import.name) {
                    return Ok(func(args));
                }
            }
            return Err(Trap::NeedsAHost(format!(
                "{}.{}",
                import.module, import.name
            )));
        };
        let func = &self.module.funcs[at];
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
                    let count = self.arity(*index)?;
                    let at = stack.len() - count;
                    let args: Vec<Value> = stack.split_off(at);
                    if let Some(result) = self.call(*index, &args)? {
                        stack.push(result);
                    }
                }
                Ins::CallIndirect(_) => {
                    let slot = pop(stack)?.as_i64() as usize;
                    let index = *self.module.table.get(slot).ok_or(Trap::OutOfBounds)?;
                    let count = self.arity(index)?;
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
                Ins::I64Store(offset) | Ins::I32Store(offset) => {
                    // The width itself is [`crate::validate`]'s job now: a
                    // module that reaches here already validated, so
                    // whatever is on the stack is the width the store
                    // declared.
                    let value = pop(stack)?;
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

    /// This runner does not validate on its own, but [`crate::call`] runs
    /// [`crate::validate::validate`] first, so a store handed the wrong
    /// width is refused before a byte of the module runs rather than
    /// silently accepted the way this runner alone would take it.
    #[test]
    fn a_store_handed_the_wrong_width_says_so_rather_than_running() {
        let mut module = module_with(
            vec![Ins::I32Const(0), Ins::I32Const(7), Ins::I64Store(0)],
            vec![],
            vec![],
        );
        module.memory_pages = Some(1);
        assert!(
            matches!(call(&module, "f", &[]), Err(Trap::Invalid(_))),
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
