//! Whether a walk builds a list nothing else can reach.
//!
//! `for` is the only loop in this language and a `for` is a fold: the
//! accumulator is bound again each turn rather than assigned. So the lists a
//! walk builds on the way exist only as values of that one name, and whether
//! any of them can be observed is a question about what the body does with it.
//!
//! When the answer is "nothing", the walk has no reason to build a list a turn
//! and can build one. That is
//! `design/decisions/2026-08-04-a-walk-that-only-pushes.md`, and the
//! accumulator that is a record of lists is
//! `design/decisions/2026-08-04-a-walk-that-pushes-into-a-record.md`. Both
//! rules live here rather than in the pass that uses them so that the
//! measurement over the corpus asks the compiler rather than asking a second
//! copy of the same idea.

use deed_ast::{Block, Expr, FieldInit, Stmt};
use deed_diagnostics::Span;

/// One walk that carries an accumulator, as the rules here read it.
///
/// The `while` clause is part of it because the accumulator is in scope there
/// and read there. Leaving it out is how `std/list`'s `take` came to be
/// compiled in place while nothing had looked at the condition it stops on.
pub struct Walk<'a> {
    /// What the accumulator is called.
    pub name: &'a str,
    /// What it starts from.
    pub init: &'a Expr,
    /// The `while` clause, read before each turn, or `None` without one.
    pub keep: Option<&'a Expr>,
    /// What a turn does.
    pub body: &'a Block,
}

/// Whether the name written at a span is one the language provides.
///
/// The rules here let a walk read its accumulator's length, and they find
/// that read by name. Shadowing a builtin is a warning rather than an error,
/// so a file can declare or import a `length` of its own, and a function a
/// program wrote could hand the accumulator to something that keeps it.
/// Resolving names is the caller's job; this only knows the shape.
pub type Provided<'a> = &'a dyn Fn(Span) -> bool;

/// Whether a walk over a list can build one list rather than one a turn.
///
/// Three things have to hold, and the middle one is about a single place: the
/// value of a path through the body, which is what the next turn is handed.
///
/// The accumulator starts from the empty list, because a reserved block does.
///
/// Every path's value is either the bare name or one `push` straight onto it,
/// so the block that comes out of a turn is the block that went in and a turn
/// grows it by at most one. Without this, `std/list`'s `intersperse`
/// qualifies: it writes `push(push(out, sep), item)`, and two things go wrong
/// at once. The walk would grow by two on a turn while the room reserved is
/// one a turn, and the accumulator would come out of the turn as a copy that
/// was never given room at all, so the next turn would write past the end.
///
/// And those, together with reading the accumulator's own length, are the only
/// places the name appears at all, which is what the count says. Anywhere else
/// is a place that could keep the accumulator, or a second push in a turn the
/// condition above never sees, and there is no attempt to work out whether it
/// would matter.
///
/// Reading the length is allowed because the length is written as the walk
/// goes, so it answers what it would have answered, and an `Int` is not a way
/// to keep a list. See
/// `design/decisions/2026-08-05-a-walk-may-read-its-own-length.md`.
pub fn only_pushes(walk: &Walk<'_>, provided: Provided<'_>) -> bool {
    if !starts_empty(walk.init) {
        return false;
    }

    let body = Expr::Block(walk.body.clone());
    let Some(values) = path_values(&body) else {
        return false;
    };
    if !values
        .iter()
        .all(|value| is_name(walk.name, value) || pushes_onto(walk.name, value))
    {
        return false;
    }

    let mut seen = mentions(walk.name, &body);
    let mut read = lengths_read(&body, provided, &|held| is_name(walk.name, held));
    if let Some(keep) = walk.keep {
        seen += mentions(walk.name, keep);
        read += lengths_read(keep, provided, &|held| is_name(walk.name, held));
    }
    seen == values.len() + read
}

/// Whether this is the empty list, which is what a reserved block starts as.
fn starts_empty(init: &Expr) -> bool {
    matches!(init, Expr::List { elements, .. } if elements.is_empty())
}

/// Whether this expression is the bare name.
fn is_name(name: &str, expr: &Expr) -> bool {
    matches!(expr, Expr::Ident(ident) if ident.name == name)
}

