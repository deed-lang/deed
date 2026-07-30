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

    let mut order: Vec<&ast::FnDecl> = Vec::new();
    for item in &module.items {
        if let Item::Function(function) = item {
            order.push(function);
        }
    }

    // Two passes: every signature first, so a body can call anything the
    // module declares regardless of where it sits in the file. One pass would
    // make calling forward an error, which is a rule this language does not
    // have.
    let mut by_def: HashMap<DefId, crate::FuncId> = HashMap::new();
    for declaration in &order {
        let name = declaration.sig.name.name.to_string();
        let params = declaration
            .sig
            .params
            .iter()
            .map(|param| match &param.ty {
                Some(ty) => written(ty),
                None => Err(unlowered("a parameter with no type", param.span)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let ret = match &declaration.sig.ret {
            None => Ty::Unit,
            Some(ty) => written(ty)?,
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

/// The MIR type of whatever the checker recorded at this span.
fn lower_ty_at(types: &Types, span: Span) -> Result<Ty, Unlowered> {
    match types.type_of(span) {
        Some(ty) => lower_ty(ty, span),
        None => Err(unlowered("a value the checker recorded no type for", span)),
    }
}

/// The MIR type of a type somebody wrote down.
///
/// A signature in this language is complete, so every type in one is written
/// rather than worked out, and reading it here asks the checker for nothing.
/// It also already passed the checker, so a name that is not one of these is
/// a type this backend has not got to rather than a mistake.
fn written(ty: &ast::Type) -> Result<Ty, Unlowered> {
    Ok(match ty {
        ast::Type::Unit(_) => Ty::Unit,
        ast::Type::Fn { .. } => Ty::Closure,
        ast::Type::Error(span) => return Err(unlowered("a type that did not parse", *span)),
        ast::Type::Named { name, args, span } => match (name.name.as_str(), args.len()) {
            ("Int", 0) => Ty::Int,
            ("Bool", 0) => Ty::Bool,
            ("String", 0) => Ty::Str,
            ("List", 1) => Ty::List(Box::new(written(&args[0])?)),
            (other, _) => {
                return Err(unlowered(&format!("the type `{other}`"), *span));
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
    function: Function,
    /// Where each bound name lives.
    slots: HashMap<DefId, Local>,
}

impl Lowering<'_> {
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
                    let ty = lower_ty_at(self.types, init.span())?;
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
                    None => {
                        // A function used as a value, which needs a closure
                        // to carry, or a name from somewhere this does not
                        // reach yet.
                        return Err(unlowered("a name that is not a local", ident.span));
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
                let operand = lower_ty_at(self.types, lhs.span())?;
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
                let func = *self
                    .by_def
                    .get(&def)
                    .ok_or_else(|| unlowered("a call to something not declared here", name.span))?;
                Expr::Call {
                    func,
                    args: args
                        .iter()
                        .map(|arg| self.expr(arg))
                        .collect::<Result<Vec<_>, _>>()?,
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
                    Some(tail) => lower_ty_at(self.types, tail.span())?,
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
            other => {
                return Err(unlowered("this expression", other.span()));
            }
        })
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
