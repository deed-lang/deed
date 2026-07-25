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
use crate::cycles::{self, CallGraph};
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
        recursive: HashSet::new(),
    };

    checker.collect(module);
    checker.recursive = cycles::on_a_cycle(&checker.call_graph(module));
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
    /// Functions that can reach themselves, so may not return.
    recursive: HashSet<DefId>,
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

    /// Whether an imported name is an effect in the module it came from.
    fn is_imported_effect(&self, def: DefId) -> bool {
        self.resolutions
            .import(def)
            .is_some_and(|export| export.kind == vow_resolve::ExportKind::Effect)
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

    /// Who calls whom, among the functions declared in this module.
    ///
    /// Local calls only. A cycle that leaves the module and comes back is not
    /// visible here, and pretending otherwise would mean reading another
    /// module's bodies, which is exactly what a module boundary is for. The
    /// declared row is what crosses, so a function that admits to `Diverge`
    /// still passes it on to its callers wherever they are.
    fn call_graph(&self, module: &Module) -> CallGraph {
        let mut graph = CallGraph::new();
        for item in &module.items {
            let Item::Function(function) = item else {
                continue;
            };
            let Some(def) = self.resolutions.resolution(function.sig.name.span) else {
                continue;
            };
            let mut called = Vec::new();
            self.calls_in_block(&function.body, &mut called);
            graph.insert(def, called);
        }
        graph
    }

    fn calls_in_block(&self, block: &Block, found: &mut Vec<DefId>) {
        for stmt in &block.stmts {
            match stmt {
                Stmt::Let { init, .. } => self.calls_in(init, found),
                Stmt::Assign { value, .. } => self.calls_in(value, found),
                Stmt::Return { value, .. } => {
                    if let Some(value) = value {
                        self.calls_in(value, found);
                    }
                }
                Stmt::Assert { condition, .. } => self.calls_in(condition, found),
                Stmt::Expr(expr) => self.calls_in(expr, found),
                Stmt::Error(_) => {}
            }
        }
        if let Some(tail) = &block.tail {
            self.calls_in(tail, found);
        }
    }

    /// Every local function `expr` can call, including from inside a closure.
    ///
    /// Contract expressions are left out, the same way they are left out of the
    /// row. A `where` clause describing a recursive function does not run it.
    fn calls_in(&self, expr: &Expr, found: &mut Vec<DefId>) {
        match expr {
            Expr::Call { callee, args, .. } => {
                let def = match &**callee {
                    Expr::Ident(ident) => self.resolutions.resolution(ident.span),
                    Expr::Field { name, .. } => self.resolutions.resolution(name.span),
                    _ => None,
                };
                match def {
                    Some(def) if self.kind_of(def) == DefKind::Function => found.push(def),
                    _ => self.calls_in(callee, found),
                }
                for arg in args {
                    self.calls_in(arg, found);
                }
            }
            Expr::Field { receiver, .. } => self.calls_in(receiver, found),
            Expr::StructLit { path, fields, .. } => {
                self.calls_in(path, found);
                for field in fields {
                    if let Some(value) = &field.value {
                        self.calls_in(value, found);
                    }
                }
            }
            Expr::Unary { operand, .. } => self.calls_in(operand, found),
            Expr::Binary { lhs, rhs, .. } => {
                self.calls_in(lhs, found);
                self.calls_in(rhs, found);
            }
            Expr::Try { operand, .. } => self.calls_in(operand, found),
            Expr::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.calls_in(condition, found);
                self.calls_in_block(then_branch, found);
                if let Some(else_branch) = else_branch {
                    self.calls_in(else_branch, found);
                }
            }
            Expr::Match {
                scrutinee, arms, ..
            } => {
                self.calls_in(scrutinee, found);
                for arm in arms {
                    self.calls_in(&arm.body, found);
                }
            }
            Expr::Block(block) => self.calls_in_block(block, found),
            Expr::Closure { body, .. } => self.calls_in(body, found),
            Expr::With { handlers, body, .. } => {
                for handler in handlers {
                    self.calls_in(handler, found);
                }
                self.calls_in_block(body, found);
            }
            Expr::Int { .. }
            | Expr::Str { .. }
            | Expr::Bool { .. }
            | Expr::Unit(_)
            | Expr::Ident(_)
            | Expr::Old { .. }
            | Expr::Unchanged { .. }
            | Expr::Error(_) => {}
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

                // An effect from another module. Its operations are part of
                // its declaration and the resolver gives them definitions of
                // their own now, so a row naming one is checked the same way a
                // local one is. The `DefId` is this module's handle on the
                // import, and both the row and the body reach it through that
                // same handle, so they compare.
                DefKind::Import if self.is_imported_effect(def) => {
                    let item = match &entry.operation {
                        Some(operation) => EffectItem::operation(def, operation.name.clone()),
                        None => EffectItem::whole(def),
                    };
                    row.insert(item.clone());
                    sites.push((item, entry.span));
                }

                // Anything else from another module is not an effect, and the
                // resolver already said whether the name exists.
                DefKind::Import => {
                    unverifiable = true;
                    let name = self.name_of(def);
                    self.emit(
                        Diagnostic::warning(
                            codes::UNVERIFIABLE_ROW,
                            self.file,
                            entry.span,
                            format!("`{name}` is not an effect, so this row is not checked"),
                        )
                        .with_primary_label("not checked"),
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
                        // `Diverge` is the one effect with nothing to install.
                        // A test that calls something which may not return is
                        // running it on purpose, and there is no handler to
                        // suggest, so asking for one would be asking for a
                        // thing that cannot be written.
                        if self.resolutions.builtin("Diverge") == Some(item.effect) {
                            continue;
                        }
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

        let mut performed = self.infer_block(&function.body);

        // Not returning is something the function does, so it goes in the row
        // with everything else it does. There is no termination proving here:
        // a function that can reach itself may not return as far as this pass
        // is concerned, and `factorial` has to say so like anything else.
        if let Some(def) = def
            && self.recursive.contains(&def)
            && let Some(diverge) = self.resolutions.builtin("Diverge")
        {
            performed.insert(EffectItem::whole(diverge));
        }

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

            // `Diverge` is not something the body calls, so a message about
            // performing it would send the reader looking for a line that is
            // not there.
            let diverges = self.resolutions.builtin("Diverge") == Some(item.effect);
            let message = if diverges {
                format!("`{name}` can reach itself, so it may not return")
            } else {
                format!("`{name}` performs {described} without declaring it")
            };
            let note = if diverges {
                "nothing here proves termination, so any call cycle needs `Diverge`; see design/02-syntax.md"
            } else {
                "a function can only do what its signature admits to"
            };

            self.emit(
                Diagnostic::error(
                    codes::UNDECLARED_EFFECT,
                    self.file,
                    function.sig.name.span,
                    message,
                )
                .with_primary_label(format!("add {described} to the `uses` clause"))
                .with_note(note),
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

            // A closure performs its effects when it is called, and the honest
            // place to charge them is the call site. That needs the row to be
            // part of the closure's type, which is the row polymorphism
            // question 03-effects has been putting off.
            //
            // Charging the function that writes the closure is the
            // conservative answer, and it is sound rather than a guess because
            // a closure cannot leave the function that wrote it: there is no
            // syntax for a closure type, so it cannot be a parameter, a return
            // type or a field, and a parameter without a type is now an error.
            // It over-approximates in one direction only, so a closure defined
            // and never called still charges its author.
            Expr::Closure { body, .. } => row.extend(&self.infer_expr(body)),

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

        if let Some(effect) = self.handler_effects.get(&def).copied() {
            return Some(effect);
        }

        // A handler from another module. What it implements is part of its
        // declaration, so it comes across, and the effect it names is looked
        // up among this module's imports from that same module. When the
        // importer never imported the effect it cannot perform it either, so
        // there is nothing for the row to be wrong about.
        let export = self.resolutions.import(def)?;
        if export.kind != vow_resolve::ExportKind::Handler {
            return None;
        }
        let effect_name = export.members.first()?;
        let home = self.resolutions.import_module(def)?;

        self.resolutions.defs().find_map(|(id, data)| {
            (data.kind == DefKind::Import
                && data.name == *effect_name
                && self.resolutions.import_module(id) == Some(home)
                && self.is_imported_effect(id))
            .then_some(id)
        })
    }
}
