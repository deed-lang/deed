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
    for item in &module.items {
        let name = match item {
            Item::Record(record) => record.name.name.to_string(),
            Item::Choice(choice) => choice.name.name.to_string(),
            _ => continue,
        };
        let id = program.add_layout(crate::Layout {
            name: name.clone(),
            variants: Vec::new(),
        });
        layouts.insert(name, id);
    }

    for item in &module.items {
        match item {
            Item::Record(record) => {
                let id = layouts[record.name.name.as_str()];
                program.layouts[id.0].variants = vec![crate::Variant {
                    name: record.name.name.to_string(),
                    fields: record
                        .fields
                        .iter()
                        .map(|field| {
                            Ok(crate::Field {
                                name: field.name.name.to_string(),
                                ty: written(&field.ty, &layouts)?,
                            })
                        })
                        .collect::<Result<Vec<_>, Unlowered>>()?,
                }];
            }
            Item::Choice(choice) => {
                let id = layouts[choice.name.name.as_str()];
                program.layouts[id.0].variants = choice
                    .variants
                    .iter()
                    .map(|variant| {
                        Ok(crate::Variant {
                            name: variant.name.name.to_string(),
                            fields: variant
                                .fields
                                .as_ref()
                                .map(|fields| {
                                    fields
                                        .iter()
                                        .map(|field| {
                                            Ok(crate::Field {
                                                name: field.name.name.to_string(),
                                                ty: written(&field.ty, &layouts)?,
                                            })
                                        })
                                        .collect::<Result<Vec<_>, Unlowered>>()
                                })
                                .transpose()?
                                .unwrap_or_default(),
                        })
                    })
                    .collect::<Result<Vec<_>, Unlowered>>()?;
            }
            _ => {}
        }
    }

    let mut order: Vec<&ast::FnDecl> = Vec::new();
    for item in &module.items {
        if let Item::Function(function) = item {
            order.push(function);
        }
    }

    // Two passes over functions: every signature first, so a body can call
    // anything the module declares regardless of where it sits in the file.
    // One pass would make calling forward an error, which is a rule this
    // language does not have.
    let mut by_def: HashMap<DefId, crate::FuncId> = HashMap::new();
    for declaration in &order {
        let name = declaration.sig.name.name.to_string();
        let params = declaration
            .sig
            .params
            .iter()
            .map(|param| match &param.ty {
                Some(ty) => written(ty, &layouts),
                None => Err(unlowered("a parameter with no type", param.span)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let ret = match &declaration.sig.ret {
            None => Ty::Unit,
            Some(ty) => written(ty, &layouts)?,
        };
        let id = program.add_function(Function::new(name, params, ret));
        if let Some(def) = resolutions.resolution(declaration.sig.name.span) {
            by_def.insert(def, id);
        }
    }

    for (index, declaration) in order.iter().enumerate() {
        let id = crate::FuncId(index);
        let mut lowering = Lowering {
            resolutions,
            types,
            by_def: &by_def,
            layouts: &layouts,
            program: &program,
            function: program.functions[index].clone(),
            slots: HashMap::new(),
        };

        for (position, param) in declaration.sig.params.iter().enumerate() {
            if let Some(def) = resolutions.resolution(param.name.span) {
                lowering.slots.insert(def, Local(position));
            }
        }

        let body = lowering.block(&declaration.body)?;
        let mut function = lowering.function;
        function.body = body;
        program.functions[id.0] = function;
    }

    program.entry = program.find("main");
    Ok(program)
}

/// The MIR type of a type somebody wrote down.
///
/// A signature in this language is complete, so every type in one is written
/// rather than worked out, and reading it here asks the checker for nothing.
/// It also already passed the checker, so a name that is not one of these is
/// a type this backend has not got to rather than a mistake.
fn written(ty: &ast::Type, layouts: &HashMap<String, crate::LayoutId>) -> Result<Ty, Unlowered> {
    Ok(match ty {
        ast::Type::Unit(_) => Ty::Unit,
        ast::Type::Fn { .. } => Ty::Closure,
        ast::Type::Error(span) => return Err(unlowered("a type that did not parse", *span)),
        ast::Type::Named { name, args, span } => match (name.name.as_str(), args.len()) {
            ("Int", 0) => Ty::Int,
            ("Bool", 0) => Ty::Bool,
            ("String", 0) => Ty::Str,
            ("List", 1) => Ty::List(Box::new(written(&args[0], layouts)?)),
            (other, 0) => match layouts.get(other) {
                Some(id) => Ty::Aggregate(*id),
                None => return Err(unlowered(&format!("the type `{other}`"), *span)),
            },
            (other, _) => {
                return Err(unlowered(&format!("the generic type `{other}`"), *span));
            }
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
    program: &'a Program,
    function: Function,
    /// Where each bound name lives.
    slots: HashMap<DefId, Local>,
}

/// Which variants an arm names, and which field each name it binds reads.
type Arm<'p> = (Vec<usize>, Vec<(usize, &'p ast::Ident)>);

impl Lowering<'_> {
    /// The MIR type of whatever the checker recorded at this span.
    ///
    /// A refinement is a claim about a value that the checker either proved
    /// or turned into a runtime check, so what is left to compile is the
    /// base type. A named type is a layout this pass already built, found by
    /// the name its definition carries.
    fn ty_at(&self, span: Span) -> Result<Ty, Unlowered> {
        let ty = self
            .types
            .type_of(span)
            .ok_or_else(|| unlowered("a value the checker recorded no type for", span))?;
        self.convert(ty, span)
    }

    fn convert(&self, ty: &CheckedTy, span: Span) -> Result<Ty, Unlowered> {
        Ok(match ty {
            // An empty list's element type is never known and never read,
            // because there is no element to read it off.
            CheckedTy::List(element) if matches!(**element, CheckedTy::Unknown) => {
                Ty::List(Box::new(Ty::Unit))
            }
            CheckedTy::List(element) => Ty::List(Box::new(self.convert(element, span)?)),
            CheckedTy::Named { def, args } if args.is_empty() => {
                let name = &self.resolutions.def(*def).name;
                match self.layouts.get(name.as_str()) {
                    Some(id) => Ty::Aggregate(*id),
                    // A refinement is declared like a type and is not one
                    // here: its base is what a backend lays out.
                    None => return Err(unlowered(&format!("the type `{name}`"), span)),
                }
            }
            other => lower_ty(other, span)?,
        })
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
                                message: format!("an assertion at {} did not hold", span.start),
                            }],
                            value: Expr::Unit,
                        }),
                        ty: Box::new(Ty::Unit),
                    }));
                }
                ast::Stmt::Return { span, .. } => {
                    return Err(unlowered("an early `return`", *span));
                }
                ast::Stmt::Assign { span, .. } => {
                    return Err(unlowered("handler state", *span));
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
                    // Not a local, so it is a variant carrying no fields,
                    // written bare the way this language writes them. A
                    // function used as a value would also land here and is
                    // refused, since it needs a closure to carry.
                    None => {
                        let (layout, variant) = self.variant_named(&ident.name, ident.span)?;
                        if !self.program.layout(layout).variants[variant]
                            .fields
                            .is_empty()
                        {
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

                match self.by_def.get(&def) {
                    Some(func) => Expr::Call {
                        func: *func,
                        args: lowered,
                    },
                    // Not something this module declared, so it is a name the
                    // language provides. Only the ones whose answer is
                    // already sitting in memory are here; the rest are
                    // refused rather than guessed at.
                    None => {
                        let subject = match args.first() {
                            Some(first) => self.ty_at(first.span())?,
                            None => Ty::Unit,
                        };
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
                let declared = &self.program.layout(layout).variants[variant].fields;
                let mut values = Vec::new();
                for field in declared {
                    let written = fields
                        .iter()
                        .find(|given| given.name.name.as_str() == field.name)
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
                let Ty::Aggregate(layout) = receiver_ty else {
                    return Err(unlowered("reading a field of this", *span));
                };
                let held = self.program.layout(layout);
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
            other => {
                return Err(unlowered("this expression", other.span()));
            }
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
                message: "no arm of this match applied".to_string(),
            }],
            value: Expr::Unit,
        };

        for arm in arms.iter().rev() {
            let (variants, bindings) = self.arm_pattern(&arm.pattern, layout)?;
            let mut stmts = Vec::new();
            for (field, name) in bindings {
                let field_ty = self.program.layout(layout).variants[variants[0]].fields[field]
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
        if variants.len() == self.program.layout(layout).variants.len() {
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
        let held = self.program.layout(layout);
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
        for (index, layout) in self.program.layouts.iter().enumerate() {
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
