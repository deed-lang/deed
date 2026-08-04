//! Validating a module the way a real WebAssembly engine would, before
//! [`crate::run`] is asked to execute a single instruction of it.
//!
//! [`crate::run`] says plainly that it is a test oracle rather than an
//! engine: it does not validate, so a module only a permissive runner
//! accepts is a module a real one refuses, and that gap is invisible until
//! somebody outside this workspace loads the output. #567 found one shape of
//! it at runtime, by accident, when an `i32` stored through an `i64.store`
//! ran here and would have loaded nowhere else.
//!
//! This is the general fix: the specified validation algorithm (a value
//! stack and a control stack, both allowed to answer "anything" once a
//! branch has made the rest of a block unreachable), run once over every
//! function this backend emits, ahead of running any of them.

use crate::wasm::{FuncType, Ins, Module, ValType};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invalid(pub String);

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One operand: a real type, or the placeholder standing for "whatever type
/// would make this valid", which is what the stack holds once code has
/// become unreachable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Val(ValType),
    Unknown,
}

/// One block, loop, if-branch, or the function body itself.
struct Frame {
    /// What a branch to this frame's label must find on the stack.
    ///
    /// Empty for a loop: branching there re-enters its start, which takes
    /// nothing. The declared result, for everything else: branching out of
    /// a block early has to leave behind what falling off its end would
    /// have.
    label_types: Vec<ValType>,
    /// What falling off the end of this frame leaves on the stack.
    end_types: Vec<ValType>,
    height: usize,
    unreachable: bool,
}

struct Checker<'m> {
    module: &'m Module,
    locals: Vec<ValType>,
    stack: Vec<Slot>,
    frames: Vec<Frame>,
}

impl<'m> Checker<'m> {
    fn push(&mut self, ty: ValType) {
        self.stack.push(Slot::Val(ty));
    }

    fn pop(&mut self) -> Result<Slot, Invalid> {
        let frame = self.frames.last().expect("a frame while a body is checked");
        if self.stack.len() == frame.height {
            return if frame.unreachable {
                Ok(Slot::Unknown)
            } else {
                Err(Invalid(
                    "an instruction wants a value this block does not have".to_string(),
                ))
            };
        }
        Ok(self.stack.pop().expect("checked above the frame's height"))
    }

    fn pop_expect(&mut self, expected: ValType) -> Result<(), Invalid> {
        match self.pop()? {
            Slot::Unknown => Ok(()),
            Slot::Val(found) if found == expected => Ok(()),
            Slot::Val(found) => Err(Invalid(format!(
                "expected {expected:?} on the stack but found {found:?}"
            ))),
        }
    }

    fn local_type(&self, index: u32) -> Result<ValType, Invalid> {
        self.locals.get(index as usize).copied().ok_or_else(|| {
            Invalid(format!(
                "names a local ({index}) this function does not have"
            ))
        })
    }

    fn func_type(&self, index: u32) -> Result<FuncType, Invalid> {
        let type_index = match (index as usize).checked_sub(self.module.imports.len()) {
            None => self.module.imports[index as usize].type_index,
            Some(at) => {
                self.module
                    .funcs
                    .get(at)
                    .ok_or_else(|| {
                        Invalid(format!(
                            "calls a function index ({index}) the module does not have"
                        ))
                    })?
                    .type_index
            }
        };
        self.module
            .types
            .get(type_index as usize)
            .cloned()
            .ok_or_else(|| {
                Invalid(format!(
                    "names a type index ({type_index}) the module does not have"
                ))
            })
    }

    fn push_frame(&mut self, label_types: Vec<ValType>, end_types: Vec<ValType>) {
        let height = self.stack.len();
        self.frames.push(Frame {
            label_types,
            end_types,
            height,
            unreachable: false,
        });
    }

    /// Checks the frame's `end_types` are on top of the stack, and leaves
    /// exactly them behind once the frame is gone.
    fn pop_frame(&mut self) -> Result<Vec<ValType>, Invalid> {
        let end_types = self
            .frames
            .last()
            .expect("a frame while a body is checked")
            .end_types
            .clone();
        for ty in end_types.iter().rev() {
            self.pop_expect(*ty)?;
        }
        let frame = self.frames.last().expect("still the same frame");
        if self.stack.len() != frame.height {
            return Err(Invalid(
                "a block leaves values behind that it did not declare as its result".to_string(),
            ));
        }
        self.frames.pop();
        Ok(end_types)
    }

