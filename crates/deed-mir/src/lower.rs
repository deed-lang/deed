//! Lowering the checked tree into [`crate::Program`].
//!
//! The tree this reads has already been resolved, typed and effect-checked,
//! so nothing here decides anything: a name is looked up rather than
//! resolved, a type is read off the table the checker filled rather than
//! inferred, and an operator's meaning comes from the type of its operands
//! rather than from a second set of rules. Anywhere this had to make a
//! judgement, it would be a second checker, and the two of them disagreeing
//! is the failure this arrangement exists to rule out.
//!
//! What is not lowered yet is refused by name. See [`Unlowered`].

use std::collections::HashMap;

use deed_ast::{self as ast, Item, Module};
use deed_diagnostics::Span;
use deed_resolve::{DefId, Resolutions};
use deed_typeck::Tier;
use deed_typeck::Types;
use deed_typeck::ty::Ty as CheckedTy;

use crate::{BinaryOp, Block, Expr, Function, Local, Program, Stmt, Ty, UnaryOp};

/// A shape the lowering does not handle yet, named rather than approximated.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Unlowered {
    pub what: String,
    pub span: Span,
}

impl std::fmt::Display for Unlowered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} is not lowered yet", self.what)
    }
}

fn unlowered(what: &str, span: Span) -> Unlowered {
    Unlowered {
        what: what.to_string(),
        span,
    }
}

#[derive(Clone, Copy)]
enum Nominal<'a> {
    Record(&'a ast::RecordDecl),
    Choice(&'a ast::ChoiceDecl),
}

impl Nominal<'_> {
    fn generics(&self) -> &[ast::Ident] {
        match self {
            Self::Record(record) => &record.generics,
            Self::Choice(choice) => &choice.generics,
        }
    }
}

/// Lowers every function a module declares.
///
/// Takes the whole checked module rather than one function, because a call
/// needs the callee's index and a function may call one declared below it.
pub fn lower(
    module: &Module,
    resolutions: &Resolutions,
    types: &Types,
) -> Result<Program, Unlowered> {
    let mut program = Program::new();

    // Every record and choice first, so a signature naming one can find it.
    // Two passes, and for the same reason as functions below: a record may
    // hold one declared under it.
    let mut layouts: HashMap<String, crate::LayoutId> = HashMap::new();

    // An alias is a name for something else, and a refinement is a claim
    // about a value rather than a shape it has, so both come out as whatever
    // they are written over. Nothing downstream needs to know either existed.
    let mut aliases: HashMap<String, &ast::TypeAlias> = HashMap::new();
    let mut nominals: HashMap<String, Nominal<'_>> = HashMap::new();
    for item in &module.items {
        match item {
            Item::TypeAlias(alias) => {
                aliases.insert(alias.name.name.to_string(), alias);
            }
            Item::Record(record) => {
                nominals.insert(record.name.name.to_string(), Nominal::Record(record));
            }
            Item::Choice(choice) => {
                nominals.insert(choice.name.name.to_string(), Nominal::Choice(choice));
            }
            _ => {}
        }
    }

    for item in &module.items {
        let (name, generics) = match item {
            Item::Record(record) => (record.name.name.to_string(), &record.generics),
            Item::Choice(choice) => (choice.name.name.to_string(), &choice.generics),
            _ => continue,
        };
        if !generics.is_empty() {
            continue;
        }
        let id = program.add_layout(crate::Layout {
            name: name.clone(),
            variants: Vec::new(),
        });
        layouts.insert(name, id);
    }

    // Taken out while the fields are filled in, because a field's type may
    // be a `Result`, and a `Result` is a layout nobody declared that has to
    // be appended as it is met.
    let mut shapes = std::mem::take(&mut program.layouts);

    for item in &module.items {
        match item {
            Item::Record(record) => {
                if !record.generics.is_empty() {
                    continue;
                }
                let id = layouts[record.name.name.as_str()];
                let variants = vec![crate::Variant {
                    name: record.name.name.to_string(),
                    fields: record
                        .fields
                        .iter()
                        .map(|field| {
                            Ok(crate::Field {
                                name: field.name.name.to_string(),
                                ty: written(
                                    &field.ty,
                                    &layouts,
                                    &aliases,
                                    &nominals,
                                    &HashMap::new(),
                                    &mut shapes,
                                )?,
                            })
                        })
                        .collect::<Result<Vec<_>, Unlowered>>()?,
                }];
                shapes[id.0].variants = variants;
            }
            Item::Choice(choice) => {
                if !choice.generics.is_empty() {
                    continue;
                }
                let id = layouts[choice.name.name.as_str()];
                let mut variants = Vec::new();
                for variant in &choice.variants {
                    let mut fields = Vec::new();
                    for field in variant.fields.iter().flatten() {
                        fields.push(crate::Field {
                            name: field.name.name.to_string(),
                            ty: written(
                                &field.ty,
                                &layouts,
                                &aliases,
                                &nominals,
                                &HashMap::new(),
                                &mut shapes,
                            )?,
                        });
                    }
                    variants.push(crate::Variant {
                        name: variant.name.name.to_string(),
                        fields,
                    });
                }
                shapes[id.0].variants = variants;
            }
            _ => {}
        }
    }
    program.layouts = shapes;

    let mut order: Vec<&ast::FnDecl> = Vec::new();
    for item in &module.items {
        if let Item::Function(function) = item {
            order.push(function);
        }
    }

    // Effects, which dispatch reduces to a number and a position. What an
    // operation is called stays for the sake of anybody reading the MIR;
    // nothing looks one up by name after this.
    let mut effects: HashMap<String, crate::EffectId> = HashMap::new();
    let mut signatures: HashMap<String, &ast::EffectDecl> = HashMap::new();
    for item in &module.items {
        if let Item::Effect(effect) = item {
            let id = program.add_effect(crate::Effect {
                name: effect.name.name.to_string(),
                operations: effect
                    .operations
                    .iter()
                    .map(|operation| operation.name.name.to_string())
                    .collect(),
            });
            effects.insert(effect.name.name.to_string(), id);
            signatures.insert(effect.name.name.to_string(), effect);
        }
    }

    let mut handlers: HashMap<String, &ast::HandlerDecl> = HashMap::new();
    for item in &module.items {
        if let Item::Handler(handler) = item {
            handlers.insert(handler.name.name.to_string(), handler);
        }
    }

    // What each alias comes out as, worked out once now that the layouts it
    // may name are all built.
    let mut alias_types: HashMap<String, Ty> = HashMap::new();
    let mut shapes = std::mem::take(&mut program.layouts);
    for (name, alias) in &aliases {
        if !alias.generics.is_empty() {
            continue;
        }
        if let Ok(lowered) = written(
            &alias.ty,
            &layouts,
            &aliases,
            &nominals,
            &HashMap::new(),
            &mut shapes,
        ) {
            alias_types.insert(name.clone(), lowered);
        }
    }

    // Two passes over functions: every signature first, so a body can call
    // anything the module declares regardless of where it sits in the file.
    // One pass would make calling forward an error, which is a rule this
    // language does not have.
    let mut by_def: HashMap<DefId, crate::FuncId> = HashMap::new();
    for declaration in &order {
        // A generic declaration has no one signature until a call says what
        // its parameters stand for, so it is lowered per instantiation
        // rather than once here.
        if !declaration.sig.generics.is_empty() {
            continue;
        }
        let name = declaration.sig.name.name.to_string();
        let mut params = Vec::new();
        for param in &declaration.sig.params {
            let Some(ty) = &param.ty else {
                return Err(unlowered("a parameter with no type", param.span));
            };
            params.push(written(
                ty,
                &layouts,
                &aliases,
                &nominals,
                &HashMap::new(),
                &mut shapes,
            )?);
        }
        let ret = match &declaration.sig.ret {
            None => Ty::Unit,
            Some(ty) => written(
                ty,
                &layouts,
                &aliases,
                &nominals,
                &HashMap::new(),
                &mut shapes,
            )?,
        };
        let id = program.add_function(Function::new(name, params, ret));
        if let Some(def) = resolutions.resolution(declaration.sig.name.span) {
            by_def.insert(def, id);
        }
    }
    program.layouts = shapes;

    let mut lifted: Vec<Function> = Vec::new();
    let mut declarations: HashMap<String, &ast::FnDecl> = HashMap::new();
    for declaration in &order {
        declarations.insert(declaration.sig.name.name.to_string(), declaration);
    }
    let mut instantiated: HashMap<String, crate::FuncId> = HashMap::new();
    let mut answered: HashMap<String, (crate::EffectId, crate::LayoutId, Vec<crate::FuncId>)> =
        HashMap::new();

    // Only the ones that got a signature above, in the same order, since a
    // generic declaration is lowered by the calls that name it rather than
    // once here.
    let concrete: Vec<&&ast::FnDecl> = order
        .iter()
        .filter(|declaration| declaration.sig.generics.is_empty())
        .collect();

    for (index, declaration) in concrete.iter().enumerate() {
        let id = crate::FuncId(index);
        let mut lowering = Lowering {
            resolutions,
            types,
            by_def: &by_def,
            layouts: &layouts,
            alias_types: &alias_types,
            shapes: program.layouts.clone(),
            declared: program.functions.len() + lifted.len(),
            lifted: Vec::new(),
            bindings: HashMap::new(),
            declarations: &declarations,
            instantiated: &mut instantiated,
            effects: &effects,
            signatures: &signatures,
            handlers: &handlers,
            aliases: &aliases,
            nominals: &nominals,
            answered: &mut answered,
            state: None,
            function: program.functions[index].clone(),
            slots: HashMap::new(),
        };

        for (position, param) in declaration.sig.params.iter().enumerate() {
            if let Some(def) = resolutions.resolution(param.name.span) {
                lowering.slots.insert(def, Local(position));
            }
        }

        let body = lowering.block(&declaration.body)?;
        let contract = lowering.contract(declaration)?;
        // A closure body adds a layout for its environment, so what the
        // lowering ended up with is what the program has.
        program.layouts = lowering.shapes;
        lifted.extend(lowering.lifted);
        let mut function = lowering.function;
        function.body = Block {
            stmts: contract.into_iter().chain(body.stmts).collect(),
            value: body.value,
        };
        program.functions[id.0] = function;
    }

    for function in lifted {
        program.add_function(function);
    }

    program.entry = program.find("main");
    Ok(program)
}

