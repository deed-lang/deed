//! Name resolution.
//!
//! This is the first pass that asks what a program means rather than whether it
//! is well formed. It answers exactly one question, "what does this name refer
//! to", and leaves everything about types alone.
//!
//! It is also where the parser's deliberate ambiguities get settled. `a.b` is
//! module qualification or field access depending on what `a` turns out to be,
//! and a single segment pattern is a binding or a variant depending on the
//! same kind of question. The parser could not know. This pass can.

use std::collections::{HashMap, HashSet};

use vow_ast::{
    Block, ChoiceDecl, EffectDecl, EffectRef, Expr, FnDecl, HandlerDecl, Ident, Item, Module,
    Pattern, RecordDecl, Stmt, Type, TypeAlias,
};
use vow_diagnostics::{Applicability, Diagnostic, FileId, Span};

use crate::codes;
use crate::defs::{DefData, DefId, DefKind, Dot, Resolutions};
use crate::exports::{ExportKind, Universe};

/// Names the language provides without anyone importing them.
///
/// Deliberately tiny. Every entry is a name that cannot be looked up in any
/// file, which is exactly the kind of thing P2 is a budget for.
///
/// `Result`, `ok` and `err` are here rather than in a library because a
/// language where you cannot write a failing function without an import is not
/// finished. Errors as values and `?` are core to the design.
///
/// `System`, `Console`, `Clock` and `Dir` are capability types. They have to be
/// built in for the same reason there is no ambient authority: if a program
/// could declare its own `Console` and conjure one, none of the rest would mean
/// anything.
///
/// `length` is here because a `String` you cannot measure is a `String` you
/// cannot check. It measures a `List` too, for the same reason.
///
/// `List` is built in rather than declared because there is no way to declare
/// a generic type yet. Nothing else in the language can hold more than one of
/// something, and a language where the only way to have two of a thing is to
/// name two variables is not one anybody can write a program in. `at` and
/// `push` come with it: a list you cannot read out of and cannot extend is a
/// literal, not a collection. `at` hands back a `Result` rather than the
/// element, because an index that is not there is not a bug in the caller and
/// nothing in this language traps.
///
/// `split`, `join`, `to_string` and `to_int` are two pairs of inverses, and
/// they are here because until them a program could hold text and a number and
/// get from neither to the other. Nothing could take input apart, put output
/// together, or print a count, which is most of what a program does.
///
/// The prelude is a place names go to become unavailable to everyone else, so
/// it stays small and every addition to it is argued for rather than assumed.
pub const PRELUDE: &[&str] = &[
    "Int",
    "String",
    "Bool",
    "Result",
    "ok",
    "err",
    "length",
    "List",
    "at",
    "push",
    "split",
    "join",
    "to_string",
    "to_int",
    "System",
    "Console",
    "Clock",
    "Dir",
];

/// Operations of the built-in `Io` effect.
///
/// Each takes the capability it acts on. The row says what kind of operation,
/// the argument says which resource, which is the split `04-capabilities.md`
/// describes.
///
/// `save` writes a file and `write` writes to a console, and by that split they
/// should be one name with the resource in the argument. They are two because
/// a signature here is one list of types per name and there is no overloading,
/// so the limitation shows through the design rather than the other way round.
///
/// `args` is the odd one. It hands back data rather than doing anything, and
/// it takes the whole `System` rather than a narrower capability, so it reads
/// like it does not belong. It goes in the row anyway, because how a program
/// was invoked is input from outside and every other way of getting input from
/// outside says so in a signature. A program that reads its arguments behaves
/// differently depending on them, and that is worth writing down.
pub const IO_OPERATIONS: &[&str] = &["write", "now", "open", "read", "save", "args"];

/// The effects the language provides, available in every module without an
/// import.
///
/// A row naming one of these travels between modules under
/// [`PRELUDE_MODULE`], because the module that declared it is nobody's module
/// and every module can name it.
pub const PRELUDE_EFFECTS: &[&str] = &["Io", "Diverge"];

/// The module path builtins are named under.
///
/// Not a real module and not writable in source, so nothing can collide with
/// it. There is exactly one `Io` and every module has to agree about that,
/// which it would not if each one named it after itself.
pub const PRELUDE_MODULE: &str = "<prelude>";

/// How many "did you mean" suggestions one file gets.
///
/// See [`Resolver::suggest`] for why there is a limit at all. The number is
/// chosen so that a file being edited, which has a handful of unresolved names
/// while something is half typed, never notices it.
const SUGGESTION_BUDGET: usize = 24;

pub struct Resolved {
    pub resolutions: Resolutions,
    pub diagnostics: Vec<Diagnostic>,
}