    fn set_unreachable(&mut self) {
        let frame = self
            .frames
            .last_mut()
            .expect("a frame while a body is checked");
        self.stack.truncate(frame.height);
        frame.unreachable = true;
    }

    fn branch_target(&self, depth: u32) -> Result<Vec<ValType>, Invalid> {
        let index = self
            .frames
            .len()
            .checked_sub(1 + depth as usize)
            .ok_or_else(|| Invalid(format!("branches {depth} blocks out with no block there")))?;
        Ok(self.frames[index].label_types.clone())
    }

    fn instruction(&mut self, ins: &Ins) -> Result<(), Invalid> {
        match ins {
            Ins::Marked { inner, .. } => {
                self.instruction(inner)?;
            }
            Ins::Unreachable => self.set_unreachable(),
            Ins::Nop => {}
            Ins::Block { result, body } => {
                let results: Vec<ValType> = result.iter().copied().collect();
                self.push_frame(results.clone(), results);
                for inner in body {
                    self.instruction(inner)?;
                }
                for ty in self.pop_frame()? {
                    self.push(ty);
                }
            }
            Ins::Loop { result, body } => {
                let results: Vec<ValType> = result.iter().copied().collect();
                self.push_frame(Vec::new(), results);
                for inner in body {
                    self.instruction(inner)?;
                }
                for ty in self.pop_frame()? {
                    self.push(ty);
                }
            }
            Ins::If {
                result,
                then,
                otherwise,
            } => {
                self.pop_expect(ValType::I32)?;
                let results: Vec<ValType> = result.iter().copied().collect();
                if !results.is_empty() && otherwise.is_empty() {
                    return Err(Invalid(
                        "an `if` with a result needs an `else`: falling through the implicit empty one would not produce it".to_string(),
                    ));
                }
                self.push_frame(results.clone(), results.clone());
                for inner in then {
                    self.instruction(inner)?;
                }
                let produced = self.pop_frame()?;
                self.push_frame(results.clone(), results);
                for inner in otherwise {
                    self.instruction(inner)?;
                }
                self.pop_frame()?;
                for ty in produced {
                    self.push(ty);
                }
            }
            Ins::Br(depth) => {
                let target = self.branch_target(*depth)?;
                for ty in target.iter().rev() {
                    self.pop_expect(*ty)?;
                }
                self.set_unreachable();
            }
            Ins::BrIf(depth) => {
                self.pop_expect(ValType::I32)?;
                let target = self.branch_target(*depth)?;
                for ty in target.iter().rev() {
                    self.pop_expect(*ty)?;
                }
                for ty in target {
                    self.push(ty);
                }
            }
            Ins::Return => {
                let results = self.frames[0].end_types.clone();
                for ty in results.iter().rev() {
                    self.pop_expect(*ty)?;
                }
                self.set_unreachable();
            }
            Ins::Call(index) => {
                let ty = self.func_type(*index)?;
                for param in ty.params.iter().rev() {
                    self.pop_expect(*param)?;
                }
                for result in &ty.results {
                    self.push(*result);
                }
            }
            Ins::CallIndirect(type_index) => {
                self.pop_expect(ValType::I32)?;
                let ty = self
                    .module
                    .types
                    .get(*type_index as usize)
                    .cloned()
                    .ok_or_else(|| {
                        Invalid(format!(
                            "call_indirect names a type index ({type_index}) the module does not have"
                        ))
                    })?;
                for param in ty.params.iter().rev() {
                    self.pop_expect(*param)?;
                }
                for result in &ty.results {
                    self.push(*result);
                }
            }
            Ins::Drop => {
                self.pop()?;
            }
            Ins::LocalGet(index) => {
                let ty = self.local_type(*index)?;
                self.push(ty);
            }
            Ins::LocalSet(index) => {
                let ty = self.local_type(*index)?;
                self.pop_expect(ty)?;
            }
            Ins::LocalTee(index) => {
                let ty = self.local_type(*index)?;
                self.pop_expect(ty)?;
                self.push(ty);
            }
            Ins::I32Const(_) => self.push(ValType::I32),
            Ins::I64Const(_) => self.push(ValType::I64),
            Ins::I32Add | Ins::I32Sub | Ins::I32Mul | Ins::I32And | Ins::I32Or => {
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
                self.push(ValType::I32);
            }
            Ins::I32Eq | Ins::I32Ne | Ins::I32LtS | Ins::I32LeS | Ins::I32GtS | Ins::I32GeS => {
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
                self.push(ValType::I32);
            }
            Ins::I32Eqz => {
                self.pop_expect(ValType::I32)?;
                self.push(ValType::I32);
            }
            Ins::I64Add | Ins::I64Sub | Ins::I64Mul | Ins::I64Xor | Ins::I64DivS | Ins::I64RemS => {
                self.pop_expect(ValType::I64)?;
                self.pop_expect(ValType::I64)?;
                self.push(ValType::I64);
            }
            Ins::I64Eq | Ins::I64Ne | Ins::I64LtS | Ins::I64LeS | Ins::I64GtS | Ins::I64GeS => {
                self.pop_expect(ValType::I64)?;
                self.pop_expect(ValType::I64)?;
                self.push(ValType::I32);
            }
            Ins::I64Eqz => {
                self.pop_expect(ValType::I64)?;
                self.push(ValType::I32);
            }
            Ins::I32WrapI64 => {
                self.pop_expect(ValType::I64)?;
                self.push(ValType::I32);
            }
            Ins::I64ExtendI32S => {
                self.pop_expect(ValType::I32)?;
                self.push(ValType::I64);
            }
            Ins::I32Load(_) => {
                self.require_memory()?;
                self.pop_expect(ValType::I32)?;
                self.push(ValType::I32);
            }
            Ins::I64Load(_) => {
                self.require_memory()?;
                self.pop_expect(ValType::I32)?;
                self.push(ValType::I64);
            }
            Ins::I32Store(_) => {
                self.require_memory()?;
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
            }
            Ins::I64Store(_) => {
                self.require_memory()?;
                self.pop_expect(ValType::I64)?;
                self.pop_expect(ValType::I32)?;
            }
            Ins::I32Load8U(_) => {
                self.require_memory()?;
                self.pop_expect(ValType::I32)?;
                self.push(ValType::I32);
            }
            Ins::I32Store8(_) => {
                self.require_memory()?;
                self.pop_expect(ValType::I32)?;
                self.pop_expect(ValType::I32)?;
            }
        }
        Ok(())
    }

