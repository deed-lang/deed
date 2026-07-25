//! Effect and capability checking.
//!
//! This is the pass the language exists for. Everything before it exists in
//! other compilers; this one is the argument.
//!
//! Two rules, and the second matters more than the first.
//!
//! **Too narrow.** The body performs an effect the signature does not declare.
//! Obvious, and the one everybody expects.
//!
//! **Too wide.** The signature declares an effect the body never performs. If
//! over-declaring were allowed, every signature would drift towards listing
//! everything, and a row nobody can trust is a row nobody reads. The value of
//! an effect row is entirely in it being tight, so this is an error too.
//!
//! Contract expressions do not contribute to the row. A `where` or `ensures`
//! clause describes state rather than changing it, and an obligation that had
//! to be paid for in permissions would be an obligation people stop writing.
//! That is a decision rather than an oversight, and it is written up in
//! `design/03-effects.md`.

use std::collections::{HashMap, HashSet};

use vow_ast::{Block, EffectRef, Expr, FnDecl, Item, Module, Stmt};
use vow_diagnostics::{Diagnostic, FileId, Span};
use vow_resolve::{DefId, DefKind, Resolutions};

use crate::codes;
use crate::row::{EffectItem, Row};

pub struct Analysis {
    pub effects: Effects,
    pub diagnostics: Vec<Diagnostic>,
}

impl Analysis {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// What each function declares and what it actually does.
#[derive(Default)]
pub struct Effects {
    declared: HashMap<DefId, Row>,
    performed: HashMap<DefId, Row>,
    unverifiable: HashSet<DefId>,
}

impl Effects {
    pub fn declared(&self, function: DefId) -> Option<&Row> {
        self.declared.get(&function)
    }

    pub fn performed(&self, function: DefId) -> Option<&Row> {
        self.performed.get(&function)
    }

    /// Whether the row for this function could not be checked at all.
    pub fn is_unverifiable(&self, function: DefId) -> bool {
        self.unverifiable.contains(&function)
    }
}

/// Checks one resolved module.
pub fn analyse(file: FileId, module: &Module, resolutions: &Resolutions) -> Analysis {
    let mut checker = Checker {
        file,
        resolutions,
        effects: Effects::default(),
        diagnostics: Vec::new(),
        handler_effects: HashMap::new(),
        declared_sites: HashMap::new(),
    };

    checker.collect(module);
    checker.check_module(module);

    Analysis {
        effects: checker.effects,
        diagnostics: checker.diagnostics,
    }
}

struct Checker<'a> {
    file: FileId,
    resolutions: &'a Resolutions,
    effects: Effects,
    diagnostics: Vec<Diagnostic>,
    /// Local handler definition to the effect it implements.
    handler_effects: HashMap<DefId, DefId>,
    /// Where each declared entry was written, for diagnostics.
    declared_sites: HashMap<DefId, Vec<(EffectItem, Span)>>,
}