/// The layout of a `Result`, built the first time one is named and found
/// after that.
///
/// A choice with two variants, which is a shape the backend already knows.
/// What makes it different from `Tone` or `CounterError` is that nobody
/// writes it down, so there is no declaration to hang it on and it has to be
/// synthesized. One per pair of types, since `Result<Int, String>` and
/// `Result<String, String>` hold different things and a shared layout would
/// read the wrong width out of memory.
///
/// Named for what it holds rather than numbered, so two calls asking for the
/// same one find it and anybody reading the MIR can tell them apart.
fn result_layout(shapes: &mut Vec<crate::Layout>, ok: Ty, err: Ty) -> crate::LayoutId {
    let name = format!("Result<{ok:?}, {err:?}>");
    if let Some(at) = shapes.iter().position(|shape| shape.name == name) {
        return crate::LayoutId(at);
    }

    let variant = |name: &str, ty: Ty| crate::Variant {
        name: name.to_string(),
        fields: vec![crate::Field {
            name: "value".to_string(),
            ty,
        }],
    };
    shapes.push(crate::Layout {
        name,
        variants: vec![variant("ok", ok), variant("err", err)],
    });
    crate::LayoutId(shapes.len() - 1)
}

fn instantiate_nominal(
    name: &str,
    args: &[Ty],
    span: Span,
    layouts: &HashMap<String, crate::LayoutId>,
    aliases: &HashMap<String, &ast::TypeAlias>,
    nominals: &HashMap<String, Nominal<'_>>,
    shapes: &mut Vec<crate::Layout>,
) -> Result<crate::LayoutId, Unlowered> {
    if args.is_empty() {
        if let Some(id) = layouts.get(name) {
            return Ok(*id);
        }
    }

    let nominal = match nominals.get(name) {
        Some(nominal) => nominal,
        None => {
            return Err(unlowered(&format!("the generic type `{name}`"), span));
        }
    };
    if nominal.generics().len() != args.len() {
        return Err(unlowered(&format!("the generic type `{name}`"), span));
    }

    let held = args
        .iter()
        .map(|ty| format!("{ty:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let instantiated = format!("{name}<{held}>");
    if let Some(at) = shapes.iter().position(|shape| shape.name == instantiated) {
        return Ok(crate::LayoutId(at));
    }

    let mut bindings = HashMap::new();
    for (generic, actual) in nominal.generics().iter().zip(args.iter().cloned()) {
        bindings.insert(generic.name.to_string(), actual);
    }

    let id = crate::LayoutId(shapes.len());
    shapes.push(crate::Layout {
        name: instantiated.clone(),
        variants: Vec::new(),
    });

    let variants = match nominal {
        Nominal::Record(record) => vec![crate::Variant {
            name: record.name.name.to_string(),
            fields: record
                .fields
                .iter()
                .map(|field| {
                    Ok(crate::Field {
                        name: field.name.name.to_string(),
                        ty: written(&field.ty, layouts, aliases, nominals, &bindings, shapes)?,
                    })
                })
                .collect::<Result<Vec<_>, Unlowered>>()?,
        }],
        Nominal::Choice(choice) => {
            let mut variants = Vec::new();
            for variant in &choice.variants {
                let mut fields = Vec::new();
                for field in variant.fields.iter().flatten() {
                    fields.push(crate::Field {
                        name: field.name.name.to_string(),
                        ty: written(&field.ty, layouts, aliases, nominals, &bindings, shapes)?,
                    });
                }
                variants.push(crate::Variant {
                    name: variant.name.name.to_string(),
                    fields,
                });
            }
            variants
        }
    };
    shapes[id.0].variants = variants;
    Ok(id)
}

/// The MIR type of a type somebody wrote down.
///
/// A signature in this language is complete, so every type in one is written
/// rather than worked out, and reading it here asks the checker for nothing.
/// It also already passed the checker, so a name that is not one of these is
/// a type this backend has not got to rather than a mistake.
fn written(
    ty: &ast::Type,
    layouts: &HashMap<String, crate::LayoutId>,
    aliases: &HashMap<String, &ast::TypeAlias>,
    nominals: &HashMap<String, Nominal<'_>>,
    bindings: &HashMap<String, Ty>,
    shapes: &mut Vec<crate::Layout>,
) -> Result<Ty, Unlowered> {
    Ok(match ty {
        ast::Type::Unit(_) => Ty::Unit,
        ast::Type::Fn { .. } => Ty::Closure,
        ast::Type::Error(span) => return Err(unlowered("a type that did not parse", *span)),
        ast::Type::Named { name, args, span } => match (name.name.as_str(), args.len()) {
            ("Int", 0) => Ty::Int,
            ("Bool", 0) => Ty::Bool,
            ("String", 0) => Ty::Str,
            ("List", 1) => Ty::List(Box::new(written(
                &args[0], layouts, aliases, nominals, bindings, shapes,
            )?)),
            ("Result", 2) => {
                let ok = written(&args[0], layouts, aliases, nominals, bindings, shapes)?;
                let err = written(&args[1], layouts, aliases, nominals, bindings, shapes)?;
                Ty::Aggregate(result_layout(shapes, ok, err))
            }
            // The four capabilities the language provides. All one type
            // here, because a handle is a handle: what a program may do with
            // one is decided by the effect row and by which host operation
            // it is handed to, and neither of those is a question about its
            // representation. See `design/04-capabilities.md`.
            ("System" | "Console" | "Clock" | "Dir", 0) => Ty::Capability,
            // A type parameter first, since a copy of a generic function was
            // lowered for exactly one thing and that is what it stands for
            // here.
            (other, 0) if bindings.contains_key(other) => bindings[other].clone(),
            (other, _) => match aliases.get(other) {
                Some(alias) => {
                    if alias.generics.len() != args.len() {
                        return Err(unlowered(&format!("the generic type `{other}`"), *span));
                    }
                    let mut alias_bindings = bindings.clone();
                    for (generic, actual) in alias.generics.iter().zip(args) {
                        alias_bindings.insert(
                            generic.name.to_string(),
                            written(actual, layouts, aliases, nominals, bindings, shapes)?,
                        );
                    }
                    written(
                        &alias.ty,
                        layouts,
                        aliases,
                        nominals,
                        &alias_bindings,
                        shapes,
                    )?
                }
                None if args.is_empty() => match layouts.get(other) {
                    Some(id) => Ty::Aggregate(*id),
                    None => return Err(unlowered(&format!("the type `{other}`"), *span)),
                },
                None if nominals.contains_key(other) => {
                    let actuals = args
                        .iter()
                        .map(|arg| written(arg, layouts, aliases, nominals, bindings, shapes))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ty::Aggregate(instantiate_nominal(
                        other, &actuals, *span, layouts, aliases, nominals, shapes,
                    )?)
                }
                None => return Err(unlowered(&format!("the generic type `{other}`"), *span)),
            },
        },
    })
}

fn lower_ty(ty: &CheckedTy, span: Span) -> Result<Ty, Unlowered> {
    Ok(match ty {
        CheckedTy::Unit => Ty::Unit,
        CheckedTy::Bool => Ty::Bool,
        CheckedTy::Int => Ty::Int,
        CheckedTy::Str => Ty::Str,
        CheckedTy::List(element) => Ty::List(Box::new(lower_ty(element, span)?)),
        CheckedTy::Fn { .. } => Ty::Closure,
        // A refinement is a claim about a value that the checker either
        // proved or turned into a runtime check. Either way what is left to
        // compile is the base type, and this layer is where that is said.
        CheckedTy::Never => Ty::Unit,
        // The capabilities the language provides, which the checker files
        // as coming from the prelude. All one handle here, for the reason
        // `written` gives.
        CheckedTy::External { module, name, args }
            if &**module == "<prelude>"
                && args.is_empty()
                && matches!(&**name, "System" | "Console" | "Clock" | "Dir") =>
        {
            Ty::Capability
        }
        other => {
            return Err(unlowered(&format!("a value of type `{other:?}`"), span));
        }
    })
}

struct Lowering<'a> {
    resolutions: &'a Resolutions,
    types: &'a Types,
    by_def: &'a HashMap<DefId, crate::FuncId>,
    layouts: &'a HashMap<String, crate::LayoutId>,
    alias_types: &'a HashMap<String, Ty>,
    /// Every layout the module declares, copied rather than borrowed so a
    /// closure body can be added to the program while one is being lowered.
    shapes: Vec<crate::Layout>,
    /// How many functions the program already has, since a closure body
    /// becomes one and needs to know its own index.
    declared: usize,
    /// Closure bodies, lifted out and appended once everything is lowered.
    lifted: Vec<Function>,
    /// What each type parameter stands for in this copy of the function.
    ///
    /// Empty for a function declaring none, which is most of them. A generic
    /// function is lowered once per set of type arguments it is called with,
    /// and this is what tells the copies apart.
    bindings: HashMap<String, Ty>,
    /// Every function the module declares, by name, so a generic one can be
    /// lowered again when a call needs a copy it does not have.
    declarations: &'a HashMap<String, &'a ast::FnDecl>,
    /// Which copies already exist, by the name they were given.
    instantiated: &'a mut HashMap<String, crate::FuncId>,
    /// Which effect each name stands for.
    effects: &'a HashMap<String, crate::EffectId>,
    /// What each effect declares, since a handler operation writes no types
    /// of its own and inherits them from here.
    signatures: &'a HashMap<String, &'a ast::EffectDecl>,
    handlers: &'a HashMap<String, &'a ast::HandlerDecl>,
    aliases: &'a HashMap<String, &'a ast::TypeAlias>,
    nominals: &'a HashMap<String, Nominal<'a>>,
    /// Which handlers have been lowered, by name: the effect they answer
    /// for, the shape of their state, and a function per operation. A
    /// handler is one set of bodies however many `with` blocks name it, so
    /// this is worked out once and the installations share it.
    answered: &'a mut HashMap<String, (crate::EffectId, crate::LayoutId, Vec<crate::FuncId>)>,
    /// Inside a handler operation: the shape of the state and the slot
    /// holding it. `None` everywhere else, which is what makes writing to
    /// state outside a handler impossible to lower rather than merely
    /// rejected.
    state: Option<(crate::LayoutId, Local)>,
    function: Function,
    /// Where each bound name lives.
    slots: HashMap<DefId, Local>,
}

/// Which variants an arm names, and which field each name it binds reads.
type Arm<'p> = (Vec<usize>, Vec<(usize, &'p ast::Ident)>);