    fn require_memory(&self) -> Result<(), Invalid> {
        if self.module.memory_pages.is_none() {
            return Err(Invalid(
                "loads or stores, but the module declares no memory".to_string(),
            ));
        }
        Ok(())
    }
}

/// Checks every function this backend emits against the module's own
/// declared types, the way loading the module for real would.
pub fn validate(module: &Module) -> Result<(), Invalid> {
    for (at, func) in module.funcs.iter().enumerate() {
        let named = |why: Invalid| {
            let index = module.imports.len() + at;
            let name = module
                .names
                .iter()
                .find(|(one, _)| *one as usize == index)
                .map(|(_, name)| name.as_str())
                .unwrap_or("a function with no name");
            Invalid(format!("in `{name}`: {}", why.0))
        };
        let signature = module.types.get(func.type_index as usize).ok_or_else(|| {
            Invalid(format!(
                "a function names a type index ({}) the module does not have",
                func.type_index
            ))
        })?;

        let mut locals = signature.params.clone();
        locals.extend(func.locals.iter().copied());

        let mut checker = Checker {
            module,
            locals,
            stack: Vec::new(),
            frames: Vec::new(),
        };
        checker.push_frame(signature.results.clone(), signature.results.clone());
        for ins in &func.body {
            checker.instruction(ins).map_err(named)?;
        }
        checker.pop_frame().map_err(named)?;
    }

    for index in &module.table {
        if (*index as usize) >= module.imports.len() + module.funcs.len() {
            return Err(Invalid(format!(
                "the table names a function index ({index}) the module does not have"
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm::Func;

    fn module_with(types: Vec<FuncType>, func: Func) -> Module {
        let mut module = Module::new();
        module.types = types;
        module.funcs.push(func);
        module
    }

    /// A module that does not validate says which function it was in.
    ///
    /// A module is one list of instructions after another and the failure is
    /// about one of them, so without the name the answer is "somewhere in
    /// here". Every function the backend writes has a name in the name
    /// section already, and the index a name is filed under counts the
    /// imports first.
    #[test]
    fn what_does_not_validate_says_which_function_it_was_in() {
        let mut module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![ValType::I64],
            }],
            Func {
                type_index: 0,
                locals: vec![],
                body: vec![Ins::I64Add],
            },
        );
        module.imports.push(crate::wasm::Import {
            module: "deed:io".to_string(),
            name: "write".to_string(),
            type_index: 0,
        });
        module.names.push((0, "the import".to_string()));
        module.names.push((1, "the one that is wrong".to_string()));

        let why = validate(&module).expect_err("this does not validate");
        assert!(
            why.0.contains("the one that is wrong"),
            "the failure should name the function it was in: {}",
            why.0
        );
        assert!(
            !why.0.contains("the import"),
            "an import is not a body and cannot be the one: {}",
            why.0
        );
    }

    #[test]
    fn a_function_that_returns_what_it_declares_validates() {
        let module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![ValType::I64],
            }],
            Func {
                type_index: 0,
                locals: vec![],
                body: vec![Ins::I64Const(1)],
            },
        );
        assert_eq!(validate(&module), Ok(()));
    }

    #[test]
    fn storing_an_i64_value_through_an_i32_store_is_refused() {
        let mut module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![],
            }],
            Func {
                type_index: 0,
                locals: vec![],
                body: vec![Ins::I32Const(0), Ins::I64Const(1), Ins::I32Store(0)],
            },
        );
        module.memory_pages = Some(1);
        assert!(validate(&module).is_err());
    }

    #[test]
    fn a_function_that_returns_the_wrong_type_is_refused() {
        let module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![ValType::I64],
            }],
            Func {
                type_index: 0,
                locals: vec![],
                body: vec![Ins::I32Const(1)],
            },
        );
        assert!(validate(&module).is_err());
    }

    #[test]
    fn a_function_that_returns_nothing_when_something_was_promised_is_refused() {
        let module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![ValType::I64],
            }],
            Func {
                type_index: 0,
                locals: vec![],
                body: vec![],
            },
        );
        assert!(validate(&module).is_err());
    }

    #[test]
    fn popping_past_what_unreachable_code_left_behind_is_allowed() {
        // `Drop` normally demands a value be there. After `Unreachable`
        // there is none, and the point of the polymorphic stack is that
        // this is not an error: the rest of the block is dead code, so
        // the checker cannot know what would have been on the stack, and
        // is not supposed to guess.
        let module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![ValType::I64],
            }],
            Func {
                type_index: 0,
                locals: vec![],
                body: vec![Ins::Unreachable, Ins::Drop],
            },
        );
        assert_eq!(validate(&module), Ok(()));
    }

    #[test]
    fn an_if_with_a_result_and_no_else_is_refused() {
        let module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![ValType::I64],
            }],
            Func {
                type_index: 0,
                locals: vec![],
                body: vec![
                    Ins::I32Const(1),
                    Ins::If {
                        result: Some(ValType::I64),
                        then: vec![Ins::I64Const(1)],
                        otherwise: Vec::new(),
                    },
                ],
            },
        );
        assert!(validate(&module).is_err());
    }

    #[test]
    fn loading_through_a_module_with_no_memory_is_refused() {
        let module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![ValType::I64],
            }],
            Func {
                type_index: 0,
                locals: vec![],
                body: vec![Ins::I32Const(0), Ins::I64Load(0)],
            },
        );
        assert!(validate(&module).is_err());
    }

    #[test]
    fn a_branch_out_of_a_block_carrying_the_wrong_type_is_refused() {
        let module = module_with(
            vec![FuncType {
                params: vec![],
                results: vec![],
            }],
            Func {
                type_index: 0,
                locals: vec![],
                body: vec![Ins::Block {
                    result: Some(ValType::I64),
                    body: vec![Ins::I32Const(1), Ins::Br(0)],
                }],
            },
        );
        assert!(validate(&module).is_err());
    }

    #[test]
    fn calling_a_function_with_the_wrong_argument_type_is_refused() {
        let module = {
            let mut module = Module::new();
            module.types = vec![
                FuncType {
                    params: vec![ValType::I64],
                    results: vec![],
                },
                FuncType {
                    params: vec![],
                    results: vec![],
                },
            ];
            module.funcs.push(Func {
                type_index: 0,
                locals: vec![],
                body: vec![],
            });
            module.funcs.push(Func {
                type_index: 1,
                locals: vec![],
                body: vec![Ins::I32Const(1), Ins::Call(0)],
            });
            module
        };
        assert!(validate(&module).is_err());
    }
}
