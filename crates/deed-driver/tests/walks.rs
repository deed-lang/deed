//! What shape the walks in this repository actually are.
//!
//! `for` is the only loop, so every list this language builds is built by one,
//! and `crates/deed-driver/tests/allocation.rs` measured what building one
//! costs a compiled program: the whole answer allocated once per element.
//!
//! What decides whether that is worth machinery is how much of the corpus is
//! the shape the machinery would answer. A walk whose accumulator is only ever
//! handed to `push` cannot have any of its intermediate lists observed by
//! anything, because nothing else is holding one. That is the condition under
//! which a walk could build one list rather than a list per turn, and this
//! counts how many walks meet it.
//!
//! Counted from the parse tree rather than from the text, so a walk spelled
//! across three lines counts the same as one spelled across one, and a `push`
//! inside a string does not count at all.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use deed_ast::{Accumulator, Block, Expr, Item, Stmt};
use deed_driver::shipped_modules;

/// Where the repository is, from this test binary.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels under the repository root")
        .to_path_buf()
}

/// Every `.deed` file the library and the corpus are made of.
fn sources() -> Vec<(String, String)> {
    let mut found = Vec::new();
    for module in shipped_modules() {
        let text = deed_driver::shipped_source(module).expect("a module that ships has a source");
        found.push((module.to_string(), text.to_string()));
    }
    let examples = root().join("examples");
    let mut names: BTreeSet<PathBuf> = BTreeSet::new();
    for entry in std::fs::read_dir(&examples).expect("examples/ should be there") {
        let path = entry.expect("a readable entry").path();
        if path
            .extension()
            .is_some_and(|extension| extension == "deed")
        {
            names.insert(path);
        }
    }
    for path in names {
        let text = std::fs::read_to_string(&path).expect("a readable example");
        found.push((path.display().to_string(), text));
    }

    assert!(
        found.len() > 20,
        "the library and the corpus should be more than {} files",
        found.len()
    );
    found
}

/// What one walk's accumulator is used for.
#[derive(PartialEq, Eq, Debug)]
enum Shape {
    /// Every mention of the accumulator is `push`'s first argument or the
    /// value of a branch, so no intermediate list is reachable from anywhere.
    Appending,
    /// Something else: a number added up, a record rebuilt, a name handed to
    /// something that could keep it.
    Other,
}

/// Walks every expression under `expr`, including itself.
fn each(expr: &Expr, visit: &mut dyn FnMut(&Expr)) {
    visit(expr);
    fn block(block: &Block, visit: &mut dyn FnMut(&Expr)) {
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
            block(then_branch, visit);
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
            block(body, visit);
        }
        Expr::Closure { body, .. } => each(body, visit),
        Expr::With { handlers, body, .. } => {
            for handler in handlers {
                each(handler, visit);
            }
            block(body, visit);
        }
        Expr::Block(inner) => block(inner, visit),
        _ => {}
    }
}

/// Whether every mention of `name` under `expr` is one a walk could build in
/// place: `push`'s first argument, or a value being handed on unchanged.
fn only_pushed(name: &str, expr: &Expr) -> bool {
    let mut mentions = 0usize;
    let mut allowed = 0usize;
    each(expr, &mut |found| {
        if let Expr::Ident(ident) = found
            && ident.name == name
        {
            mentions += 1;
        }
        if let Expr::Call { callee, args, .. } = found
            && let Expr::Ident(callee) = &**callee
            && callee.name == "push"
            && let Some(Expr::Ident(first)) = args.first()
            && first.name == name
        {
            allowed += 1;
        }
    });
    mentions > 0 && mentions == allowed + handed_on(name, expr)
}

/// How many times `name` is the value of a branch rather than being used.
///
/// `filter` is `if keep(item) { push(kept, item) } else { kept }`, and the
/// `else` is the accumulator going round again untouched. That is not a use
/// that could keep it.
fn handed_on(name: &str, expr: &Expr) -> usize {
    let mut count = 0usize;
    let is_name = |expr: &Expr| matches!(expr, Expr::Ident(ident) if ident.name == name);
    let tail_is_name = |block: &Block| block.tail.as_deref().is_some_and(&is_name);

    each(expr, &mut |found| match found {
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => {
            if tail_is_name(then_branch) {
                count += 1;
            }
            match else_branch.as_deref() {
                Some(Expr::Block(block)) if tail_is_name(block) => count += 1,
                Some(other) if is_name(other) => count += 1,
                _ => {}
            }
        }
        Expr::Match { arms, .. } => {
            for arm in arms {
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

/// Classifies every walk in one file.
fn shapes(text: &str) -> Vec<Shape> {
    let mut sources = deed_diagnostics::SourceMap::new();
    let file = sources.add("walk.deed".to_string(), text.to_string());
    let lexed = deed_lexer::tokenize(file, sources.file(file).text());
    let parsed = deed_parser::parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "every file here should parse");

    let mut found = Vec::new();
    let mut classify = |expr: &Expr| {
        let Expr::For {
            accumulator: Some(Accumulator { name, .. }),
            body,
            ..
        } = expr
        else {
            return;
        };
        let whole = Expr::Block(body.clone());
        found.push(if only_pushed(&name.name, &whole) {
            Shape::Appending
        } else {
            Shape::Other
        });
    };

    for item in &parsed.module.items {
        let body = match item {
            Item::Function(decl) => &decl.body,
            Item::Test(decl) => &decl.body,
            _ => continue,
        };
        each(&Expr::Block(body.clone()), &mut classify);
    }
    found
}

/// Most of the walks that build something are the shape one list would do.
///
/// This is the measurement the reclamation decision needs before any of the
/// machinery is worth writing. `allocation.rs` says what a walk that builds a
/// list costs; this says how much of the library and the corpus is that walk.
///
/// A floor rather than a number, because the corpus grows. If it ever drops
/// below, the shape stopped being the common one and the argument for
/// answering it specifically is weaker than it was.
#[test]
fn most_walks_that_build_a_list_only_ever_push_onto_it() {
    let mut appending = 0usize;
    let mut other = 0usize;
    for (name, text) in sources() {
        for shape in shapes(&text) {
            match shape {
                Shape::Appending => appending += 1,
                Shape::Other => other += 1,
            }
            let _ = &name;
        }
    }

    assert!(
        appending >= 30 && appending > other,
        "{appending} walks in the library and the corpus build a list by pushing onto an \
         accumulator nothing else can reach, against {other} of every other shape, so \
         answering that shape specifically buys less than it looked"
    );
}
