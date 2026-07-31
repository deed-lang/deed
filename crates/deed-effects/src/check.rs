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
//!
//! **Unmentioned in a contract.** What that decision cannot mean is that a
//! clause may perform an effect the signature says nothing about. Installing a
//! handler is the caller's job and the signature is the only place a caller
//! learns one is needed, so such a call passes every check here and then has
//! nowhere to send the operation. The row may still be narrower than the
//! clauses, which is the decision holding: the requirement is that the effect
//! is named, not that the operation is.

use std::collections::{HashMap, HashSet};

use deed_ast::{Block, EffectRef, Expr, FnDecl, Item, Module, Pattern, Stmt, Type};
use deed_diagnostics::{Diagnostic, FileId, Span};
use deed_resolve::{DefId, DefKind, Resolutions, RowEntry};

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
    /// The same, keyed by where the name was written.
    ///
    /// A handler operation has no definition of its own, so it is not in
    /// `declared` and cannot be. A span is what both halves of it have.
    declared_at: HashMap<Span, Row>,
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

    /// Every function declared in this module and the row it wrote down.
    ///
    /// For handing the rows to something that runs the program, so that what
    /// actually happens can be compared against what was promised. The rows
    /// are the point of the language, and until this existed the only thing
    /// checking them was the pass that produced them.
    ///
    /// Keyed by where the name was written, because a handler operation has no
    /// definition of its own and is the half worth checking.
    pub fn declarations(&self) -> impl Iterator<Item = (Span, &Row)> {
        self.declared_at.iter().map(|(span, row)| (*span, row))
    }

    /// Whether the row for this function could not be checked at all.
    pub fn is_unverifiable(&self, function: DefId) -> bool {
        self.unverifiable.contains(&function)
    }
}

