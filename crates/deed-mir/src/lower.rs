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

/// One checked module, for a caller that has more than one.
///
/// A call into another module needs that module's syntax to lower a body
/// from, and its own resolutions and types to read it with. A `DefId` is an
/// index into one module's table and a `Span` is an offset into one file, so
/// neither means anything on the other side and the three travel together.
pub struct Alongside<'a> {
    pub module: &'a Module,
    pub resolutions: &'a Resolutions,
    pub types: &'a Types,
}

/// Lowers every function a module declares.
///
/// Takes the whole checked module rather than one function, because a call
/// needs the callee's index and a function may call one declared below it.
///
/// Test blocks are not lowered. See [`lower_with_tests`] for a version that
/// includes them.
pub fn lower(
    module: &Module,
    resolutions: &Resolutions,
    types: &Types,
) -> Result<Program, Unlowered> {
    lower_impl(module, resolutions, types, &[], false)
}

/// Lowers a module that calls into others, with those others alongside.
///
/// Only what is reached is lowered. A module that ships thirty functions and
/// is imported for one contributes one, which matters because the rest of it
/// may use shapes this backend cannot compile and refusing the whole program
/// over a function nobody called would be wrong.
pub fn lower_alongside(
    module: &Module,
    resolutions: &Resolutions,
    types: &Types,
    alongside: &[Alongside<'_>],
) -> Result<Program, Unlowered> {
    lower_impl(module, resolutions, types, alongside, false)
}

/// Lowers every function a module declares, and also lowers test blocks.
///
/// Test blocks that cannot be lowered are silently skipped; the backend
/// compiles a subset of the language on purpose. The blocks that do lower
/// are in [`Program::tests`].
///
/// See [`lower`] for the version without test blocks.
pub fn lower_with_tests(
    module: &Module,
    resolutions: &Resolutions,
    types: &Types,
) -> Result<Program, Unlowered> {
    lower_impl(module, resolutions, types, &[], true)
}

/// The same, with the modules this one calls into alongside.
pub fn lower_with_tests_alongside(
    module: &Module,
    resolutions: &Resolutions,
    types: &Types,
    alongside: &[Alongside<'_>],
) -> Result<Program, Unlowered> {
    lower_impl(module, resolutions, types, alongside, true)
}

/// Everything about one module that lowering a body from it needs.
///
/// Built for every module the caller handed over, before any body is
/// lowered, because a call into another module has to be able to read that
/// module's syntax with that module's tables and no others.
struct Unit<'a> {
    /// What the module calls itself, which is what a `use` on the other side
    /// names it by.
    path: String,
    module: &'a Module,
    resolutions: &'a Resolutions,
    types: &'a Types,
    layouts: HashMap<String, crate::LayoutId>,
    alias_types: HashMap<String, Ty>,
    aliases: HashMap<String, &'a ast::TypeAlias>,
    nominals: HashMap<String, Nominal<'a>>,
    /// Every function the module declares, in the order it declares them.
    order: Vec<&'a ast::FnDecl>,
    declarations: HashMap<String, &'a ast::FnDecl>,
    effects: HashMap<String, crate::EffectId>,
    signatures: HashMap<String, &'a ast::EffectDecl>,
    handlers: HashMap<String, &'a ast::HandlerDecl>,
    /// Which unit declares each handler this one can install.
    ///
    /// A handler's operations are code, and code is read against the tables
    /// of the module it was written in. Borrowing the declaration is enough to
    /// install one and not enough to lower it: every name in an operation body
    /// belongs to the other module's resolutions, so lowering it here found
    /// nothing at all.
    handler_from: HashMap<String, usize>,
    guards: HashMap<Span, Guard>,
    refinements: HashMap<DefId, Refinement<'a>>,
}

/// Reads one module into the tables lowering a body from it needs.
///
/// Adds the layouts and effects it declares to `program`, so a layout id
/// means the same thing whichever module it came from.
fn tables<'a>(
    module: &'a Module,
    resolutions: &'a Resolutions,
    types: &'a Types,
    program: &mut Program,
) -> Result<Unit<'a>, Unlowered> {
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
    let mut declarations: HashMap<String, &ast::FnDecl> = HashMap::new();
    for item in &module.items {
        if let Item::Function(function) = item {
            order.push(function);
            declarations.insert(function.sig.name.name.to_string(), function);
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
    program.layouts = shapes;

    // What each refinement says, and what it says it about. `value` is the
    // only name the language introduces on its own, and resolution gives it a
    // definition whose span is the alias name, which is what makes it findable
    // from here without matching on the word.
    let mut refinements: HashMap<DefId, Refinement<'_>> = HashMap::new();
    for alias in aliases.values() {
        let (Some(def), Some(predicate)) = (
            resolutions.resolution(alias.name.span),
            alias.refinement.as_ref(),
        ) else {
            continue;
        };
        let subject = resolutions
            .defs()
            .find(|(_, data)| {
                data.kind == deed_resolve::DefKind::Local
                    && data.name == "value"
                    && data.span == alias.name.span
            })
            .map(|(id, _)| id);
        refinements.insert(
            def,
            Refinement {
                predicate,
                subject,
                name: alias.name.name.to_string(),
            },
        );
    }

    // Everything the checker could not settle, which is what the compiled
    // program has to check for itself.
    let guards: HashMap<Span, Guard> = types
        .obligations()
        .iter()
        .filter(|obligation| obligation.tier == deed_typeck::Tier::Guarded)
        .map(|obligation| {
            (
                obligation.span,
                Guard {
                    refinement: obligation.refinement,
                    inside_ok: obligation.inside_ok,
                },
            )
        })
        .collect();

    Ok(Unit {
        path: module
            .name
            .as_ref()
            .map(ast::ModulePath::to_string_path)
            .unwrap_or_default(),
        module,
        resolutions,
        types,
        layouts,
        alias_types,
        aliases,
        nominals,
        order,
        declarations,
        effects,
        signatures,
        handlers,
        handler_from: HashMap::new(),
        guards,
        refinements,
    })
}

fn lower_impl(
    module: &Module,
    resolutions: &Resolutions,
    types: &Types,
    alongside: &[Alongside<'_>],
    include_tests: bool,
) -> Result<Program, Unlowered> {
    let mut program = Program::new();

    let mut units = vec![tables(module, resolutions, types, &mut program)?];
    for extra in alongside {
        // A module that cannot be read at all is one nothing here can call
        // into, which is a refusal at the call rather than at the file: a
        // program that imports something for one function should not be
        // turned away over the rest of it.
        if let Ok(unit) = tables(extra.module, extra.resolutions, extra.types, &mut program) {
            units.push(unit);
        }
    }

    // A type crosses a boundary the same way a function does. What it comes
    // out as is what the module that declared it built, so each module's
    // tables gain the names it borrowed rather than a copy of the shape,
    // which would give one record two layouts and make a value of it fit
    // neither.
    //
    // By name and only for what a `use` line asked for, so two modules that
    // both declare a `Point` keep their own.
    let mut pending: Vec<(usize, usize, String)> = units
        .iter()
        .enumerate()
        .flat_map(|(at, unit)| {
            unit.module.uses.iter().flat_map(move |used| {
                let path = used.path.to_string_path();
                used.names
                    .iter()
                    .map(move |name| (at, path.clone(), name.name.to_string()))
            })
        })
        .filter_map(|(at, path, name)| {
            let from = units.iter().position(|unit| unit.path == path)?;
            (from != at).then_some((at, from, name))
        })
        .collect();

    let mut taken: Vec<(usize, usize, String)> = Vec::new();
    while let Some(one) = pending.pop() {
        if taken.contains(&one) {
            continue;
        }
        taken.push(one.clone());
        let (at, from, name) = one;

        if let Some(id) = units[from].layouts.get(&name).copied() {
            units[at].layouts.insert(name.clone(), id);
        }
        if let Some(nominal) = units[from].nominals.get(&name).copied() {
            units[at].nominals.insert(name.clone(), nominal);
            // What a record's fields or a choice's variants are written over
            // may be another name that module declares and this one never
            // asked for. `use std/table.{Table}` is enough to need `Entry`,
            // which nobody writes down anywhere on this side.
            let mut named = Vec::new();
            match nominal {
                Nominal::Record(record) => {
                    for field in &record.fields {
                        names_in_type(&field.ty, &mut named);
                    }
                }
                Nominal::Choice(choice) => {
                    for variant in &choice.variants {
                        for field in variant.fields.iter().flatten() {
                            names_in_type(&field.ty, &mut named);
                        }
                    }
                }
            }
            for name in named {
                pending.push((at, from, name));
            }
        }
        if let Some(alias) = units[from].aliases.get(&name).copied() {
            units[at].aliases.insert(name.clone(), alias);
            let mut named = Vec::new();
            names_in_type(&alias.ty, &mut named);
            for name in named {
                pending.push((at, from, name));
            }
        }
        if let Some(ty) = units[from].alias_types.get(&name).cloned() {
            units[at].alias_types.insert(name.clone(), ty);
        }
        if let Some(id) = units[from].effects.get(&name).copied() {
            units[at].effects.insert(name.clone(), id);
        }
        if let Some(effect) = units[from].signatures.get(&name).copied() {
            units[at].signatures.insert(name.clone(), effect);
        }
        if let Some(handler) = units[from].handlers.get(&name).copied() {
            units[at].handlers.insert(name.clone(), handler);
            let owner = units[from].handler_from.get(&name).copied().unwrap_or(from);
            units[at].handler_from.insert(name.clone(), owner);
        }
        // A function's signature names types, and a `use` that asked for the
        // function had no reason to ask for them as well. `use
        // std/table.{set}` is the whole of what a program writes, and `set`
        // hands back a `Table<K, V>` over an `Entry<K, V>`, neither of which
        // appears anywhere on this side.
        if let Some(declaration) = units[from].declarations.get(&name).copied() {
            let mut named = Vec::new();
            for param in &declaration.sig.params {
                if let Some(ty) = &param.ty {
                    names_in_type(ty, &mut named);
                }
            }
            if let Some(ret) = &declaration.sig.ret {
                names_in_type(ret, &mut named);
            }
            for name in named {
                pending.push((at, from, name));
            }
        }
    }

    let mut shapes = std::mem::take(&mut program.layouts);

    // Two passes over functions: every signature first, so a body can call
    // anything the module declares regardless of where it sits in the file.
    // One pass would make calling forward an error, which is a rule this
    // language does not have.
    let mut by_def: HashMap<DefId, crate::FuncId> = HashMap::new();
    for declaration in &units[0].order {
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
                &units[0].layouts,
                &units[0].aliases,
                &units[0].nominals,
                &HashMap::new(),
                &mut shapes,
            )?);
        }
        let ret = match &declaration.sig.ret {
            None => Ty::Unit,
            Some(ty) => written(
                ty,
                &units[0].layouts,
                &units[0].aliases,
                &units[0].nominals,
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
    let mut instantiated: HashMap<String, crate::FuncId> = HashMap::new();
    let mut answered: HashMap<String, (crate::EffectId, crate::LayoutId, Vec<crate::FuncId>)> =
        HashMap::new();

    // Only the ones that got a signature above, in the same order, since a
    // generic declaration is lowered by the calls that name it rather than
    // once here.
    let concrete: Vec<&&ast::FnDecl> = units[0]
        .order
        .iter()
        .filter(|declaration| declaration.sig.generics.is_empty())
        .collect();

    for (index, declaration) in concrete.iter().enumerate() {
        let id = crate::FuncId(index);
        let mut lowering = Lowering::at(
            &units,
            0,
            &by_def,
            program.layouts.clone(),
            program.functions.len() + lifted.len(),
            &mut instantiated,
            &mut answered,
            program.functions[index].clone(),
        );

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

    // Lower test blocks into the program. Only when the caller asked for
    // them: test bodies may use features the backend cannot compile, and
    // adding them to a program that `deed build` will compile would break
    // files that used to compile cleanly.
    //
    // A test block that cannot be lowered is silently skipped: the backend
    // compiles a subset of the language on purpose. Each block that does lower
    // becomes a body function and one probe function per `assert refuses`.
    // See `crate::TestBlock` and `deed test --compiled`.
    if include_tests {
        let mut test_index: usize = 0;
        for item in &module.items {
            let ast::Item::Test(test) = item else {
                continue;
            };

            // Save enough state to roll back any partial work if the block fails.
            let prev_functions = program.functions.len();
            let saved_instantiated = instantiated.clone();
            let saved_answered = answered.clone();

            let outcome: Option<crate::TestBlock> = 'block: {
                let mut refuses_names: Vec<String> = Vec::new();

                // Lower one probe function per `assert refuses` statement.
                for (stmt_index, stmt) in test.body.stmts.iter().enumerate() {
                    let ast::Stmt::Refuses { subject, .. } = stmt else {
                        continue;
                    };
                    let probe_name = format!("__test_refuses_{test_index}_{stmt_index}__");
                    let probe_id = program.add_function(crate::Function::new(
                        probe_name.clone(),
                        vec![],
                        Ty::Unit,
                    ));
                    let mut lowering = Lowering::at(
                        &units,
                        0,
                        &by_def,
                        program.layouts.clone(),
                        program.functions.len(),
                        &mut instantiated,
                        &mut answered,
                        program.functions[probe_id.0].clone(),
                    );
                    let value = match lowering.expr(subject) {
                        Ok(v) => v,
                        Err(_) => break 'block None,
                    };
                    program.layouts = lowering.shapes;
                    for f in lowering.lifted {
                        program.add_function(f);
                    }
                    let mut func = lowering.function;
                    func.body = Block {
                        stmts: vec![Stmt::Discard(value)],
                        value: Expr::Unit,
                    };
                    program.functions[probe_id.0] = func;
                    refuses_names.push(probe_name);
                }

                // Lower the body function, skipping `assert refuses` statements.
                let body_name = format!("__test_body_{test_index}__");
                let body_id =
                    program.add_function(crate::Function::new(body_name.clone(), vec![], Ty::Unit));
                let filtered = ast::Block {
                    stmts: test
                        .body
                        .stmts
                        .iter()
                        .filter(|s| !matches!(s, ast::Stmt::Refuses { .. }))
                        .cloned()
                        .collect(),
                    tail: test.body.tail.clone(),
                    span: test.body.span,
                };
                let mut lowering = Lowering::at(
                    &units,
                    0,
                    &by_def,
                    program.layouts.clone(),
                    program.functions.len(),
                    &mut instantiated,
                    &mut answered,
                    program.functions[body_id.0].clone(),
                );
                let body = match lowering.block(&filtered) {
                    Ok(b) => b,
                    Err(_) => break 'block None,
                };
                program.layouts = lowering.shapes;
                for f in lowering.lifted {
                    program.add_function(f);
                }
                let mut func = lowering.function;
                func.body = body;
                program.functions[body_id.0] = func;

                Some(crate::TestBlock {
                    name: test.name.clone(),
                    body: body_name,
                    refuses: refuses_names,
                })
            };

            match outcome {
                Some(test_block) => program.tests.push(test_block),
                None => {
                    program.functions.truncate(prev_functions);
                    instantiated = saved_instantiated;
                    answered = saved_answered;
                }
            }

            test_index += 1;
        }
    } // if include_tests

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
            // The five capabilities the language provides. All one type
            // here, because a handle is a handle: what a program may do with
            // one is decided by the effect row and by which host operation
            // it is handed to, and neither of those is a question about its
            // representation. See `design/04-capabilities.md`.
            ("System" | "Console" | "Clock" | "Dir" | "Net", 0) => Ty::Capability,
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
                && matches!(&**name, "System" | "Console" | "Clock" | "Dir" | "Net") =>
        {
            Ty::Capability
        }
        other => {
            return Err(unlowered(&format!("a value of type `{other:?}`"), span));
        }
    })
}

