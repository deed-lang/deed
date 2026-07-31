//! Compiling [`deed_mir`] to a WebAssembly module.
//!
//! What this handles: `Unit`, `Bool` and `Int`, the operators over them,
//! string literals, lists, records and choices, direct calls, closures, `if`,
//! blocks, slots, generics (monomorphized), contracts, effect handlers and
//! capabilities (as host imports, #569). What still is not: the prelude
//! functions beyond `ok`, `err` and `length`, and early `return`; see #621
//! and `design/05-backend.md` for the rest of the gap, enumerated rather
//! than remembered.
//!
//! Refusing rather than approximating is the checker's own rule. A backend
//! that quietly compiled something into the wrong shape would produce a
//! program that runs and answers wrongly, which is worse than one that does
//! not build.

use deed_diagnostics::Span;
use deed_mir::{BinaryOp, Block, Expr, FuncId, Function, Local, Program, Stmt, Ty, UnaryOp};

use crate::layout;
use crate::wasm::{
    Func, FuncType, FunctionSpans, Import, Ins, InstructionSpan, Module, SpanRole, ValType,
    instruction_size,
};

const ARITHMETIC_CODE: &str = "DEED6007";
const ARITHMETIC_MESSAGE: &str = "this arithmetic has no answer";

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
    //
    // Nothing is turned away by type any more. Every type this IR has now
    // has a representation, including a capability, which is a handle the
    // host gave out rather than anything the program can look inside. What
    // is still refused is refused by `Builder::fail` where it is met, and
    // the earlier refusals were the backend not having got somewhere yet
    // rather than a rule.
    for function in &program.functions {
        index_of_signature(&mut module, function);
    }

    // Then everything the host has to supply, before any function is added,
    // because an import is numbered ahead of every function the module
    // defines. Adding one later would move every function already placed.
    let wanted = host_calls(program);
    for (name, signature) in &wanted {
        let type_index = module.intern_type(signature.clone());
        let (namespace, operation) = name.split_once('.').unwrap_or(("deed", name));
        module.add_import(&format!("deed:{namespace}"), operation, type_index);
    }
    // Give each import a debug name so a runtime can name the frame. The
    // operation name (the part after the namespace dot) is what a reader
    // of the source would recognize.
    for (index, import) in module.imports.iter().enumerate() {
        module.names.push((index as u32, import.name.clone()));
    }
    let shift = module.imports.len() as u32;

    let mut strings = Strings::new();

    for function in &program.functions {
        let type_index = index_of_signature(&mut module, function);
        let interned = module.types.clone();
        let imported = module.imports.clone();
        let mut body = Builder::new(program, function, &mut strings, interned, imported, shift);
        body.block(&function.body)?;
        body.instructions.push(Ins::Return);
        let func_index = shift + module.funcs.len() as u32;
        let spans = body.finish_sites(func_index, &body.instructions);

        let compiled = Func {
            type_index,
            locals: body.extra_locals,
            body: body.instructions,
        };
        let func_index = module.add_func(compiled);
        module.export(function.name.clone(), func_index);
        if !spans.sites.is_empty() {
            module.spans.push(spans);
        }
        // Record the function's source name for the name section, so a
        // trap says which function rather than its index.
        module.names.push((func_index, function.name.clone()));
        // Every function goes in the table at its own index, so a code
        // pointer and a function index are the same number and lowering does
        // not need a second numbering to keep straight.
        module.intern_table(func_index);
    }

    // The bump pointer starts past every literal the data section placed.
    let mut placed = strings.data;
    placed.push((layout::BUMP, (strings.next as i64).to_le_bytes().to_vec()));
    placed.push((
        layout::FRAME_BUMP,
        (layout::FRAME_START as i64).to_le_bytes().to_vec(),
    ));
    module.data = placed;

    Ok(module)
}

/// Every host call the program makes, by name, with the signature it wants.
///
/// Collected up front rather than as bodies are compiled, because imports
/// are numbered before defined functions and a body cannot add one without
/// moving every function already placed.
fn host_calls(program: &Program) -> Vec<(String, FuncType)> {
    fn walk(expr: &Expr, found: &mut Vec<(String, FuncType)>) {
        if let Expr::Host { name, args, ret } = expr
            && !found.iter().any(|(seen, _)| seen == name)
        {
            found.push((
                name.clone(),
                FuncType {
                    params: args
                        .iter()
                        .filter_map(|arg| static_ty(arg).and_then(|ty| val_type(&ty)))
                        .collect(),
                    results: val_type(ret).into_iter().collect(),
                },
            ));
        }

        match expr {
            Expr::Unary { operand, .. } => walk(operand, found),
            Expr::Binary { left, right, .. } => {
                walk(left, found);
                walk(right, found);
            }
            Expr::Call { args, .. }
            | Expr::Make { fields: args, .. }
            | Expr::List { items: args, .. }
            | Expr::Runtime { args, .. }
            | Expr::Perform { args, .. }
            | Expr::Host { args, .. } => {
                for arg in args {
                    walk(arg, found);
                }
            }
            Expr::CallIndirect { callee, args, .. } => {
                walk(callee, found);
                for arg in args {
                    walk(arg, found);
                }
            }
            Expr::Field { value, .. } | Expr::Discriminant { value, .. } => walk(value, found),
            Expr::If {
                condition,
                then,
                otherwise,
                ..
            } => {
                walk(condition, found);
                walk_block(then, found);
                walk_block(otherwise, found);
            }
            Expr::Block(block) => walk_block(block, found),
            Expr::ElementAt { list, index, .. } => {
                walk(list, found);
                walk(index, found);
            }
            Expr::Install { state, body, .. } => {
                walk(state, found);
                walk_block(body, found);
            }
            _ => {}
        }
    }

    fn walk_block(block: &Block, found: &mut Vec<(String, FuncType)>) {
        for stmt in &block.stmts {
            walk_stmt(stmt, found);
        }
        walk(&block.value, found);
    }

    fn walk_stmt(stmt: &Stmt, found: &mut Vec<(String, FuncType)>) {
        match stmt {
            Stmt::Assign { value, .. } | Stmt::Discard(value) | Stmt::Return { value } => {
                walk(value, found)
            }
            Stmt::Fail { .. } => {}
            Stmt::While { condition, body } => {
                walk(condition, found);
                for stmt in body {
                    walk_stmt(stmt, found);
                }
            }
            Stmt::SetField { object, value, .. } => {
                walk(object, found);
                walk(value, found);
            }
        }
    }

    let mut found = Vec::new();
    for function in &program.functions {
        walk_block(&function.body, &mut found);
    }
    found
}

