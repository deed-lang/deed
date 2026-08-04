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
/// Two things have to hold, and the second was learned rather than designed.
///
/// Every mention of `name` is one that keeps nothing. Two positions qualify,
/// and they are the two the library is written in: `push(name, x)` reads the
/// list and hands back the next one, so the list it read is finished with, and
/// a branch whose value is the bare name hands the accumulator on untouched,
/// which is what `filter`'s `else` does. Anywhere else is a place that could
/// keep it, and there is no attempt to work out whether it would.
///
/// And the value of every path through the body is either the bare name or a
/// `push` straight onto it. Without this half, `std/list`'s `intersperse`
/// qualifies: it writes `push(push(out, sep), item)`, whose mentions are all
/// pushes. Two things go wrong at once. The walk would grow by two on a turn
/// while the room reserved is one a turn, and the accumulator would come out
/// of the turn as a copy that was never given room at all, so the next turn
/// would write past the end of it. One condition rules out both, because both
/// are the same mistake: the block that comes out has to be the block that
/// went in.
pub fn only_pushes(name: &str, body: &Block) -> bool {
    let whole = Expr::Block(body.clone());
    let mentions = mentions(name, &whole);
    mentions > 0
        && mentions == pushed(name, &whole) + handed_on(name, &whole)
        && grows_by_one(name, &Expr::Block(body.clone()))
}

/// Whether the value of every path through `expr` is the accumulator or one
/// `push` onto it.
fn grows_by_one(name: &str, expr: &Expr) -> bool {
    match expr {
        Expr::Ident(ident) => ident.name == name,
        Expr::Call { callee, args, .. } => {
            matches!(&**callee, Expr::Ident(callee) if callee.name == "push")
                && matches!(args.first(), Some(Expr::Ident(first)) if first.name == name)
        }
        Expr::Block(block) => block
            .tail
            .as_deref()
            .is_some_and(|tail| grows_by_one(name, tail)),
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => {
            then_branch
                .tail
                .as_deref()
                .is_some_and(|tail| grows_by_one(name, tail))
                && else_branch
                    .as_deref()
                    .is_some_and(|otherwise| grows_by_one(name, otherwise))
        }
        Expr::Match { arms, .. } => arms.iter().all(|arm| grows_by_one(name, &arm.body)),
        _ => false,
    }
}

/// How many times `name` appears at all.
fn mentions(name: &str, expr: &Expr) -> usize {
    let mut count = 0;
    each(expr, &mut |found| {
        if let Expr::Ident(ident) = found
            && ident.name == name
        {
            count += 1;
        }
    });
    count
}

/// How many of those are `push`'s first argument.
fn pushed(name: &str, expr: &Expr) -> usize {
    let mut count = 0;
    each(expr, &mut |found| {
        if let Expr::Call { callee, args, .. } = found
            && let Expr::Ident(callee) = &**callee
            && callee.name == "push"
            && let Some(Expr::Ident(first)) = args.first()
            && first.name == name
        {
            count += 1;
        }
    });
    count
}

/// How many are the value of a branch rather than a use.
fn handed_on(name: &str, expr: &Expr) -> usize {
    let is_name = |expr: &Expr| matches!(expr, Expr::Ident(ident) if ident.name == name);
    let tail_is_name = |block: &Block| block.tail.as_deref().is_some_and(&is_name);

    let mut count = 0;
    each(expr, &mut |found| match found {
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => {
            if tail_is_name(then_branch) {
                count += 1;
            }
            // An `else` is always a block or another `if`, never a bare name.
            if let Some(Expr::Block(block)) = else_branch.as_deref()
                && tail_is_name(block)
            {
                count += 1;
            }
        }
        Expr::Match { arms, .. } => {
            for arm in &arms[..] {
                match &arm.body {
                    Expr::Block(block) if tail_is_name(block) => count += 1,
                    other if is_name(other) => count += 1,
                    _ => {}
                }
            }
        }
        _ => {}
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