struct Lowering<'a> {
    /// Every module the caller handed over, subject first.
    units: &'a [Unit<'a>],
    /// Which of them the body being lowered is written in.
    ///
    /// The fields below are that unit's, swapped in and out around a body
    /// from another module rather than looked up on every read: a `Span` is
    /// an offset into one file and a `DefId` is an index into one table, so
    /// reading a body with the wrong module's tables would answer rather than
    /// fail.
    at: usize,
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
    /// The shape the expression being lowered has to come out as, when
    /// something around it says so. Only an equality does today, and only a
    /// `Result` with a half nobody wrote down reads it.
    expected: Option<Ty>,
    function: Function,
    /// The accumulator of the walk being lowered, when that walk builds one
    /// list rather than one a turn.
    ///
    /// A slot rather than a name, so a `push` onto anything else inside the
    /// same body is still the copy it always was.
    appending: Option<Appending>,
    /// Where each bound name lives.
    slots: HashMap<DefId, Local>,
    /// Every refinement the checker could not settle, by the span of the
    /// expression that has to satisfy it.
    ///
    /// Read off the checker's own table rather than worked out again here,
    /// for the reason `deed_driver::Checked::guards` gives: when the two were
    /// separate the checker said "so it becomes a runtime check" over places
    /// that had no check.
    guards: &'a HashMap<Span, Guard>,
    /// What each refinement says, and what name it says it about.
    refinements: &'a HashMap<DefId, Refinement<'a>>,
    /// Whether a refinement's own predicate is being lowered.
    ///
    /// A predicate is an ordinary expression and is lowered as one, so
    /// without this a guard inside one would ask for itself forever.
    checking: bool,
}

