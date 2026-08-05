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
use deed_diagnostics::{SourceMap, Span};
use deed_driver::{Checked, check_all, shipped_modules, shipped_source};
use deed_mir::{only_pushes, pushed_fields};
use deed_resolve::{DefKind, Resolutions};

/// Where the repository is, from this test binary.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels under the repository root")
        .to_path_buf()
}

/// Every `.deed` file the corpus is made of, by name.
fn examples() -> Vec<(String, String)> {
    let directory = root().join("examples");
    let mut names: BTreeSet<PathBuf> = BTreeSet::new();
    for entry in std::fs::read_dir(&directory).expect("examples/ should be there") {
        let path = entry.expect("a readable entry").path();
        if path
            .extension()
            .is_some_and(|extension| extension == "deed")
        {
            names.insert(path);
        }
    }
    names
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .expect("a file has a name")
                .to_string_lossy()
                .to_string();
            (
                name,
                std::fs::read_to_string(&path).expect("a readable example"),
            )
        })
        .collect()
}

/// The library and the corpus, checked.
///
/// Checked rather than parsed, because the rules ask whether the `length` a
/// walk reads is the one the language provides and only resolution knows
/// that. All of it at once, the way the command line does it: an example may
/// import another example as well as the library.
fn subjects() -> Vec<Checked> {
    let mut sources = SourceMap::new();
    let mut ids = Vec::new();
    for (name, text) in examples() {
        ids.push(sources.add(format!("examples/{name}"), text));
    }
    for module in shipped_modules() {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("<shipped>/{module}.deed"), text.to_string()));
    }

    let found = check_all(&sources, &ids);
    for checked in &found {
        assert!(
            !checked.has_errors(),
            "`{}` should check cleanly",
            sources.file(checked.file).name()
        );
    }
    assert!(
        found.len() > 20,
        "the library and the corpus should be more than {} files",
        found.len()
    );
    found
}

/// Whether the name written at a span is one the language provides.
fn provided(resolutions: &Resolutions) -> impl Fn(Span) -> bool + '_ {
    move |span| {
        resolutions
            .resolution(span)
            .is_some_and(|def| resolutions.def(def).kind == DefKind::Builtin)
    }
}

/// One walk that carries an accumulator: what it is called, what it starts
/// from, what stops it, and what a turn does.
struct Walk {
    name: String,
    init: Expr,
    keep: Option<Expr>,
    body: Block,
    /// Whether the accumulator is a list, which is what the checker says
    /// rather than what the literal looks like: `concat` starts from a name.
    builds_a_list: bool,
}

impl Walk {
    fn shape(&self) -> deed_mir::Walk<'_> {
        deed_mir::Walk {
            name: &self.name,
            init: &self.init,
            keep: self.keep.as_ref(),
            body: &self.body,
        }
    }
}

/// Every walk in one module that carries an accumulator.
fn walks(checked: &Checked) -> Vec<Walk> {
    let mut found = Vec::new();
    for item in &checked.module.items {
        let body = match item {
            Item::Function(decl) => &decl.body,
            Item::Test(decl) => &decl.body,
            _ => continue,
        };
        collect(body, &mut found);
    }
    for walk in &mut found {
        walk.builds_a_list = matches!(
            checked.types.type_of(walk.init.span()),
            Some(deed_typeck::ty::Ty::List(_))
        );
    }
    found
}