impl Resolved {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Resolves one module. Always succeeds, possibly with diagnostics.
///
/// `universe` is every other module being compiled alongside this one. An
/// empty universe means a single file on its own, and any `use` in it has
/// nowhere to point, which is now reported rather than assumed away.
pub fn resolve(file: FileId, module: &Module, universe: &Universe) -> Resolved {
    let mut resolver = Resolver {
        file,
        universe,
        resolutions: Resolutions::default(),
        scopes: Vec::new(),
        diagnostics: Vec::new(),
        used: HashSet::new(),
        suggestions: SUGGESTION_BUDGET,
    };

    resolver.push_scope(ScopeKind::Prelude);
    for name in PRELUDE {
        let def = resolver.resolutions.add_def(DefData {
            kind: DefKind::Builtin,
            name: (*name).to_string(),
            span: Span::at(0),
            parent: None,
        });
        resolver.insert(name, def);
        resolver.resolutions.record_builtin(name, def);
        // Builtins are never "unused".
        resolver.used.insert(def);
    }

    // The one effect the language provides. It is an effect rather than a set
    // of free functions so that writing to a console still has to be declared
    // in a `uses` clause like anything else.
    let io = resolver.resolutions.add_def(DefData {
        kind: DefKind::Effect,
        name: "Io".to_string(),
        span: Span::at(0),
        parent: None,
    });
    resolver.insert("Io", io);
    resolver.resolutions.record_builtin("Io", io);
    resolver.used.insert(io);

    // Not returning is something a function can do, so it goes in the row like
    // anything else a function can do. It has no operations: there is nothing
    // to call, only something to admit to. Built in for the same reason `Io`
    // is, since a program that could declare its own would be a program that
    // could opt out of saying it might not finish.
    let diverge = resolver.resolutions.add_def(DefData {
        kind: DefKind::Effect,
        name: "Diverge".to_string(),
        span: Span::at(0),
        parent: None,
    });
    resolver.insert("Diverge", diverge);
    resolver.resolutions.record_builtin("Diverge", diverge);
    resolver.used.insert(diverge);

    for operation in IO_OPERATIONS {
        let def = resolver.resolutions.add_def(DefData {
            kind: DefKind::EffectOp,
            name: (*operation).to_string(),
            span: Span::at(0),
            parent: Some(io),
        });
        resolver.resolutions.record_builtin(operation, def);
        resolver.used.insert(def);
    }

    resolver.push_scope(ScopeKind::Module);
    resolver.collect(module);
    resolver.resolve_module(module);
    resolver.report_unused_imports();

    Resolved {
        resolutions: resolver.resolutions,
        diagnostics: resolver.diagnostics,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Prelude,
    Module,
    Local,
}

struct Scope {
    kind: ScopeKind,
    names: HashMap<String, DefId>,
}

struct Resolver<'a> {
    file: FileId,
    universe: &'a Universe,
    resolutions: Resolutions,
    scopes: Vec<Scope>,
    diagnostics: Vec<Diagnostic>,
    used: HashSet<DefId>,
    /// How many more "did you mean" suggestions this file gets. See
    /// [`Resolver::suggest`].
    suggestions: usize,
}

impl Resolver<'_> {
    // -- scopes ------------------------------------------------------------

