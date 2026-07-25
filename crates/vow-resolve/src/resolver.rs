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

/// Names the language provides without anyone importing them.
///
/// Deliberately tiny. Every entry is a name that cannot be shadowed without a
/// warning and cannot be looked up in any file, which is exactly the kind of
/// thing P2 is a budget for.
pub const PRELUDE: &[&str] = &["Int", "String", "Bool", "System"];

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
pub fn resolve(file: FileId, module: &Module) -> Resolved {
    let mut resolver = Resolver {
        file,
        resolutions: Resolutions::default(),
        scopes: Vec::new(),
        diagnostics: Vec::new(),
        used: HashSet::new(),
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
        // Builtins are never "unused".
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

struct Resolver {
    file: FileId,
    resolutions: Resolutions,
    scopes: Vec<Scope>,
    diagnostics: Vec<Diagnostic>,
    used: HashSet<DefId>,
}

impl Resolver {
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
        self.resolutions.record_unresolved(ident.span);
        None
    }

    /// Resolves `container.name`, classifying the `.` on the way.
    fn resolve_member(&mut self, container: DefId, ident: &Ident) -> Option<DefId> {
        match self.resolutions.def(container).kind {
            // The name lives in a module that has not been loaded. Nothing can
            // be said about it, and saying nothing is the honest answer.
            DefKind::Import => {
                self.resolutions.record_dot(ident.span, Dot::Foreign);
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
                    self.resolutions.record_unresolved(ident.span);
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

    fn suggest(&self, name: &str) -> Option<String> {
        let mut best: Option<(usize, String)> = None;
        let mut ambiguous = false;

        for scope in &self.scopes {
            for candidate in scope.names.keys() {
                let distance = levenshtein(name, candidate);
                match &best {
                    Some((best_distance, _)) if distance < *best_distance => {
                        best = Some((distance, candidate.clone()));
                        ambiguous = false;
                    }
                    Some((best_distance, _)) if distance == *best_distance => ambiguous = true,
                    None => best = Some((distance, candidate.clone())),
                    _ => {}
                }
            }
        }

        let (distance, candidate) = best?;
        // One edit for short names, proportionally more for long ones. A
        // suggestion that is not obviously right is worse than none, because a
        // machine-applicable fix gets applied.
        let threshold = (name.chars().count() / 3).max(1);
        (distance <= threshold && !ambiguous).then_some(candidate)
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
            for name in &import.names {
                self.declare_item(name, DefKind::Import, None);
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
                Item::Test(_) | Item::Error(_) => {}
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
                Item::Error(_) => {}
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
            self.declare_local(&field.name, DefKind::Local);
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
            self.resolve_expr(&obligation.condition);
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
            Stmt::Assert { condition, .. } => self.resolve_expr(condition),
            Stmt::Expr(expr) => self.resolve_expr(expr),
            Stmt::Error(_) => {}
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

/// Edit distance, used only for suggestions.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();

    if a.is_empty() {
        return b.len();
    }

    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];

    for (i, &ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (current[j] + 1)
                .min(previous[j + 1] + 1)
                .min(previous[j] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::levenshtein;

    #[test]
    fn edit_distance_is_symmetric_and_zero_on_equal() {
        assert_eq!(levenshtein("balance", "balance"), 0);
        assert_eq!(levenshtein("balance", "balanse"), 1);
        assert_eq!(levenshtein("balanse", "balance"), 1);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }
}