/// Matches a written parameter type against what an argument actually is,
/// recording what each type parameter must stand for.
///
/// Walks both in step and stops where they stop agreeing. There is no
/// unification here and none is needed: the checker already accepted the
/// call, so the shapes line up and this is reading an answer rather than
/// searching for one.
fn bind(written: &ast::Type, actual: &Ty, generics: &[ast::Ident], out: &mut HashMap<String, Ty>) {
    match written {
        ast::Type::Named { name, args, .. } => {
            if args.is_empty()
                && generics.iter().any(|one| one.name == name.name)
                && !out.contains_key(name.name.as_str())
            {
                out.insert(name.name.to_string(), actual.clone());
                return;
            }
            if name.name.as_str() == "List"
                && args.len() == 1
                && let Ty::List(element) = actual
            {
                bind(&args[0], element, generics, out);
            }
        }
        // A row variable is not a type and carries nothing to bind; the
        // effect checker already answered what it stands for.
        ast::Type::Fn { .. } | ast::Type::Unit(_) | ast::Type::Error(_) => {}
    }
}

/// Every name an expression mentions, by where it was written.
///
/// Used to work out what a closure captures. Over-approximating is safe here
/// and under-approximating is not, so this walks everything rather than
/// trying to know which positions can bind.
fn names_in(expr: &ast::Expr, out: &mut Vec<Span>) {
    match expr {
        ast::Expr::Ident(ident) => out.push(ident.span),
        ast::Expr::Field { receiver, .. } => names_in(receiver, out),
        ast::Expr::Call { callee, args, .. } => {
            names_in(callee, out);
            for arg in args {
                names_in(arg, out);
            }
        }
        ast::Expr::List { elements, .. } => {
            for element in elements {
                names_in(element, out);
            }
        }
        ast::Expr::StructLit { fields, .. } => {
            for field in fields {
                out.push(field.name.span);
                if let Some(value) = &field.value {
                    names_in(value, out);
                }
            }
        }
        ast::Expr::Unary { operand, .. } => names_in(operand, out),
        ast::Expr::Binary { lhs, rhs, .. } => {
            names_in(lhs, out);
            names_in(rhs, out);
        }
        ast::Expr::Try { operand, .. } => names_in(operand, out),
        ast::Expr::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            names_in(condition, out);
            names_in_block(then_branch, out);
            if let Some(other) = else_branch {
                names_in(other, out);
            }
        }
        ast::Expr::Match {
            scrutinee, arms, ..
        } => {
            names_in(scrutinee, out);
            for arm in arms {
                names_in(&arm.body, out);
            }
        }
        ast::Expr::For {
            iterable,
            accumulator,
            keep,
            body,
            ..
        } => {
            names_in(iterable, out);
            if let Some(one) = accumulator {
                names_in(&one.init, out);
            }
            if let Some(more) = keep {
                names_in(more, out);
            }
            names_in_block(body, out);
        }
        ast::Expr::Block(block) => names_in_block(block, out),
        ast::Expr::Closure { body, .. } => names_in(body, out),
        _ => {}
    }
}

fn names_in_block(block: &ast::Block, out: &mut Vec<Span>) {
    for stmt in &block.stmts {
        match stmt {
            ast::Stmt::Let { init, .. } => names_in(init, out),
            ast::Stmt::Expr(expr) => names_in(expr, out),
            ast::Stmt::Assert { condition, .. } => names_in(condition, out),
            ast::Stmt::Assign { value, .. } => names_in(value, out),
            ast::Stmt::Return { value, .. } => {
                if let Some(value) = value {
                    names_in(value, out);
                }
            }
            ast::Stmt::Refuses { .. } => {}
        }
    }
    if let Some(tail) = &block.tail {
        names_in(tail, out);
    }
}