/// Which of a record accumulator's fields are lists nothing but pushes onto.
///
/// The same argument as [`only_pushes`], one field at a time. A walk that
/// carries `Parts { kept: [], rest: [] }` and hands back a record built here
/// each turn keeps none of the records it made, so the lists in them are
/// reachable only through the one the next turn is given, and a field only
/// ever pushed onto can be the same block throughout.
///
/// The record itself is still built a turn. It is a fixed size, so that is
/// linear in the length of the walk rather than quadratic, and building it
/// over the top of the last one is a question about whether anything holds it,
/// which is the question this whole line of work exists to avoid asking.
///
/// Three things have to hold. The field starts as an empty list, because a
/// reserved block starts empty. Every path's value is a record literal of the
/// same shape as the one the walk started from, whose entry for the field is
/// either that field read off the accumulator or one `push` onto it, so the
/// block that comes out of a turn is the block that went in. And those, along
/// with reading the field's own length, are the only places the field is read,
/// so nothing else holds it. Fields that fail any of them are left alone:
/// `scan` carries a `Pair` whose left is an ordinary value and whose right is
/// built by pushing.
pub fn pushed_fields(walk: &Walk<'_>, provided: Provided<'_>) -> Vec<String> {
    let Expr::StructLit {
        path,
        fields,
        span: _,
    } = walk.init
    else {
        return Vec::new();
    };
    let name = walk.name;
    let candidates: Vec<&str> = fields
        .iter()
        .filter(|field| {
            matches!(field.value.as_ref(), Some(Expr::List { elements, .. }) if elements.is_empty())
        })
        .map(|field| field.name.name.as_str())
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }

    let body = Expr::Block(walk.body.clone());
    let places: Vec<&Expr> = std::iter::once(&body).chain(walk.keep).collect();

    // Nothing holds the record itself: every mention of it is a field read.
    let seen: usize = places.iter().map(|place| mentions(name, place)).sum();
    let reads: usize = places.iter().map(|place| field_reads(name, place)).sum();
    if seen != reads {
        return Vec::new();
    }

    let Some(values) = path_values(&body) else {
        return Vec::new();
    };
    let mut rebuilt: Vec<&[FieldInit]> = Vec::new();
    for value in &values {
        // The same shape each turn, so a field sits in the same place each
        // turn. A choice numbers its fields inside a variant, so a turn that
        // handed back a different one would put a reserved block where a
        // reader is looking for something else.
        let Expr::StructLit {
            path: built,
            fields,
            ..
        } = value
        else {
            return Vec::new();
        };
        if !same_path(path, built) {
            return Vec::new();
        }
        rebuilt.push(fields);
    }
    if rebuilt.is_empty() {
        return Vec::new();
    }

    candidates
        .into_iter()
        .filter(|field| pushed_field(name, field, &places, provided, &rebuilt))
        .map(str::to_string)
        .collect()
}

/// Whether two struct literals name the same thing.
///
/// A bare name and nothing else. Anything a program can write in front of a
/// `{` that is not one is a path nothing resolves, so a walk carrying one
/// never reaches here, and refusing is the direction that costs nothing.
fn same_path(one: &Expr, other: &Expr) -> bool {
    matches!(
        (one, other),
        (Expr::Ident(one), Expr::Ident(other)) if one.name == other.name
    )
}

/// Whether one field of a record accumulator is only ever pushed onto.
fn pushed_field(
    name: &str,
    field: &str,
    places: &[&Expr],
    provided: Provided<'_>,
    rebuilt: &[&[FieldInit]],
) -> bool {
    let mut pushes = 0;
    let mut handed = 0;
    for fields in rebuilt {
        let given = fields
            .iter()
            .find(|entry| entry.name.name == field)
            .and_then(|entry| entry.value.as_ref());
        match given {
            Some(value) if reads_field(name, field, value) => handed += 1,
            Some(Expr::Call { callee, args, .. })
                if matches!(&**callee, Expr::Ident(callee) if callee.name == "push")
                    && args
                        .first()
                        .is_some_and(|first| reads_field(name, field, first)) =>
            {
                pushes += 1;
            }
            // A turn that hands the field something else, or a shorthand
            // entry, or no entry at all.
            _ => return false,
        }
    }

    let seen: usize = places
        .iter()
        .map(|place| reads_of_field(name, field, place))
        .sum();
    let read: usize = places
        .iter()
        .map(|place| lengths_read(place, provided, &|held| reads_field(name, field, held)))
        .sum();

    // A field the walk never appends to would be reserved for nothing.
    pushes > 0 && seen == pushes + handed + read
}

/// Whether this expression reads that field off the accumulator.
fn reads_field(name: &str, field: &str, expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Field { receiver, name: read, .. }
            if read.name == field && is_name(name, receiver)
    )
}

/// How many times a field is read off the accumulator, anywhere.
fn reads_of_field(name: &str, field: &str, expr: &Expr) -> usize {
    let mut count = 0;
    each(expr, &mut |found| {
        if reads_field(name, field, found) {
            count += 1;
        }
    });
    count
}

/// How many mentions of the name are a field read of it.
fn field_reads(name: &str, expr: &Expr) -> usize {
    let mut count = 0;
    each(expr, &mut |found| {
        if let Expr::Field { receiver, .. } = found
            && is_name(name, receiver)
        {
            count += 1;
        }
    });
    count
}

/// Whether this expression is one `push` straight onto the name.
fn pushes_onto(name: &str, expr: &Expr) -> bool {
    let Expr::Call { callee, args, .. } = expr else {
        return false;
    };
    matches!(&**callee, Expr::Ident(callee) if callee.name == "push")
        && args.first().is_some_and(|first| is_name(name, first))
}

/// How many times something the walk is about is handed to `length`.
///
/// One argument and nothing else, so `length(out)` counts and a call that
/// happens to be spelled the same with the accumulator somewhere in a longer
/// argument list does not. The answer is an `Int`, so nothing the call hands
/// back can be the list, and the length a reserved block reports is written as
/// the walk goes, so the number is the one the walk would have got.
fn lengths_read(expr: &Expr, provided: Provided<'_>, held: &dyn Fn(&Expr) -> bool) -> usize {
    let mut count = 0;
    each(expr, &mut |found| {
        if let Expr::Call { callee, args, .. } = found
            && let Expr::Ident(callee) = &**callee
            && callee.name == "length"
            && provided(callee.span)
            && args.len() == 1
            && held(&args[0])
        {
            count += 1;
        }
    });
    count
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