impl<'a> Checker<'a> {
    fn emit(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    fn kind_of(&self, def: DefId) -> DefKind {
        self.resolutions.def(def).kind
    }

    fn name_of(&self, def: DefId) -> String {
        self.resolutions.def(def).name.clone()
    }

    fn describe(&self, item: &EffectItem) -> String {
        match &item.operation {
            Some(operation) => format!("`{}.{operation}`", self.name_of(item.effect)),
            None => format!("`{}`", self.name_of(item.effect)),
        }
    }

    // -- collecting --------------------------------------------------------

    fn collect(&mut self, module: &Module) {
        for item in &module.items {
            match item {
                Item::Handler(handler) => {
                    if let (Some(handler_def), Some(effect_def)) = (
                        self.resolutions.resolution(handler.name.span),
                        self.resolutions.resolution(handler.effect.span),
                    ) && self.kind_of(effect_def) == DefKind::Effect
                    {
                        self.handler_effects.insert(handler_def, effect_def);
                    }
                }
                Item::Function(function) => {
                    let Some(def) = self.resolutions.resolution(function.sig.name.span) else {
                        continue;
                    };
                    let (row, sites, unverifiable) = self.lower_row(&function.contract.uses);
                    if unverifiable {
                        self.effects.unverifiable.insert(def);
                    }
                    self.effects.declared.insert(def, row);
                    self.declared_sites.insert(def, sites);
                }
                _ => {}
            }
        }
    }

    /// Turns a `uses` clause into a row, reporting entries that cannot mean
    /// what they look like they mean.
    fn lower_row(&mut self, uses: &[EffectRef]) -> (Row, Vec<(EffectItem, Span)>, bool) {
        let mut row = Row::new();
        let mut sites = Vec::new();
        let mut unverifiable = false;

        for entry in uses {
            if entry.effect.name.is_empty() {
                continue;
            }
            let Some(def) = self.resolutions.resolution(entry.effect.span) else {
                // Name resolution already complained.
                unverifiable = true;
                continue;
            };

            match self.kind_of(def) {
                DefKind::Effect => {
                    let item = match (&entry.operation, entry.all) {
                        (Some(operation), _) => EffectItem::operation(def, operation.name.clone()),
                        (None, _) => EffectItem::whole(def),
                    };
                    row.insert(item.clone());
                    sites.push((item, entry.span));
                }

                // An effect from a module that has not been loaded. Its
                // operations are invisible, so nothing about this row can be
                // checked in either direction.
                DefKind::Import => {
                    unverifiable = true;
                    let name = self.name_of(def);
                    self.emit(
                        Diagnostic::warning(
                            codes::UNVERIFIABLE_ROW,
                            self.file,
                            entry.span,
                            format!("`{name}` comes from a module that has not been loaded, so this row is not checked"),
                        )
                        .with_primary_label("not checked")
                        .with_note("declare the effect in this module, or wait for cross module loading"),
                    );
                }

                // `uses sys.*`. This is the hole design/04-capabilities.md
                // already worried about, and it is worth the compiler saying so
                // out loud rather than reporting a clean check it never did.
                DefKind::Param | DefKind::Local => {
                    unverifiable = true;
                    let name = self.name_of(def);
                    let what = if entry.all {
                        format!("`{name}.*` grants everything that capability carries")
                    } else {
                        format!("`{name}` is a value, not an effect")
                    };
                    self.emit(
                        Diagnostic::warning(
                            codes::UNVERIFIABLE_ROW,
                            self.file,
                            entry.span,
                            format!("{what}, so this function's row is not checked"),
                        )
                        .with_primary_label("not checked")
                        .with_note(
                            "granting everything is the same as promising nothing; see the open questions in design/04-capabilities.md",
                        ),
                    );
                }

                other => {
                    let name = self.name_of(def);
                    self.emit(
                        Diagnostic::error(
                            codes::NOT_AN_EFFECT,
                            self.file,
                            entry.span,
                            format!("`{name}` is a {}, not an effect", other.describe()),
                        )
                        .with_primary_label("not an effect")
                        .with_note("a `uses` clause names effects declared with `effect`"),
                    );
                }
            }
        }

        (row, sites, unverifiable)
    }

    // -- checking ----------------------------------------------------------

    fn check_module(&mut self, module: &Module) {
        for item in &module.items {
            match item {
                Item::Function(function) => {
                    let def = self.resolutions.resolution(function.sig.name.span);
                    self.check_fn(function, def);
                }
                Item::Handler(handler) => {
                    for operation in &handler.operations {
                        self.check_fn(operation, None);
                    }
                }
                Item::Test(test) => {
                    let performed = self.infer_block(&test.body);
                    for item in performed.iter() {
                        let described = self.describe(item);
                        self.emit(
                            Diagnostic::error(
                                codes::UNHANDLED_EFFECT,
                                self.file,
                                test.name_span,
                                format!("this test performs {described} with no handler for it"),
                            )
                            .with_primary_label("unhandled effect")
                            .with_note(
                                "wrap the calls in a `with` block naming a handler for the effect",
                            ),
                        );
                    }
                }
                _ => {}
            }
        }
    }

    fn check_fn(&mut self, function: &FnDecl, def: Option<DefId>) {
        // A handler operation has no definition of its own, so its row is
        // lowered here rather than during collection.
        let (declared, sites, unverifiable) = match def {
            Some(def) => (
                self.effects.declared.get(&def).cloned().unwrap_or_default(),
                self.declared_sites.get(&def).cloned().unwrap_or_default(),
                self.effects.is_unverifiable(def),
            ),
            None => self.lower_row(&function.contract.uses),
        };

        let performed = self.infer_block(&function.body);
        if let Some(def) = def {
            self.effects.performed.insert(def, performed.clone());
        }

        if unverifiable {
            return;
        }

        // Too narrow.
        for item in performed.iter() {
            if declared.covers(item) {
                continue;
            }
            let described = self.describe(item);
            let name = function.sig.name.name.clone();
            self.emit(
                Diagnostic::error(
                    codes::UNDECLARED_EFFECT,
                    self.file,
                    function.sig.name.span,
                    format!("`{name}` performs {described} without declaring it"),
                )
                .with_primary_label(format!("add {described} to the `uses` clause"))
                .with_note("a function can only do what its signature admits to"),
            );
        }

        // Too wide.
        for (item, span) in &sites {
            if performed.iter().any(|done| item.covers(done)) {
                continue;
            }
            let described = self.describe(item);
            self.emit(
                Diagnostic::error(
                    codes::UNUSED_EFFECT,
                    self.file,
                    *span,
                    format!("{described} is declared but never performed"),
                )
                .with_primary_label("not used")
                .with_note(
                    "an effect row is only worth reading if it is tight, so declaring authority that is not used is an error rather than a warning",
                ),
            );
        }
    }

    // -- inference ---------------------------------------------------------

    fn infer_block(&mut self, block: &Block) -> Row {
        let mut row = Row::new();
        for stmt in &block.stmts {
            row.extend(&self.infer_stmt(stmt));
        }
        if let Some(tail) = &block.tail {
            row.extend(&self.infer_expr(tail));
        }
        row
    }

    fn infer_stmt(&mut self, stmt: &Stmt) -> Row {
        match stmt {
            Stmt::Let { init, .. } => self.infer_expr(init),
            Stmt::Assign { value, .. } => self.infer_expr(value),
            Stmt::Return { value, .. } => match value {
                Some(value) => self.infer_expr(value),
                None => Row::new(),
            },
            Stmt::Assert { condition, .. } => self.infer_expr(condition),
            Stmt::Expr(expr) => self.infer_expr(expr),
            Stmt::Error(_) => Row::new(),
        }
    }

    fn infer_expr(&mut self, expr: &Expr) -> Row {
        let mut row = Row::new();

        match expr {
            Expr::Int { .. }
            | Expr::Str { .. }
            | Expr::Bool { .. }
            | Expr::Unit(_)
            | Expr::Ident(_)
            | Expr::Error(_) => {}

            Expr::Field { receiver, .. } => row.extend(&self.infer_expr(receiver)),

            Expr::Call { callee, args, .. } => {
                if let Some(performed) = self.call_effects(callee) {
                    row.extend(&performed);
                } else {
                    row.extend(&self.infer_expr(callee));
                }
                for arg in args {
                    row.extend(&self.infer_expr(arg));
                }
            }

            Expr::StructLit { path, fields, .. } => {
                row.extend(&self.infer_expr(path));
                for field in fields {
                    if let Some(value) = &field.value {
                        row.extend(&self.infer_expr(value));
                    }
                }
            }

            Expr::Unary { operand, .. } => row.extend(&self.infer_expr(operand)),
            Expr::Binary { lhs, rhs, .. } => {
                row.extend(&self.infer_expr(lhs));
                row.extend(&self.infer_expr(rhs));
            }
            Expr::Try { operand, .. } => row.extend(&self.infer_expr(operand)),

            Expr::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                row.extend(&self.infer_expr(condition));
                row.extend(&self.infer_block(then_branch));
                if let Some(else_branch) = else_branch {
                    row.extend(&self.infer_expr(else_branch));
                }
            }

            Expr::Match {
                scrutinee, arms, ..
            } => {
                row.extend(&self.infer_expr(scrutinee));
                for arm in arms {
                    row.extend(&self.infer_expr(&arm.body));
                }
            }

            Expr::Block(block) => row.extend(&self.infer_block(block)),

            // A closure performs its effects when it is called, not when it is
            // written. Attributing them here would be wrong, and attributing
            // them at the call site needs the row to be part of the type. That
            // is not built yet and it is an open question in 03-effects.
            Expr::Closure { .. } => {}

            // Specification, not action. See the note at the top of this file.
            Expr::Old { .. } | Expr::Unchanged { .. } => {}

            Expr::With { handlers, body, .. } => {
                let mut handled: HashSet<DefId> = HashSet::new();
                let mut handles_everything = false;

                for handler in handlers {
                    row.extend(&self.infer_expr(handler));
                    match self.handled_effect(handler) {
                        Some(effect) => {
                            handled.insert(effect);
                        }
                        None => handles_everything = true,
                    }
                }

                let inside = self.infer_block(body);
                if !handles_everything {
                    for item in inside.iter() {
                        if !handled.contains(&item.effect) {
                            row.insert(item.clone());
                        }
                    }
                }
            }
        }

        row
    }