/// Where the walk being lowered writes the block it is building.
///
/// Both cases name a slot rather than a name, so a `push` onto anything else
/// inside the same body is still the copy it always was.
enum Appending {
    /// The accumulator is the list.
    Whole(Local),
    /// The accumulator is a record, and these of its fields are the lists.
    Fields(Local, Vec<usize>),
}

/// One refinement, as the lowering needs it.
struct Refinement<'a> {
    /// What has to hold, which is an ordinary expression.
    predicate: &'a ast::Expr,
    /// The `value` the predicate talks about. `None` for a predicate that
    /// never mentions it, which is a predicate with nothing to bind.
    subject: Option<DefId>,
    /// What the refinement is called, for the sentence a failure prints.
    name: String,
}

/// A refinement one expression has to satisfy at runtime.
#[derive(Clone, Copy)]
struct Guard {
    refinement: DefId,
    /// Whether the value is the one inside the `ok` rather than the
    /// expression itself. A `Result` that came back from a call has nothing
    /// naming its payload, so the obligation lands on the whole expression
    /// and has to say that what it is about is one level in.
    inside_ok: bool,
}

/// Which variants an arm names, and where each name it binds reads from.
///
/// A binding is a path rather than one field, because a pattern may reach
/// through a value into what it holds: `err(OverLimit { limit })` names a
/// field of the record inside the failure.
type Arm<'p> = (
    Vec<usize>,
    Vec<(Vec<(crate::LayoutId, usize, usize)>, &'p ast::Ident)>,
);

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

/// Every name a written type mentions, including inside its arguments.
fn names_in_type(ty: &ast::Type, out: &mut Vec<String>) {
    if let ast::Type::Named { name, args, .. } = ty {
        out.push(name.name.to_string());
        for arg in args {
            names_in_type(arg, out);
        }
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
            ast::Stmt::Abandon { .. } => {}
        }
    }
    if let Some(tail) = &block.tail {
        names_in(tail, out);
    }
}

impl<'a> Lowering<'a> {
    /// A lowering that reads bodies from one of the modules handed over.
    #[allow(clippy::too_many_arguments)]
    fn at(
        units: &'a [Unit<'a>],
        at: usize,
        by_def: &'a HashMap<DefId, crate::FuncId>,
        shapes: Vec<crate::Layout>,
        declared: usize,
        instantiated: &'a mut HashMap<String, crate::FuncId>,
        answered: &'a mut HashMap<String, (crate::EffectId, crate::LayoutId, Vec<crate::FuncId>)>,
        function: Function,
    ) -> Self {
        let unit = &units[at];
        Lowering {
            units,
            at,
            resolutions: unit.resolutions,
            types: unit.types,
            by_def,
            layouts: &unit.layouts,
            alias_types: &unit.alias_types,
            shapes,
            declared,
            lifted: Vec::new(),
            bindings: HashMap::new(),
            declarations: &unit.declarations,
            instantiated,
            effects: &unit.effects,
            signatures: &unit.signatures,
            handlers: &unit.handlers,
            aliases: &unit.aliases,
            nominals: &unit.nominals,
            answered,
            state: None,
            expected: None,
            function,
            appending: None,
            slots: HashMap::new(),
            guards: &unit.guards,
            refinements: &unit.refinements,
            checking: false,
        }
    }

    /// Swaps in another module's tables, and hands back what was there.
    fn enter(&mut self, at: usize) -> Entered<'a> {
        let unit = &self.units[at];
        Entered {
            at: std::mem::replace(&mut self.at, at),
            resolutions: std::mem::replace(&mut self.resolutions, unit.resolutions),
            types: std::mem::replace(&mut self.types, unit.types),
            layouts: std::mem::replace(&mut self.layouts, &unit.layouts),
            alias_types: std::mem::replace(&mut self.alias_types, &unit.alias_types),
            declarations: std::mem::replace(&mut self.declarations, &unit.declarations),
            effects: std::mem::replace(&mut self.effects, &unit.effects),
            signatures: std::mem::replace(&mut self.signatures, &unit.signatures),
            handlers: std::mem::replace(&mut self.handlers, &unit.handlers),
            aliases: std::mem::replace(&mut self.aliases, &unit.aliases),
            nominals: std::mem::replace(&mut self.nominals, &unit.nominals),
            guards: std::mem::replace(&mut self.guards, &unit.guards),
            refinements: std::mem::replace(&mut self.refinements, &unit.refinements),
        }
    }

    /// Puts back what [`Self::enter`] took.
    fn leave(&mut self, was: Entered<'a>) {
        self.at = was.at;
        self.resolutions = was.resolutions;
        self.types = was.types;
        self.layouts = was.layouts;
        self.alias_types = was.alias_types;
        self.declarations = was.declarations;
        self.effects = was.effects;
        self.signatures = was.signatures;
        self.handlers = was.handlers;
        self.aliases = was.aliases;
        self.nominals = was.nominals;
        self.guards = was.guards;
        self.refinements = was.refinements;
    }
}

