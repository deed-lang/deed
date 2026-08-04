//! Whether a walk builds a list nothing else can reach.
//!
//! `for` is the only loop in this language and a `for` is a fold: the
//! accumulator is bound again each turn rather than assigned. So the lists a
//! walk builds on the way exist only as values of that one name, and whether
//! any of them can be observed is a question about what the body does with it.
//!
//! When the answer is "nothing", the walk has no reason to build a list a turn
//! and can build one. That is
//! `design/decisions/2026-08-04-a-walk-that-only-pushes.md`, and this is the
//! rule it rests on. It lives here rather than in the pass that uses it so
//! that the measurement over the corpus asks the compiler rather than asking
//! a second copy of the same idea.

use deed_ast::{Block, Expr, Stmt};

/// Whether a walk over a list can build one list rather than one a turn.
///
/// Two things have to hold, and both are about the same one place: the value
/// of a path through the body, which is what the next turn is handed.
///
/// Every path's value is either the bare name or one `push` straight onto it,
/// so the block that comes out of a turn is the block that went in and a turn
/// grows it by at most one. Without this, `std/list`'s `intersperse`
/// qualifies: it writes `push(push(out, sep), item)`, and two things go wrong
/// at once. The walk would grow by two on a turn while the room reserved is
/// one a turn, and the accumulator would come out of the turn as a copy that
/// was never given room at all, so the next turn would write past the end.
///
/// And those are the only places the name appears at all, which is what the
/// count says. Anywhere else is a place that could keep the accumulator, or a
/// second push in a turn the first condition never sees, and there is no
/// attempt to work out whether it would matter.
pub fn only_pushes(name: &str, body: &Block) -> bool {
    let whole = Expr::Block(body.clone());
    let Some(values) = path_values(&whole) else {
        return false;
    };
    values
        .iter()
        .all(|value| is_name(name, value) || pushes_onto(name, value))
        && mentions(name, &whole) == values.len()
}

/// Whether this expression is the bare name.
fn is_name(name: &str, expr: &Expr) -> bool {
    matches!(expr, Expr::Ident(ident) if ident.name == name)
}

/// Whether this expression is one `push` straight onto the name.
fn pushes_onto(name: &str, expr: &Expr) -> bool {
    let Expr::Call { callee, args, .. } = expr else {
        return false;
    };
    matches!(&**callee, Expr::Ident(callee) if callee.name == "push")
        && args.first().is_some_and(|first| is_name(name, first))
}

/// The value of every path through `expr`, or `None` when some path has none.
///
/// A block hands on its tail, a branch hands on each of its arms, and
/// everything else is a value in its own right. A block with no tail is a
/// path that produces nothing, which no walk this rule is about can have.
fn path_values(expr: &Expr) -> Option<Vec<&Expr>> {
    let mut found = Vec::new();
    paths(expr, &mut found).then_some(found)
}

fn paths<'a>(expr: &'a Expr, found: &mut Vec<&'a Expr>) -> bool {
    match expr {
        Expr::Block(block) => block.tail.as_deref().is_some_and(|tail| paths(tail, found)),
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => {
            then_branch
                .tail
                .as_deref()
                .is_some_and(|tail| paths(tail, found))
                && else_branch
                    .as_deref()
                    .is_some_and(|otherwise| paths(otherwise, found))
        }
        Expr::Match { arms, .. } => arms.iter().all(|arm| paths(&arm.body, found)),
        other => {
            found.push(other);
            true
        }
    }
}

/// How many times `name` appears at all.
fn mentions(name: &str, expr: &Expr) -> usize {
    let mut count = 0;
    each(expr, &mut |found| {
        if is_name(name, found) {
            count += 1;
        }
    });
    count
}

/// Every expression under `expr`, including itself.
fn each(expr: &Expr, visit: &mut dyn FnMut(&Expr)) {
    visit(expr);
    match expr {
        Expr::Field { receiver, .. } => each(receiver, visit),
        Expr::Call { callee, args, .. } => {
            each(callee, visit);
            for arg in args {
                each(arg, visit);
            }
        }
        Expr::List { elements, .. } => {
            for element in elements {
                each(element, visit);
            }
        }
        Expr::StructLit { fields, .. } => {
            for field in fields {
                if let Some(value) = &field.value {
                    each(value, visit);
                }
            }
        }
        Expr::Unary { operand, .. } => each(operand, visit),
        Expr::Binary { lhs, rhs, .. } => {
            each(lhs, visit);
            each(rhs, visit);
        }
        Expr::Try { operand, .. } => each(operand, visit),
        Expr::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            each(condition, visit);
            walk_block(then_branch, visit);
            if let Some(otherwise) = else_branch {
                each(otherwise, visit);
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            each(scrutinee, visit);
            for arm in arms {
                each(&arm.body, visit);
            }
        }
        Expr::For {
            iterable,
            accumulator,
            keep,
            body,
            ..
        } => {
            each(iterable, visit);
            if let Some(accumulator) = accumulator {
                each(&accumulator.init, visit);
            }
            if let Some(keep) = keep {
                each(keep, visit);
            }
            walk_block(body, visit);
        }
        Expr::Closure { body, .. } => each(body, visit),
        Expr::With { handlers, body, .. } => {
            for handler in handlers {
                each(handler, visit);
            }
            walk_block(body, visit);
        }
        Expr::Block(inner) => walk_block(inner, visit),
        _ => {}
    }
}

fn walk_block(block: &Block, visit: &mut dyn FnMut(&Expr)) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let { init, .. } => each(init, visit),
            Stmt::Assign { value, .. } => each(value, visit),
            Stmt::Expr(inner) => each(inner, visit),
            Stmt::Assert { condition, .. } => each(condition, visit),
            Stmt::Refuses { subject, .. } => each(subject, visit),
            Stmt::Return { value, .. } => {
                if let Some(value) = value {
                    each(value, visit);
                }
            }
            Stmt::Abandon { .. } => {}
        }
    }
    if let Some(tail) = &block.tail {
        each(tail, visit);
    }
}