    fn push_scope(&mut self, kind: ScopeKind) {
        self.scopes.push(Scope {
            kind,
            names: HashMap::new(),
        });
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn insert(&mut self, name: &str, def: DefId) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.names.insert(name.to_string(), def);
        }
    }

    fn lookup(&self, name: &str) -> Option<(DefId, ScopeKind)> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.names.get(name).map(|def| (*def, scope.kind)))
    }

    fn member_of(&self, container: DefId, name: &str) -> Option<DefId> {
        self.resolutions
            .defs()
            .find(|(_, def)| def.parent == Some(container) && def.name == name)
            .map(|(id, _)| id)
    }

    // -- declaring ---------------------------------------------------------

    /// Declares something at module level, reporting a clash with whatever was
    /// already there.
    fn declare_item(&mut self, ident: &Ident, kind: DefKind, parent: Option<DefId>) -> DefId {
        if let Some(scope) = self.scopes.last()
            && let Some(previous) = scope.names.get(&ident.name).copied()
        {
            let previous_span = self.resolutions.def(previous).span;
            let previous_kind = self.resolutions.def(previous).kind;
            self.diagnostics.push(
                Diagnostic::error(
                    codes::DUPLICATE_DEFINITION,
                    self.file,
                    ident.span,
                    format!("`{}` is declared twice in this module", ident.name),
                )
                .with_primary_label(format!("redeclared as a {}", kind.describe()))
                .with_secondary(
                    previous_span,
                    format!("first declared as a {} here", previous_kind.describe()),
                ),
            );
        } else if PRELUDE.contains(&ident.name.as_str())
            || ident.name == "Io"
            || ident.name == "Diverge"
        {
            // Silently shadowing a builtin would put everything that depends on
            // it quietly back to being unchecked, which is the worst way for
            // this to go wrong.
            self.diagnostics.push(
                Diagnostic::warning(
                    codes::SHADOWED_DECLARATION,
                    self.file,
                    ident.span,
                    format!("`{}` hides a name the language provides", ident.name),
                )
                .with_primary_label("hides a builtin")
                .with_note(
                    "the builtin becomes unreachable in this file, and anything that relied on it stops being checked",
                ),
            );
        }

        let def = self.resolutions.add_def(DefData {
            kind,
            name: ident.name.clone(),
            span: ident.span,
            parent,
        });
        self.insert(&ident.name, def);
        self.resolutions.record_name(ident.span, def);
        def
    }

    /// Declares a member that is only reachable through its container, such as
    /// an effect operation. It does not enter any scope.
    fn declare_member(&mut self, ident: &Ident, kind: DefKind, parent: DefId) -> DefId {
        let def = self.resolutions.add_def(DefData {
            kind,
            name: ident.name.clone(),
            span: ident.span,
            parent: Some(parent),
        });
        self.resolutions.record_name(ident.span, def);
        def
    }

    /// Declares a parameter or a local binding.
    ///
    /// Shadowing another binding is an error. Shadowing a declaration is a
    /// warning. The reasoning is in `design/02-syntax.md`: a reader should be
    /// able to point at a name and know what it means without tracking where
    /// they are in the function.
    fn declare_local(&mut self, ident: &Ident, kind: DefKind) -> DefId {
        if let Some((previous, scope_kind)) = self.lookup(&ident.name) {
            let previous_data = self.resolutions.def(previous);
            let previous_span = previous_data.span;
            let previous_kind = previous_data.kind;

            match scope_kind {
                ScopeKind::Local => self.diagnostics.push(
                    Diagnostic::error(
                        codes::SHADOWED_BINDING,
                        self.file,
                        ident.span,
                        format!("`{}` is already bound", ident.name),
                    )
                    .with_primary_label("shadows an existing binding")
                    .with_secondary(previous_span, format!("the {} bound here", previous_kind.describe()))
                    .with_note("Vow does not allow shadowing, so that a name means one thing for the whole function"),
                ),
                ScopeKind::Module | ScopeKind::Prelude => self.diagnostics.push(
                    Diagnostic::warning(
                        codes::SHADOWED_DECLARATION,
                        self.file,
                        ident.span,
                        format!("`{}` hides a {}", ident.name, previous_kind.describe()),
                    )
                    .with_primary_label("hides a declaration")
                    .with_secondary(previous_span, "declared here"),
                ),
            }
        }

        let def = self.resolutions.add_def(DefData {
            kind,
            name: ident.name.clone(),
            span: ident.span,
            parent: None,
        });
        self.insert(&ident.name, def);
        self.resolutions.record_name(ident.span, def);
        def
    }

    // -- using -------------------------------------------------------------

    fn use_name(&mut self, ident: &Ident) -> Option<DefId> {
        // The parser produces empty identifiers as placeholders after an error.
        // Reporting them would be reporting the same mistake twice.
        if ident.name.is_empty() {
            return None;
        }

        if let Some((def, _)) = self.lookup(&ident.name) {
            self.used.insert(def);
            self.resolutions.record_name(ident.span, def);
            return Some(def);
        }

        let suggestion = self.suggest(&ident.name);
        let mut diagnostic = Diagnostic::error(
            codes::UNKNOWN_NAME,
            self.file,
            ident.span,
            format!("cannot find `{}` in this scope", ident.name),
        )
        .with_primary_label("not found");

        if let Some(candidate) = suggestion {
            diagnostic = diagnostic.with_fix(
                format!("there is a `{candidate}` in scope"),
                ident.span,
                candidate,
                Applicability::MachineApplicable,
            );
        }

        self.diagnostics.push(diagnostic);
        None
    }

    /// Resolves `container.name`, classifying the `.` on the way.
    fn resolve_member(&mut self, container: DefId, ident: &Ident) -> Option<DefId> {
        match self.resolutions.def(container).kind {
            // The name lives in another module. An effect's operations and a
            // choice's variants are part of its declaration, so those cross a
            // file boundary as syntax and get definitions of their own here.
            // Anything else stays foreign, because a record's fields are the
            // type checker's business and it reads them from that module's
            // surface rather than from a name.
            DefKind::Import => {
                let Some(export) = self.resolutions.import(container).cloned() else {
                    self.resolutions.record_dot(ident.span, Dot::Foreign);
                    return None;
                };
                if !matches!(export.kind, ExportKind::Effect | ExportKind::Choice) {
                    self.resolutions.record_dot(ident.span, Dot::Foreign);
                    return None;
                }

                if export.members.contains(&ident.name) {
                    let kind = match export.kind {
                        ExportKind::Effect => DefKind::EffectOp,
                        _ => DefKind::Variant,
                    };
                    let member = self.member_of(container, &ident.name).unwrap_or_else(|| {
                        // Created on demand, once per name, so two mentions of
                        // the same operation are the same definition.
                        self.resolutions.add_def(DefData {
                            kind,
                            name: ident.name.clone(),
                            span: ident.span,
                            parent: Some(container),
                        })
                    });
                    self.used.insert(member);
                    self.resolutions.record_name(ident.span, member);
                    return Some(member);
                }

                self.resolutions.record_dot(ident.span, Dot::Foreign);
                let suggestion = closest(&ident.name, export.members.iter().map(String::as_str));
                let container_data = self.resolutions.def(container);
                let container_name = container_data.name.clone();
                let what = export.kind.describe();

                let mut diagnostic = Diagnostic::error(
                    codes::UNKNOWN_MEMBER,
                    self.file,
                    ident.span,
                    format!("`{container_name}` is {what} with no `{}`", ident.name),
                )
                .with_primary_label("no such member");
                if let Some(candidate) = suggestion {
                    diagnostic = diagnostic.with_fix(
                        format!("there is a `{candidate}`"),
                        ident.span,
                        candidate,
                        Applicability::MaybeIncorrect,
                    );
                }
                self.diagnostics.push(diagnostic);
                None
            }
            DefKind::Choice | DefKind::Effect => match self.member_of(container, &ident.name) {
                Some(member) => {
                    self.used.insert(member);
                    self.resolutions.record_name(ident.span, member);
                    Some(member)
                }
                None => {
                    let container_data = self.resolutions.def(container);
                    let container_name = container_data.name.clone();
                    let container_kind = container_data.kind.describe();
                    self.diagnostics.push(
                        Diagnostic::error(
                            codes::UNKNOWN_MEMBER,
                            self.file,
                            ident.span,
                            format!(
                                "the {container_kind} `{container_name}` has no member `{}`",
                                ident.name
                            ),
                        )
                        .with_primary_label("no such member"),
                    );
                    None
                }
            },
            // A value. Which field this is, and whether it exists, is a
            // question the type checker gets to answer.
            _ => {
                self.resolutions.record_dot(ident.span, Dot::Field);
                None
            }
        }
    }

    /// Suggests a name, while there is budget for it.
    ///
    /// Finding the nearest name costs a pass over everything in scope, so doing
    /// it once per unresolved name is quadratic in the size of the file. A file
    /// full of unresolved names is the normal state of a file being edited,
    /// which is exactly when P9's latency claim is about something, so there is
    /// a budget and it is small.
    ///
    /// Spending it in source order is what makes the output the same every
    /// time. Cutting off by count rather than by cost would depend on how fast
    /// the machine was, and a diagnostic that changes with the weather is not
    /// an API.
    ///
    /// Past the budget the diagnostic still points at the name and still says
    /// it was not found. Only the "did you mean" goes, and a file with two
    /// dozen unresolved names has a problem that a typo hint does not solve.
    fn suggest(&mut self, name: &str) -> Option<String> {
        if self.suggestions == 0 {
            return None;
        }
        self.suggestions -= 1;

        let mut candidates = Vec::new();
        for scope in &self.scopes {
            candidates.extend(scope.names.keys().map(String::as_str));
        }
        closest(name, candidates.into_iter())
    }

    fn report_unused_imports(&mut self) {
        let unused: Vec<(String, Span)> = self
            .resolutions
            .defs()
            .filter(|(id, def)| def.kind == DefKind::Import && !self.used.contains(id))
            .map(|(_, def)| (def.name.clone(), def.span))
            .collect();

        for (name, span) in unused {
            self.diagnostics.push(
                Diagnostic::warning(
                    codes::UNUSED_IMPORT,
                    self.file,
                    span,
                    format!("`{name}` is imported but never used"),
                )
                .with_primary_label("unused import")
                .with_note("imports are explicit and there is no wildcard form, so an unused one is only noise"),
            );
        }
    }

    // -- walking -----------------------------------------------------------

    /// Collects every module level name before resolving any body, so that
    /// declaration order does not matter.
    fn collect(&mut self, module: &Module) {
        for import in &module.uses {
            let path = import.path.to_string_path();
            let exports = self.universe.get(&path);

            if exports.is_none() {
                let suggestion = closest(&path, self.universe.paths());
                let mut diagnostic = Diagnostic::error(
                    codes::UNKNOWN_MODULE,
                    self.file,
                    import.path.span,
                    format!("no module `{path}` among the files being compiled"),
                )
                .with_primary_label("not found")
                .with_note(
                    "a module is named by its own `module` line, and only the files handed \
                     to the compiler are looked at",
                );
                if let Some(candidate) = suggestion {
                    diagnostic = diagnostic.with_fix(
                        format!("there is a module `{candidate}`"),
                        import.path.span,
                        candidate,
                        Applicability::MaybeIncorrect,
                    );
                }
                self.diagnostics.push(diagnostic);
            }

            for name in &import.names {
                // Declared whatever happened above, so one missing module
                // produces one diagnostic rather than one per name plus a
                // cascade of unresolved uses further down the file.
                let def = self.declare_item(name, DefKind::Import, None);
                self.resolutions.record_import_module(def, &path);

                let Some(exports) = exports else { continue };
                match exports.get(&name.name) {
                    Some(export) => {
                        self.resolutions.record_export(def, export.clone());
                    }
                    None => {
                        let suggestion = closest(&name.name, exports.names());
                        let mut diagnostic = Diagnostic::error(
                            codes::UNKNOWN_EXPORT,
                            self.file,
                            name.span,
                            format!("`{path}` declares no `{}`", name.name),
                        )
                        .with_primary_label("not declared there");
                        if let Some(candidate) = suggestion {
                            diagnostic = diagnostic.with_fix(
                                format!("`{path}` declares a `{candidate}`"),
                                name.span,
                                candidate,
                                Applicability::MaybeIncorrect,
                            );
                        }
                        self.diagnostics.push(diagnostic);
                    }
                }
            }
        }

        for item in &module.items {
            match item {
                Item::TypeAlias(alias) => {
                    self.declare_item(&alias.name, DefKind::Type, None);
                }
                Item::Record(record) => {
                    self.declare_item(&record.name, DefKind::Record, None);
                }
                Item::Choice(choice) => {
                    let id = self.declare_item(&choice.name, DefKind::Choice, None);
                    // Variants are usable unqualified, which is what makes
                    // `err(InsufficientFunds { .. })` read the way it does.
                    for variant in &choice.variants {
                        self.declare_item(&variant.name, DefKind::Variant, Some(id));
                    }
                }
                Item::Effect(effect) => {
                    let id = self.declare_item(&effect.name, DefKind::Effect, None);
                    // Operations are reachable only through the effect.
                    for operation in &effect.operations {
                        self.declare_member(&operation.name, DefKind::EffectOp, id);
                    }
                }
                Item::Handler(handler) => {
                    self.declare_item(&handler.name, DefKind::Handler, None);
                }
                Item::Function(function) => {
                    self.declare_item(&function.sig.name, DefKind::Function, None);
                }
                Item::Test(_) => {}
            }
        }
    }

    fn resolve_module(&mut self, module: &Module) {
        for item in &module.items {
            match item {
                Item::TypeAlias(alias) => self.resolve_type_alias(alias),
                Item::Record(record) => self.resolve_record(record),
                Item::Choice(choice) => self.resolve_choice(choice),
                Item::Effect(effect) => self.resolve_effect(effect),
                Item::Handler(handler) => self.resolve_handler(handler),
                Item::Function(function) => self.resolve_fn(function),
                Item::Test(test) => self.resolve_block(&test.body),
            }
        }
    }

    fn resolve_type_alias(&mut self, alias: &TypeAlias) {
        self.resolve_type(&alias.ty);

        if let Some(refinement) = &alias.refinement {
            // `value` is the thing being refined. It is the only name the
            // language introduces implicitly, and it exists because a
            // refinement has nothing else to talk about.
            self.push_scope(ScopeKind::Local);
            let def = self.resolutions.add_def(DefData {
                kind: DefKind::Local,
                name: "value".to_string(),
                span: alias.name.span,
                parent: None,
            });
            self.insert("value", def);
            self.used.insert(def);
            self.resolve_expr(refinement);
            self.pop_scope();
        }
    }

    fn resolve_record(&mut self, record: &RecordDecl) {
        for field in &record.fields {
            self.resolve_type(&field.ty);
        }
    }

    fn resolve_choice(&mut self, choice: &ChoiceDecl) {
        for variant in &choice.variants {
            for field in variant.fields.iter().flatten() {
                self.resolve_type(&field.ty);
            }
        }
    }

    fn resolve_effect(&mut self, effect: &EffectDecl) {
        for operation in &effect.operations {
            for param in &operation.params {
                if let Some(ty) = &param.ty {
                    self.resolve_type(ty);
                }
            }
            if let Some(ret) = &operation.ret {
                self.resolve_type(ret);
            }
        }
    }

    fn resolve_handler(&mut self, handler: &HandlerDecl) {
        self.use_name(&handler.effect);

        self.push_scope(ScopeKind::Local);
        for field in &handler.state {
            self.resolve_type(&field.ty);
            self.declare_local(&field.name, DefKind::State);
        }
        for operation in &handler.operations {
            self.resolve_fn(operation);
        }
        self.pop_scope();
    }

    fn resolve_fn(&mut self, function: &FnDecl) {
        self.push_scope(ScopeKind::Local);

        for param in &function.sig.params {
            if let Some(ty) = &param.ty {
                self.resolve_type(ty);
            }
            self.declare_local(&param.name, DefKind::Param);
        }
        if let Some(ret) = &function.sig.ret {
            self.resolve_type(ret);
        }

        for requirement in &function.contract.requires {
            self.resolve_expr(requirement);
        }
        for effect in &function.contract.uses {
            self.resolve_effect_ref(effect);
        }
        for obligation in &function.contract.ensures {
            // `result` is what the function produced. It is bound per
            // obligation rather than once, because an `ok` clause and an `err`
            // clause see different things and therefore different types.
            self.push_scope(ScopeKind::Local);
            let def = self.resolutions.add_def(DefData {
                kind: DefKind::Local,
                name: "result".to_string(),
                span: obligation.outcome_span,
                parent: None,
            });
            self.insert("result", def);
            self.used.insert(def);
            self.resolve_expr(&obligation.condition);
            self.pop_scope();
        }

        self.resolve_block(&function.body);
        self.pop_scope();
    }

    fn resolve_effect_ref(&mut self, effect: &EffectRef) {
        let Some(def) = self.use_name(&effect.effect) else {
            return;
        };
        if let Some(operation) = &effect.operation {
            self.resolve_member(def, operation);
        }
    }

    fn resolve_type(&mut self, ty: &Type) {
        match ty {
            Type::Named { name, args, .. } => {
                self.use_name(name);
                for arg in args {
                    self.resolve_type(arg);
                }
            }
            Type::Fn {
                params, row, ret, ..
            } => {
                for param in params {
                    self.resolve_type(param);
                }
                // A row inside a type names effects the same way a contract
                // does, so naming one here counts as using the import and
                // naming something that is not an effect is an error here too.
                for effect in row {
                    self.resolve_effect_ref(effect);
                }
                self.resolve_type(ret);
            }
            Type::Unit(_) | Type::Error(_) => {}
        }
    }

    fn resolve_block(&mut self, block: &Block) {
        self.push_scope(ScopeKind::Local);
        for stmt in &block.stmts {
            self.resolve_stmt(stmt);
        }
        if let Some(tail) = &block.tail {
            self.resolve_expr(tail);
        }
        self.pop_scope();
    }

    fn resolve_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let {
                pattern, ty, init, ..
            } => {
                // The initialiser is resolved first, so `let x = x` reads the
                // outer `x` rather than itself.
                self.resolve_expr(init);
                if let Some(ty) = ty {
                    self.resolve_type(ty);
                }
                self.bind_pattern(pattern);
            }
            Stmt::Return { value, .. } => {
                if let Some(value) = value {
                    self.resolve_expr(value);
                }
            }
            Stmt::Assign { target, value, .. } => {
                self.resolve_expr(value);
                self.use_name(target);
            }
            Stmt::Assert { condition, .. } => self.resolve_expr(condition),
            Stmt::Expr(expr) => self.resolve_expr(expr),
        }
    }

    fn resolve_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Int { .. }
            | Expr::Str { .. }
            | Expr::Bool { .. }
            | Expr::Unit(_)
            | Expr::Error(_) => {}

            Expr::Ident(ident) => {
                self.use_name(ident);
            }

            Expr::Field { receiver, name, .. } => self.resolve_field(receiver, name),

            Expr::Call { callee, args, .. } => {
                self.resolve_expr(callee);
                for arg in args {
                    self.resolve_expr(arg);
                }
            }

            Expr::List { elements, .. } => {
                for element in elements {
                    self.resolve_expr(element);
                }
            }

            Expr::StructLit { path, fields, .. } => {
                self.resolve_expr(path);
                for field in fields {
                    match &field.value {
                        Some(value) => self.resolve_expr(value),
                        // Shorthand: `Receipt { from }` means `from: from`, so
                        // the label is also a reference.
                        None => {
                            self.use_name(&field.name);
                        }
                    }
                }
            }

            Expr::Unary { operand, .. } => self.resolve_expr(operand),
            Expr::Binary { lhs, rhs, .. } => {
                self.resolve_expr(lhs);
                self.resolve_expr(rhs);
            }
            Expr::Try { operand, .. } => self.resolve_expr(operand),

            Expr::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.resolve_expr(condition);
                self.resolve_block(then_branch);
                if let Some(else_branch) = else_branch {
                    self.resolve_expr(else_branch);
                }
            }

            Expr::Match {
                scrutinee, arms, ..
            } => {
                self.resolve_expr(scrutinee);
                for arm in arms {
                    self.push_scope(ScopeKind::Local);
                    self.bind_pattern(&arm.pattern);
                    self.resolve_expr(&arm.body);
                    self.pop_scope();
                }
            }

            Expr::For {
                binder,
                iterable,
                accumulator,
                body,
                ..
            } => {
                self.resolve_expr(iterable);
                // What the accumulator starts as is worked out before the loop
                // exists, so the accumulator is not in scope in it. `with sum =
                // sum` is a name that does not resolve rather than a value
                // that refers to itself.
                if let Some(accumulator) = accumulator {
                    self.resolve_expr(&accumulator.init);
                }

                self.push_scope(ScopeKind::Local);
                self.declare_local(binder, DefKind::Local);
                if let Some(accumulator) = accumulator {
                    self.declare_local(&accumulator.name, DefKind::Local);
                }
                self.resolve_block(body);
                self.pop_scope();
            }

            Expr::Block(block) => self.resolve_block(block),

            Expr::Closure { params, body, .. } => {
                self.push_scope(ScopeKind::Local);
                for param in params {
                    if let Some(ty) = &param.ty {
                        self.resolve_type(ty);
                    }
                    self.declare_local(&param.name, DefKind::Param);
                }
                self.resolve_expr(body);
                self.pop_scope();
            }

            Expr::Old { expr, .. } => self.resolve_expr(expr),
            Expr::Unchanged { effect, .. } => self.resolve_effect_ref(effect),

            Expr::With { handlers, body, .. } => {
                for handler in handlers {
                    self.resolve_expr(handler);
                }
                self.resolve_block(body);
            }
        }
    }

    fn resolve_field(&mut self, receiver: &Expr, name: &Ident) {
        // A path only means qualification when it starts at a name. Anything
        // else on the left is a value, and `.name` is a field of it.
        let container = match receiver {
            Expr::Ident(ident) => self.use_name(ident),
            other => {
                self.resolve_expr(other);
                None
            }
        };

        match container {
            Some(container) => {
                self.resolve_member(container, name);
            }
            None => self.resolutions.record_dot(name.span, Dot::Field),
        }
    }

    // -- patterns ----------------------------------------------------------

    fn bind_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Wildcard(_)
            | Pattern::Int { .. }
            | Pattern::Str { .. }
            | Pattern::Bool { .. }
            | Pattern::Error(_) => {}

            Pattern::Path { segments, .. } => {
                match segments.split_first() {
                    None => {}
                    Some((first, [])) => {
                        // The one place the language leans on capitalisation.
                        // Without it a mistyped variant silently becomes a
                        // binding that matches everything, which is a bug the
                        // compiler would never mention.
                        if starts_upper(&first.name) {
                            self.use_name(first);
                        } else {
                            self.declare_local(first, DefKind::Local);
                        }
                    }
                    Some(_) => self.resolve_path(segments),
                }
            }

            Pattern::Tuple { path, elements, .. } => {
                self.resolve_path(path);
                for element in elements {
                    self.bind_pattern(element);
                }
            }

            Pattern::Record { path, fields, .. } => {
                self.resolve_path(path);
                for field in fields {
                    match &field.pattern {
                        Some(pattern) => self.bind_pattern(pattern),
                        None => {
                            self.declare_local(&field.name, DefKind::Local);
                        }
                    }
                }
            }
        }
    }

    fn resolve_path(&mut self, segments: &[Ident]) {
        let Some((first, rest)) = segments.split_first() else {
            return;
        };
        let mut container = self.use_name(first);
        for segment in rest {
            container = match container {
                Some(id) => self.resolve_member(id, segment),
                None => {
                    self.resolutions.record_dot(segment.span, Dot::Field);
                    None
                }
            };
        }
    }
}