/// One module's tables, held while another module's are in place.
struct Entered<'a> {
    at: usize,
    resolutions: &'a Resolutions,
    types: &'a Types,
    layouts: &'a HashMap<String, crate::LayoutId>,
    alias_types: &'a HashMap<String, Ty>,
    declarations: &'a HashMap<String, &'a ast::FnDecl>,
    effects: &'a HashMap<String, crate::EffectId>,
    signatures: &'a HashMap<String, &'a ast::EffectDecl>,
    handlers: &'a HashMap<String, &'a ast::HandlerDecl>,
    aliases: &'a HashMap<String, &'a ast::TypeAlias>,
    nominals: &'a HashMap<String, Nominal<'a>>,
    guards: &'a HashMap<Span, Guard>,
    refinements: &'a HashMap<DefId, Refinement<'a>>,
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

        let bindings = self.type_arguments(declaration, generics, args, span)?;

        // In the order the declaration wrote them. Sorted, `keys<Int, Str>`
        // and `keys<Str, Int>` were one name, so the second call reached the
        // first call's body and read the wrong half of every entry.
        let spelled: Vec<String> = generics
            .iter()
            .map(|generic| match bindings.get(generic.name.as_str()) {
                Some(ty) => format!("{ty:?}"),
                None => "?".to_string(),
            })
            .collect();
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

        self.copy_of(declaration, copy, bindings)
    }

    /// What each type parameter stands for at this call.
    ///
    /// Read in the caller's tables, because the arguments are the caller's
    /// and their types are what say what the parameters stand for. Split out
    /// so a call into another module can work them out here and then lower
    /// the body over there.
    ///
    /// The return type is read as well as the arguments. A parameter that
    /// only appears inside a callback's type cannot be read off an argument,
    /// because a function value reaches this layer as one shape whatever it
    /// takes and hands back, and `map`'s `B` is exactly that. What the call
    /// itself came out as says it.
    fn type_arguments(
        &mut self,
        declaration: &ast::FnDecl,
        generics: &[ast::Ident],
        args: &[ast::Expr],
        span: Span,
    ) -> Result<HashMap<String, Ty>, Unlowered> {
        let mut bindings: HashMap<String, Ty> = HashMap::new();
        for (param, arg) in declaration.sig.params.iter().zip(args) {
            let Some(written) = &param.ty else {
                continue;
            };
            let actual = self.ty_of_argument(arg)?;
            bind(written, &actual, generics, &mut bindings);
        }
        if bindings.len() != generics.len()
            && let Some(written) = &declaration.sig.ret
            && let Ok(actual) = self.ty_at(span)
        {
            bind(written, &actual, generics, &mut bindings);
        }

        // Last, what the checker recorded, for a parameter that appears only
        // inside a type somebody declared.
        if bindings.len() != generics.len() {
            let mut found = Vec::new();
            for (param, arg) in declaration.sig.params.iter().zip(args) {
                let (Some(written), Some(actual)) =
                    (&param.ty, self.types.type_of(arg.span()).cloned())
                else {
                    continue;
                };
                self.bind_checked(written, &actual, generics, &HashMap::new(), &mut found);
            }
            for (name, actual) in found {
                if bindings.contains_key(&name) {
                    continue;
                }
                if let Ok(lowered) = self.convert(&actual, span) {
                    bindings.insert(name, lowered);
                }
            }
        }

        // What is left is a parameter no argument said anything about, which
        // means no value of that type reached the call and none can come back
        // holding one. `holds([], key)` over a `Table<K, V>` is the shape: the
        // list is empty, so `V` is a type the call has no example of.
        //
        // A layout is still needed, because the empty list has an element
        // width, so it stands in as a number for the same reason an empty
        // list's element type does. See `Lowering::convert`.
        for generic in generics {
            bindings
                .entry(generic.name.clone())
                .or_insert_with(|| Ty::Int);
        }
        Ok(bindings)
    }

    /// What a type parameter stands for, read against what the checker
    /// recorded rather than against what the value came out as here.
    ///
    /// A generic type somebody declared reaches this layer as one layout per
    /// set of arguments, with the arguments gone: `Option<Int>` and
    /// `Option<String>` are two layouts and neither says what it holds. A
    /// parameter that appears only inside one of those cannot be read off the
    /// lowered type, and the checker still has what it stood for.
    ///
    /// `rename` carries the names an alias introduced. A parameter written
    /// `Table<K, V>` says nothing on its own: `Table` is a name for
    /// `List<Entry<K, V>>` with its own two parameters, and what the caller's
    /// `K` stands for is inside what the alias expands to, under whatever the
    /// alias called it.
    fn bind_checked(
        &self,
        written: &ast::Type,
        actual: &CheckedTy,
        generics: &[ast::Ident],
        rename: &HashMap<String, String>,
        out: &mut Vec<(String, CheckedTy)>,
    ) {
        let ast::Type::Named { name, args, .. } = written else {
            return;
        };
        let name = match rename.get(name.name.as_str()) {
            Some(outer) => outer.clone(),
            None => name.name.to_string(),
        };

        if args.is_empty() {
            if generics
                .iter()
                .any(|one| one.name.as_str() == name.as_str())
            {
                out.push((name, actual.clone()));
            }
            return;
        }

        if let Some(alias) = self.aliases.get(name.as_str()).copied()
            && alias.generics.len() == args.len()
        {
            let mut inner = HashMap::new();
            for (generic, arg) in alias.generics.iter().zip(args) {
                if let ast::Type::Named {
                    name: written,
                    args,
                    ..
                } = arg
                    && args.is_empty()
                {
                    let outer = match rename.get(written.name.as_str()) {
                        Some(outer) => outer.clone(),
                        None => written.name.to_string(),
                    };
                    inner.insert(generic.name.to_string(), outer);
                }
            }
            self.bind_checked(&alias.ty, actual, generics, &inner, out);
            return;
        }

        // No guard on which name was written. The checker already accepted
        // the call, so the shapes line up and this is reading an answer
        // rather than searching for one, which is the same reasoning `bind`
        // gives for walking both in step.
        let actual_args: Vec<&CheckedTy> = match actual {
            CheckedTy::Named { args, .. } | CheckedTy::External { args, .. } => {
                args.iter().collect()
            }
            CheckedTy::List(element) => vec![element],
            CheckedTy::Result(ok, err) => vec![ok, err],
            _ => return,
        };
        for (written, actual) in args.iter().zip(actual_args) {
            self.bind_checked(written, actual, generics, rename, out);
        }
    }

    /// Lowers one copy of a declaration and hands back where it landed.
    ///
    /// The declaration is read in whatever tables are in place, so a call
    /// into another module enters that module first and this is the same
    /// piece of work either way.
    fn copy_of(
        &mut self,
        declaration: &ast::FnDecl,
        copy: String,
        bindings: HashMap<String, Ty>,
    ) -> Result<crate::FuncId, Unlowered> {
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

        let body = self.block(&declaration.body);
        let contract = body.and_then(|body| Ok((self.contract(declaration)?, body)));
        let lowered = std::mem::replace(&mut self.function, outer_function);
        self.slots = outer_slots;
        self.bindings = outer_bindings;
        let (contract, body) = contract?;

        let at = id.0 - self.declared;
        self.lifted[at].locals = lowered.locals;
        self.lifted[at].body = Block {
            stmts: contract.into_iter().chain(body.stmts).collect(),
            value: body.value,
        };
        Ok(id)
    }

    /// A call into another module.
    ///
    /// The callee is lowered from its own syntax with its own tables, and
    /// only when something reaches it: a module that ships thirty functions
    /// and is imported for one contributes one, which matters because the
    /// rest of it may use shapes this backend cannot compile.
    /// The function a bound operator means.
    ///
    /// Reached by module and name rather than by definition, for the reason
    /// the checker recorded it that way: nothing imported the function, so
    /// there is no definition here that stands for it. Lowered as it is
    /// reached, the same way a call into another module is.
    fn operator_target(
        &mut self,
        module: &str,
        function: &str,
        span: Span,
    ) -> Result<crate::FuncId, Unlowered> {
        let Some(at) = self.units.iter().position(|unit| unit.path == module) else {
            return Err(unlowered(
                &format!(
                    "an operator meaning `{function}`, whose module `{module}` was not compiled alongside"
                ),
                span,
            ));
        };
        let Some(declaration) = self.units[at].declarations.get(function).copied() else {
            return Err(unlowered(
                &format!("an operator meaning `{function}`, which `{module}` does not declare"),
                span,
            ));
        };

        let copy = format!("{module}/{function}<>");
        if let Some(found) = self.instantiated.get(&copy) {
            return Ok(*found);
        }
        let was = self.enter(at);
        let lowered = self.copy_of(declaration, copy, HashMap::new());
        self.leave(was);
        lowered
    }

    fn crossed(
        &mut self,
        def: DefId,
        args: &[ast::Expr],
        span: Span,
    ) -> Result<Option<crate::FuncId>, Unlowered> {
        let (at, name) = match self.resolutions.def(def).kind {
            deed_resolve::DefKind::Import => {
                let name = self.resolutions.def(def).name.clone();
                let Some(path) = self.resolutions.import_module(def) else {
                    return Ok(None);
                };
                let path = path.to_string();
                let Some(at) = self.units.iter().position(|unit| unit.path == path) else {
                    return Err(unlowered(
                        &format!(
                            "a call to `{name}`, whose module `{path}` was not compiled alongside"
                        ),
                        span,
                    ));
                };
                (at, name)
            }
            // A call to something the module being read declares itself.
            // When that is not the module being compiled, its functions have
            // no index of their own and are lowered as they are reached, the
            // same way an imported one is. Nothing here is on the path of a
            // program that imports nothing.
            deed_resolve::DefKind::Function if self.at != 0 => {
                (self.at, self.resolutions.def(def).name.clone())
            }
            _ => return Ok(None),
        };
        let path = self.units[at].path.clone();
        let Some(declaration) = self.units[at].declarations.get(&name).copied() else {
            return Ok(None);
        };

        // What the type parameters stand for is read here, in the caller's
        // tables, because the arguments are the caller's.
        let generics = declaration.sig.generics.clone();
        let bindings = self.type_arguments(declaration, &generics, args, span)?;
        if bindings.len() != generics.len() {
            return Err(unlowered(
                &format!("a call to `{name}` whose type arguments this cannot work out"),
                span,
            ));
        }

        let spelled: Vec<String> = generics
            .iter()
            .map(|generic| match bindings.get(generic.name.as_str()) {
                Some(ty) => format!("{ty:?}"),
                None => "?".to_string(),
            })
            .collect();
        let copy = format!("{path}/{name}<{}>", spelled.join(", "));
        if let Some(found) = self.instantiated.get(&copy) {
            return Ok(Some(*found));
        }

        let was = self.enter(at);
        let lowered = self.copy_of(declaration, copy, bindings);
        self.leave(was);
        lowered.map(Some)
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
        // Only for a callee in the module being compiled. A call that came
        // from another module was answered for in the caller's table, which
        // the callee's own module never saw, so "every caller proved it" over
        // there is a statement about the wrong set of callers.
        if self.at == 0 && self.every_caller_proved(name) {
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
                        span: clause.span(),
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
    ///
    /// A function an `assert refuses` aims at answers no however its calls
    /// settled. That call is written to break the clause and the checker
    /// records no tier for it, so without this the one caller that needs the
    /// check is the one caller nothing knew about.
    fn every_caller_proved(&self, callee: &str) -> bool {
        if self.types.is_refuted(callee) {
            return false;
        }
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
    /// What an argument is, for working out a type parameter from it.
    ///
    /// A name that is already in a slot answers for itself when the checker
    /// could not. An accumulator is the case: `with seen = None` on an
    /// `Option<String>` is only settled by the body, and the name is read
    /// before that in the `while` above it.
    fn ty_of_argument(&mut self, arg: &ast::Expr) -> Result<Ty, Unlowered> {
        match self.ty_at(arg.span()) {
            Ok(ty) => Ok(ty),
            Err(why) => {
                if let ast::Expr::Ident(ident) = arg
                    && let Some(def) = self.resolutions.resolution(ident.span)
                    && let Some(local) = self.slots.get(&def).copied()
                {
                    return Ok(self.function.local_ty(local).clone());
                }
                Err(why)
            }
        }
    }

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
            // because there is no element to read it off. It stands in as a
            // number rather than as `()`: a `()` has no representation at
            // all, so a walk over one counts elements that take no room, and
            // `filter([], ...)` came out as a function whose body disagreed
            // with itself about how much of the stack it had.
            CheckedTy::List(element) if matches!(**element, CheckedTy::Unknown) => {
                Ty::List(Box::new(Ty::Int))
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
                    // One half is written and the other is not, which is what
                    // `ok(4)` is: what a success holds says nothing about
                    // what the error would have held. The function this is
                    // inside says the shape when it hands one back, and what
                    // this is being compared against says it otherwise. When
                    // neither does, the half nobody settled stands in as a
                    // number, the same as a type argument nothing says.
                    let context = [self.function.ret.clone(), self.expected.clone().unwrap_or(Ty::Unit)]
                        .into_iter()
                        .find(|ty| {
                            matches!(ty, Ty::Aggregate(id) if self.layout(*id).name.starts_with("Result<"))
                        });
                    match context {
                        Some(ty) => ty,
                        None => {
                            let ok = self.type_argument(ok, span)?;
                            let err = self.type_argument(err, span)?;
                            Ty::Aggregate(result_layout(&mut self.shapes, ok, err))
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
                    .map(|arg| self.type_argument(arg, span))
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
            // A type that came from another module. The checker carries it
            // by module and name rather than by definition, for the reason a
            // `DefId` cannot cross: it is an index into one module's table.
            // The name is enough here, because the borrowing pass put what
            // the module that declared it built under that name.
            CheckedTy::External { module, name, args } if &**module != "<prelude>" => {
                let name = name.to_string();
                if args.is_empty() {
                    match self.layouts.get(name.as_str()) {
                        Some(id) => Ty::Aggregate(*id),
                        None => match self.alias_types.get(name.as_str()) {
                            Some(ty) => ty.clone(),
                            None => return Err(unlowered(&format!("the type `{name}`"), span)),
                        },
                    }
                } else {
                    let actuals = args
                        .iter()
                        .map(|arg| self.type_argument(arg, span))
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
            }
            other => lower_ty(other, span)?,
        })
    }

    /// One type argument of a generic type, where not knowing it is allowed.
    ///
    /// `Empty` is a `Map<K, V>` and nothing at that name says what either
    /// stands for, because the value holds none of either. The layout still
    /// needs one, so it stands in as a number, which is what an empty list's
    /// element type already does and for the same reason: there is no element
    /// to read it off.
    fn type_argument(&mut self, ty: &CheckedTy, span: Span) -> Result<Ty, Unlowered> {
        match ty {
            CheckedTy::Unknown => Ok(Ty::Int),
            other => self.convert(other, span),
        }
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
                                span: *span,
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
                ast::Stmt::Abandon { span } => {
                    // `abandon` lowers to a contract-style failure with its
                    // own code. The compiled backend emits an `unreachable`
                    // the same way it does for any `Stmt::Fail`.
                    stmts.push(Stmt::Fail {
                        code: crate::codes::ABANDONED.to_string(),
                        message: format!(
                            "this computation was abandoned by its handler at {}",
                            span.start
                        ),
                        span: *span,
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

    /// One expression, with whatever the checker could not settle about it
    /// checked around it.
    ///
    /// Every way a refined value can come into existence goes through here,
    /// which is the point: the arrangement this replaces checked arguments
    /// and annotated `let`s and nothing else, so a return value carried a
    /// warning and no check.
    fn expr(&mut self, expr: &ast::Expr) -> Result<Expr, Unlowered> {
        let lowered = self.raw_expr(expr)?;
        self.guarded(expr.span(), lowered)
    }

    /// What a refinement the checker left `Guarded` costs at runtime.
    ///
    /// The value is bound, the predicate is run against it, and a failure
    /// ends the run the way any contract failure does. Nothing at all when
    /// the checker settled it, which is the whole of what the tier buys.
    fn guarded(&mut self, span: Span, value: Expr) -> Result<Expr, Unlowered> {
        if self.checking {
            return Ok(value);
        }
        let Some(guard) = self.guards.get(&span).copied() else {
            return Ok(value);
        };
        let Some(refinement) = self.refinements.get(&guard.refinement) else {
            return Ok(value);
        };
        let predicate = refinement.predicate;
        let name = refinement.name.clone();
        // A predicate that never says `value` says nothing about the value,
        // and there is nothing to bind it to.
        let Some(subject) = refinement.subject else {
            return Ok(value);
        };

        let ty = self.ty_at(span)?;
        let held = self.function.add_local(ty.clone());

        // What the predicate is about, and whether there is one to ask about.
        let (bound_ty, read, only_when) = if guard.inside_ok {
            let Ty::Aggregate(layout) = ty else {
                return Err(unlowered(
                    "a refinement on a `Result` this cannot see inside",
                    span,
                ));
            };
            let (ok, _) = self.outcomes(layout, span)?;
            let payload = self.layout(layout).variants[ok].fields[0].ty.clone();
            (
                payload,
                Expr::Field {
                    value: Box::new(Expr::Local(held)),
                    layout,
                    variant: ok,
                    field: 0,
                },
                Some(Expr::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(Expr::Discriminant {
                        value: Box::new(Expr::Local(held)),
                        layout,
                    }),
                    right: Box::new(Expr::Int(ok as i64)),
                    span,
                }),
            )
        } else {
            (ty, Expr::Local(held), None)
        };

        let bound = self.function.add_local(bound_ty);

        // The predicate talks about `value`, which is in scope nowhere else,
        // so it is bound here and whatever the name meant before is put back.
        let outer = self.slots.insert(subject, bound);
        self.checking = true;
        let condition = self.raw_expr(predicate);
        self.checking = false;
        match outer {
            Some(was) => {
                self.slots.insert(subject, was);
            }
            None => {
                self.slots.remove(&subject);
            }
        }

        let mut check = vec![
            Stmt::Assign {
                local: bound,
                value: read,
            },
            Stmt::Discard(Expr::If {
                condition: Box::new(condition?),
                then: Box::new(Block::of(Expr::Unit)),
                otherwise: Box::new(Block {
                    stmts: vec![Stmt::Fail {
                        code: crate::codes::REFINEMENT_FAILED.to_string(),
                        message: format!("this value does not satisfy `{name}`"),
                        span,
                    }],
                    value: Expr::Unit,
                }),
                ty: Box::new(Ty::Unit),
            }),
        ];

        if let Some(condition) = only_when {
            check = vec![Stmt::Discard(Expr::If {
                condition: Box::new(condition),
                then: Box::new(Block {
                    stmts: check,
                    value: Expr::Unit,
                }),
                otherwise: Box::new(Block::of(Expr::Unit)),
                ty: Box::new(Ty::Unit),
            })];
        }

        let mut stmts = vec![Stmt::Assign { local: held, value }];
        stmts.extend(check);
        Ok(Expr::Block(Box::new(Block {
            stmts,
            value: Expr::Local(held),
        })))
    }

    fn raw_expr(&mut self, expr: &ast::Expr) -> Result<Expr, Unlowered> {
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

                        // A function used as a value, which is a closure
                        // that captured nothing. Declared here or imported:
                        // both are a name that is not a local and not a
                        // variant, and the value is built the same way.
                        if let Some(def) = self.resolutions.resolution(ident.span)
                            && self.names_a_function(def)
                        {
                            return self.function_value(&ident.name, def, ident.span);
                        }

                        // So it is a variant carrying no fields, written
                        // bare the way this language writes them.
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
                let left = self.expr(lhs)?;
                // The left side is the shape the right side has to be, and it
                // is the only thing that says so when the right side is
                // `ok("twenty")`: what an `ok` holds says nothing about what
                // the `err` half would have held, and two `Result` layouts
                // hold different widths.
                let outer = self.expected.replace(operand.clone());
                let right = self.expr(rhs);
                self.expected = outer;
                let right = right?;

                if let Some((module, function)) = self.types.operators().get(span).cloned() {
                    let func = self.operator_target(&module, &function, *span)?;
                    return Ok(Expr::Call {
                        func,
                        args: vec![left, right],
                        span: *span,
                    });
                }

                Expr::Binary {
                    op: binary(*op, &operand, *span)?,
                    left: Box::new(left),
                    right: Box::new(right),
                    span: *span,
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
                        span: *span,
                    });
                }

                // `by_def` is the module being compiled and only it: a
                // `DefId` is an index into one module's resolution table, so
                // a definition from somewhere else that happens to have the
                // same number would find a function here and call it. Which
                // is what happened, silently, to a recursive function in an
                // imported module: `run` in `std/task` reached whatever had
                // its number in the program that imported it.
                let known = (self.at == 0).then(|| self.by_def.get(&def)).flatten();
                match known {
                    Some(func) => Expr::Call {
                        func: *func,
                        args: lowered,
                        span: *span,
                    },
                    // Not something with a signature of its own. A generic
                    // declaration is one of these and gets a copy per set of
                    // type arguments; everything else is a name the language
                    // provides, and only the ones whose answer is already
                    // sitting in memory are here.
                    None => {
                        // A call into another module, lowered from the
                        // module it was written in.
                        if let Some(func) = self.crossed(def, args, *span)? {
                            return Ok(Expr::Call {
                                func,
                                args: lowered,
                                span: *span,
                            });
                        }

                        if let Some(found) = self.declarations.get(name.name.as_str())
                            && !found.sig.generics.is_empty()
                        {
                            let generics = found.sig.generics.clone();
                            let copy = self.instantiate(&name.name, &generics, args, *span)?;
                            return Ok(Expr::Call {
                                func: copy,
                                args: lowered,
                                span: *span,
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
                            ("at", _) => crate::runtime::LIST_AT,
                            // The accumulator of a walk that only pushes is
                            // written where it stands rather than copied. The
                            // slot is what identifies it, so a `push` onto
                            // anything else in the same body is still a copy.
                            ("push", _) if self.appends_in_place(lowered.first()) => {
                                crate::runtime::LIST_APPEND
                            }
                            ("push", _) => crate::runtime::LIST_PUSH,
                            ("repeat", _) => crate::runtime::LIST_REPEAT,
                            ("split", _) => crate::runtime::STR_SPLIT,
                            ("join", _) => crate::runtime::STR_JOIN,
                            ("trim", _) => crate::runtime::STR_TRIM,
                            ("upper", _) => crate::runtime::STR_UPPER,
                            ("lower", _) => crate::runtime::STR_LOWER,
                            ("to_string", _) => crate::runtime::INT_TO_STR,
                            ("to_int", _) => crate::runtime::STR_TO_INT,
                            (other, _) => {
                                return Err(unlowered(&format!("a call to `{other}`"), name.span));
                            }
                        };
                        // What the checker recorded for the call, rather than
                        // a type written here per name: `at` on a list of
                        // records and `at` on a list of numbers hand back
                        // different `Result`s and only the checker knows
                        // which.
                        let ret = self.ty_at(*span)?;
                        Expr::Runtime {
                            name: which,
                            args: lowered,
                            ret: Box::new(ret),
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
                if let Some(limit) = expr.int_limit() {
                    return Ok(Expr::Int(limit));
                }

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
            ast::Expr::Try { operand, span } => self.propagate(operand, *span)?,
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

        // A handler declared in another module is lowered against that
        // module's tables. Installing one only needs the declaration, which
        // is why borrowing it was enough to get this far, but its operations
        // are code: every name in them was resolved over there, and read here
        // they resolved to nothing at all.
        let owner = self.units[self.at].handler_from.get(name).copied();
        let was = owner.map(|at| self.enter(at));
        let answered = self.answer_here(name, span);
        if let Some(was) = was {
            self.leave(was);
        }
        answered
    }

    fn answer_here(
        &mut self,
        name: &str,
        span: Span,
    ) -> Result<(crate::EffectId, crate::LayoutId, Vec<crate::FuncId>), Unlowered> {
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
        // Every operation gets its place in the list before any body is
        // lowered. A body may lift something of its own, a closure or a copy
        // of a generic function or a function from another module, and each
        // of those is pushed here too. So the second operation of a handler
        // whose first one lifted anything landed one slot past where the
        // numbers above said it was, and every `perform` after it reached a
        // function that answers a different question.
        let mut places = Vec::new();
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
            places.push((self.lifted.len() - 1, body, params, ret));
        }

        for (at, body, params, ret) in places {
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
        let lifted = Function::new(
            format!("closure{index}"),
            lifted_params.clone(),
            ret.clone(),
        );

        // The place is taken before the body is lowered, not after. A body
        // may lift something of its own: another closure, a copy of a generic
        // function, a wrapper for a function named as a value, or a handler's
        // operations. Each of those pushed first, so the closure landed one
        // slot past the number its own value carries, and calling it reached
        // whatever had taken that slot. `|| Task.fork(step)` is the shortest
        // way to see it: naming `step` as a value lifts a wrapper, and the
        // closure came out pointing at the wrapper.
        self.lifted.push(lifted);

        // Inside the body, a captured name reads out of the environment and
        // a parameter is a parameter. Saved and put back, because the
        // enclosing function goes on being lowered afterwards.
        let outer_slots = self.slots.clone();
        let outer_function = std::mem::replace(
            &mut self.function,
            Function::new(String::new(), lifted_params, ret),
        );

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
        let done = std::mem::replace(&mut self.function, outer_function);
        let at = index - self.declared;
        self.lifted[at].locals = done.locals;
        self.lifted[at].body = Block {
            stmts: reads,
            value: lowered,
        };
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
                span,
            }],
            value: Expr::Unit,
        };

        for arm in arms.iter().rev() {
            let (variants, bindings) = self.arm_pattern(&arm.pattern, layout)?;
            let mut stmts = Vec::new();
            for (path, name) in bindings {
                let mut read = Expr::Local(held);
                let mut field_ty = Ty::Unit;
                for (layout, variant, field) in path {
                    field_ty = self.layout(layout).variants[variant].fields[field]
                        .ty
                        .clone();
                    read = Expr::Field {
                        value: Box::new(read),
                        layout,
                        variant,
                        field,
                    };
                }
                let local = self.function.add_local(field_ty);
                if let Some(def) = self.resolutions.resolution(name.span) {
                    self.slots.insert(def, local);
                }
                stmts.push(Stmt::Assign { local, value: read });
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

    /// `expr?`, which is the `match` on a `Result` that nobody wrote.
    ///
    /// The failure case ends the enclosing function, so the value the whole
    /// thing has is what the success case carries and the rest of the body
    /// only runs when there is one.
    ///
    /// The failure is rebuilt in the enclosing function's own `Result` rather
    /// than handed back as it arrived. The two are the same shape today and
    /// passing it along would work, but only until a layout moves under it,
    /// and this is the layer that is supposed to know.
    fn propagate(&mut self, operand: &ast::Expr, span: Span) -> Result<Expr, Unlowered> {
        let subject = self.ty_at(operand.span())?;
        let Ty::Aggregate(layout) = subject.clone() else {
            return Err(unlowered("`?` on something that is not a Result", span));
        };
        let (ok, err) = self.outcomes(layout, span)?;

        // Where the failure goes, and the only thing this needs to know about
        // the function it is written in.
        let Ty::Aggregate(outer) = self.function.ret.clone() else {
            return Err(unlowered(
                "`?` in a function that does not answer with a Result",
                span,
            ));
        };
        let (_, outer_err) = self.outcomes(outer, span)?;

        let held = self.function.add_local(subject);
        let value = self.expr(operand)?;

        let failed = Expr::Binary {
            op: BinaryOp::Eq,
            left: Box::new(Expr::Discriminant {
                value: Box::new(Expr::Local(held)),
                layout,
            }),
            right: Box::new(Expr::Int(err as i64)),
            span,
        };

        Ok(Expr::Block(Box::new(Block {
            stmts: vec![
                Stmt::Assign { local: held, value },
                Stmt::Discard(Expr::If {
                    condition: Box::new(failed),
                    then: Box::new(Block {
                        stmts: vec![Stmt::Return {
                            value: Expr::Make {
                                layout: outer,
                                variant: outer_err,
                                fields: vec![Expr::Field {
                                    value: Box::new(Expr::Local(held)),
                                    layout,
                                    variant: err,
                                    field: 0,
                                }],
                            },
                        }],
                        value: Expr::Unit,
                    }),
                    otherwise: Box::new(Block::of(Expr::Unit)),
                    ty: Box::new(Ty::Unit),
                }),
            ],
            value: Expr::Field {
                value: Box::new(Expr::Local(held)),
                layout,
                variant: ok,
                field: 0,
            },
        })))
    }

    /// Which variant of a `Result` layout carries the answer and which the
    /// failure.
    ///
    /// Read off the layout rather than written down, so the one place that
    /// decides the order stays one place.
    fn outcomes(&self, layout: crate::LayoutId, span: Span) -> Result<(usize, usize), Unlowered> {
        let variants = &self.layout(layout).variants;
        let at = |name: &str| variants.iter().position(|one| one.name == name);
        match (at("ok"), at("err")) {
            (Some(ok), Some(err)) => Ok((ok, err)),
            _ => Err(unlowered("`?` on something that is not a Result", span)),
        }
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
                span: Span::new(0, 0),
            };
            condition = Some(match condition {
                None => test,
                Some(so_far) => Expr::Binary {
                    op: BinaryOp::Or,
                    left: Box::new(so_far),
                    right: Box::new(test),
                    span: Span::new(0, 0),
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
                    // `{ limit }` binds `limit`, and `{ limit: reached }`
                    // binds `reached` to the same field. A dotted name here
                    // is not a binder and never reaches this pass: the
                    // resolver has nothing to resolve the first segment to
                    // and turns the program away.
                    let bound = match &field.pattern {
                        None => &field.name,
                        Some(ast::Pattern::Path { segments, span }) => segments
                            .last()
                            .ok_or_else(|| unlowered("an empty pattern", *span))?,
                        Some(ast::Pattern::Wildcard(_)) => continue,
                        Some(other) => {
                            return Err(unlowered("a pattern inside a pattern", other.span()));
                        }
                    };
                    bindings.push((vec![(layout, at, index)], bound));
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
                        ast::Pattern::Path { segments, span } => {
                            let name = segments
                                .last()
                                .ok_or_else(|| unlowered("an empty pattern", *span))?;
                            bindings.push((vec![(layout, at, position)], name));
                        }
                        // One level in. `err(OverLimit { limit })` names a
                        // field of the record the failure holds, and what it
                        // reads is the same read twice over.
                        other => {
                            let Ty::Aggregate(inner) =
                                held.variants[at].fields[position].ty.clone()
                            else {
                                return Err(unlowered("a pattern inside a pattern", other.span()));
                            };
                            let (named, held_by) = self.arm_pattern(other, inner)?;
                            // Only where the inner pattern always applies.
                            // One that has to be tested would need a
                            // condition of its own, and the arm has one
                            // condition.
                            if named.len() != self.layout(inner).variants.len() {
                                return Err(unlowered(
                                    "a pattern inside a pattern that has to be tested",
                                    other.span(),
                                ));
                            }
                            for (path, name) in held_by {
                                let mut reached = vec![(layout, at, position)];
                                reached.extend(path);
                                bindings.push((reached, name));
                            }
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

    /// Whether this `push` writes where it stands rather than copying.
    ///
    /// The first argument is what says so: the accumulator's own slot, or a
    /// field of it that the walk reserved room for. Anything else is a list
    /// somebody could still be holding.
    fn appends_in_place(&self, first: Option<&Expr>) -> bool {
        match (&self.appending, first) {
            (Some(Appending::Whole(carried)), Some(Expr::Local(local))) => local == carried,
            (Some(Appending::Fields(carried, built)), Some(Expr::Field { value, field, .. })) => {
                matches!(&**value, Expr::Local(local) if local == carried) && built.contains(field)
            }
            _ => false,
        }
    }

    /// The accumulator a walk starts from, with the fields it builds reserved.
    ///
    /// `built` names them, because the rule is about the source; which slot
    /// each one is comes from the layout the literal already lowered to, so
    /// the two places that know a record's field order stay one place. The
    /// rule holds every turn to the same shape as this one, so a field is in
    /// the same place every turn. The indices reserved are written into
    /// `reserved`.
    fn reserve(
        &mut self,
        start: Expr,
        built: &[String],
        list: Local,
        reserved: &mut Vec<usize>,
    ) -> Expr {
        let Expr::Make {
            layout,
            variant,
            mut fields,
        } = start
        else {
            return start;
        };

        let places: Vec<(usize, Ty)> = self.layout(layout).variants[variant]
            .fields
            .iter()
            .enumerate()
            .filter(|(_, field)| built.iter().any(|name| name == &field.name))
            .map(|(at, field)| (at, field.ty.clone()))
            .collect();
        for (at, ty) in places {
            fields[at] = Expr::Runtime {
                name: crate::runtime::LIST_ROOM,
                args: vec![Expr::Runtime {
                    name: crate::runtime::LIST_LEN,
                    args: vec![Expr::Local(list)],
                    ret: Box::new(Ty::Int),
                }],
                ret: Box::new(ty),
            };
            reserved.push(at);
        }

        Expr::Make {
            layout,
            variant,
            fields,
        }
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

        // A walk whose accumulator is only ever pushed onto builds one list
        // rather than one a turn. Decided from the body before any of it is
        // lowered, because the rule is about what the source does with a name.
        // See `design/decisions/2026-08-04-a-walk-that-only-pushes.md`.
        let in_place = accumulator.is_some_and(|one| {
            matches!(&*one.init, ast::Expr::List { elements, .. } if elements.is_empty())
                && crate::shape::only_pushes(&one.name.name, body)
        });

        // The same argument one field at a time, for the accumulator that is
        // a record of lists. A list accumulator answers none, because the two
        // rules ask about different shapes of what a walk starts from. See
        // `design/decisions/2026-08-04-a-walk-that-pushes-into-a-record.md`.
        let built_fields = match accumulator {
            Some(one) => crate::shape::pushed_fields(&one.name.name, &one.init, body),
            None => Vec::new(),
        };

        let mut reserved = Vec::new();
        let (carried, start) = match accumulator {
            Some(one) => {
                let ty = self.ty_at(one.init.span())?;
                let value = if in_place {
                    // As long as the list being walked, which bounds it.
                    Expr::Runtime {
                        name: crate::runtime::LIST_ROOM,
                        args: vec![Expr::Runtime {
                            name: crate::runtime::LIST_LEN,
                            args: vec![Expr::Local(list)],
                            ret: Box::new(Ty::Int),
                        }],
                        ret: Box::new(ty.clone()),
                    }
                } else {
                    let built = self.expr(&one.init)?;
                    self.reserve(built, &built_fields, list, &mut reserved)
                };
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
            span,
        };
        if let Some(more) = keep {
            // Read before each turn, with the accumulator in scope, which is
            // where the language already says it is read.
            condition = Expr::Binary {
                op: BinaryOp::And,
                left: Box::new(condition),
                right: Box::new(self.expr(more)?),
                span,
            };
        }

        let turn = {
            let outer = self.appending.take();
            self.appending = if in_place {
                Some(Appending::Whole(carried))
            } else if reserved.is_empty() {
                None
            } else {
                Some(Appending::Fields(carried, reserved))
            };
            let turn = self.block(body);
            self.appending = outer;
            turn?
        };
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
                    span,
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

    /// Whether a name is a function, wherever it was declared.
    fn names_a_function(&self, def: DefId) -> bool {
        match self.resolutions.def(def).kind {
            deed_resolve::DefKind::Function => true,
            deed_resolve::DefKind::Import => self
                .resolutions
                .import(def)
                .is_some_and(|export| export.kind == deed_resolve::ExportKind::Function),
            _ => false,
        }
    }

    /// A function used as a value, which is a closure that captured nothing.
    ///
    /// A call through a value passes the environment first and a call by name
    /// does not, so the two cannot be the same function however empty the
    /// environment is. What the value points at is a wrapper that takes the
    /// environment, ignores it, and calls the real one. One wrapper per
    /// function rather than one per mention, so `map(step, xs)` written twice
    /// costs one.
    fn function_value(&mut self, name: &str, def: DefId, span: Span) -> Result<Expr, Unlowered> {
        // A function from another module is lowered in its own module first,
        // the same way a call into one is, and the wrapper is built over what
        // came back. `insert(m, k, v, cmp_string)` is the shape: the callback
        // a keyed library takes is named rather than written out, and the name
        // is an import.
        let known = (self.at == 0).then(|| self.by_def.get(&def)).flatten();
        let func = match known {
            Some(func) => *func,
            None => self
                .crossed(def, &[], span)?
                .ok_or_else(|| unlowered(&format!("the name `{name}`"), span))?,
        };
        let declaration = match self.declarations.get(name).copied() {
            Some(declaration) => declaration,
            None => self
                .resolutions
                .import_module(def)
                .and_then(|path| self.units.iter().find(|unit| unit.path == path))
                .and_then(|unit| unit.declarations.get(name).copied())
                .ok_or_else(|| unlowered(&format!("the name `{name}`"), span))?,
        };

        // The environment a value carries. Nothing is captured, so it holds
        // the code pointer and nothing else.
        let shape = crate::LayoutId(self.shapes.len());
        self.shapes.push(crate::Layout {
            name: format!("value of `{name}`"),
            variants: vec![crate::Variant {
                name: "closure".to_string(),
                fields: vec![crate::Field {
                    name: "code".to_string(),
                    ty: Ty::Int,
                }],
            }],
        });

        let known = format!("value of `{name}`");
        if let Some(index) = self.instantiated.get(&known).copied() {
            return Ok(Expr::Make {
                layout: shape,
                variant: 0,
                fields: vec![Expr::Int(index.0 as i64)],
            });
        }

        let mut params = vec![Ty::Aggregate(shape)];
        for param in &declaration.sig.params {
            let Some(ty) = &param.ty else {
                return Err(unlowered("a parameter with no type", param.span));
            };
            params.push(written(
                ty,
                self.layouts,
                self.aliases,
                self.nominals,
                &self.bindings.clone(),
                &mut self.shapes,
            )?);
        }
        let ret = match &declaration.sig.ret {
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

        let index = crate::FuncId(self.declared + self.lifted.len());
        self.instantiated.insert(known.clone(), index);
        let mut wrapper = Function::new(known, params, ret);
        wrapper.body = Block::of(Expr::Call {
            func,
            args: (1..=declaration.sig.params.len())
                .map(|position| Expr::Local(Local(position)))
                .collect(),
            span,
        });
        self.lifted.push(wrapper);

        Ok(Expr::Make {
            layout: shape,
            variant: 0,
            fields: vec![Expr::Int(index.0 as i64)],
        })
    }

    /// Which layout and which variant a name refers to.
    ///
    /// A record's only variant carries the record's own name, so one lookup
    /// answers both shapes.
    ///
    /// A generic choice has no layout until something asks for one at a set
    /// of type arguments, so a variant of one is not found by name at all.
    /// `Empty` and `Node` in `std/map` are that shape, and every test in that
    /// file writes one. What the checker recorded at the name says which
    /// `Map<K, V>` it is, and building that layout is the same work a call
    /// returning one would have done.
    fn variant_named(
        &mut self,
        name: &str,
        span: Span,
    ) -> Result<(crate::LayoutId, usize), Unlowered> {
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
        if let Ok(Ty::Aggregate(id)) = self.ty_at(span)
            && let Some(at) = self
                .layout(id)
                .variants
                .iter()
                .position(|variant| variant.name == name)
        {
            return Ok((id, at));
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