fn collect(block: &Block, found: &mut Vec<Walk>) {
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

fn in_expr(expr: &Expr, found: &mut Vec<Walk>) {
    match expr {
        Expr::For {
            accumulator,
            iterable,
            keep,
            body,
            ..
        } => {
            in_expr(iterable, found);
            if let Some(more) = keep {
                in_expr(more, found);
            }
            if let Some(Accumulator { name, init, .. }) = accumulator {
                in_expr(init, found);
                found.push(Walk {
                    name: name.name.clone(),
                    init: (**init).clone(),
                    keep: keep.as_deref().cloned(),
                    body: body.clone(),
                    builds_a_list: false,
                });
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

/// Of the walks that build a list, how many the rule accepts and how many it
/// does not.
///
/// A walk carrying a number or a flag allocates nothing, so counting those in
/// would be measuring how often this language folds rather than how much of
/// the waste this answers. The denominator is the one the name of the test
/// below always claimed.
fn counted() -> (usize, usize) {
    let mut appending = 0usize;
    let mut other = 0usize;
    for checked in subjects() {
        let provided = provided(&checked.resolutions);
        for walk in walks(&checked) {
            if !walk.builds_a_list {
                continue;
            }
            if only_pushes(&walk.shape(), &provided) {
                appending += 1;
            } else {
                other += 1;
            }
        }
    }
    (appending, other)
}

/// How many of the other walks carry a record with a field built in place,
/// and how many fields that is between them.
fn into_records() -> (usize, usize) {
    let mut carrying = 0usize;
    let mut fields = 0usize;
    for checked in subjects() {
        let provided = provided(&checked.resolutions);
        for walk in walks(&checked) {
            if only_pushes(&walk.shape(), &provided) {
                continue;
            }
            let built = pushed_fields(&walk.shape(), &provided);
            if !built.is_empty() {
                carrying += 1;
                fields += built.len();
            }
        }
    }
    (carrying, fields)
}

/// What a number the record says has to be.
fn printed(page: &str, marker: &str) -> usize {
    let path = root().join("design").join("decisions").join(page);
    let text = std::fs::read_to_string(&path).expect("the decision record should be readable");
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(marker))
        .unwrap_or_else(|| panic!("{page} should carry a line starting `{marker}`"))
        .to_string();
    line.split_whitespace()
        .next_back()
        .and_then(|last| last.parse().ok())
        .unwrap_or_else(|| panic!("`{line}` should end in a number"))
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
    let (appending, other) = counted();

    assert!(
        appending >= 30 && appending > other,
        "{appending} walks in the library and the corpus build a list by pushing onto an \
         accumulator nothing else can reach, against {other} of every other shape, so \
         answering that shape specifically buys less than it looked"
    );
}

/// Some of the rest carry a record whose fields are the lists being built.
///
/// The floor is small because the shape is: this is the tail of the same
/// measurement, and what makes it worth answering is not how many walks it is
/// but that `partition`, `unzip` and `scan` are all of them and all three used
/// to allocate along the square of what they walked.
#[test]
fn some_of_the_other_walks_build_a_list_inside_a_record() {
    let (carrying, fields) = into_records();

    assert!(
        carrying >= 3 && fields >= carrying,
        "{carrying} walks in the library and the corpus build {fields} lists inside a \
         record accumulator, so answering that shape specifically buys less than it looked"
    );
}

/// The decision records print these numbers, so they have to be these numbers.
///
/// The tests above are floors on purpose, and a floor is exactly what lets an
/// exact number written down elsewhere go quietly wrong. It has happened
/// twice. The first record shipped saying forty-four and thirty-four against a
/// rule that answered forty and thirty-eight, and nothing was in a position to
/// notice. Then this test was written, and the counts it pinned were still
/// wrong, because the rule it asked was missing a condition that lived at the
/// call site and the denominator counted walks that build no list. A record
/// whose measurement no longer holds is worse than one with no measurement,
/// because the reasoning above it is read as resting on something.
///
/// Both pages print the counts now, and both are read off disk, so a number
/// updated in one of them and not the other says so.
#[test]
fn the_decision_records_print_the_counts_this_measures() {
    const PUSHES: &str = "2026-08-04-a-walk-that-only-pushes.md";
    const LENGTH: &str = "2026-08-05-a-walk-may-read-its-own-length.md";
    const START: &str = "2026-08-05-a-walk-may-start-from-a-list.md";
    const RECORD: &str = "2026-08-04-a-walk-that-pushes-into-a-record.md";
    const IN_PLACE: &str = "walks that build a list and only ever push onto it";
    const REST: &str = "walks that build a list some other way";

    let (appending, other) = counted();
    for page in [PUSHES, LENGTH, START] {
        assert_eq!(
            (printed(page, IN_PLACE), printed(page, REST)),
            (appending, other),
            "the counts in {page} are not the ones the rule gives today"
        );
    }

    let (carrying, fields) = into_records();
    assert_eq!(
        (
            printed(
                RECORD,
                "walks that carry a record with a field built in place"
            ),
            printed(RECORD, "fields those walks build in place"),
        ),
        (carrying, fields),
        "the counts in {RECORD} are not the ones the rule gives today"
    );
}