    /// Effects performed by calling `callee`, when that can be worked out.
    ///
    /// `None` means the callee is not something with a known row, and the
    /// caller should walk it as an ordinary expression instead.
    fn call_effects(&mut self, callee: &Expr) -> Option<Row> {
        let def = match callee {
            Expr::Ident(ident) => self.resolutions.resolution(ident.span),
            Expr::Field { name, .. } => self.resolutions.resolution(name.span),
            _ => None,
        }?;

        match self.kind_of(def) {
            DefKind::EffectOp => {
                let effect = self.resolutions.def(def).parent?;
                let mut row = Row::new();
                row.insert(EffectItem::operation(effect, self.name_of(def)));
                Some(row)
            }
            // A function's declared row is its contract. Using the declaration
            // rather than the inferred row keeps this modular and means
            // recursion needs no fixpoint.
            DefKind::Function => Some(self.effects.declared.get(&def).cloned().unwrap_or_default()),
            _ => None,
        }
    }

    /// The effect a handler expression implements, or `None` when that cannot
    /// be known, in which case it is treated as handling everything.
    fn handled_effect(&self, handler: &Expr) -> Option<DefId> {
        let def = match handler {
            Expr::Ident(ident) => self.resolutions.resolution(ident.span),
            Expr::StructLit { path, .. } => match &**path {
                Expr::Ident(ident) => self.resolutions.resolution(ident.span),
                Expr::Field { name, .. } => self.resolutions.resolution(name.span),
                _ => None,
            },
            _ => None,
        }?;

        self.handler_effects.get(&def).copied()
    }
}