impl Lowering<'_> {
    /// The copy of a generic function this call goes to, lowering it if this
    /// is the first call with these type arguments.
    ///
    /// What each parameter stands for is read off the arguments, which the
    /// checker already agreed about, rather than worked out again here: a
    /// parameter written `List<T>` against an argument of `List<Int>` says
    /// `T` is `Int`, and every type parameter appears in a parameter's type
    /// because `DEED4023` refuses one that does not. That rule is what makes
    /// this possible without type arguments ever being written down.
    ///
    /// Named for what it was lowered for, so two copies cannot collide and
    /// the name in a stack trace says which one it is.
    fn instantiate(
        &mut self,
        name: &str,
        generics: &[ast::Ident],
        args: &[ast::Expr],
        span: Span,
    ) -> Result<crate::FuncId, Unlowered> {
        let declaration = *self
            .declarations
            .get(name)
            .ok_or_else(|| unlowered("a call to something not declared here", span))?;

        let mut bindings: HashMap<String, Ty> = HashMap::new();
        for (param, arg) in declaration.sig.params.iter().zip(args) {
            let Some(written) = &param.ty else {
                continue;
            };
            let actual = self.ty_at(arg.span())?;
            bind(written, &actual, generics, &mut bindings);
        }

        let mut spelled: Vec<String> = generics
            .iter()
            .map(|generic| match bindings.get(generic.name.as_str()) {
                Some(ty) => format!("{ty:?}"),
                None => "?".to_string(),
            })
            .collect();
        spelled.sort();
        let copy = format!("{name}<{}>", spelled.join(", "));

        if let Some(found) = self.instantiated.get(&copy) {
            return Ok(*found);
        }

        if bindings.len() != generics.len() {
            return Err(unlowered(
                &format!("a call to `{name}` whose type arguments this cannot work out"),
                span,
            ));
        }

        let params = declaration
            .sig
            .params
            .iter()
            .map(|param| match &param.ty {
                Some(ty) => written(
                    ty,
                    self.layouts,
                    self.aliases,
                    self.nominals,
                    &bindings,
                    &mut self.shapes,
                ),
                None => Err(unlowered("a parameter with no type", param.span)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let ret = match &declaration.sig.ret {
            None => Ty::Unit,
            Some(ty) => written(
                ty,
                self.layouts,
                self.aliases,
                self.nominals,
                &bindings,
                &mut self.shapes,
            )?,
        };

        // Registered before the body is lowered, so a generic function that
        // calls itself with the same arguments finds the copy rather than
        // asking for another one forever.
        let id = crate::FuncId(self.declared + self.lifted.len());
        self.instantiated.insert(copy.clone(), id);
        self.lifted
            .push(Function::new(copy, params.clone(), ret.clone()));

        let outer_slots = std::mem::take(&mut self.slots);
        let outer_bindings = std::mem::replace(&mut self.bindings, bindings);
        let outer_function = std::mem::replace(&mut self.function, Function::new("", params, ret));

        for (position, param) in declaration.sig.params.iter().enumerate() {
            if let Some(def) = self.resolutions.resolution(param.name.span) {
                self.slots.insert(def, Local(position));
            }
        }

        let body = self.block(&declaration.body)?;
        let lowered = std::mem::replace(&mut self.function, outer_function);
        self.slots = outer_slots;
        self.bindings = outer_bindings;

        let at = id.0 - self.declared;
        self.lifted[at].locals = lowered.locals;
        self.lifted[at].body = body;
        Ok(id)
    }

    fn layout(&self, id: crate::LayoutId) -> &crate::Layout {
        &self.shapes[id.0]
    }

    /// The checks a function's contract asks for on the way in.
    ///
    /// A `where` clause is answered where the call is written, and the
    /// checker already recorded how much each call site settled. A callee
    /// every caller proved cannot be reached with something its clause
    /// refuses, so it needs no check at all. One with a caller the checker
    /// could not follow keeps it.
    ///
    /// This is the one place a tier is worth something at runtime rather
    /// than in a report, and it is worth saying plainly: a precondition
    /// every caller proved costs no instructions.
    ///
    /// Per callee rather than per call site, which checks more than strictly
    /// needed and never less. Doing it per call site means translating the
    /// clause into the caller's names, which is a real piece of work the
    /// checker already does once and is not worth doing twice for what it
    /// would save.
    ///
    /// The interpreter checks every one of them, proven or not, because it
    /// has no compile step to spend the proof in. That difference cannot
    /// change an answer: a clause the checker proved is one no run can
    /// break, so the check that is skipped is one that would have passed.
    fn contract(&mut self, declaration: &ast::FnDecl) -> Result<Vec<Stmt>, Unlowered> {
        let name = declaration.sig.name.name.as_str();
        if self.every_caller_proved(name) {
            return Ok(Vec::new());
        }

        let mut checks = Vec::new();
        for clause in &declaration.contract.requires {
            let condition = self.expr(clause)?;
            checks.push(Stmt::Discard(Expr::If {
                condition: Box::new(condition),
                then: Box::new(Block::of(Expr::Unit)),
                otherwise: Box::new(Block {
                    stmts: vec![Stmt::Fail {
                        code: crate::codes::PRECONDITION_FAILED.to_string(),
                        message: format!("this call does not satisfy what `{name}` requires"),
                    }],
                    value: Expr::Unit,
                }),
                ty: Box::new(Ty::Unit),
            }));
        }
        Ok(checks)
    }

    /// Whether every call to this function settled its preconditions.
    ///
    /// A function nothing calls answers yes, which is right: a clause that
    /// no call can break is a clause with nothing to check.
    fn every_caller_proved(&self, callee: &str) -> bool {
        self.types
            .preconditions()
            .iter()
            .filter(|precondition| precondition.callee == callee)
            .all(|precondition| precondition.tier == Tier::Proven)
    }

    /// The MIR type of whatever the checker recorded at this span.
    ///
    /// A refinement is a claim about a value that the checker either proved
    /// or turned into a runtime check, so what is left to compile is the
    /// base type. A named type is a layout this pass already built, found by
    /// the name its definition carries.
    fn ty_at(&mut self, span: Span) -> Result<Ty, Unlowered> {
        let ty = self
            .types
            .type_of(span)
            .ok_or_else(|| unlowered("a value the checker recorded no type for", span))?
            .clone();
        self.convert(&ty, span)
    }

    fn convert(&mut self, ty: &CheckedTy, span: Span) -> Result<Ty, Unlowered> {
        Ok(match ty {
            // A type parameter, standing for whatever this copy of the
            // function was lowered for. The checker carries it by position
            // and by name; the name is what a call bound.
            CheckedTy::Param { name, .. } => match self.bindings.get(name.as_ref()) {
                Some(bound) => bound.clone(),
                None => return Err(unlowered(&format!("the type `{name}`"), span)),
            },
            // An empty list's element type is never known and never read,
            // because there is no element to read it off.
            CheckedTy::List(element) if matches!(**element, CheckedTy::Unknown) => {
                Ty::List(Box::new(Ty::Unit))
            }
            CheckedTy::List(element) => Ty::List(Box::new(self.convert(element, span)?)),
            // A choice with two variants that nobody wrote down. See
            // [`result_layout`].
            //
            // One half may be unsettled, because `ok(4)` says what the
            // value is and nothing about what the error would have been.
            // The other half is in the signature, which in this language is
            // always complete, so that is where it comes from. Guessing
            // instead would be a program building one shape and reading
            // another, which is what a typed IR exists to rule out.
            CheckedTy::Result(ok, err) => {
                let settled = |half: &CheckedTy| !matches!(half, CheckedTy::Unknown);
                if settled(ok) && settled(err) {
                    let ok = self.convert(ok, span)?;
                    let err = self.convert(err, span)?;
                    Ty::Aggregate(result_layout(&mut self.shapes, ok, err))
                } else {
                    match self.function.ret.clone() {
                        Ty::Aggregate(id) if self.layout(id).name.starts_with("Result<") => {
                            Ty::Aggregate(id)
                        }
                        _ => {
                            return Err(unlowered(
                                "a `Result` whose halves this cannot work out",
                                span,
                            ));
                        }
                    }
                }
            }
            CheckedTy::Named { def, args } if args.is_empty() => {
                let name = &self.resolutions.def(*def).name;
                match self.layouts.get(name.as_str()) {
                    Some(id) => Ty::Aggregate(*id),
                    // An alias is a name for something else, and a
                    // refinement is a claim about a value rather than a shape
                    // it has. Both come out as what they are written over.
                    None => match self.alias_types.get(name.as_str()) {
                        Some(ty) => ty.clone(),
                        None => return Err(unlowered(&format!("the type `{name}`"), span)),
                    },
                }
            }
            CheckedTy::Named { def, args } => {
                let name = &self.resolutions.def(*def).name;
                let actuals = args
                    .iter()
                    .map(|arg| self.convert(arg, span))
                    .collect::<Result<Vec<_>, _>>()?;
                Ty::Aggregate(instantiate_nominal(
                    name.as_str(),
                    &actuals,
                    span,
                    self.layouts,
                    self.aliases,
                    self.nominals,
                    &mut self.shapes,
                )?)
            }
            other => lower_ty(other, span)?,
        })
    }

    /// Which `Result` layout an `ok` or an `err` is building.
    fn result_at(&mut self, span: Span) -> Result<crate::LayoutId, Unlowered> {
        match self.ty_at(span)? {
            Ty::Aggregate(id) if self.layout(id).name.starts_with("Result<") => Ok(id),
            _ => Err(unlowered("`ok` or `err` on something else", span)),
        }
    }

    fn block(&mut self, block: &ast::Block) -> Result<Block, Unlowered> {
        let mut stmts = Vec::new();
        for stmt in &block.stmts {
            match stmt {
                ast::Stmt::Let {
                    pattern,
                    init,
                    span,
                    ..
                } => {
                    let value = self.expr(init)?;
                    let ty = self.ty_at(init.span())?;
                    let local = self.function.add_local(ty);
                    match pattern {
                        ast::Pattern::Path { segments, .. } if segments.len() == 1 => {
                            if let Some(def) = self.resolutions.resolution(segments[0].span) {
                                self.slots.insert(def, local);
                            }
                        }
                        ast::Pattern::Wildcard(_) => {}
                        _ => return Err(unlowered("a `let` that takes a value apart", *span)),
                    }
                    stmts.push(Stmt::Assign { local, value });
                }
                ast::Stmt::Expr(expr) => {
                    let value = self.expr(expr)?;
                    stmts.push(Stmt::Discard(value));
                }
                ast::Stmt::Assert { condition, span } => {
                    // An assertion that does not hold ends the run, and that
                    // is the same shape a broken contract has.
                    let checked = self.expr(condition)?;
                    stmts.push(Stmt::Discard(Expr::If {
                        condition: Box::new(checked),
                        then: Box::new(Block::of(Expr::Unit)),
                        otherwise: Box::new(Block {
                            stmts: vec![Stmt::Fail {
                                code: crate::codes::ASSERTION_FAILED.to_string(),
                                message: format!("an assertion at {} did not hold", span.start),
                            }],
                            value: Expr::Unit,
                        }),
                        ty: Box::new(Ty::Unit),
                    }));
                }
                ast::Stmt::Return { value, .. } => {
                    stmts.push(Stmt::Return {
                        value: match value {
                            Some(value) => self.expr(value)?,
                            None => Expr::Unit,
                        },
                    });
                }
                ast::Stmt::Assign {
                    target,
                    value,
                    span,
                } => {
                    // The only assignment the language has, and it writes a
                    // field of the enclosing handler's state. Outside one
                    // there is nothing to write to, and the resolver already
                    // said so.
                    let Some((shape, cell)) = self.state else {
                        return Err(unlowered("an assignment outside a handler", *span));
                    };
                    let field = self.layout(shape).variants[0]
                        .fields
                        .iter()
                        .position(|field| field.name == target.name.as_str())
                        .ok_or_else(|| {
                            unlowered("an assignment to something that is not state", *span)
                        })?;
                    let lowered = self.expr(value)?;
                    stmts.push(Stmt::SetField {
                        object: Expr::Local(cell),
                        layout: shape,
                        variant: 0,
                        field,
                        value: lowered,
                    });
                }
                ast::Stmt::Refuses { span, .. } => {
                    return Err(unlowered("`assert refuses`", *span));
                }
            }
        }

        let value = match &block.tail {
            Some(tail) => self.expr(tail)?,
            None => Expr::Unit,
        };
        Ok(Block { stmts, value })
    }

    fn expr(&mut self, expr: &ast::Expr) -> Result<Expr, Unlowered> {
        Ok(match expr {
            ast::Expr::Int { value, .. } => Expr::Int(*value),
            ast::Expr::Bool { value, .. } => Expr::Bool(*value),
            ast::Expr::Str { value, .. } => Expr::Str(value.clone()),
            ast::Expr::Unit(_) => Expr::Unit,
            ast::Expr::Ident(ident) => {
                let def = self
                    .resolutions
                    .resolution(ident.span)
                    .ok_or_else(|| unlowered("a name nothing resolved", ident.span))?;
                match self.slots.get(&def) {
                    Some(local) => Expr::Local(*local),
                    // Not a local. Inside a handler operation it may be a
                    // field of the state, which is written bare the way a
                    // parameter is and reads out of the cell.
                    None => {
                        if let Some((shape, cell)) = self.state
                            && let Some(field) = self.layout(shape).variants[0]
                                .fields
                                .iter()
                                .position(|field| field.name == ident.name.as_str())
                        {
                            return Ok(Expr::Field {
                                value: Box::new(Expr::Local(cell)),
                                layout: shape,
                                variant: 0,
                                field,
                            });
                        }

                        // So it is a variant carrying no fields, written
                        // bare the way this language writes them. A function
                        // used as a value would also land here and is
                        // refused, since it needs a closure to carry.
                        let (layout, variant) = self.variant_named(&ident.name, ident.span)?;
                        if !self.layout(layout).variants[variant].fields.is_empty() {
                            return Err(unlowered(
                                "a variant with fields written without them",
                                ident.span,
                            ));
                        }
                        Expr::Make {
                            layout,
                            variant,
                            fields: Vec::new(),
                        }
                    }
                }
            }
            ast::Expr::Unary { op, operand, .. } => Expr::Unary {
                op: match op {
                    ast::UnaryOp::Not => UnaryOp::Not,
                    ast::UnaryOp::Neg => UnaryOp::Negate,
                },
                operand: Box::new(self.expr(operand)?),
            },
            ast::Expr::Binary {
                op, lhs, rhs, span, ..
            } => {
                let operand = self.ty_at(lhs.span())?;
                Expr::Binary {
                    op: binary(*op, &operand, *span)?,
                    left: Box::new(self.expr(lhs)?),
                    right: Box::new(self.expr(rhs)?),
                }
            }
            ast::Expr::Call { callee, args, span } => {
                // `Io.write(out, text)`. Nothing a compiled module can do by
                // itself, so it becomes a call to the host. The capability
                // is the first argument and stays there: the row says what
                // kind of thing is happening and the handle says which
                // resource, and neither is enough alone.
                if let ast::Expr::Field { receiver, name, .. } = &**callee
                    && let ast::Expr::Ident(qualifier) = &**receiver
                    && qualifier.name.as_str() == "Io"
                {
                    let ret = self.ty_at(*span)?;
                    let lowered = args
                        .iter()
                        .map(|arg| self.expr(arg))
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(Expr::Host {
                        name: format!("io.{}", name.name),
                        args: lowered,
                        ret: Box::new(ret),
                    });
                }

                // `Log.note("hi")`. Whether the receiver is an effect or a
                // value is a question the resolver already answered, and the
                // effects table is what it comes out as here.
                if let ast::Expr::Field { receiver, name, .. } = &**callee
                    && let ast::Expr::Ident(qualifier) = &**receiver
                    && let Some(effect) = self.effects.get(qualifier.name.as_str()).copied()
                {
                    let operation = self
                        .signatures
                        .get(qualifier.name.as_str())
                        .and_then(|declared| {
                            declared
                                .operations
                                .iter()
                                .position(|sig| sig.name.name == name.name)
                        })
                        .ok_or_else(|| {
                            unlowered("an operation this effect does not declare", name.span)
                        })?;
                    let lowered = args
                        .iter()
                        .map(|arg| self.expr(arg))
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(Expr::Perform {
                        effect,
                        operation,
                        args: lowered,
                        ret: Box::new(self.ty_at(*span)?),
                    });
                }

                let ast::Expr::Ident(name) = &**callee else {
                    return Err(unlowered(
                        "a call through something other than a name",
                        *span,
                    ));
                };
                let def = self
                    .resolutions
                    .resolution(name.span)
                    .ok_or_else(|| unlowered("a call to a name nothing resolved", name.span))?;

                let lowered = args
                    .iter()
                    .map(|arg| self.expr(arg))
                    .collect::<Result<Vec<_>, _>>()?;

                // A local holding a function value is called through the
                // value rather than by name, since which body it points at
                // is not known here.
                if let Some(local) = self.slots.get(&def).copied() {
                    let ret = self.ty_at(*span)?;
                    return Ok(Expr::CallIndirect {
                        callee: Box::new(Expr::Local(local)),
                        args: lowered,
                        ret: Box::new(ret),
                    });
                }

                match self.by_def.get(&def) {
                    Some(func) => Expr::Call {
                        func: *func,
                        args: lowered,
                    },
                    // Not something with a signature of its own. A generic
                    // declaration is one of these and gets a copy per set of
                    // type arguments; everything else is a name the language
                    // provides, and only the ones whose answer is already
                    // sitting in memory are here.
                    None => {
                        if let Some(found) = self.declarations.get(name.name.as_str())
                            && !found.sig.generics.is_empty()
                        {
                            let generics = found.sig.generics.clone();
                            let copy = self.instantiate(&name.name, &generics, args, *span)?;
                            return Ok(Expr::Call {
                                func: copy,
                                args: lowered,
                            });
                        }

                        let subject = match args.first() {
                            Some(first) => self.ty_at(first.span())?,
                            None => Ty::Unit,
                        };

                        // `ok(x)` and `err(x)`, the only way to build a
                        // `Result`. Which variant is which comes from the
                        // layout rather than from a number written here, so
                        // the two places that know the order are one place.
                        if matches!(name.name.as_str(), "ok" | "err") {
                            let layout = self.result_at(*span)?;
                            let variant = self
                                .layout(layout)
                                .variants
                                .iter()
                                .position(|held| held.name == name.name.as_str())
                                .ok_or_else(|| {
                                    unlowered("`ok` or `err` on something else", name.span)
                                })?;
                            return Ok(Expr::Make {
                                layout,
                                variant,
                                fields: lowered,
                            });
                        }

                        let which = match (name.name.as_str(), &subject) {
                            ("length", Ty::Str) => crate::runtime::STR_LEN,
                            ("length", Ty::List(_)) => crate::runtime::LIST_LEN,
                            (other, _) => {
                                return Err(unlowered(&format!("a call to `{other}`"), name.span));
                            }
                        };
                        Expr::Runtime {
                            name: which,
                            args: lowered,
                            ret: Box::new(Ty::Int),
                        }
                    }
                }
            }
            ast::Expr::If {
                condition,
                then_branch,
                else_branch,
                span,
            } => {
                // The checker records a type against each arm rather than
                // against the `if`, so this asks the arm. Both arms agree by
                // the time anything gets here, which is what the checker was
                // for.
                let ty = match &then_branch.tail {
                    Some(tail) => self.ty_at(tail.span())?,
                    None => Ty::Unit,
                };
                let _ = span;
                let otherwise = match else_branch {
                    None => Block::of(Expr::Unit),
                    Some(other) => Block::of(self.expr(other)?),
                };
                Expr::If {
                    condition: Box::new(self.expr(condition)?),
                    then: Box::new(self.block(then_branch)?),
                    otherwise: Box::new(otherwise),
                    ty: Box::new(ty),
                }
            }
            ast::Expr::Block(block) => Expr::Block(Box::new(self.block(block)?)),
            ast::Expr::List { elements, span } => {
                let element = match elements.first() {
                    Some(first) => self.ty_at(first.span())?,
                    // Nothing to look at, and nothing that reads the element
                    // type of an empty list needs it to be right, since there
                    // is no element to read.
                    None => Ty::Unit,
                };
                let _ = span;
                Expr::List {
                    element: Box::new(element),
                    items: elements
                        .iter()
                        .map(|item| self.expr(item))
                        .collect::<Result<Vec<_>, _>>()?,
                }
            }
            ast::Expr::StructLit { path, fields, span } => {
                let ast::Expr::Ident(name) = &**path else {
                    return Err(unlowered("a literal with a dotted name", *span));
                };
                let (layout, variant) = self.variant_named(&name.name, *span)?;
                // Written in whatever order somebody typed, stored in the
                // order the layout declared, because a field's place is a
                // property of the type rather than of one literal.
                let declared: Vec<String> = self.layout(layout).variants[variant]
                    .fields
                    .iter()
                    .map(|field| field.name.clone())
                    .collect();
                let mut values = Vec::new();
                for field in &declared {
                    let written = fields
                        .iter()
                        .find(|given| given.name.name.as_str() == field.as_str())
                        .ok_or_else(|| unlowered("a literal missing a field", *span))?;
                    values.push(match &written.value {
                        Some(value) => self.expr(value)?,
                        // `Pair { left, right }`, the shorthand.
                        None => {
                            let def = self
                                .resolutions
                                .resolution(written.name.span)
                                .ok_or_else(|| unlowered("a shorthand field", written.name.span))?;
                            match self.slots.get(&def) {
                                Some(local) => Expr::Local(*local),
                                None => {
                                    return Err(unlowered(
                                        "a shorthand field naming something that is not a local",
                                        written.name.span,
                                    ));
                                }
                            }
                        }
                    });
                }
                Expr::Make {
                    layout,
                    variant,
                    fields: values,
                }
            }
            ast::Expr::Field {
                receiver,
                name,
                span,
            } => {
                let receiver_ty = self.ty_at(receiver.span())?;

                // `sys.console`. A capability has no fields to read: it is a
                // handle, and narrowing it is something only the host can
                // do. So this is a call rather than a load, which is also
                // what makes it impossible for a compiled program to widen
                // one by reaching into its own memory.
                if receiver_ty == Ty::Capability {
                    let held = self.expr(receiver)?;
                    return Ok(Expr::Host {
                        name: format!("sys.{}", name.name),
                        args: vec![held],
                        ret: Box::new(self.ty_at(*span)?),
                    });
                }

                let Ty::Aggregate(layout) = receiver_ty else {
                    return Err(unlowered("reading a field of this", *span));
                };
                let held = self.layout(layout);
                let (variant, field) = held
                    .variants
                    .iter()
                    .enumerate()
                    .find_map(|(at, one)| {
                        one.fields
                            .iter()
                            .position(|field| field.name == name.name.as_str())
                            .map(|index| (at, index))
                    })
                    .ok_or_else(|| unlowered("a field this type does not declare", *span))?;
                Expr::Field {
                    value: Box::new(self.expr(receiver)?),
                    layout,
                    variant,
                    field,
                }
            }
            ast::Expr::Match {
                scrutinee,
                arms,
                span,
            } => self.match_arms(scrutinee, arms, *span)?,
            ast::Expr::For {
                binder,
                index,
                iterable,
                accumulator,
                keep,
                body,
                span,
            } => self.walk(
                binder,
                index.as_ref(),
                iterable,
                accumulator.as_ref(),
                keep.as_deref(),
                body,
                *span,
            )?,
            ast::Expr::Closure { params, body, span } => self.closure(params, body, *span)?,
            ast::Expr::With {
                handlers,
                body,
                span,
            } => self.install(handlers, body, *span)?,
            other => {
                return Err(unlowered("this expression", other.span()));
            }
        })
    }

    /// `with H { .. }`, or several handlers at once.
    ///
    /// Several nest, and the one written last is the innermost, so a program
    /// naming two handlers of the same effect gets the second. That falls
    /// out of the search `Perform` does rather than being arranged here.
    fn install(
        &mut self,
        handlers: &[ast::Expr],
        body: &ast::Block,
        span: Span,
    ) -> Result<Expr, Unlowered> {
        let mut installations = Vec::new();
        for handler in handlers {
            // `InMemory { count: 0 }` gives the state its first value, and a
            // handler with no state is written bare.
            let (name, initial) = match handler {
                ast::Expr::Ident(ident) => (ident, Vec::new()),
                ast::Expr::StructLit { path, fields, .. } => match &**path {
                    ast::Expr::Ident(ident) => (ident, fields.clone()),
                    other => return Err(unlowered("a handler written this way", other.span())),
                },
                other => return Err(unlowered("a handler written this way", other.span())),
            };

            let (effect, shape, operations) = self.answer(&name.name, name.span)?;

            // In the order the state was declared, not the order it was
            // written, since the layout is what the operations read.
            let declared: Vec<String> = self.layout(shape).variants[0]
                .fields
                .iter()
                .map(|field| field.name.clone())
                .collect();
            let mut start = Vec::new();
            for field in &declared {
                let written = initial
                    .iter()
                    .find(|init| init.name.name.as_str() == field.as_str())
                    .ok_or_else(|| unlowered("an installation missing a state field", span))?;
                let value = match &written.value {
                    Some(value) => self.expr(value)?,
                    None => return Err(unlowered("a shorthand state field", written.span)),
                };
                start.push(value);
            }

            installations.push((effect, shape, operations, start));
        }

        let ty = match &body.tail {
            Some(tail) => self.ty_at(tail.span())?,
            None => Ty::Unit,
        };
        let mut inner = Expr::Block(Box::new(self.block(body)?));

        // Written first means outermost, so they are wrapped from the inside.
        for (effect, shape, operations, start) in installations.into_iter().rev() {
            inner = Expr::Install {
                effect,
                state: Box::new(Expr::Make {
                    layout: shape,
                    variant: 0,
                    fields: start,
                }),
                operations,
                body: Box::new(Block::of(inner)),
                ty: Box::new(ty.clone()),
            };
        }
        Ok(inner)
    }

    /// Lowers a handler's operations, once per handler however many `with`
    /// blocks name it.
    ///
    /// Each operation becomes an ordinary function taking the state cell
    /// first and its own parameters after, which is the same shape a closure
    /// body gets and for the same reason: the code is one thing and what it
    /// works on is another.
    fn answer(
        &mut self,
        name: &str,
        span: Span,
    ) -> Result<(crate::EffectId, crate::LayoutId, Vec<crate::FuncId>), Unlowered> {
        if let Some(found) = self.answered.get(name) {
            return Ok(found.clone());
        }

        let handler = *self
            .handlers
            .get(name)
            .ok_or_else(|| unlowered("a handler not declared here", span))?;
        let effect = *self
            .effects
            .get(handler.effect.name.as_str())
            .ok_or_else(|| unlowered("a handler for an effect not declared here", span))?;
        let declared = *self
            .signatures
            .get(handler.effect.name.as_str())
            .ok_or_else(|| unlowered("a handler for an effect not declared here", span))?;

        let mut held = Vec::new();
        for field in &handler.state {
            let bindings = self.bindings.clone();
            held.push(crate::Field {
                name: field.name.name.to_string(),
                ty: written(
                    &field.ty,
                    self.layouts,
                    self.aliases,
                    self.nominals,
                    &bindings,
                    &mut self.shapes,
                )?,
            });
        }

        // After the fields, since one of them may be a `Result` and that
        // appends a layout of its own.
        let shape = crate::LayoutId(self.shapes.len());
        self.shapes.push(crate::Layout {
            name: format!("state of {name}"),
            variants: vec![crate::Variant {
                name: "state".to_string(),
                fields: held,
            }],
        });

        // Registered before the bodies are lowered, so an operation that
        // performs its own effect finds the functions rather than asking for
        // them again forever.
        let mut operations = Vec::new();
        for (position, _) in declared.operations.iter().enumerate() {
            operations.push(crate::FuncId(self.declared + self.lifted.len() + position));
        }
        self.answered
            .insert(name.to_string(), (effect, shape, operations.clone()));

        // Every operation the effect declares, in that order, so dispatch
        // can index rather than search. A handler that leaves one out did
        // not pass the checker.
        for signature in &declared.operations {
            let body = handler
                .operations
                .iter()
                .find(|operation| operation.sig.name.name == signature.name.name)
                .ok_or_else(|| unlowered("a handler missing an operation", handler.span))?;

            // The types come from the effect. A handler writes its parameter
            // names and nothing else, which is what keeps the two from
            // drifting apart.
            let mut params = vec![Ty::Aggregate(shape)];
            for param in &signature.params {
                params.push(match &param.ty {
                    Some(ty) => written(
                        ty,
                        self.layouts,
                        self.aliases,
                        self.nominals,
                        &self.bindings.clone(),
                        &mut self.shapes,
                    )?,
                    None => return Err(unlowered("an operation with no type", param.span)),
                });
            }
            let ret = match &signature.ret {
                None => Ty::Unit,
                Some(ty) => written(
                    ty,
                    self.layouts,
                    self.aliases,
                    self.nominals,
                    &self.bindings.clone(),
                    &mut self.shapes,
                )?,
            };

            self.lifted.push(Function::new(
                format!("{name}.{}", signature.name.name),
                params.clone(),
                ret.clone(),
            ));
            let at = self.lifted.len() - 1;

            let outer_slots = std::mem::take(&mut self.slots);
            let outer_state = self.state.replace((shape, Local(0)));
            let outer_function =
                std::mem::replace(&mut self.function, Function::new("", params, ret));

            for (position, param) in body.sig.params.iter().enumerate() {
                if let Some(def) = self.resolutions.resolution(param.name.span) {
                    self.slots.insert(def, Local(position + 1));
                }
            }

            let lowered = self.block(&body.body)?;
            let done = std::mem::replace(&mut self.function, outer_function);
            self.slots = outer_slots;
            self.state = outer_state;

            self.lifted[at].locals = done.locals;
            self.lifted[at].body = lowered;
        }

        Ok((effect, shape, operations))
    }

    /// A closure becomes a top-level function and a value that points at it.    ///
    /// The value is a code pointer and an environment, laid out as an
    /// aggregate whose first field is the pointer and whose rest is whatever
    /// the body reads from outside itself. Nothing about it is special to
    /// this backend: it is the conversion every compiler does, written out
    /// because the alternative is a runtime that keeps frames alive and this
    /// language already refuses to let a closure outlive what it captured in
    /// any way that would need one.
    ///
    /// What it captures is worked out from what the body names, so a closure
    /// that reads nothing carries nothing.
    fn closure(
        &mut self,
        params: &[ast::Param],
        body: &ast::Expr,
        span: Span,
    ) -> Result<Expr, Unlowered> {
        let captured: Vec<(DefId, Local)> = {
            let mut found = Vec::new();
            let mut seen = Vec::new();
            names_in(body, &mut seen);
            for name in seen {
                if let Some(def) = self.resolutions.resolution(name)
                    && let Some(local) = self.slots.get(&def)
                    && !found.iter().any(|(known, _)| *known == def)
                {
                    found.push((def, *local));
                }
            }
            found
        };

        // The environment holds one field per captured name, and the code
        // pointer sits in front of them so a call can find it without
        // knowing how many there are.
        let mut fields = vec![crate::Field {
            name: "code".to_string(),
            ty: Ty::Int,
        }];
        for (index, (_, local)) in captured.iter().enumerate() {
            fields.push(crate::Field {
                name: format!("held{index}"),
                ty: self.function.local_ty(*local).clone(),
            });
        }
        let shape = crate::LayoutId(self.shapes.len());
        self.shapes.push(crate::Layout {
            name: format!("closure at {}", span.start),
            variants: vec![crate::Variant {
                name: "closure".to_string(),
                fields,
            }],
        });

        // The lifted body takes its environment first and its own parameters
        // after, which is the order a call has them in.
        let mut lifted_params = vec![Ty::Aggregate(shape)];
        for param in params {
            lifted_params.push(match &param.ty {
                Some(ty) => written(
                    ty,
                    self.layouts,
                    self.aliases,
                    self.nominals,
                    &self.bindings.clone(),
                    &mut self.shapes,
                )?,
                None => self.ty_at(param.span)?,
            });
        }
        let ret = self.ty_at(body.span())?;
        let index = self.declared + self.lifted.len();
        let mut lifted = Function::new(format!("closure{index}"), lifted_params, ret);

        // Inside the body, a captured name reads out of the environment and
        // a parameter is a parameter. Saved and put back, because the
        // enclosing function goes on being lowered afterwards.
        let outer_slots = self.slots.clone();
        let outer_function = std::mem::replace(&mut self.function, lifted);

        for (position, param) in params.iter().enumerate() {
            if let Some(def) = self.resolutions.resolution(param.name.span) {
                self.slots.insert(def, Local(position + 1));
            }
        }
        let mut reads = Vec::new();
        for (at, (def, _)) in captured.iter().enumerate() {
            let ty = self.shapes[shape.0].variants[0].fields[at + 1].ty.clone();
            let local = self.function.add_local(ty);
            self.slots.insert(*def, local);
            reads.push(Stmt::Assign {
                local,
                value: Expr::Field {
                    value: Box::new(Expr::Local(Local(0))),
                    layout: shape,
                    variant: 0,
                    field: at + 1,
                },
            });
        }

        let lowered = self.expr(body)?;
        lifted = std::mem::replace(&mut self.function, outer_function);
        lifted.body = Block {
            stmts: reads,
            value: lowered,
        };
        self.lifted.push(lifted);
        self.slots = outer_slots;

        let mut held = vec![Expr::Int(index as i64)];
        for (_, local) in &captured {
            held.push(Expr::Local(*local));
        }
        Ok(Expr::Make {
            layout: shape,
            variant: 0,
            fields: held,
        })
    }

    /// A `match` becomes a chain of `if`s over the discriminant.
    ///
    /// Nothing is lost by not having a jump table: the checker already
    /// refused a match that does not cover its choice, so the last arm is
    /// reached exactly when every earlier test failed and there is no
    /// fallthrough to invent. A table would be faster on a wide choice and
    /// there is no program yet where that is measurable.
    fn match_arms(
        &mut self,
        scrutinee: &ast::Expr,
        arms: &[ast::MatchArm],
        span: Span,
    ) -> Result<Expr, Unlowered> {
        let subject_ty = self.ty_at(scrutinee.span())?;
        let Ty::Aggregate(layout) = subject_ty.clone() else {
            return Err(unlowered("a match on something other than a choice", span));
        };

        let held = self.function.add_local(subject_ty);
        let value = self.expr(scrutinee)?;

        let ty = match arms.first() {
            Some(first) => self.ty_at(first.body.span())?,
            None => Ty::Unit,
        };

        // Built back to front, so each arm's else is the chain below it.
        let mut chain = Block {
            stmts: vec![Stmt::Fail {
                code: crate::codes::NOT_RUNNABLE.to_string(),
                message: "no arm of this match applied".to_string(),
            }],
            value: Expr::Unit,
        };

        for arm in arms.iter().rev() {
            let (variants, bindings) = self.arm_pattern(&arm.pattern, layout)?;
            let mut stmts = Vec::new();
            for (field, name) in bindings {
                let field_ty = self.layout(layout).variants[variants[0]].fields[field]
                    .ty
                    .clone();
                let local = self.function.add_local(field_ty);
                if let Some(def) = self.resolutions.resolution(name.span) {
                    self.slots.insert(def, local);
                }
                stmts.push(Stmt::Assign {
                    local,
                    value: Expr::Field {
                        value: Box::new(Expr::Local(held)),
                        layout,
                        variant: variants[0],
                        field,
                    },
                });
            }

            let taken = Block {
                stmts,
                value: self.expr(&arm.body)?,
            };

            // No condition at all means this arm always applies, which is
            // what a catch-all is and what the last arm of an exhaustive
            // match on a single-variant layout is.
            let Some(condition) = self.discriminates(&variants, layout, held) else {
                chain = taken;
                continue;
            };

            chain = Block::of(Expr::If {
                condition: Box::new(condition),
                then: Box::new(taken),
                otherwise: Box::new(chain),
                ty: Box::new(ty.clone()),
            });
        }

        Ok(Expr::Block(Box::new(Block {
            stmts: vec![Stmt::Assign { local: held, value }],
            value: Expr::Block(Box::new(chain)),
        })))
    }

    /// Whether the value in `held` is one of these variants.
    ///
    /// `None` when every variant is named, since a test that is always true
    /// is a branch nothing needs.
    fn discriminates(
        &self,
        variants: &[usize],
        layout: crate::LayoutId,
        held: Local,
    ) -> Option<Expr> {
        if variants.len() == self.layout(layout).variants.len() {
            return None;
        }
        let mut condition: Option<Expr> = None;
        for variant in variants {
            let test = Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(Expr::Discriminant {
                    value: Box::new(Expr::Local(held)),
                    layout,
                }),
                right: Box::new(Expr::Int(*variant as i64)),
            };
            condition = Some(match condition {
                None => test,
                Some(so_far) => Expr::Binary {
                    op: BinaryOp::Or,
                    left: Box::new(so_far),
                    right: Box::new(test),
                },
            });
        }
        condition
    }

    /// Which variants an arm's pattern names, and what it binds.
    fn arm_pattern<'p>(
        &self,
        pattern: &'p ast::Pattern,
        layout: crate::LayoutId,
    ) -> Result<Arm<'p>, Unlowered> {
        let held = self.layout(layout);
        Ok(match pattern {
            ast::Pattern::Wildcard(_) => ((0..held.variants.len()).collect(), Vec::new()),
            ast::Pattern::Path { segments, span } => {
                let name = segments
                    .last()
                    .ok_or_else(|| unlowered("an empty pattern", *span))?;
                let at = held
                    .variants
                    .iter()
                    .position(|variant| variant.name == name.name.as_str())
                    .ok_or_else(|| unlowered("a pattern naming no variant", *span))?;
                (vec![at], Vec::new())
            }
            ast::Pattern::Record { path, fields, span } => {
                let name = path
                    .last()
                    .ok_or_else(|| unlowered("an empty pattern", *span))?;
                let at = held
                    .variants
                    .iter()
                    .position(|variant| variant.name == name.name.as_str())
                    .ok_or_else(|| unlowered("a pattern naming no variant", *span))?;
                let mut bindings = Vec::new();
                for field in fields {
                    let index = held.variants[at]
                        .fields
                        .iter()
                        .position(|declared| declared.name == field.name.name.as_str())
                        .ok_or_else(|| unlowered("a field the variant does not have", *span))?;
                    bindings.push((index, &field.name));
                }
                (vec![at], bindings)
            }
            // `ok(text)` and `err(why)`. The parser already refuses anything
            // but a plain binder inside one, so there is exactly one thing
            // being named and it is the variant's only field.
            ast::Pattern::Tuple {
                path,
                elements,
                span,
            } => {
                let name = path
                    .last()
                    .ok_or_else(|| unlowered("an empty pattern", *span))?;
                let at = held
                    .variants
                    .iter()
                    .position(|variant| variant.name == name.name.as_str())
                    .ok_or_else(|| unlowered("a pattern naming no variant", *span))?;

                let mut bindings = Vec::new();
                for (position, element) in elements.iter().enumerate() {
                    match element {
                        ast::Pattern::Wildcard(_) => {}
                        ast::Pattern::Path { segments, .. } if segments.len() == 1 => {
                            bindings.push((position, &segments[0]));
                        }
                        other => {
                            return Err(unlowered("a pattern inside a pattern", other.span()));
                        }
                    }
                }
                (vec![at], bindings)
            }
            ast::Pattern::OneOf {
                alternatives,
                span: _,
            } => {
                let mut variants = Vec::new();
                for alternative in alternatives {
                    let (named, _) = self.arm_pattern(alternative, layout)?;
                    variants.extend(named);
                }
                (variants, Vec::new())
            }
            other => return Err(unlowered("this pattern", other.span())),
        })
    }

    /// A `for` walk: a counter, the list's own length as the bound, and a
    /// body that rebinds the accumulator.
    ///
    /// The index is generated here rather than read from the program, so
    /// every element read is inside the list by construction and none of
    /// them needs a bound check. That is the one place this backend indexes
    /// without asking, and it is why [`Expr::ElementAt`] exists separately
    /// from the prelude's `at`.
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        binder: &ast::Ident,
        index: Option<&ast::Ident>,
        iterable: &ast::Expr,
        accumulator: Option<&ast::Accumulator>,
        keep: Option<&ast::Expr>,
        body: &ast::Block,
        span: Span,
    ) -> Result<Expr, Unlowered> {
        let list_ty = self.ty_at(iterable.span())?;
        let Ty::List(element_ty) = list_ty.clone() else {
            return Err(unlowered("a walk over something that is not a list", span));
        };

        let list = self.function.add_local(list_ty);
        let counter = self.function.add_local(Ty::Int);
        let element = self.function.add_local((*element_ty).clone());

        let (carried, start) = match accumulator {
            Some(one) => {
                let ty = self.ty_at(one.init.span())?;
                let value = self.expr(&one.init)?;
                (self.function.add_local(ty), value)
            }
            None => (self.function.add_local(Ty::Unit), Expr::Unit),
        };

        if let Some(one) = accumulator
            && let Some(def) = self.resolutions.resolution(one.name.span)
        {
            self.slots.insert(def, carried);
        }
        if let Some(def) = self.resolutions.resolution(binder.span) {
            self.slots.insert(def, element);
        }
        if let Some(at) = index
            && let Some(def) = self.resolutions.resolution(at.span)
        {
            self.slots.insert(def, counter);
        }

        let walked = self.expr(iterable)?;

        let mut condition = Expr::Binary {
            op: BinaryOp::LtInt,
            left: Box::new(Expr::Local(counter)),
            right: Box::new(Expr::Runtime {
                name: crate::runtime::LIST_LEN,
                args: vec![Expr::Local(list)],
                ret: Box::new(Ty::Int),
            }),
        };
        if let Some(more) = keep {
            // Read before each turn, with the accumulator in scope, which is
            // where the language already says it is read.
            condition = Expr::Binary {
                op: BinaryOp::And,
                left: Box::new(condition),
                right: Box::new(self.expr(more)?),
            };
        }

        let turn = self.block(body)?;
        let inner = vec![
            Stmt::Assign {
                local: element,
                value: Expr::ElementAt {
                    list: Box::new(Expr::Local(list)),
                    index: Box::new(Expr::Local(counter)),
                    element: element_ty,
                },
            },
            Stmt::Assign {
                local: carried,
                value: Expr::Block(Box::new(turn)),
            },
            Stmt::Assign {
                local: counter,
                value: Expr::Binary {
                    op: BinaryOp::AddInt,
                    left: Box::new(Expr::Local(counter)),
                    right: Box::new(Expr::Int(1)),
                },
            },
        ];

        Ok(Expr::Block(Box::new(Block {
            stmts: vec![
                Stmt::Assign {
                    local: list,
                    value: walked,
                },
                Stmt::Assign {
                    local: counter,
                    value: Expr::Int(0),
                },
                Stmt::Assign {
                    local: carried,
                    value: start,
                },
                Stmt::While {
                    condition,
                    body: inner,
                },
            ],
            value: Expr::Local(carried),
        })))
    }

    /// Which layout and which variant a name refers to.
    ///
    /// A record's only variant carries the record's own name, so one lookup
    /// answers both shapes.
    fn variant_named(&self, name: &str, span: Span) -> Result<(crate::LayoutId, usize), Unlowered> {
        if let Some(id) = self.layouts.get(name) {
            return Ok((*id, 0));
        }
        for (index, layout) in self.shapes.iter().enumerate() {
            if let Some(at) = layout
                .variants
                .iter()
                .position(|variant| variant.name == name)
            {
                return Ok((crate::LayoutId(index), at));
            }
        }
        Err(unlowered(&format!("the name `{name}`"), span))
    }
}