/// Checks one resolved module.
///
/// `row_required` says, for each expression span the type checker saw a
/// function value cross into a function type, which effects that type left
/// room for. Deciding which values owe a row needs types and settling whether
/// one is kept needs rows, so the two passes each answer the half they can.
///
/// `function_rows` is the same handoff the other way round: for each expression
/// whose type is a function type with a row, what calling it performs. A row is
/// part of a type, so the pass that works out types is the one that knows.
pub fn analyse(
    file: FileId,
    module: &Module,
    resolutions: &Resolutions,
    row_required: &HashMap<Span, Vec<RowEntry>>,
    function_rows: &HashMap<Span, Vec<RowEntry>>,
) -> Analysis {
    let mut checker = Checker {
        file,
        resolutions,
        here: module
            .name
            .as_ref()
            .map(|name| name.to_string_path())
            .unwrap_or_default(),
        effects: Effects::default(),
        diagnostics: Vec::new(),
        handler_effects: HashMap::new(),
        handler_rows: HashMap::new(),
        declared_sites: HashMap::new(),
        recursive: HashSet::new(),
        row_required: row_required.clone(),
        function_rows: function_rows.clone(),
        checked_rows: HashSet::new(),
        row_from: HashMap::new(),
        closure_rows: HashMap::new(),
        in_contract: false,
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
    /// What this module calls itself.
    ///
    /// A row entry arriving from another module names the module the effect
    /// was declared in. When that is this one, the effect is local and no
    /// import is needed to name it.
    here: String,
    effects: Effects,
    diagnostics: Vec<Diagnostic>,
    /// Local handler definition to the effect it implements.
    handler_effects: HashMap<DefId, DefId>,
    /// Local handler definition to what implementing it performs.
    ///
    /// A handler is code, and the code has a row. `with` discharges the effect
    /// the handler implements, which is what a handler is for, and says nothing
    /// about what the handler does to implement it. Those effects belong to
    /// whoever installed it, because installing it is the decision that caused
    /// them.
    handler_rows: HashMap<DefId, Row>,
    /// Where each declared entry was written, for diagnostics.
    declared_sites: HashMap<DefId, Vec<(EffectItem, Span)>>,
    /// Functions that can reach themselves, so may not return.
    recursive: HashSet<DefId>,
    /// What each expression handing over a function value is allowed to
    /// perform, as the type it is crossing into wrote it.
    row_required: HashMap<Span, Vec<RowEntry>>,
    /// What calling the function value at each span performs, as its own type
    /// wrote it.
    ///
    /// The answer for every function value that did not come from a closure
    /// written on the spot. Deriving it from the shape of the expression
    /// instead used to leave five routes out, one per test under "where a
    /// function value can come from" in `function_rows.rs`, and each of those
    /// routes was a function that performed an effect and declared nothing.
    function_rows: HashMap<Span, Vec<RowEntry>>,
    /// Expressions already complained about, so that walking one twice does
    /// not say the same thing twice.
    ///
    /// A closure argument is walked once as an argument and once as the value
    /// a row variable stands for, which are two questions about one piece of
    /// code and deserve one answer.
    checked_rows: HashSet<Span>,
    /// Which parameters' rows flow into each local function's own row.
    ///
    /// Positions, the same way they cross a module boundary. A call charges
    /// itself with whatever it passed at each of these, which is what makes a
    /// row variable mean anything outside the declaration that wrote it.
    row_from: HashMap<DefId, Vec<usize>>,
    /// The row of each local bound to a closure, within the body being checked.
    ///
    /// A closure is the one value in the language that holds code, and a name
    /// bound to one is the only thing that can stand for it. Kept per body,
    /// because a local from one body means nothing in another.
    closure_rows: HashMap<DefId, Row>,
    /// Whether the expression being walked is a `where` or `ensures` clause.
    ///
    /// Only two things read it, and both are about `old(...)`. A clause is the
    /// one place `old` is allowed, and the expression inside it does run, on
    /// entry, so walking a contract has to go in where walking a body steps
    /// over.
    in_contract: bool,
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
            .is_some_and(|export| export.kind == deed_resolve::ExportKind::Effect)
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
                    // What implementing the effect costs. A `with` block
                    // discharges the effect the handler implements and not the
                    // ones the handler itself performs, and those have to be
                    // charged to somebody or a function holding a `Console`
                    // could install a handler that writes to it and still
                    // declare an empty row.
                    if let Some(def) = self.resolutions.resolution(handler.name.span) {
                        let mut row = Row::new();
                        for operation in &handler.operations {
                            let (performed, _, _) = self.lower_row(&operation.contract.uses);
                            row.extend(&performed);
                        }
                        self.handler_rows.insert(def, row);
                    }
                }
                Item::Function(function) => {
                    self.check_row_variables(&function.sig);
                    let Some(def) = self.resolutions.resolution(function.sig.name.span) else {
                        continue;
                    };
                    let (row, sites, unverifiable) = self.lower_row(&function.contract.uses);
                    if unverifiable {
                        self.effects.unverifiable.insert(def);
                    }
                    self.effects.declared.insert(def, row);
                    self.declared_sites.insert(def, sites);
                    self.row_from.insert(
                        def,
                        deed_resolve::exports::row_sources(&function.sig, &function.contract),
                    );
                }
                _ => {}
            }
        }
    }

    /// Checks that every row variable is somewhere a call site can fill in.
    ///
    /// The legal positions are the row of a parameter whose type is a function
    /// type, and the declaration's own `uses` clause. Both are readable from
    /// the call: the first is the argument, the second is what the call is
    /// charged with. Everywhere else, the variable arrives at a caller standing
    /// for something the caller has no way to name, so it would be dropped, and
    /// a dropped entry in a row is an effect that happens and is not declared.
    ///
    /// This is what `row_sources` has always assumed. Saying it out loud turns
    /// an assumption the code relies on into a rule a reader can check against.
    fn check_row_variables(&mut self, sig: &deed_ast::FnSig) {
        if sig.rows.is_empty() {
            return;
        }
        let names: Vec<&str> = sig.rows.iter().map(|row| row.name.as_str()).collect();

        let mut misplaced = Vec::new();
        for param in &sig.params {
            // The row of the parameter's own function type is the one place a
            // variable belongs, so it is skipped and everything under it is not.
            match &param.ty {
                Some(Type::Fn { params, ret, .. }) => {
                    for nested in params {
                        find_row_variables(nested, &names, &mut misplaced);
                    }
                    find_row_variables(ret, &names, &mut misplaced);
                }
                Some(ty) => find_row_variables(ty, &names, &mut misplaced),
                None => {}
            }
        }
        if let Some(ret) = &sig.ret {
            find_row_variables(ret, &names, &mut misplaced);
        }

        for (name, span) in misplaced {
            self.emit(
                Diagnostic::error(
                    codes::MISPLACED_ROW_VARIABLE,
                    self.file,
                    span,
                    format!("`{name}` is a row variable, and this is not a place a caller could work out what it stands for"),
                )
                .with_primary_label("nothing at the call site says what this is")
                .with_note(
                    "a row variable stands for whatever a callback performs, so it belongs in the row of a parameter that is one, and in the `uses` clause; written anywhere else it reaches a caller as an effect that caller has no name for",
                ),
            );
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
                Stmt::Refuses { subject, .. } => self.calls_in(subject, found),
                Stmt::Abandon { .. } => {}
                Stmt::Expr(expr) => self.calls_in(expr, found),
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
            Expr::List { elements, .. } => {
                for element in elements {
                    self.calls_in(element, found);
                }
            }
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
            Expr::For {
                iterable,
                accumulator,
                keep,
                body,
                ..
            } => {
                self.calls_in(iterable, found);
                if let Some(accumulator) = accumulator {
                    self.calls_in(&accumulator.init, found);
                }
                if let Some(keep) = keep {
                    self.calls_in(keep, found);
                }
                self.calls_in_block(body, found);
            }
            Expr::With {
                handlers,
                body,
                finally,
                ..
            } => {
                for handler in handlers {
                    self.calls_in(handler, found);
                }
                self.calls_in_block(body, found);
                if let Some(finally) = finally {
                    self.calls_in_block(finally, found);
                }
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
                // A row variable, which stands for whatever the callback it is
                // attached to performs. Carried as an item like any other, so
                // that the body of the function that declared it is checked
                // the ordinary way: calling the callback performs `r`, and the
                // contract has to say `uses r`. What `r` turned out to be is a
                // question for the call site, and it is answered there.
                DefKind::RowParam => {
                    let item = EffectItem::whole(def);
                    row.insert(item.clone());
                    sites.push((item, entry.span));
                }

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

                // Anything else from another module is not an effect, and this
                // is the same mistake as naming a local record: the export
                // says which kind of thing the name is, so the answer is as
                // definite here as it is for a declaration in this file. The
                // module boundary changes where the answer comes from, not how
                // sure of it the compiler is, so it does not change the
                // severity either. It used to be a warning, which meant one
                // wrong `uses` entry quietly switched off effect checking for
                // the whole function.
                DefKind::Import => match self.resolutions.import(def).map(|export| export.kind) {
                    Some(kind) => {
                        let name = self.name_of(def);
                        self.emit(
                            Diagnostic::error(
                                codes::NOT_AN_EFFECT,
                                self.file,
                                entry.span,
                                format!("`{name}` is {}, not an effect", kind.describe()),
                            )
                            .with_primary_label("not an effect")
                            .with_note(
                                "a `uses` clause names effects declared with `effect`, and an \
                                 imported one is checked the same way a local one is",
                            ),
                        );
                    }
                    // Nothing behind the import, so what it is was never known
                    // here. Name resolution already complained, and guessing on
                    // top of that would be a second message about one mistake.
                    None => unverifiable = true,
                },

                // A capability value in a `uses` clause. Two spellings, one
                // subject: `sys.*` is the hole design/04-capabilities.md
                // already worried about, and a bare `sys` is that hole with
                // the star left off. Neither is an effect the row can be
                // checked against, and the compiler saying so out loud is
                // better than reporting a clean check it never did. This stays
                // a warning where an imported non-effect is an error, because
                // what is written here is a value that really does carry
                // authority, and the question of what a row should say about
                // one is still open in that document.
                DefKind::Param | DefKind::Local => {
                    unverifiable = true;
                    let name = self.name_of(def);
                    let (what, note) = if entry.all {
                        (
                            format!("`{name}.*` grants everything that capability carries"),
                            "granting everything is the same as promising nothing; see the open questions in design/04-capabilities.md",
                        )
                    } else {
                        (
                            format!("`{name}` is a value, not an effect"),
                            "a row names effects, not the capabilities they are performed with; `sys.*` is how a row admits it grants everything one carries",
                        )
                    };
                    self.emit(
                        Diagnostic::warning(
                            codes::UNVERIFIABLE_ROW,
                            self.file,
                            entry.span,
                            format!("{what}, so this function's row is not checked"),
                        )
                        .with_primary_label("not checked")
                        .with_note(note),
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

        // Keyed by where the name was written, so that a handler operation is
        // in the table too. It has no definition of its own, which is why the
        // row is lowered here, and it is exactly the place an effect gets
        // implemented, so leaving it out of what crosses to the runtime would
        // leave the interesting half unchecked.
        self.effects
            .declared_at
            .insert(function.sig.name.span, declared.clone());

        // A parameter of function type carries its row in its type, and
        // calling it performs whatever that row says. Without this the row
        // would be checked where the value was handed over and then forgotten,
        // so a function taking `Fn() uses Log.note -> ()` and calling it could
        // declare nothing itself.
        for param in &function.sig.params {
            let Some(Type::Fn { row, .. }) = &param.ty else {
                continue;
            };
            let Some(def) = self.resolutions.resolution(param.name.span) else {
                continue;
            };
            let (row, _, _) = self.lower_row(row);
            self.closure_rows.insert(def, row);
        }

        let mut performed = self.infer_block(&function.body);

        // What the clauses perform, kept apart from the body's row rather than
        // added to it. The decision that a contract does not contribute is
        // still in force: this is not an entry the row has to carry, it is a
        // question about an entry the row has to already have.
        let in_contract = self.contract_row(&function.contract);

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

        // Unmentioned in a contract.
        //
        // By effect and not by operation, which is forced rather than chosen:
        // asking for the operation would ask for a row entry that the too wide
        // rule below then rejects, since the body is not the thing performing
        // it. `examples/transfer.deed` reads `Ledger.total()` in an `ensures`
        // clause and declares three other entries, and it stays legal because
        // `Ledger` is among them.
        let mut said = HashSet::new();
        for (item, span) in &in_contract {
            if declared.iter().any(|entry| entry.effect == item.effect) || !said.insert(item.effect)
            {
                continue;
            }
            let effect = self.name_of(item.effect);
            let described = self.describe(item);
            let name = function.sig.name.name.clone();

            self.emit(
                Diagnostic::error(
                    codes::CONTRACT_EFFECT_NOT_DECLARED,
                    self.file,
                    *span,
                    format!(
                        "this performs {described}, and `{name}` does not mention `{effect}`"
                    ),
                )
                .with_primary_label(format!("`{effect}` is not in the `uses` clause"))
                .with_secondary(function.sig.name.span, "the signature a caller reads")
                .with_note(
                    "a handler is installed by the caller, and the signature is the only place a caller learns one is needed, so a clause cannot reach an effect the row is silent about",
                )
                .with_note(
                    "naming the effect is enough; the row does not have to list the operation, because a contract still does not contribute to it",
                ),
            );
        }

        // Too wide.
        for (item, span) in &sites {
            if performed.iter().any(|done| item.covers(done))
                || in_contract.iter().any(|(done, _)| item.covers(done))
            {
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

    /// What a function's `where` and `ensures` clauses perform, and where.
    ///
    /// One entry per thing performed, carrying the span of the clause that
    /// performed it, because that is the line the reader has to change and the
    /// signature above it is already the secondary label.
    ///
    /// `unchanged(E)` is not in here. It reads the state captured on entry and
    /// compares it, rather than asking a handler for anything, so it is the one
    /// piece of contract syntax that names an effect without reaching for one.
    fn contract_row(&mut self, contract: &deed_ast::Contract) -> Vec<(EffectItem, Span)> {
        let was = std::mem::replace(&mut self.in_contract, true);
        let mut found = Vec::new();
        let clauses = contract
            .requires
            .iter()
            .map(|clause| (clause, clause.span()))
            .chain(
                contract
                    .ensures
                    .iter()
                    .map(|clause| (&clause.condition, clause.span)),
            );
        for (clause, span) in clauses.collect::<Vec<_>>() {
            for item in self.infer_expr(clause).iter() {
                found.push((item.clone(), span));
            }
        }
        self.in_contract = was;
        found
    }

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
            Stmt::Let { pattern, init, .. } => {
                let row = self.infer_expr(init);
                self.remember_closure(pattern, init);
                row
            }
            Stmt::Assign { value, .. } => self.infer_expr(value),
            Stmt::Return { value, .. } => match value {
                Some(value) => self.infer_expr(value),
                None => Row::new(),
            },
            Stmt::Assert { condition, .. } => self.infer_expr(condition),
            // Asserting that something breaks its contract still runs it, so
            // what it performs is performed.
            Stmt::Refuses { subject, .. } => self.infer_expr(subject),
            Stmt::Abandon { .. } => Row::new(),
            Stmt::Expr(expr) => self.infer_expr(expr),
        }
    }

    /// Notes the row of a closure a `let` gave a name to.
    ///
    /// The name is the only thing that can stand for the closure afterwards,
    /// so without this a closure passed on by name is a function value nobody
    /// can say anything about.
    fn remember_closure(&mut self, pattern: &Pattern, init: &Expr) {
        let Pattern::Path { segments, .. } = pattern else {
            return;
        };
        let [only] = segments.as_slice() else {
            return;
        };
        let Some(def) = self.resolutions.resolution(only.span) else {
            return;
        };

        let row = self.function_value_row(init);
        if matches!(init, Expr::Closure { .. } | Expr::Ident(_)) {
            self.closure_rows.insert(def, row);
        }
    }

    fn infer_expr(&mut self, expr: &Expr) -> Row {
        if let Some(allowed) = self.row_required.get(&expr.span()).cloned() {
            self.check_row(expr, &allowed);
        }
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
                if let Some(performed) = self.call_effects(callee, args) {
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

            Expr::List { elements, .. } => {
                for element in elements {
                    row.extend(&self.infer_expr(element));
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

            // A `for` walks a list that is already there, so it stops. That
            // is the whole point of it being a fold rather than a loop: the
            // alternative to having one is recursion, and recursion has to
            // declare `Diverge` every time, which is how a row that was
            // supposed to mean something drifts into meaning nothing.
            Expr::For {
                iterable,
                accumulator,
                keep,
                body,
                ..
            } => {
                row.extend(&self.infer_expr(iterable));
                if let Some(accumulator) = accumulator {
                    row.extend(&self.infer_expr(&accumulator.init));
                }
                // Read once per turn, so whatever it performs is performed by
                // the walk. Charged once, like the body, since a row says what
                // can happen rather than how often.
                if let Some(keep) = keep {
                    row.extend(&self.infer_expr(keep));
                }
                row.extend(&self.infer_block(body));
            }

            // A closure performs its effects when it is called, and the honest
            // place to charge them is the call site. That is what happens for
            // a closure that has been handed over: its type carries a row, the
            // row is checked where it crossed, and calling it charges the
            // caller.
            //
            // Charging the function that wrote the closure as well is the
            // conservative answer for the case where it never crosses
            // anywhere. It over-approximates in one direction only, so a
            // closure defined and never called still charges its author.
            // Removing it means deciding what a closure that escapes without
            // an annotation performs, which is the row variable question
            // 03-effects has been putting off.
            Expr::Closure { body, .. } => row.extend(&self.infer_expr(body)),

            // Specification, not action. See the note at the top of this file.
            //
            // Except that `old(e)` does evaluate `e`, on entry, and a contract
            // is the only place it can appear. So a body steps over this and a
            // contract walk goes in: what a clause performs is the question
            // DEED5009 is asking, and `old(Counter.value())` performs it.
            Expr::Old { expr, .. } if self.in_contract => row.extend(&self.infer_expr(expr)),
            Expr::Old { .. } | Expr::Unchanged { .. } => {}

            Expr::With {
                handlers,
                body,
                finally,
                ..
            } => {
                let mut handled: HashSet<DefId> = HashSet::new();
                let mut handles_everything = false;
                // What the handlers themselves perform, kept with the body's
                // row rather than added straight to this function's, so that
                // installing a handler for `Log` whose operations perform
                // `Log` is answered by the same handler like anything else.
                let mut inside = Row::new();

                for handler in handlers {
                    row.extend(&self.infer_expr(handler));
                    inside.extend(&self.handler_row(handler));
                    match self.handled_effect(handler) {
                        Some(effect) => {
                            handled.insert(effect);
                        }
                        None => handles_everything = true,
                    }
                }

                inside.extend(&self.infer_block(body));
                if !handles_everything {
                    for item in inside.iter() {
                        if !handled.contains(&item.effect) {
                            row.insert(item.clone());
                        }
                    }
                }

                // The `finally` clause runs after the body and may perform
                // effects too; they are charged to whoever installed the
                // handler.
                if let Some(finally) = finally {
                    row.extend(&self.infer_block(finally));
                }
            }
        }

        row
    }

    /// The row of the function value at `span`, as its type wrote it.
    ///
    /// A row variable is dropped, because it names nothing here: what it stood
    /// for is decided at the call that supplied it, and a value carrying one
    /// out of that call is refused where it is declared. See
    /// [`Checker::check_row_variables`].
    fn row_from_type(&mut self, span: Span) -> Row {
        let Some(entries) = self.function_rows.get(&span).cloned() else {
            return Row::new();
        };
        entries
            .iter()
            .filter_map(|entry| self.item_for(entry))
            .collect()
    }

    /// The row of a function value.
    ///
    /// A closure written on the spot is the one shape whose row is not in its
    /// type: the type checker records it as fitting anywhere and leaves working
    /// out what it does to this pass. Everything else has a row because it has
    /// a type, and reading it off the type is the whole answer.
    ///
    /// This used to match on the shape of the expression and return an empty
    /// row for anything it did not recognise. An empty row means "performs
    /// nothing", so the shapes it did not recognise were a function that came
    /// back from a call, a branch of an `if`, an element of a list and a field
    /// of a record, each of which could perform an effect through a caller that
    /// declared none.
    fn function_value_row(&mut self, expr: &Expr) -> Row {
        match expr {
            Expr::Closure { body, .. } => self.infer_expr(body),
            Expr::Ident(ident) => {
                let Some(def) = self.resolutions.resolution(ident.span) else {
                    return self.row_from_type(expr.span());
                };
                match self.kind_of(def) {
                    DefKind::Function => {
                        self.effects.declared.get(&def).cloned().unwrap_or_default()
                    }
                    DefKind::Import => {
                        let Some(export) = self.resolutions.import(def) else {
                            return Row::new();
                        };
                        if export.kind != deed_resolve::ExportKind::Function {
                            return Row::new();
                        }
                        let entries = export.row.clone();
                        self.translate(&entries, ident.span)
                    }
                    // A local bound to a closure has a row this pass worked
                    // out and the type checker did not. Anything else gets the
                    // same answer as every other expression.
                    _ => match self.closure_rows.get(&def).cloned() {
                        Some(row) => row,
                        None => self.row_from_type(expr.span()),
                    },
                }
            }
            _ => self.row_from_type(expr.span()),
        }
    }

    /// Checks that a value crossing into a function type stays inside the row
    /// that type wrote down.
    ///
    /// `Fn(Int) -> Int` promises to perform nothing, and
    /// `Fn(Int) uses Log.note -> Int` promises to perform no more than that.
    /// Leaving a row off cannot mean "any row": a value carrying an unstated
    /// effect through a signature would undo the point of having rows.
    fn check_row(&mut self, expr: &Expr, allowed: &[RowEntry]) {
        if !self.checked_rows.insert(expr.span()) {
            return;
        }
        let row = self.function_value_row(expr);
        if row.is_empty() {
            return;
        }

        // Entries the type wrote down that this module has no name for cannot
        // match anything the value performs anyway, because performing them
        // here would need a name for them too.
        let permitted: Row = allowed
            .iter()
            .filter_map(|entry| self.item_for(entry))
            .collect();

        let over: Vec<String> = row
            .iter()
            .filter(|item| !permitted.covers(item))
            .map(|item| self.describe(item))
            .collect();
        if over.is_empty() {
            return;
        }

        let performed = over.join(", ");
        let room = if permitted.is_empty() {
            "a function type with no row promises nothing".to_string()
        } else {
            let named: Vec<String> = permitted.iter().map(|item| self.describe(item)).collect();
            format!(
                "this function type leaves room only for {}",
                named.join(", ")
            )
        };

        self.emit(
            Diagnostic::error(
                codes::IMPURE_FUNCTION_VALUE,
                self.file,
                expr.span(),
                format!("this performs {performed}, and {room}"),
            )
            .with_primary_label("performs an effect the type does not allow")
            .with_note(
                "write the effect into the function type, as in `Fn(Int) uses Log.note -> Int`; leaving a row off cannot mean any row, because a value carrying an unstated effect through a signature would undo the point of having rows",
            ),
        );
    }

    /// Effects performed by calling `callee`, when that can be worked out.
    ///
    /// `None` means the callee is not something with a known row, and the
    /// caller should walk it as an ordinary expression instead.
    ///
    /// `args` is what makes a row variable mean something. A declared row
    /// holding one is a row with a hole in it, and the hole is filled by
    /// whatever was passed at the parameter the variable came from.
    fn call_effects(&mut self, callee: &Expr, args: &[Expr]) -> Option<Row> {
        let def = match callee {
            Expr::Ident(ident) => self.resolutions.resolution(ident.span),
            Expr::Field { name, .. } => self.resolutions.resolution(name.span),
            _ => None,
        };
        // Not a name at all, so there is no declaration to read and the type is
        // the only thing that knows. `pass(logs)(n)` is this shape.
        let Some(def) = def else {
            let row = self.row_from_type(callee.span());
            return (!row.is_empty()).then_some(row);
        };

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
            DefKind::Function => {
                let declared = self.effects.declared.get(&def).cloned().unwrap_or_default();
                let sources = self.row_from.get(&def).cloned().unwrap_or_default();
                Some(self.fill_row(declared, &sources, args))
            }
            // A function from another module. Its row is its contract too, and
            // it is in the export, so a call into another file is no longer
            // free. It used to be, which meant the effect system stopped at
            // the module boundary and therefore stopped where most calls are.
            DefKind::Import => {
                let export = self.resolutions.import(def)?;
                if export.kind != deed_resolve::ExportKind::Function {
                    return None;
                }
                let entries: Vec<RowEntry> = export
                    .row
                    .iter()
                    // A row variable names nothing on the far side. What it
                    // stood for is the argument, and that is filled in below.
                    .filter(|entry| !entry.variable)
                    .cloned()
                    .collect();
                let sources = export.row_from.clone();
                let complete = export.row_complete;
                let span = callee.span();

                // The export dropped a starred entry, so what came across is
                // not the whole row. Inheriting it silently would move the
                // loophole rather than close it: the caller would look pure.
                //
                // `row_complete` is the star and only the star, so this fires
                // for a callee whose row said `Log.*` even though that one was
                // checked where it was written, and not at all for one whose
                // row named a bare capability even though that one was not.
                // See `codes::UNVERIFIABLE_ROW` for why that is written down
                // rather than fixed.
                if !complete {
                    let name = self.name_of(def);
                    self.emit(
                        Diagnostic::warning(
                            codes::UNVERIFIABLE_ROW,
                            self.file,
                            span,
                            format!(
                                "`{name}` has a row that is not checked, so this call is not either"
                            ),
                        )
                        .with_primary_label("not checked")
                        .with_note(
                            "granting everything is the same as promising nothing; see the open questions in design/04-capabilities.md",
                        ),
                    );
                }

                let translated = self.translate(&entries, span);
                Some(self.fill_row(translated, &sources, args))
            }
            // A name bound to a function value: a parameter whose type wrote a
            // row, a local bound to a closure, or anything else the type
            // checker gave a function type. Calling it performs what it said it
            // would.
            DefKind::Param | DefKind::Local => match self.closure_rows.get(&def).cloned() {
                Some(row) => Some(row),
                None => {
                    let row = self.row_from_type(callee.span());
                    (!row.is_empty()).then_some(row)
                }
            },
            _ => None,
        }
    }

    /// A declared row with its variables replaced by what was passed.
    ///
    /// A row variable is a hole in a row: the declaration said "whatever the
    /// callback does", and only the call site knows what that was. So the
    /// variable itself is dropped, because it names nothing a caller could
    /// declare, and the row of whatever was passed at each of `sources` goes
    /// in instead.
    ///
    /// This is what makes one `map` work for a callback that logs, a callback
    /// that reads a file and a callback that does neither, rather than there
    /// being three of them.
    ///
    /// `sources` is a list rather than a position because one variable may
    /// appear in more than one parameter, and when it does the answer is the
    /// union of what was passed at each. It is not an equality: two callbacks
    /// sharing a variable are not made to perform the same things, they are
    /// two places a row is read off and the caller is charged with the sum.
    fn fill_row(&mut self, declared: Row, sources: &[usize], args: &[Expr]) -> Row {
        if sources.is_empty() {
            return declared;
        }

        let mut row: Row = declared
            .iter()
            .filter(|item| self.kind_of(item.effect) != DefKind::RowParam)
            .cloned()
            .collect();

        for index in sources {
            if let Some(arg) = args.get(*index) {
                let passed = self.function_value_row(arg);
                row.extend(&passed);
            }
        }
        row
    }

    /// This module's own way of saying one entry of a row from elsewhere.
    ///
    /// `None` when the effect is not in scope here, which is a mistake in some
    /// callers and simply not interesting in others, so saying so is left to
    /// them.
    fn item_for(&self, entry: &RowEntry) -> Option<EffectItem> {
        let effect = self.name_for(entry)?;
        Some(match &entry.operation {
            Some(operation) => EffectItem::operation(effect, operation.clone()),
            None => EffectItem::whole(effect),
        })
    }

    /// Turns a row from another module into one this module can talk about.
    ///
    /// Each entry names the module its effect was declared in, so the match is
    /// against a local declaration or an import of the same effect from the
    /// same place. When neither exists the caller performs something it has no
    /// word for, and a row it cannot write is a row it cannot promise.
    fn translate(&mut self, entries: &[RowEntry], span: Span) -> Row {
        let mut row = Row::new();
        for entry in entries {
            match self.item_for(entry) {
                Some(item) => {
                    row.insert(item);
                }
                None => {
                    let what = match &entry.operation {
                        Some(operation) => format!("`{}.{operation}`", entry.effect),
                        None => format!("`{}`", entry.effect),
                    };
                    let module = &entry.module;
                    let effect = &entry.effect;
                    self.emit(
                        Diagnostic::error(
                            codes::EFFECT_NOT_IMPORTED,
                            self.file,
                            span,
                            format!("this performs {what}, and `{effect}` is not in scope here"),
                        )
                        .with_primary_label(format!("add `use {module}.{{{effect}}}`"))
                        .with_note(
                            "a function cannot declare an effect it has no name for, and a row it cannot declare is one it cannot keep",
                        ),
                    );
                }
            }
        }
        row
    }

    /// This module's own handle on an effect declared somewhere.
    fn name_for(&self, entry: &deed_resolve::RowEntry) -> Option<DefId> {
        // The language provides it, so it is in scope everywhere and there is
        // nothing to import.
        if entry.module == deed_resolve::PRELUDE_MODULE {
            return self.resolutions.builtin(&entry.effect);
        }

        self.resolutions.defs().find_map(|(def, data)| {
            if data.name != entry.effect {
                return None;
            }
            match data.kind {
                // Declared right here, so no import is involved.
                DefKind::Effect if entry.module == self.here => Some(def),
                DefKind::Import
                    if self.is_imported_effect(def)
                        && self.resolutions.import_module(def) == Some(entry.module.as_str()) =>
                {
                    Some(def)
                }
                _ => None,
            }
        })
    }

    /// What installing a handler performs.
    ///
    /// A `with` block answers for the effect the handler implements. It does
    /// not answer for what the handler does to implement it: a handler for
    /// `Log` that writes to a console is a program writing to a console, and
    /// the function that chose to install it is the one that decided so. Before
    /// this, those effects were charged to nobody, so a function holding a
    /// `Console` could install such a handler and still declare an empty row,
    /// which is exactly the claim an empty row is not allowed to make.
    fn handler_row(&mut self, handler: &Expr) -> Row {
        let def = match handler {
            Expr::Ident(ident) => self.resolutions.resolution(ident.span),
            Expr::StructLit { path, .. } => match &**path {
                Expr::Ident(ident) => self.resolutions.resolution(ident.span),
                Expr::Field { name, .. } => self.resolutions.resolution(name.span),
                _ => None,
            },
            _ => None,
        };
        let Some(def) = def else {
            return Row::new();
        };

        if let Some(row) = self.handler_rows.get(&def) {
            return row.clone();
        }

        // A handler from another module. What it performs is part of its
        // declaration, so it travels in the export the same way a function's
        // row does, and for the same reason: without it the effect system
        // would stop at the module boundary, which is where most calls are.
        let Some(export) = self.resolutions.import(def) else {
            return Row::new();
        };
        if export.kind != deed_resolve::ExportKind::Handler {
            return Row::new();
        }
        let entries = export.row.clone();
        self.translate(&entries, handler.span())
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
        if export.kind != deed_resolve::ExportKind::Handler {
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

/// Every mention of one of `names` in a row anywhere inside `ty`.
///
/// Syntax alone. A row variable is a name in a `uses` list and nothing else
/// gives it away, so nothing has to be resolved first and this can run during
/// collection, before anything has been checked.
fn find_row_variables(ty: &Type, names: &[&str], found: &mut Vec<(String, Span)>) {
    match ty {
        Type::Named { args, .. } => {
            for arg in args {
                find_row_variables(arg, names, found);
            }
        }
        Type::Fn {
            params, row, ret, ..
        } => {
            for entry in row {
                if entry.operation.is_none()
                    && !entry.all
                    && names.contains(&entry.effect.name.as_str())
                {
                    found.push((entry.effect.name.clone(), entry.span));
                }
            }
            for param in params {
                find_row_variables(param, names, found);
            }
            find_row_variables(ret, names, found);
        }
        Type::Unit(_) | Type::Error(_) => {}
    }
}
