//! What shape the walks in this repository actually are.
//!
//! `for` is the only loop, so every list this language builds is built by one,
//! and `crates/deed-driver/tests/allocation.rs` measured what building one
//! costs a compiled program. What decides whether that is worth machinery is
//! how much of the corpus is the shape the machinery answers.
//!
//! The rule is asked of the compiler rather than restated here. A second copy
//! of it would drift, and then this would be measuring how common a shape is
//! that nothing compiles specially. See
//! `design/decisions/2026-08-04-a-walk-that-only-pushes.md`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use deed_ast::{Accumulator, Block, Expr, Item, Stmt};
use deed_driver::shipped_modules;
use deed_mir::only_pushes;

/// Where the repository is, from this test binary.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels under the repository root")
        .to_path_buf()
}

/// Every `.deed` file the library and the corpus are made of.
fn sources() -> Vec<String> {
    let mut found = Vec::new();
    for module in shipped_modules() {
        let text = deed_driver::shipped_source(module).expect("a module that ships has a source");
        found.push(text.to_string());
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
        found.push(std::fs::read_to_string(&path).expect("a readable example"));
    }

    assert!(
        found.len() > 20,
        "the library and the corpus should be more than {} files",
        found.len()
    );
    found
}

/// Every walk in one file that carries an accumulator, with its name.
fn walks(text: &str) -> Vec<(String, Block)> {
    let mut sources = deed_diagnostics::SourceMap::new();
    let file = sources.add("walk.deed".to_string(), text.to_string());
    let lexed = deed_lexer::tokenize(file, sources.file(file).text());
    let parsed = deed_parser::parse(file, &lexed.tokens);
    assert!(!parsed.has_errors(), "every file here should parse");

    let mut found = Vec::new();
    for item in &parsed.module.items {
        let body = match item {
            Item::Function(decl) => &decl.body,
            Item::Test(decl) => &decl.body,
            _ => continue,
        };
        collect(body, &mut found);
    }
    found
}

fn collect(block: &Block, found: &mut Vec<(String, Block)>) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Let { init, .. } => in_expr(init, found),
            Stmt::Assign { value, .. } => in_expr(value, found),
            Stmt::Expr(inner) => in_expr(inner, found),
            Stmt::Assert { condition, .. } => in_expr(condition, found),
            Stmt::Refuses { subject, .. } => in_expr(subject, found),
            Stmt::Return { value, .. } => {
                if let Some(value) = value {
                    in_expr(value, found);
                }
            }
            Stmt::Abandon { .. } => {}
        }
    }
    if let Some(tail) = &block.tail {
        in_expr(tail, found);
    }
}

fn in_expr(expr: &Expr, found: &mut Vec<(String, Block)>) {
    match expr {
        Expr::For {
            accumulator,
            iterable,
            body,
            ..
        } => {
            in_expr(iterable, found);
            if let Some(Accumulator { name, init, .. }) = accumulator {
                in_expr(init, found);
                found.push((name.name.clone(), body.clone()));
            }
            collect(body, found);
        }
        Expr::Field { receiver, .. } => in_expr(receiver, found),
        Expr::Call { callee, args, .. } => {
            in_expr(callee, found);
            for arg in args {
                in_expr(arg, found);
            }
        }
        Expr::List { elements, .. } => {
            for element in elements {
                in_expr(element, found);
            }
        }
        Expr::StructLit { fields, .. } => {
            for field in fields {
                if let Some(value) = &field.value {
                    in_expr(value, found);
                }
            }
        }
        Expr::Unary { operand, .. } => in_expr(operand, found),
        Expr::Binary { lhs, rhs, .. } => {
            in_expr(lhs, found);
            in_expr(rhs, found);
        }
        Expr::Try { operand, .. } => in_expr(operand, found),
        Expr::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            in_expr(condition, found);
            collect(then_branch, found);
            if let Some(otherwise) = else_branch {
                in_expr(otherwise, found);
            }
        }
        Expr::Match {
            scrutinee, arms, ..
        } => {
            in_expr(scrutinee, found);
            for arm in arms {
                in_expr(&arm.body, found);
            }
        }
        Expr::Closure { body, .. } => in_expr(body, found),
        Expr::With { handlers, body, .. } => {
            for handler in handlers {
                in_expr(handler, found);
            }
            collect(body, found);
        }
        Expr::Block(inner) => collect(inner, found),
        _ => {}
    }
}

/// Most walks that carry an accumulator are ones a single list would do.
///
/// This is the measurement the change rests on: `allocation.rs` says what a
/// walk that builds a list costs, and this says how much of the library and
/// the corpus is the walk that can be built in one. A floor and a comparison
/// rather than a number, because the corpus grows. If the shape ever stops
/// being the common one, the argument for answering it specifically is weaker
/// than it was and the decision should be reread.
#[test]
fn most_walks_that_build_a_list_only_ever_push_onto_it() {
    let mut appending = 0usize;
    let mut other = 0usize;
    for text in sources() {
        for (name, body) in walks(&text) {
            if only_pushes(&name, &body) {
                appending += 1;
            } else {
                other += 1;
            }
        }
    }

    assert!(
        appending >= 30 && appending > other,
        "{appending} walks in the library and the corpus build a list by pushing onto an \
         accumulator nothing else can reach, against {other} of every other shape, so \
         answering that shape specifically buys less than it looked"
    );
}