/// Which of the split operators an operator means here.
///
/// The surface language has five operators that mean more than one thing,
/// and the operand type is what tells them apart. That question is answered
/// once, here, so nothing downstream has to ask it again.
fn binary(op: ast::BinaryOp, operand: &Ty, span: Span) -> Result<BinaryOp, Unlowered> {
    Ok(match op {
        ast::BinaryOp::Add => match operand {
            Ty::Int => BinaryOp::AddInt,
            Ty::Str => BinaryOp::ConcatStr,
            _ => return Err(unlowered("`+` on this type", span)),
        },
        ast::BinaryOp::Sub => BinaryOp::SubInt,
        ast::BinaryOp::Mul => BinaryOp::MulInt,
        ast::BinaryOp::Div => BinaryOp::DivInt,
        ast::BinaryOp::Rem => BinaryOp::RemInt,
        ast::BinaryOp::Eq => BinaryOp::Eq,
        ast::BinaryOp::Ne => BinaryOp::Ne,
        ast::BinaryOp::And => BinaryOp::And,
        ast::BinaryOp::Or => BinaryOp::Or,
        ast::BinaryOp::Lt => order(operand, span, BinaryOp::LtInt, BinaryOp::LtStr)?,
        ast::BinaryOp::Le => order(operand, span, BinaryOp::LeInt, BinaryOp::LeStr)?,
        ast::BinaryOp::Gt => order(operand, span, BinaryOp::GtInt, BinaryOp::GtStr)?,
        ast::BinaryOp::Ge => order(operand, span, BinaryOp::GeInt, BinaryOp::GeStr)?,
    })
}

fn order(
    operand: &Ty,
    span: Span,
    on_int: BinaryOp,
    on_str: BinaryOp,
) -> Result<BinaryOp, Unlowered> {
    match operand {
        Ty::Int => Ok(on_int),
        Ty::Str => Ok(on_str),
        _ => Err(unlowered("an ordering on this type", span)),
    }
}