/// What an expression produces, for the cases an import's signature needs,
/// which are the ones that do not depend on the function being compiled.
///
/// A host call takes capabilities and literals. Anything else would be a
/// program handing the host something out of its own memory, which nothing
/// in the language can express.
fn static_ty(expr: &Expr) -> Option<Ty> {
    Some(match expr {
        Expr::Unit => Ty::Unit,
        Expr::Bool(_) => Ty::Bool,
        Expr::Int(_) => Ty::Int,
        Expr::Str(_) => Ty::Str,
        Expr::Host { ret, .. } => (**ret).clone(),
        Expr::Call { .. } | Expr::Local(_) | Expr::Field { .. } => Ty::Capability,
        _ => return None,
    })
}

fn index_of_signature(module: &mut Module, function: &Function) -> u32 {
    let params = function.params.iter().filter_map(val_type).collect();
    let results = val_type(&function.ret).into_iter().collect();
    module.intern_type(FuncType { params, results })
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

#[derive(Clone, Copy)]
struct SiteDraft {
    span: Span,
    role: SpanRole,
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
        let mut bytes = (text.chars().count() as i64).to_le_bytes().to_vec();
        bytes.extend_from_slice(&(text.len() as i64).to_le_bytes());
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
    /// Every signature the module has interned, so an indirect call can name
    /// the one it goes through.
    signatures: Vec<FuncType>,
    /// What the host supplies, in the order it is numbered.
    imports: Vec<Import>,
    /// How many imports come before the first defined function.
    ///
    /// A MIR `FuncId` is an index into the program's own functions, and a
    /// WebAssembly function index counts imports first. This is the
    /// difference, and it is applied in exactly two places: a direct call
    /// and the table.
    shift: u32,
    /// Source spans attached to selected instructions in this function.
    sites: Vec<SiteDraft>,
}

impl<'a> Builder<'a> {
    fn new(
        program: &'a Program,
        function: &'a Function,
        strings: &'a mut Strings,
        signatures: Vec<FuncType>,
        imports: Vec<Import>,
        shift: u32,
    ) -> Self {
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
            signatures,
            imports,
            shift,
            sites: Vec::new(),
        }
    }

    /// Where the host call of this name is numbered.
    fn import(&self, name: &str) -> Result<u32, Unsupported> {
        let (namespace, operation) = name.split_once('.').unwrap_or(("deed", name));
        let namespace = format!("deed:{namespace}");
        self.imports
            .iter()
            .position(|found| found.module == namespace && found.name == operation)
            .map(|at| at as u32)
            .ok_or_else(|| self.fail(&format!("a call to `{name}`, which nothing imported")))
    }

    /// Which interned signature this is.
    ///
    /// Every signature an indirect call can reach is already interned,
    /// because the body it reaches is a function the program declares and
    /// the first pass walked all of them. A call that found none would be a
    /// call to something that was never lowered.
    fn signature(&self, wanted: FuncType) -> Result<u32, Unsupported> {
        self.signatures
            .iter()
            .position(|found| *found == wanted)
            .map(|at| at as u32)
            .ok_or_else(|| self.fail("a call to a shape nothing declares"))
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

    fn failure_instructions(&mut self, code: &str, message: &str) -> Vec<Ins> {
        let at_code = self.strings.place(code);
        let at_message = self.strings.place(message);
        vec![
            Ins::I32Const(layout::FAILURE_CODE as i32),
            Ins::I64Const(at_code as i64),
            Ins::I64Store(0),
            Ins::I32Const(layout::FAILURE_MESSAGE as i32),
            Ins::I64Const(at_message as i64),
            Ins::I64Store(0),
            Ins::Unreachable,
        ]
    }

    fn trap_on_true(&mut self) {
        let failure = self.failure_instructions(ARITHMETIC_CODE, ARITHMETIC_MESSAGE);
        self.instructions.push(Ins::If {
            result: None,
            then: failure,
            otherwise: Vec::new(),
        });
    }

    fn checked_negate(&mut self, operand: &Expr) -> Result<(), Unsupported> {
        let value = self.temporary(ValType::I64);
        self.expr(operand)?;
        self.instructions.push(Ins::LocalSet(value));

        self.instructions.push(Ins::LocalGet(value));
        self.instructions.push(Ins::I64Const(i64::MIN));
        self.instructions.push(Ins::I64Eq);
        self.trap_on_true();

        self.instructions.push(Ins::I64Const(0));
        self.instructions.push(Ins::LocalGet(value));
        self.instructions.push(Ins::I64Sub);
        Ok(())
    }

    fn checked_binary(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
    ) -> Result<(), Unsupported> {
        let left_slot = self.temporary(ValType::I64);
        let right_slot = self.temporary(ValType::I64);

        self.expr(left)?;
        self.instructions.push(Ins::LocalSet(left_slot));
        self.expr(right)?;
        self.instructions.push(Ins::LocalSet(right_slot));

        match op {
            BinaryOp::AddInt => {
                let sum = self.temporary(ValType::I64);
                self.instructions.push(Ins::LocalGet(left_slot));
                self.instructions.push(Ins::LocalGet(right_slot));
                self.instructions.push(Ins::I64Add);
                self.instructions.push(Ins::LocalSet(sum));

                self.instructions.push(Ins::LocalGet(right_slot));
                self.instructions.push(Ins::I64Const(0));
                self.instructions.push(Ins::I64GtS);
                self.instructions.push(Ins::LocalGet(sum));
                self.instructions.push(Ins::LocalGet(left_slot));
                self.instructions.push(Ins::I64LtS);
                self.instructions.push(Ins::I32And);

                self.instructions.push(Ins::LocalGet(right_slot));
                self.instructions.push(Ins::I64Const(0));
                self.instructions.push(Ins::I64LtS);
                self.instructions.push(Ins::LocalGet(sum));
                self.instructions.push(Ins::LocalGet(left_slot));
                self.instructions.push(Ins::I64GtS);
                self.instructions.push(Ins::I32And);

                self.instructions.push(Ins::I32Or);
                self.trap_on_true();
                self.instructions.push(Ins::LocalGet(sum));
            }
            BinaryOp::SubInt => {
                let diff = self.temporary(ValType::I64);
                self.instructions.push(Ins::LocalGet(left_slot));
                self.instructions.push(Ins::LocalGet(right_slot));
                self.instructions.push(Ins::I64Sub);
                self.instructions.push(Ins::LocalSet(diff));

                self.instructions.push(Ins::LocalGet(right_slot));
                self.instructions.push(Ins::I64Const(0));
                self.instructions.push(Ins::I64GtS);
                self.instructions.push(Ins::LocalGet(diff));
                self.instructions.push(Ins::LocalGet(left_slot));
                self.instructions.push(Ins::I64GtS);
                self.instructions.push(Ins::I32And);

                self.instructions.push(Ins::LocalGet(right_slot));
                self.instructions.push(Ins::I64Const(0));
                self.instructions.push(Ins::I64LtS);
                self.instructions.push(Ins::LocalGet(diff));
                self.instructions.push(Ins::LocalGet(left_slot));
                self.instructions.push(Ins::I64LtS);
                self.instructions.push(Ins::I32And);

                self.instructions.push(Ins::I32Or);
                self.trap_on_true();
                self.instructions.push(Ins::LocalGet(diff));
            }
            BinaryOp::MulInt => {
                let product = self.temporary(ValType::I64);
                self.instructions.push(Ins::LocalGet(left_slot));
                self.instructions.push(Ins::LocalGet(right_slot));
                self.instructions.push(Ins::I64Mul);
                self.instructions.push(Ins::LocalSet(product));

                self.instructions.push(Ins::LocalGet(left_slot));
                self.instructions.push(Ins::I64Const(-1));
                self.instructions.push(Ins::I64Eq);
                self.instructions.push(Ins::LocalGet(right_slot));
                self.instructions.push(Ins::I64Const(i64::MIN));
                self.instructions.push(Ins::I64Eq);
                self.instructions.push(Ins::I32And);

                self.trap_on_true();

                self.instructions.push(Ins::LocalGet(left_slot));
                self.instructions.push(Ins::I64Eqz);
                self.instructions.push(Ins::LocalGet(right_slot));
                self.instructions.push(Ins::I64Eqz);
                self.instructions.push(Ins::I32Or);
                self.instructions.push(Ins::I32Eqz);
                let failure = self.failure_instructions(ARITHMETIC_CODE, ARITHMETIC_MESSAGE);
                self.instructions.push(Ins::If {
                    result: None,
                    then: vec![
                        Ins::LocalGet(product),
                        Ins::LocalGet(left_slot),
                        Ins::I64DivS,
                        Ins::LocalGet(right_slot),
                        Ins::I64Ne,
                        Ins::If {
                            result: None,
                            then: failure,
                            otherwise: Vec::new(),
                        },
                    ],
                    otherwise: Vec::new(),
                });

                self.instructions.push(Ins::LocalGet(product));
            }
            BinaryOp::DivInt | BinaryOp::RemInt => {
                self.instructions.push(Ins::LocalGet(right_slot));
                self.instructions.push(Ins::I64Eqz);

                self.instructions.push(Ins::LocalGet(left_slot));
                self.instructions.push(Ins::I64Const(i64::MIN));
                self.instructions.push(Ins::I64Eq);
                self.instructions.push(Ins::LocalGet(right_slot));
                self.instructions.push(Ins::I64Const(-1));
                self.instructions.push(Ins::I64Eq);
                self.instructions.push(Ins::I32And);

                self.instructions.push(Ins::I32Or);
                self.trap_on_true();

                self.instructions.push(Ins::LocalGet(left_slot));
                self.instructions.push(Ins::LocalGet(right_slot));
                self.instructions.push(match op {
                    BinaryOp::DivInt => Ins::I64DivS,
                    BinaryOp::RemInt => Ins::I64RemS,
                    _ => unreachable!("only division and remainder reach here"),
                });
            }
            _ => unreachable!("only checked integer operations reach here"),
        }

        Ok(())
    }

    fn site(&mut self, span: Span, role: SpanRole) -> u32 {
        self.sites.push(SiteDraft { span, role });
        (self.sites.len() - 1) as u32
    }

    fn mark(&mut self, span: Span, role: SpanRole, inner: Ins) {
        let site = self.site(span, role);
        self.instructions.push(Ins::Marked {
            site,
            inner: Box::new(inner),
        });
    }

    fn finish_sites(&self, function: u32, body: &[Ins]) -> FunctionSpans {
        fn walk(
            out: &mut [Option<InstructionSpan>],
            sites: &[SiteDraft],
            body: &[Ins],
            offset: &mut u32,
        ) {
            for instruction in body {
                match instruction {
                    Ins::Marked { site, inner } => {
                        out[*site as usize] = Some(InstructionSpan {
                            offset: *offset,
                            span: sites[*site as usize].span,
                            role: sites[*site as usize].role,
                        });
                        walk(out, sites, std::slice::from_ref(inner.as_ref()), offset);
                    }
                    Ins::Block { body, .. } | Ins::Loop { body, .. } => {
                        *offset += 2;
                        walk(out, sites, body, offset);
                        *offset += 1;
                    }
                    Ins::If {
                        then, otherwise, ..
                    } => {
                        *offset += 2;
                        walk(out, sites, then, offset);
                        if !otherwise.is_empty() {
                            *offset += 1;
                            walk(out, sites, otherwise, offset);
                        }
                        *offset += 1;
                    }
                    _ => *offset += instruction_size(instruction),
                }
            }
        }

        let mut mapped = vec![None; self.sites.len()];
        let mut offset = 0;
        walk(&mut mapped, &self.sites, body, &mut offset);
        FunctionSpans {
            function,
            sites: mapped
                .into_iter()
                .map(|site| site.expect("every marked instruction should have an offset"))
                .collect(),
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
            Stmt::Fail {
                code,
                message,
                span,
            } => {
                // A contract failure ends the program, and says which one
                // before it does. The two strings are placed the way every
                // literal is, and their addresses go in the two words
                // `layout` set aside, so whatever runs the module can read
                // them after the trap.
                let at_code = self.strings.place(code);
                let at_message = self.strings.place(message);

                self.instructions
                    .push(Ins::I32Const(layout::FAILURE_CODE as i32));
                self.instructions.push(Ins::I64Const(at_code as i64));
                self.instructions.push(Ins::I64Store(0));

                self.instructions
                    .push(Ins::I32Const(layout::FAILURE_MESSAGE as i32));
                self.instructions.push(Ins::I64Const(at_message as i64));
                self.instructions.push(Ins::I64Store(0));

                self.mark(*span, SpanRole::Trap, Ins::Unreachable);
            }
            Stmt::Return { value } => {
                self.expr(value)?;
                self.instructions.push(Ins::Return);
            }
            Stmt::While { condition, body } => {
                // A block to jump out of and a loop to jump back to, which is
                // how WebAssembly spells a walk: the condition is read at the
                // top and its inverse leaves.
                let saved = std::mem::take(&mut self.instructions);
                self.expr(condition)?;
                self.instructions.push(Ins::I32Eqz);
                self.instructions.push(Ins::BrIf(1));
                for stmt in body {
                    self.stmt(stmt)?;
                }
                self.instructions.push(Ins::Br(0));
                let inner = std::mem::replace(&mut self.instructions, saved);

                self.instructions.push(Ins::Block {
                    result: None,
                    body: vec![Ins::Loop {
                        result: None,
                        body: inner,
                    }],
                });
            }
            Stmt::SetField {
                object,
                layout: id,
                variant,
                field,
                value,
            } => {
                let tagged = self.program.layout(*id).is_tagged();
                let offset = layout::field_offset(tagged, *field);
                let _ = variant;

                let ty = self.ty_of(value)?;
                let Some(width) = val_type(&ty) else {
                    // A field with no representation holds nothing, so
                    // writing to it writes nothing. The expression still
                    // runs, since it may be a call.
                    self.expr(value)?;
                    return Ok(());
                };

                self.expr(object)?;
                self.instructions.push(Ins::I32WrapI64);
                self.expr(value)?;
                if width == ValType::I32 {
                    self.instructions.push(Ins::I64ExtendI32S);
                }
                self.instructions.push(Ins::I64Store(offset));
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

    /// Reserves `size` bytes of frame stack, leaving the address in `into`.
    ///
    /// Separate from [`Self::allocate`] because this one is given back. The
    /// bound is checked here rather than left to run into the value heap:
    /// frames sit below it, so overflowing quietly would write a handler frame
    /// over somebody's list.
    fn allocate_frame(&mut self, size: u32, into: u32) {
        self.instructions
            .push(Ins::I32Const(layout::FRAME_BUMP as i32));
        self.instructions.push(Ins::I64Load(0));
        self.instructions.push(Ins::LocalSet(into));

        self.instructions.push(Ins::LocalGet(into));
        self.instructions.push(Ins::I64Const(size as i64));
        self.instructions.push(Ins::I64Add);
        self.instructions
            .push(Ins::I64Const(layout::HEAP_START as i64));
        self.instructions.push(Ins::I64GtS);
        self.instructions.push(Ins::If {
            result: None,
            then: vec![Ins::Unreachable],
            otherwise: Vec::new(),
        });

        self.instructions
            .push(Ins::I32Const(layout::FRAME_BUMP as i32));
        self.instructions.push(Ins::LocalGet(into));
        self.instructions.push(Ins::I64Const(size as i64));
        self.instructions.push(Ins::I64Add);
        self.instructions.push(Ins::I64Store(0));
    }

    /// Writes one word at `address + offset`, where what to write is pushed
    /// by instructions rather than lowered from an expression.
    fn write(&mut self, address: u32, offset: u32, value: impl FnOnce(&mut Vec<Ins>)) {
        self.instructions.push(Ins::LocalGet(address));
        self.instructions.push(Ins::I32WrapI64);
        value(&mut self.instructions);
        self.instructions.push(Ins::I64Store(offset));
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
                UnaryOp::Negate => self.checked_negate(operand)?,
            },
            Expr::Binary {
                op,
                left,
                right,
                span,
            } => match op {
                BinaryOp::AddInt
                | BinaryOp::SubInt
                | BinaryOp::MulInt
                | BinaryOp::DivInt
                | BinaryOp::RemInt => self.checked_binary(*op, left, right)?,
                _ => {
                    self.expr(left)?;
                    self.expr(right)?;
                    self.binary(*op, left, *span)?;
                }
            },
            Expr::Call { func, args, span } => {
                for arg in args {
                    self.expr(arg)?;
                }
                self.mark(*span, SpanRole::Call, Ins::Call(func.0 as u32 + self.shift));
            }
            Expr::CallIndirect {
                callee,
                args,
                ret,
                span,
            } => {
                // The environment goes first, then the arguments, then the
                // code pointer the table is indexed by. That order is the
                // lifted body's parameter list, which is why a closure's
                // environment is its first parameter.
                let signature =
                    FuncType {
                        params: std::iter::once(ValType::I64)
                            .chain(args.iter().filter_map(|arg| {
                                self.ty_of(arg).ok().and_then(|ty| val_type(&ty))
                            }))
                            .collect(),
                        results: val_type(ret).into_iter().collect(),
                    };
                let type_index = self.signature(signature)?;

                self.expr(callee)?;
                for arg in args {
                    self.expr(arg)?;
                }
                self.expr(callee)?;
                self.instructions.push(Ins::I32WrapI64);
                self.instructions.push(Ins::I64Load(0));
                self.instructions.push(Ins::I32WrapI64);
                self.mark(*span, SpanRole::Call, Ins::CallIndirect(type_index));
            }
            Expr::Install {
                effect,
                state,
                operations,
                body,
                ..
            } => {
                // The state first, since the frame points at it.
                let held = self.temporary(ValType::I64);
                self.expr(state)?;
                self.instructions.push(Ins::LocalSet(held));

                let frame = self.temporary(ValType::I64);
                self.allocate_frame(layout::frame_size(operations.len()), frame);

                // The frame under this one, so ending the block is a matter
                // of putting back what was there. The frame itself is given
                // back too: its lifetime is exactly this block, and nothing
                // in the program can hold one, so the frame stack rewinds to
                // where it was. Values allocated by the body cannot be given
                // back the same way, because the body's value is one of them.
                self.write(frame, 0, |body| {
                    body.push(Ins::I32Const(layout::HANDLERS as i32));
                    body.push(Ins::I64Load(0));
                });
                self.write(frame, layout::WORD, |body| {
                    body.push(Ins::I64Const(effect.0 as i64));
                });
                self.write(frame, 2 * layout::WORD, |body| {
                    body.push(Ins::LocalGet(held));
                });
                for (position, operation) in operations.iter().enumerate() {
                    self.write(frame, layout::operation_offset(position), |body| {
                        body.push(Ins::I64Const(operation.0 as i64));
                    });
                }

                self.instructions
                    .push(Ins::I32Const(layout::HANDLERS as i32));
                self.instructions.push(Ins::LocalGet(frame));
                self.instructions.push(Ins::I64Store(0));

                self.block(body)?;

                // The body's value is on the stack and stays there: what
                // follows pushes an address and a value of its own and ends
                // balanced.
                self.instructions
                    .push(Ins::I32Const(layout::HANDLERS as i32));
                self.instructions.push(Ins::LocalGet(frame));
                self.instructions.push(Ins::I32WrapI64);
                self.instructions.push(Ins::I64Load(0));
                self.instructions.push(Ins::I64Store(0));

                // And the frame stack goes back to where this block found it.
                self.instructions
                    .push(Ins::I32Const(layout::FRAME_BUMP as i32));
                self.instructions.push(Ins::LocalGet(frame));
                self.instructions.push(Ins::I64Store(0));
            }
            Expr::Perform {
                effect,
                operation,
                args,
                ret,
            } => {
                // Walk down from the innermost frame until one answers for
                // this effect. Which frame that is cannot be worked out
                // here: the function doing the performing is compiled once
                // and may be called from inside any `with` block, or none.
                let cursor = self.temporary(ValType::I64);
                self.instructions
                    .push(Ins::I32Const(layout::HANDLERS as i32));
                self.instructions.push(Ins::I64Load(0));
                self.instructions.push(Ins::LocalSet(cursor));

                let search = vec![
                    // Nothing left to ask. The checker refuses a program
                    // that performs outside a handler, so reaching this
                    // means the effect row and the frames disagree.
                    Ins::LocalGet(cursor),
                    Ins::I64Eqz,
                    Ins::If {
                        result: None,
                        then: vec![Ins::Unreachable],
                        otherwise: Vec::new(),
                    },
                    Ins::LocalGet(cursor),
                    Ins::I32WrapI64,
                    Ins::I64Load(layout::WORD),
                    Ins::I64Const(effect.0 as i64),
                    Ins::I64Eq,
                    Ins::BrIf(1),
                    Ins::LocalGet(cursor),
                    Ins::I32WrapI64,
                    Ins::I64Load(0),
                    Ins::LocalSet(cursor),
                    Ins::Br(0),
                ];
                self.instructions.push(Ins::Block {
                    result: None,
                    body: vec![Ins::Loop {
                        result: None,
                        body: search,
                    }],
                });

                // The state, the arguments, then the code pointer, which is
                // the order an operation's parameters are in.
                let signature =
                    FuncType {
                        params: std::iter::once(ValType::I64)
                            .chain(args.iter().filter_map(|arg| {
                                self.ty_of(arg).ok().and_then(|ty| val_type(&ty))
                            }))
                            .collect(),
                        results: val_type(ret).into_iter().collect(),
                    };
                let type_index = self.signature(signature)?;

                self.instructions.push(Ins::LocalGet(cursor));
                self.instructions.push(Ins::I32WrapI64);
                self.instructions.push(Ins::I64Load(2 * layout::WORD));
                for arg in args {
                    self.expr(arg)?;
                }
                self.instructions.push(Ins::LocalGet(cursor));
                self.instructions.push(Ins::I32WrapI64);
                self.instructions
                    .push(Ins::I64Load(layout::operation_offset(*operation)));
                self.instructions.push(Ins::I32WrapI64);
                self.instructions.push(Ins::CallIndirect(type_index));
            }
            Expr::Host { name, args, .. } => {
                let index = self.import(name)?;
                for arg in args {
                    self.expr(arg)?;
                }
                self.instructions.push(Ins::Call(index));
            }
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
            Expr::ElementAt {
                list,
                index,
                element,
            } => {
                // address = list + (index + 1) * word, since the length sits
                // where the zeroth element would.
                self.expr(list)?;
                self.expr(index)?;
                self.instructions.push(Ins::I64Const(1));
                self.instructions.push(Ins::I64Add);
                self.instructions.push(Ins::I64Const(layout::WORD as i64));
                self.instructions.push(Ins::I64Mul);
                self.instructions.push(Ins::I64Add);
                self.instructions.push(Ins::I32WrapI64);
                self.instructions.push(Ins::I64Load(0));
                if matches!(val_type(element), Some(ValType::I32)) {
                    self.instructions.push(Ins::I32WrapI64);
                }
            }
        }
        Ok(())
    }

    fn binary(&mut self, op: BinaryOp, left: &Expr, span: Span) -> Result<(), Unsupported> {
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
        self.mark(span, SpanRole::Trap, instruction);
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
            Expr::ElementAt { element, .. } => (**element).clone(),
            Expr::Install { ty, .. } => (**ty).clone(),
            Expr::Perform { ret, .. } => (**ret).clone(),
            Expr::Host { ret, .. } => (**ret).clone(),
        })
    }

    fn callee(&self, func: FuncId) -> &Function {
        self.program.function(func)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deed_diagnostics::Span;
    use deed_mir::{Field, Layout, Variant};

    fn nowhere() -> Span {
        Span::new(0, 0)
    }

    fn adding() -> Program {
        let mut program = Program::new();
        let mut function = Function::new("add", vec![Ty::Int, Ty::Int], Ty::Int);
        function.body = Block::of(Expr::Binary {
            op: BinaryOp::AddInt,
            left: Box::new(Expr::Local(Local(0))),
            right: Box::new(Expr::Local(Local(1))),
            span: nowhere(),
        });
        program.add_function(function);
        program
    }

    fn binary(name: &str, op: BinaryOp) -> crate::wasm::Module {
        let mut program = Program::new();
        let mut function = Function::new(name, vec![Ty::Int, Ty::Int], Ty::Int);
        function.body = Block::of(Expr::Binary {
            op,
            left: Box::new(Expr::Local(Local(0))),
            right: Box::new(Expr::Local(Local(1))),
            span: nowhere(),
        });
        program.add_function(function);
        compile(&program).expect("this compiles")
    }

    fn assert_arithmetic_failure(module: &crate::wasm::Module, name: &str, args: [i64; 2]) {
        assert_eq!(
            crate::call(
                module,
                name,
                &[crate::Value::I64(args[0]), crate::Value::I64(args[1])],
            ),
            Err(crate::Trap::Failed {
                code: ARITHMETIC_CODE.to_string(),
                message: ARITHMETIC_MESSAGE.to_string(),
                span: None,
                blame_caller: false,
            })
        );
    }

    #[test]
    fn marked_offsets_follow_nested_wasm_encoding() {
        let program = adding();
        let function = program.function(program.find("add").expect("add is there"));
        let mut strings = Strings::new();
        let mut builder = Builder::new(&program, function, &mut strings, Vec::new(), Vec::new(), 0);
        let spans = [
            Span::new(1, 2),
            Span::new(3, 4),
            Span::new(5, 6),
            Span::new(7, 8),
            Span::new(9, 10),
            Span::new(11, 12),
        ];
        let sites: Vec<u32> = spans
            .iter()
            .map(|span| builder.site(*span, SpanRole::Call))
            .collect();
        let marked = |site, inner| Ins::Marked {
            site,
            inner: Box::new(inner),
        };
        let body = vec![
            marked(sites[0], Ins::I64Const(0)),
            Ins::Block {
                result: None,
                body: vec![
                    marked(sites[1], Ins::I32Const(0)),
                    Ins::Loop {
                        result: None,
                        body: vec![marked(sites[2], Ins::I64Const(1))],
                    },
                ],
            },
            Ins::If {
                result: None,
                then: vec![marked(sites[3], Ins::I32Const(0))],
                otherwise: vec![marked(sites[4], Ins::I64Const(0))],
            },
            marked(sites[5], Ins::I32Const(0)),
        ];

        let mapped = builder.finish_sites(7, &body);
        assert_eq!(mapped.function, 7);
        assert_eq!(
            mapped
                .sites
                .iter()
                .map(|site| site.offset)
                .collect::<Vec<_>>(),
            [0, 4, 8, 14, 17, 20]
        );
        assert_eq!(
            mapped
                .sites
                .iter()
                .map(|site| site.span)
                .collect::<Vec<_>>(),
            spans
        );
    }

    #[test]
    fn the_signed_minimum_cannot_be_negated_by_binary_arithmetic() {
        let multiply = binary("multiply", BinaryOp::MulInt);
        assert_arithmetic_failure(&multiply, "multiply", [-1, i64::MIN]);

        let divide = binary("divide", BinaryOp::DivInt);
        assert_arithmetic_failure(&divide, "divide", [i64::MIN, -1]);

        let remainder = binary("remainder", BinaryOp::RemInt);
        assert_arithmetic_failure(&remainder, "remainder", [i64::MIN, -1]);
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

    /// A capability compiles now, and what it compiles to is a number the
    /// program cannot look inside.
    ///
    /// This used to be the refusal test, since a capability was the one type
    /// with no representation. It is a handle the host gave out, so it has
    /// the same representation as anything else that does not live in the
    /// program's own memory, and what makes it opaque is that the only
    /// things reachable through one are host calls.
    #[test]
    fn a_capability_is_a_handle_rather_than_something_to_look_inside() {
        let mut function = Function::new("reach", vec![Ty::Capability], Ty::Capability);
        function.body = Block::of(Expr::Local(Local(0)));
        let mut program = Program::new();
        program.add_function(function);

        let module = compile(&program).expect("a handle is a number like any other");
        assert_eq!(
            module.types[0],
            FuncType {
                params: vec![ValType::I64],
                results: vec![ValType::I64],
            }
        );
        assert!(!Ty::Capability.is_boxed());
    }

    /// What a program cannot do by itself, it asks for by name.
    #[test]
    fn a_host_call_becomes_an_import_the_module_declares() {
        let mut function = Function::new("greet", vec![Ty::Capability], Ty::Unit);
        function.body = Block::of(Expr::Host {
            name: "io.write".to_string(),
            args: vec![Expr::Local(Local(0)), Expr::Str("hi".to_string())],
            ret: Box::new(Ty::Unit),
        });
        let mut program = Program::new();
        program.add_function(function);

        let module = compile(&program).expect("a host call compiles");
        assert_eq!(module.imports.len(), 1);
        assert_eq!(module.imports[0].module, "deed:io");
        assert_eq!(module.imports[0].name, "write");
        assert_eq!(
            module.types[module.imports[0].type_index as usize],
            FuncType {
                params: vec![ValType::I64, ValType::I64],
                results: Vec::new(),
            }
        );

        // The import is numbered first, so the one function the program
        // declares is callable at 1 and exported there.
        assert_eq!(module.exports, vec![("greet".to_string(), 1)]);
    }

    /// Two calls to one operation are one import.
    #[test]
    fn asking_for_the_same_thing_twice_declares_it_once() {
        let write = |text: &str| Expr::Host {
            name: "io.write".to_string(),
            args: vec![Expr::Local(Local(0)), Expr::Str(text.to_string())],
            ret: Box::new(Ty::Unit),
        };
        let mut function = Function::new("greet", vec![Ty::Capability], Ty::Unit);
        function.body = Block {
            stmts: vec![Stmt::Discard(write("hi")), Stmt::Discard(write("bye"))],
            value: Expr::Unit,
        };
        let mut program = Program::new();
        program.add_function(function);

        let module = compile(&program).expect("this compiles");
        assert_eq!(module.imports.len(), 1);
    }

    /// Two operations that share a name in different namespaces are two
    /// imports, and finding one has to look at both halves.
    #[test]
    fn two_namespaces_can_publish_the_same_operation_name() {
        let host = |name: &str| Expr::Host {
            name: name.to_string(),
            args: vec![Expr::Local(Local(0))],
            ret: Box::new(Ty::Int),
        };
        let mut function = Function::new("both", vec![Ty::Capability], Ty::Int);
        function.body = Block {
            stmts: vec![Stmt::Discard(host("io.now"))],
            value: host("clock.now"),
        };
        let mut program = Program::new();
        program.add_function(function);

        let module = compile(&program).expect("this compiles");
        assert_eq!(module.imports.len(), 2);

        // The second call has to reach the second import. Matching on the
        // operation name alone would send both to whichever was declared
        // first, and both calls would still compile.
        let calls: Vec<u32> = module.funcs[0]
            .body
            .iter()
            .filter_map(|instruction| match instruction {
                Ins::Call(index) => Some(*index),
                _ => None,
            })
            .collect();
        assert_eq!(calls, vec![0, 1], "each call should reach its own import");
    }

    /// Every shape a host call can be buried in.
    ///
    /// The collector walks bodies before any function is placed, because an
    /// import added later would renumber everything. A shape it does not
    /// walk is a program that asks for something the module never declared,
    /// and the program that finds it is an ordinary one: `Io.write` inside
    /// an `if` is not exotic.
    ///
    /// One test rather than one per shape, because what is being checked is
    /// that the walk is total, and a list of shapes with one missing looks
    /// exactly like a list of shapes.
    #[test]
    fn a_host_call_is_found_wherever_it_is_buried() {
        let host = |name: &str| Expr::Host {
            name: format!("io.{name}"),
            args: vec![Expr::Local(Local(0))],
            ret: Box::new(Ty::Int),
        };

        let mut program = Program::new();
        let shape = program.add_layout(Layout {
            name: "Pair".to_string(),
            variants: vec![Variant {
                name: "Pair".to_string(),
                fields: vec![Field {
                    name: "only".to_string(),
                    ty: Ty::Int,
                }],
            }],
        });
        let effect = program.add_effect(deed_mir::Effect {
            name: "Log".to_string(),
            operations: vec!["note".to_string()],
        });

        // Something for the indirect call and the perform to reach. Both
        // want a state or environment first and one argument after, which
        // is the same shape, so one body serves both.
        let mut answering =
            Function::new("answering", vec![Ty::Aggregate(shape), Ty::Int], Ty::Int);
        answering.body = Block::of(Expr::Int(0));
        let note = program.add_function(answering);

        // One name per shape, so a missing arm loses exactly one import and
        // the count says how many were lost.
        let buried = vec![
            Expr::Unary {
                op: UnaryOp::Negate,
                operand: Box::new(host("a")),
            },
            Expr::Binary {
                op: BinaryOp::AddInt,
                left: Box::new(host("b")),
                right: Box::new(host("c")),
                span: nowhere(),
            },
            Expr::List {
                element: Box::new(Ty::Int),
                items: vec![host("d")],
            },
            Expr::CallIndirect {
                callee: Box::new(host("e")),
                args: vec![host("f")],
                ret: Box::new(Ty::Int),
                span: nowhere(),
            },
            Expr::Field {
                value: Box::new(Expr::Make {
                    layout: shape,
                    variant: 0,
                    fields: vec![host("g")],
                }),
                layout: shape,
                variant: 0,
                field: 0,
            },
            Expr::Discriminant {
                value: Box::new(Expr::Make {
                    layout: shape,
                    variant: 0,
                    fields: vec![host("h")],
                }),
                layout: shape,
            },
            Expr::If {
                condition: Box::new(Expr::Bool(true)),
                then: Box::new(Block::of(host("i"))),
                otherwise: Box::new(Block::of(host("j"))),
                ty: Box::new(Ty::Int),
            },
            Expr::Block(Box::new(Block {
                stmts: vec![Stmt::Discard(host("k"))],
                value: host("l"),
            })),
            Expr::ElementAt {
                list: Box::new(Expr::List {
                    element: Box::new(Ty::Int),
                    items: vec![host("m")],
                }),
                index: Box::new(host("n")),
                element: Box::new(Ty::Int),
            },
            Expr::Runtime {
                name: deed_mir::runtime::LIST_LEN,
                args: vec![host("o")],
                ret: Box::new(Ty::Int),
            },
            Expr::Perform {
                effect,
                operation: 0,
                args: vec![host("p")],
                ret: Box::new(Ty::Int),
            },
            Expr::Install {
                effect,
                state: Box::new(Expr::Make {
                    layout: shape,
                    variant: 0,
                    fields: vec![host("q")],
                }),
                operations: vec![note],
                body: Box::new(Block::of(host("r"))),
                ty: Box::new(Ty::Int),
            },
            Expr::Host {
                name: "io.s".to_string(),
                args: vec![host("t")],
                ret: Box::new(Ty::Int),
            },
        ];

        let mut function = Function::new("everywhere", vec![Ty::Capability], Ty::Unit);
        function.body = Block {
            stmts: buried
                .into_iter()
                .map(Stmt::Discard)
                // And the statements that hold expressions of their own.
                .chain([
                    Stmt::Assign {
                        local: Local(0),
                        value: host("u"),
                    },
                    Stmt::While {
                        condition: Expr::Bool(false),
                        body: vec![Stmt::Discard(host("v"))],
                    },
                    Stmt::SetField {
                        object: Expr::Make {
                            layout: shape,
                            variant: 0,
                            fields: vec![host("w")],
                        },
                        layout: shape,
                        variant: 0,
                        field: 0,
                        value: host("x"),
                    },
                ])
                .collect(),
            value: Expr::Unit,
        };
        program.add_function(function);

        let module = compile(&program).expect("this compiles");
        let mut asked: Vec<String> = module
            .imports
            .iter()
            .map(|import| import.name.clone())
            .collect();
        asked.sort();

        // One letter per place a host call was buried, in order, so a shape
        // the walk misses says which one by the letter that went missing.
        let wanted: Vec<String> = (b'a'..=b'x')
            .map(|letter| (letter as char).to_string())
            .collect();
        assert_eq!(
            asked, wanted,
            "a shape the walk misses is a program asking for something the module \
             never declared"
        );
    }

    /// Every literal a host call can be handed, since the import's signature
    /// is built from what the arguments are.
    #[test]
    fn a_host_call_takes_its_arguments_at_the_width_they_are() {
        let mut function = Function::new("hand", vec![], Ty::Unit);
        function.body = Block::of(Expr::Host {
            name: "io.everything".to_string(),
            args: vec![
                Expr::Unit,
                Expr::Bool(true),
                Expr::Int(1),
                Expr::Str("s".to_string()),
            ],
            ret: Box::new(Ty::Unit),
        });
        let mut program = Program::new();
        program.add_function(function);

        let module = compile(&program).expect("this compiles");
        let signature = &module.types[module.imports[0].type_index as usize];
        // Unit has no representation and is not passed. The rest are, and a
        // boolean is narrower than the others.
        assert_eq!(
            signature.params,
            vec![ValType::I32, ValType::I64, ValType::I64]
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
            .filter(|(at, _)| *at != layout::BUMP && *at != layout::FRAME_BUMP)
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