fn starts_upper(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

/// The nearest candidate to `name`, when there is an unambiguous one.
///
/// Only candidates within the threshold are considered at all, which matters
/// more than it looks. This runs once per unresolved name and a file being
/// edited is a file full of unresolved names, so an edit distance against every
/// name in scope is quadratic in the size of the file. A scaling test caught
/// exactly that: ten times the broken input cost ninety times the time.
fn closest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<String> {
    // One edit for short names, proportionally more for long ones. A
    // suggestion that is not obviously right is worse than none, because a
    // machine-applicable fix gets applied.
    let length = name.chars().count();
    let threshold = (length / 3).max(1);

    let mut best: Option<(usize, &str)> = None;
    let mut ambiguous = false;

    for candidate in candidates {
        // An edit distance is at least the difference in length, so this rules
        // most candidates out without looking at a single character.
        if candidate.chars().count().abs_diff(length) > threshold {
            continue;
        }
        let Some(distance) = levenshtein_within(name, candidate, threshold) else {
            continue;
        };
        match best {
            Some((best_distance, _)) if distance < best_distance => {
                best = Some((distance, candidate));
                ambiguous = false;
            }
            Some((best_distance, _)) if distance == best_distance => ambiguous = true,
            None => best = Some((distance, candidate)),
            _ => {}
        }
    }

    let (_, candidate) = best?;
    (!ambiguous).then(|| candidate.to_string())
}

/// Edit distance, or `None` when it is greater than `limit`.
///
/// Only the band of cells within `limit` of the diagonal can hold a value that
/// small, so the rest are never computed. With a limit of one or two that is a
/// handful of cells per row rather than the whole table.
fn levenshtein_within(a: &str, b: &str, limit: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();

    if a.len().abs_diff(b.len()) > limit {
        return None;
    }
    if a.is_empty() {
        return (b.len() <= limit).then_some(b.len());
    }

    let too_far = limit + 1;
    if b.is_empty() {
        return (a.len() <= limit).then_some(a.len());
    }

    let mut previous: Vec<usize> = (0..=b.len()).map(|j| j.min(too_far)).collect();
    let mut current = vec![too_far; b.len() + 1];

    for (i, &ca) in a.iter().enumerate() {
        // Cell (i + 1, j + 1) can only be within the limit when the row and
        // the column are within the limit of each other, so the rest of the
        // table is never computed.
        let first = i.saturating_sub(limit);
        let last = (i + limit).min(b.len() - 1);

        current[0] = (i + 1).min(too_far);
        for cell in current.iter_mut().skip(1) {
            *cell = too_far;
        }

        for j in first..=last {
            let cost = usize::from(ca != b[j]);
            current[j + 1] = (current[j] + 1)
                .min(previous[j + 1] + 1)
                .min(previous[j] + cost)
                .min(too_far);
        }

        if current[first..=last + 1]
            .iter()
            .all(|cell| *cell >= too_far)
            && current[0] >= too_far
        {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }

    let distance = previous[b.len()];
    (distance <= limit).then_some(distance)
}

#[cfg(test)]
mod tests {
    use super::levenshtein_within;

    /// The distance when it is wanted regardless of how large it is.
    fn levenshtein(a: &str, b: &str) -> usize {
        let limit = a.chars().count().max(b.chars().count());
        levenshtein_within(a, b, limit).expect("the limit cannot be exceeded")
    }

    #[test]
    fn edit_distance_is_symmetric_and_zero_on_equal() {
        assert_eq!(levenshtein("balance", "balance"), 0);
        assert_eq!(levenshtein("balance", "balanse"), 1);
        assert_eq!(levenshtein("balanse", "balance"), 1);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn a_distance_past_the_limit_is_not_computed() {
        assert_eq!(levenshtein_within("balance", "balanse", 1), Some(1));
        assert_eq!(levenshtein_within("balance", "sitting", 2), None);
        assert_eq!(levenshtein_within("a", "abcdefg", 2), None);
        assert_eq!(levenshtein_within("kitten", "sitting", 3), Some(3));
        assert_eq!(levenshtein_within("kitten", "sitting", 2), None);
    }

    #[test]
    fn the_band_agrees_with_the_full_table() {
        // The banded version is an optimisation, so it has to answer the same
        // thing wherever it answers at all.
        let words = [
            "", "a", "ab", "abc", "balance", "balanse", "kitten", "sitting", "counter", "count",
        ];
        for a in words {
            for b in words {
                let full = levenshtein(a, b);
                for limit in 0..8 {
                    let banded = levenshtein_within(a, b, limit);
                    if full <= limit {
                        assert_eq!(banded, Some(full), "`{a}` vs `{b}` at limit {limit}");
                    } else {
                        assert_eq!(banded, None, "`{a}` vs `{b}` at limit {limit}");
                    }
                }
            }
        }
    }
}
