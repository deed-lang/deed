//! Type checking.
//!
//! Bidirectional and local. There is no global inference and no unification
//! variables, which is a decision worth stating plainly rather than leaving to
//! be inferred from the code.
//!
//! Three reasons. P2 puts a budget on how much language there is, and
//! Hindley-Milner is a lot of machinery to spend it on. P1 already requires a
//! signature to be complete, so the annotations that inference would save are
//! ones the language wants written anyway. And an error from local checking
//! points at the expression that caused it, rather than at wherever unification
//! happened to notice.
//!
//! The cost is real: every function annotates its parameters and its return
//! type. Inside a body, `let` still infers from its initialiser.

use std::collections::{HashMap, HashSet};

use vow_ast::{
    BinaryOp, Block, Expr, FieldInit, FnDecl, Ident, Item, MatchArm, Module, Outcome, Pattern,
    Stmt, Type, TypeAlias, UnaryOp,
};
use vow_diagnostics::{Diagnostic, FileId, Span};
use vow_resolve::{DefId, DefKind, Resolutions};

use crate::codes;
use crate::ty::{FieldTy, Nominal, Obligation, Tier, Ty, Types, VariantTy};

pub struct Checked {
    pub types: Types,
    pub diagnostics: Vec<Diagnostic>,
}

impl Checked {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Checks one resolved module. Always succeeds, possibly with diagnostics.
pub fn check(file: FileId, module: &Module, resolutions: &Resolutions) -> Checked {
    let mut checker = Checker {
        file,
        resolutions,
        types: Types::default(),
        diagnostics: Vec::new(),
        def_types: HashMap::new(),
        signatures: HashMap::new(),
        aliases: HashMap::new(),
        alias_targets: HashMap::new(),
        alias_stack: Vec::new(),
        returns: Vec::new(),
    };

    checker.collect(module);
    checker.check_module(module);

    Checked {
        types: checker.types,
        diagnostics: checker.diagnostics,
    }
}

#[derive(Clone)]
struct ParamTy {
    ty: Ty,
    span: Span,
}

#[derive(Clone)]
struct Signature {
    params: Vec<ParamTy>,
    ret: Ty,
    span: Span,
}

struct Checker<'a> {
    file: FileId,
    resolutions: &'a Resolutions,
    types: Types,
    diagnostics: Vec<Diagnostic>,
    /// Types of parameters, locals and handler state.
    def_types: HashMap<DefId, Ty>,
    signatures: HashMap<DefId, Signature>,
    aliases: HashMap<DefId, &'a TypeAlias>,
    alias_targets: HashMap<DefId, Ty>,
    alias_stack: Vec<DefId>,
    /// Declared return type of the function being checked.
    returns: Vec<(Ty, Span)>,
}

impl<'a> Checker<'a> {
    fn def_of(&self, ident: &Ident) -> Option<DefId> {
        self.resolutions.resolution(ident.span)
    }

    fn emit(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    // -- collecting --------------------------------------------------------

    fn collect(&mut self, module: &'a Module) {
        // The capability types the language provides. They have to be named
        // here or a diagnostic about one would say "an unnamed type".
        for name in ["System", "Console", "Clock"] {
            if let Some(def) = self.resolutions.builtin(name) {
                self.types.set_name(def, name.to_string());
            }
        }

        // `Io.write(out, line)` and `Io.now(clock)`. Each takes the capability
        // it acts on, so which console is a question about the value rather
        // than about the row.
        let capability = |checker: &Self, name: &str| {
            checker
                .resolutions
                .builtin(name)
                .map(Ty::Named)
                .unwrap_or(Ty::Unknown)
        };
        let console = capability(self, "Console");
        let clock = capability(self, "Clock");

        if let Some(def) = self.resolutions.builtin("write") {
            self.types.set_name(def, "write".to_string());
            self.signatures.insert(
                def,
                Signature {
                    params: vec![
                        ParamTy {
                            ty: console,
                            span: Span::at(0),
                        },
                        ParamTy {
                            ty: Ty::Str,
                            span: Span::at(0),
                        },
                    ],
                    ret: Ty::Unit,
                    span: Span::at(0),
                },
            );
        }
        if let Some(def) = self.resolutions.builtin("now") {
            self.types.set_name(def, "now".to_string());
            self.signatures.insert(
                def,
                Signature {
                    params: vec![ParamTy {
                        ty: clock,
                        span: Span::at(0),
                    }],
                    ret: Ty::Int,
                    span: Span::at(0),
                },
            );
        }

        for item in &module.items {
            match item {
                Item::TypeAlias(alias) => {
                    if let Some(def) = self.def_of(&alias.name) {
                        self.aliases.insert(def, alias);
                        self.types.set_name(def, alias.name.name.clone());
                    }
                }
                Item::Record(record) => {
                    if let Some(def) = self.def_of(&record.name) {
                        self.types.set_name(def, record.name.name.clone());
                    }
                }
                Item::Choice(choice) => {
                    if let Some(def) = self.def_of(&choice.name) {
                        self.types.set_name(def, choice.name.name.clone());
                    }
                    for variant in &choice.variants {
                        if let Some(def) = self.def_of(&variant.name) {
                            self.types.set_name(def, variant.name.name.clone());
                        }
                    }
                }
                Item::Effect(effect) => {
                    if let Some(def) = self.def_of(&effect.name) {
                        self.types.set_name(def, effect.name.name.clone());
                    }
                    for operation in &effect.operations {
                        if let Some(def) = self.def_of(&operation.name) {
                            self.types.set_name(def, operation.name.name.clone());
                        }
                    }
                }
                Item::Function(function) => {
                    if let Some(def) = self.def_of(&function.sig.name) {
                        self.types.set_name(def, function.sig.name.name.clone());
                    }
                }
                _ => {}
            }
        }

        for item in &module.items {
            match item {
                Item::TypeAlias(alias) => {
                    let Some(def) = self.def_of(&alias.name) else {
                        continue;
                    };
                    if let Some(predicate) = &alias.refinement {
                        let base = self.lower_type(&alias.ty);
                        self.types.set_nominal(
                            def,
                            alias.name.name.clone(),
                            Nominal::Refinement {
                                base,
                                predicate: predicate.span(),
                            },
                        );
                    } else {
                        // Force the expansion now so a cycle is reported once,
                        // at the declaration, rather than at every use.
                        self.alias_ty(def);
                    }
                }
                Item::Record(record) => {
                    let Some(def) = self.def_of(&record.name) else {
                        continue;
                    };
                    let fields = self.lower_fields(&record.fields);
                    self.types.set_nominal(
                        def,
                        record.name.name.clone(),
                        Nominal::Record { fields },
                    );
                }
                Item::Choice(choice) => {
                    let Some(def) = self.def_of(&choice.name) else {
                        continue;
                    };
                    let mut variants = Vec::new();
                    for variant in &choice.variants {
                        let Some(variant_def) = self.def_of(&variant.name) else {
                            continue;
                        };
                        let fields = variant
                            .fields
                            .as_ref()
                            .map(|fields| self.lower_fields(fields));
                        variants.push(VariantTy {
                            def: variant_def,
                            name: variant.name.name.clone(),
                            fields,
                            span: variant.span,
                        });
                    }
                    self.types.set_nominal(
                        def,
                        choice.name.name.clone(),
                        Nominal::Choice { variants },
                    );
                }
                Item::Effect(effect) => {
                    // An operation is called like a function and should be
                    // checked like one, so it gets the same signature entry.
                    for operation in &effect.operations {
                        let Some(def) = self.def_of(&operation.name) else {
                            continue;
                        };
                        let signature = self.lower_signature(operation);
                        self.signatures.insert(def, signature);
                    }
                }
                Item::Function(function) => {
                    let Some(def) = self.def_of(&function.sig.name) else {
                        continue;
                    };
                    let signature = self.lower_signature(&function.sig);
                    self.signatures.insert(def, signature);
                }
                _ => {}
            }
        }
    }

    fn lower_fields(&mut self, fields: &[vow_ast::FieldDecl]) -> Vec<FieldTy> {
        fields
            .iter()
            .map(|field| FieldTy {
                name: field.name.name.clone(),
                ty: self.lower_type(&field.ty),
                span: field.span,
            })
            .collect()
    }

    fn lower_signature(&mut self, sig: &vow_ast::FnSig) -> Signature {
        let mut params = Vec::new();
        for param in &sig.params {
            params.push(ParamTy {
                ty: match &param.ty {
                    Some(ty) => self.lower_type(ty),
                    None => Ty::Unknown,
                },
                span: param.span,
            });
        }

        let ret = match &sig.ret {
            Some(ty) => self.lower_type(ty),
            None => Ty::Unit,
        };

        Signature {
            params,
            ret,
            span: sig.span,
        }
    }

    // -- types -------------------------------------------------------------

    fn lower_type(&mut self, ty: &Type) -> Ty {
        match ty {
            Type::Unit(_) => Ty::Unit,
            Type::Error(_) => Ty::Unknown,
            Type::Named { name, args, span } => {
                // Arguments are lowered whatever happens, so mistakes inside
                // them get reported even when the head is not generic.
                let lowered_args: Vec<Ty> = args.iter().map(|arg| self.lower_type(arg)).collect();

                let Some(def) = self.def_of(name) else {
                    return Ty::Unknown;
                };

                let base = match self.resolutions.def(def).kind {
                    DefKind::Builtin => match name.name.as_str() {
                        "Int" => Ty::Int,
                        "String" => Ty::Str,
                        "Bool" => Ty::Bool,
                        // Capabilities are opaque. There is nothing to know
                        // about one except that you were handed it.
                        "System" | "Console" | "Clock" => Ty::Named(def),
                        "Result" => {
                            if lowered_args.len() == 2 {
                                return Ty::Result(
                                    Box::new(lowered_args[0].clone()),
                                    Box::new(lowered_args[1].clone()),
                                );
                            }
                            self.emit(
                                Diagnostic::error(
                                    codes::NOT_GENERIC,
                                    self.file,
                                    *span,
                                    format!(
                                        "`Result` takes exactly two type arguments, and {} {} given",
                                        lowered_args.len(),
                                        if lowered_args.len() == 1 { "was" } else { "were" }
                                    ),
                                )
                                .with_primary_label("wrong number of type arguments")
                                .with_note("it is written `Result<Value, Error>`"),
                            );
                            return Ty::Unknown;
                        }
                        _ => Ty::Unknown,
                    },
                    DefKind::Import => Ty::Unknown,
                    DefKind::Record | DefKind::Choice => Ty::Named(def),
                    DefKind::Type => self.alias_ty(def),
                    other => {
                        self.emit(
                            Diagnostic::error(
                                codes::NOT_A_TYPE,
                                self.file,
                                name.span,
                                format!("`{}` is a {}, not a type", name.name, other.describe()),
                            )
                            .with_primary_label("not a type"),
                        );
                        Ty::Unknown
                    }
                };

                if !lowered_args.is_empty() && !base.absorbs() {
                    let described = self.types.describe(&base);
                    self.emit(
                        Diagnostic::error(
                            codes::NOT_GENERIC,
                            self.file,
                            *span,
                            format!("{described} does not take type arguments"),
                        )
                        .with_primary_label("unexpected type arguments")
                        .with_note("Vow has no generic declarations yet, so only types from other modules can be applied"),
                    );
                }

                base
            }
        }
    }

    /// Expands a type alias. Refinements are nominal and stop here.
    fn alias_ty(&mut self, def: DefId) -> Ty {
        if let Some(existing) = self.alias_targets.get(&def) {
            return existing.clone();
        }

        let Some(alias) = self.aliases.get(&def).copied() else {
            return Ty::Unknown;
        };

        if alias.refinement.is_some() {
            let ty = Ty::Named(def);
            self.alias_targets.insert(def, ty.clone());
            return ty;
        }

        if self.alias_stack.contains(&def) {
            self.emit(
                Diagnostic::error(
                    codes::TYPE_ALIAS_CYCLE,
                    self.file,
                    alias.name.span,
                    format!("the type alias `{}` expands to itself", alias.name.name),
                )
                .with_primary_label("cycle starts here"),
            );
            self.alias_targets.insert(def, Ty::Unknown);
            return Ty::Unknown;
        }

        self.alias_stack.push(def);
        let target = self.lower_type(&alias.ty);
        self.alias_stack.pop();
        self.alias_targets.insert(def, target.clone());
        target
    }

    /// A refinement seen as its base type. Widening is always safe.
    fn widen(&self, ty: &Ty) -> Ty {
        if let Ty::Named(def) = ty
            && let Some(Nominal::Refinement { base, .. }) = self.types.nominal(*def)
        {
            return base.clone();
        }
        ty.clone()
    }

    /// Whether a value of one type fits where the other was wanted.
    ///
    /// Componentwise for `Result`, which is what makes `ok(x)` work without
    /// unification: it produces `Result<T, unknown>`, and unknown agrees with
    /// whatever the expected error type turns out to be.
    fn compatible(&self, actual: &Ty, expected: &Ty) -> bool {
        if actual.absorbs() || expected.absorbs() {
            return true;
        }
        match (actual, expected) {
            (Ty::Result(a_ok, a_err), Ty::Result(e_ok, e_err)) => {
                self.compatible(a_ok, e_ok) && self.compatible(a_err, e_err)
            }
            _ => actual == expected,
        }
    }

    // -- assignment --------------------------------------------------------

    /// Checks that `actual` may be used where `expected` was required.
    ///
    /// `expr` is the expression producing the value, when there is one. It is
    /// what makes proving a refinement possible at all.
    fn assign(
        &mut self,
        actual: &Ty,
        expected: &Ty,
        expr: Option<&Expr>,
        span: Span,
        because: Option<(Span, String)>,
    ) {
        if self.compatible(actual, expected) {
            return;
        }

        // Narrowing into a refinement is the interesting direction.
        if let Ty::Named(def) = expected
            && let Some(Nominal::Refinement { base, .. }) = self.types.nominal(*def)
        {
            let base = base.clone();
            if actual == &base || actual.absorbs() {
                self.discharge(*def, expr, span);
                return;
            }
        }

        // Widening out of one is always fine.
        if self.widen(actual) == *expected {
            return;
        }

        let found = self.types.describe(actual);
        let wanted = self.types.describe(expected);
        let mut diagnostic = Diagnostic::error(
            codes::TYPE_MISMATCH,
            self.file,
            span,
            format!("expected {wanted}, found {found}"),
        )
        .with_primary_label(format!("this is {found}"));

        if let Some((where_span, why)) = because {
            diagnostic = diagnostic.with_secondary(where_span, why);
        }

        self.emit(diagnostic);
    }

    /// Tries to prove that a value satisfies a refinement.
    ///
    /// Only constant expressions can be proven today, which is a small slice of
    /// the Proven tier and honestly not much. What matters is that an
    /// obligation that cannot be proven is recorded and said out loud rather
    /// than quietly becoming a runtime check.
    fn discharge(&mut self, refinement: DefId, expr: Option<&Expr>, span: Span) {
        let predicate = self
            .aliases
            .get(&refinement)
            .and_then(|alias| alias.refinement.as_ref());

        let value = expr.and_then(constant);
        let outcome = match (predicate, value) {
            (Some(predicate), Some(value)) => evaluate(predicate, value),
            _ => None,
        };

        let name = self.types.name_of(refinement).to_string();
        let predicate_text = self
            .types
            .nominal(refinement)
            .and_then(|nominal| match nominal {
                Nominal::Refinement { predicate, .. } => Some(*predicate),
                _ => None,
            });

        match outcome {
            Some(Constant::Bool(true)) => self.types.push_obligation(Obligation {
                span,
                refinement,
                tier: Tier::Proven,
            }),
            Some(Constant::Bool(false)) => {
                let mut diagnostic = Diagnostic::error(
                    codes::VIOLATED_REFINEMENT,
                    self.file,
                    span,
                    format!("this value does not satisfy `{name}`"),
                )
                .with_primary_label("violates the refinement");
                if let Some(predicate) = predicate_text {
                    diagnostic =
                        diagnostic.with_secondary(predicate, "the predicate it has to satisfy");
                }
                self.emit(diagnostic);
            }
            _ => {
                let mut diagnostic = Diagnostic::warning(
                    codes::UNPROVEN_REFINEMENT,
                    self.file,
                    span,
                    format!("cannot prove this satisfies `{name}`, so it becomes a runtime check"),
                )
                .with_primary_label("checked at runtime")
                .with_note(
                    "obligations are Proven, Tested or Guarded, and this one is Guarded; see design/02-syntax.md",
                );
                if let Some(predicate) = predicate_text {
                    diagnostic =
                        diagnostic.with_secondary(predicate, "the predicate it has to satisfy");
                }
                self.emit(diagnostic);
                self.types.push_obligation(Obligation {
                    span,
                    refinement,
                    tier: Tier::Guarded,
                });
            }
        }
    }

    // -- walking -----------------------------------------------------------

    fn check_module(&mut self, module: &Module) {
        for item in &module.items {
            match item {
                Item::Function(function) => self.check_fn(function),
                Item::Handler(handler) => {
                    for field in &handler.state {
                        let ty = self.lower_type(&field.ty);
                        if let Some(def) = self.def_of(&field.name) {
                            self.def_types.insert(def, ty);
                        }
                    }
                    for operation in &handler.operations {
                        self.check_fn(operation);
                    }
                }
                Item::Test(test) => {
                    self.check_block(&test.body);
                }
                _ => {}
            }
        }
    }

    fn check_fn(&mut self, function: &FnDecl) {
        // Reuse the signature computed during collection where there is one.
        // Lowering the same annotation twice would report anything wrong with
        // it twice, which is a cascade from a single mistake.
        let signature = self
            .def_of(&function.sig.name)
            .and_then(|def| self.signatures.get(&def).cloned());

        let (param_types, ret) = match &signature {
            Some(signature) => (
                signature.params.iter().map(|p| p.ty.clone()).collect(),
                signature.ret.clone(),
            ),
            None => {
                let mut params = Vec::new();
                for param in &function.sig.params {
                    params.push(match &param.ty {
                        Some(ty) => self.lower_type(ty),
                        None => Ty::Unknown,
                    });
                }
                let ret = match &function.sig.ret {
                    Some(ty) => self.lower_type(ty),
                    None => Ty::Unit,
                };
                (params, ret)
            }
        };

        for (param, ty) in function.sig.params.iter().zip(param_types) {
            if let Some(def) = self.def_of(&param.name) {
                self.def_types.insert(def, ty);
            }
        }

        let ret_span = function
            .sig
            .ret
            .as_ref()
            .map(Type::span)
            .unwrap_or(function.sig.name.span);

        for requirement in &function.contract.requires {
            let ty = self.infer(requirement);
            self.assign(&ty, &Ty::Bool, Some(requirement), requirement.span(), None);
        }
        for obligation in &function.contract.ensures {
            // `result` is bound per obligation and its type depends on which
            // outcome the clause is about.
            let bound = match (&ret, obligation.outcome) {
                (Ty::Result(ok, _), Outcome::Ok) => (**ok).clone(),
                (Ty::Result(_, err), Outcome::Err) => (**err).clone(),
                (other, Outcome::Ok) => other.clone(),
                // A function that does not return a `Result` cannot fail, so
                // there is nothing sensible for an `err` clause to see.
                (_, Outcome::Err) => Ty::Unknown,
            };
            if let Some(def) = self.result_def(&obligation.condition) {
                self.def_types.insert(def, bound);
            }

            let ty = self.infer(&obligation.condition);
            self.assign(
                &ty,
                &Ty::Bool,
                Some(&obligation.condition),
                obligation.condition.span(),
                None,
            );
        }

        self.returns.push((ret.clone(), ret_span));
        let body = self.check_block(&function.body);
        self.returns.pop();

        let tail_span = function
            .body
            .tail
            .as_ref()
            .map(|tail| tail.span())
            .unwrap_or(function.body.span);
        self.assign(
            &body,
            &ret,
            function.body.tail.as_deref(),
            tail_span,
            Some((ret_span, "the declared return type".to_string())),
        );
    }

    /// The definition `result` refers to inside an obligation, if it is used.
    ///
    /// Found by looking, rather than carried on the tree, because the AST holds
    /// no definitions and one binding for one keyword did not seem worth
    /// threading identities through every node for.
    fn result_def(&self, expr: &Expr) -> Option<DefId> {
        match expr {
            Expr::Ident(ident) if ident.name == "result" => self.def_of(ident),
            Expr::Field { receiver, .. } => self.result_def(receiver),
            Expr::Call { callee, args, .. } => self
                .result_def(callee)
                .or_else(|| args.iter().find_map(|arg| self.result_def(arg))),
            Expr::StructLit { path, fields, .. } => self.result_def(path).or_else(|| {
                fields
                    .iter()
                    .filter_map(|field| field.value.as_ref())
                    .find_map(|value| self.result_def(value))
            }),
            Expr::Unary { operand, .. } | Expr::Try { operand, .. } => self.result_def(operand),
            Expr::Binary { lhs, rhs, .. } => self.result_def(lhs).or_else(|| self.result_def(rhs)),
            Expr::Old { expr, .. } => self.result_def(expr),
            _ => None,
        }
    }

    fn check_block(&mut self, block: &Block) -> Ty {
        // A `return` anywhere at this level means the end of the block is not
        // reachable. Coarse, and enough to type a body that ends in `return`.
        let mut diverges = false;
        for stmt in &block.stmts {
            self.check_stmt(stmt);
            if matches!(stmt, Stmt::Return { .. }) {
                diverges = true;
            }
        }

        let ty = match &block.tail {
            Some(tail) => self.infer(tail),
            None if diverges => Ty::Never,
            None => Ty::Unit,
        };
        self.types.record_expr(block.span, ty.clone());
        ty
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let {
                pattern, ty, init, ..
            } => {
                let init_ty = self.infer(init);
                let bound = match ty {
                    Some(annotation) => {
                        let declared = self.lower_type(annotation);
                        self.assign(
                            &init_ty,
                            &declared,
                            Some(init),
                            init.span(),
                            Some((annotation.span(), "declared here".to_string())),
                        );
                        declared
                    }
                    None => init_ty,
                };
                self.bind_pattern(pattern, &bound);
            }
            Stmt::Assign {
                target,
                value,
                span,
            } => {
                let actual = self.infer(value);
                let Some(def) = self.def_of(target) else {
                    return;
                };

                let kind = self.resolutions.def(def).kind;
                if kind != DefKind::State {
                    let declared = self.resolutions.def(def).span;
                    self.emit(
                        Diagnostic::error(
                            codes::NOT_ASSIGNABLE,
                            self.file,
                            *span,
                            format!("`{}` is a {}, not handler state", target.name, kind.describe()),
                        )
                        .with_primary_label("cannot be assigned to")
                        .with_secondary(declared, "declared here")
                        .with_note(
                            "handler state is the only mutable thing in Vow, which is what lets an empty effect row mean a function cannot change anything",
                        ),
                    );
                    return;
                }

                let declared = self.def_types.get(&def).cloned().unwrap_or(Ty::Unknown);
                let field_span = self.resolutions.def(def).span;
                self.assign(
                    &actual,
                    &declared,
                    Some(value),
                    value.span(),
                    Some((field_span, "the state it is assigned to".to_string())),
                );
            }
            Stmt::Return { value, span } => {
                let (ret, ret_span) = self.returns.last().cloned().unwrap_or((Ty::Unknown, *span));
                let actual = match value {
                    Some(value) => self.infer(value),
                    None => Ty::Unit,
                };
                self.assign(
                    &actual,
                    &ret,
                    value.as_ref(),
                    value.as_ref().map(Expr::span).unwrap_or(*span),
                    Some((ret_span, "the declared return type".to_string())),
                );
            }
            Stmt::Assert { condition, .. } => {
                let ty = self.infer(condition);
                self.assign(&ty, &Ty::Bool, Some(condition), condition.span(), None);
            }
            Stmt::Expr(expr) => {
                self.infer(expr);
            }
            Stmt::Error(_) => {}
        }
    }

    fn infer(&mut self, expr: &Expr) -> Ty {
        let ty = self.infer_inner(expr);
        self.types.record_expr(expr.span(), ty.clone());
        ty
    }

    fn infer_inner(&mut self, expr: &Expr) -> Ty {
        match expr {
            Expr::Int { .. } => Ty::Int,
            Expr::Str { .. } => Ty::Str,
            Expr::Bool { .. } => Ty::Bool,
            Expr::Unit(_) => Ty::Unit,
            Expr::Error(_) => Ty::Unknown,

            Expr::Ident(ident) => self.ident_ty(ident),

            Expr::Field { receiver, name, .. } => {
                let receiver_ty = self.infer(receiver);
                // A resolved name here means the `.` was qualification, which
                // name resolution already settled. Nothing to look up.
                if let Some(def) = self.resolutions.resolution(name.span) {
                    return match self.resolutions.def(def).kind {
                        DefKind::Variant => self.variant_ty(def),
                        _ => Ty::Unknown,
                    };
                }
                self.field_ty(&receiver_ty, name)
            }

            Expr::Call { callee, args, .. } => self.infer_call(callee, args, expr.span()),

            Expr::StructLit { path, fields, .. } => {
                self.infer_struct_lit(path, fields, expr.span())
            }

            Expr::Unary { op, operand, .. } => {
                let ty = self.infer(operand);
                let widened = self.widen(&ty);
                if widened.absorbs() {
                    return Ty::Unknown;
                }
                match op {
                    UnaryOp::Neg => {
                        self.assign(&widened, &Ty::Int, Some(operand), operand.span(), None);
                        Ty::Int
                    }
                    UnaryOp::Not => {
                        self.assign(&widened, &Ty::Bool, Some(operand), operand.span(), None);
                        Ty::Bool
                    }
                }
            }

            Expr::Binary { op, lhs, rhs, .. } => {
                let left = self.infer(lhs);
                let right = self.infer(rhs);
                let left = self.widen(&left);
                let right = self.widen(&right);
                self.binary_ty(*op, &left, &right, lhs, rhs)
            }

            // `?` unwraps the success case and propagates the failure one, so
            // it only means something inside a function that can fail.
            Expr::Try { operand, span } => {
                let ty = self.infer(operand);
                if ty.absorbs() {
                    return Ty::Unknown;
                }

                let Ty::Result(ok_ty, err_ty) = ty else {
                    let described = self.types.describe(&ty);
                    self.emit(
                        Diagnostic::error(
                            codes::NOT_A_RESULT,
                            self.file,
                            *span,
                            format!("`?` needs a `Result`, and this is {described}"),
                        )
                        .with_primary_label("not a Result")
                        .with_note("`?` returns the error case and unwraps the success case"),
                    );
                    return Ty::Unknown;
                };

                match self.returns.last().cloned() {
                    Some((Ty::Result(_, expected_err), ret_span)) => self.assign(
                        &err_ty,
                        &expected_err,
                        None,
                        operand.span(),
                        Some((ret_span, "the error type this function returns".to_string())),
                    ),
                    Some((other, ret_span)) if !other.absorbs() => {
                        let described = self.types.describe(&other);
                        self.emit(
                            Diagnostic::error(
                                codes::TRY_NEEDS_RESULT_RETURN,
                                self.file,
                                *span,
                                format!("`?` can only be used in a function returning a `Result`, and this one returns {described}"),
                            )
                            .with_primary_label("nowhere to propagate the error to")
                            .with_secondary(ret_span, "the declared return type"),
                        );
                    }
                    _ => {}
                }

                *ok_ty
            }

            Expr::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                let condition_ty = self.infer(condition);
                self.assign(
                    &condition_ty,
                    &Ty::Bool,
                    Some(condition),
                    condition.span(),
                    None,
                );

                let then_ty = self.check_block(then_branch);
                match else_branch {
                    Some(else_branch) => {
                        let else_ty = self.infer(else_branch);
                        if then_ty.absorbs() {
                            else_ty
                        } else {
                            self.assign(
                                &else_ty,
                                &then_ty,
                                Some(else_branch),
                                else_branch.span(),
                                Some((then_branch.span, "the other branch".to_string())),
                            );
                            then_ty
                        }
                    }
                    None => {
                        self.assign(&then_ty, &Ty::Unit, None, then_branch.span, None);
                        Ty::Unit
                    }
                }
            }

            Expr::Match {
                scrutinee,
                arms,
                span,
            } => self.infer_match(scrutinee, arms, *span),

            Expr::Block(block) => self.check_block(block),

            Expr::Closure { params, body, .. } => {
                let mut param_types = Vec::new();
                for param in params {
                    let ty = param
                        .ty
                        .as_ref()
                        .map(|ty| self.lower_type(ty))
                        .unwrap_or(Ty::Unknown);
                    if let Some(def) = self.def_of(&param.name) {
                        self.def_types.insert(def, ty.clone());
                    }
                    param_types.push(ty);
                }
                let ret = self.infer(body);
                Ty::Fn {
                    params: param_types,
                    ret: Box::new(ret),
                }
            }

            Expr::Old { expr, .. } => self.infer(expr),
            Expr::Unchanged { .. } => Ty::Bool,

            Expr::With { handlers, body, .. } => {
                for handler in handlers {
                    self.infer(handler);
                }
                self.check_block(body)
            }
        }
    }

    fn ident_ty(&mut self, ident: &Ident) -> Ty {
        let Some(def) = self.def_of(ident) else {
            return Ty::Unknown;
        };
        if let Some(ty) = self.def_types.get(&def) {
            return ty.clone();
        }
        match self.resolutions.def(def).kind {
            DefKind::Function => match self.signatures.get(&def) {
                Some(signature) => Ty::Fn {
                    params: signature.params.iter().map(|p| p.ty.clone()).collect(),
                    ret: Box::new(signature.ret.clone()),
                },
                None => Ty::Unknown,
            },
            DefKind::Variant => self.variant_ty(def),
            // A type name is not a value. Leaving this as `Unknown` would be
            // worse than wrong: `Unknown` absorbs, so `Io.write(Console, "hi")`
            // would sail through and the capability system would be decorative.
            DefKind::Type | DefKind::Record | DefKind::Choice => {
                self.not_a_value(ident, "a type");
                Ty::Unknown
            }
            DefKind::Effect => {
                self.not_a_value(ident, "an effect");
                Ty::Unknown
            }
            DefKind::Builtin if !matches!(ident.name.as_str(), "ok" | "err") => {
                self.not_a_value(ident, "a type");
                Ty::Unknown
            }
            _ => Ty::Unknown,
        }
    }

    fn not_a_value(&mut self, ident: &Ident, what: &str) {
        let name = &ident.name;
        let mut diagnostic = Diagnostic::error(
            codes::NOT_A_VALUE,
            self.file,
            ident.span,
            format!("`{name}` is {what}, not a value"),
        )
        .with_primary_label(format!("`{name}` cannot be used here"));

        if matches!(name.as_str(), "System" | "Console" | "Clock") {
            diagnostic = diagnostic.with_note(format!(
                "a `{name}` cannot be constructed, only received, which is the point: \
                 a function that was not handed one cannot reach one"
            ));
        }

        self.emit(diagnostic);
    }

    /// The type a variant produces, which is the choice it belongs to.
    fn variant_ty(&self, variant: DefId) -> Ty {
        match self.resolutions.def(variant).parent {
            Some(parent) => Ty::Named(parent),
            None => Ty::Unknown,
        }
    }

    fn field_ty(&mut self, receiver: &Ty, name: &Ident) -> Ty {
        if receiver.absorbs() {
            return Ty::Unknown;
        }

        // `System` is the root of all authority, and its fields are the
        // narrower capabilities it carries. Handing one of them to a function
        // gives that function exactly that and nothing else.
        if let Ty::Named(def) = receiver
            && self.resolutions.builtin("System") == Some(*def)
        {
            let narrower = match name.name.as_str() {
                "console" => Some("Console"),
                "clock" => Some("Clock"),
                _ => None,
            };
            return match narrower.and_then(|name| self.resolutions.builtin(name)) {
                Some(def) => Ty::Named(def),
                None => {
                    self.emit(
                        Diagnostic::error(
                            codes::NO_SUCH_FIELD,
                            self.file,
                            name.span,
                            format!("`System` carries no `{}`", name.name),
                        )
                        .with_primary_label("no such capability")
                        .with_note("it carries `console` and `clock`"),
                    );
                    Ty::Unknown
                }
            };
        }

        let looked_through = self.widen(receiver);
        if let Ty::Named(def) = looked_through
            && let Some(Nominal::Record { fields }) = self.types.nominal(def)
            && let Some(field) = fields.iter().find(|field| field.name == name.name)
        {
            return field.ty.clone();
        }

        let described = self.types.describe(receiver);
        let mut diagnostic = Diagnostic::error(
            codes::NO_SUCH_FIELD,
            self.file,
            name.span,
            format!("{described} has no field `{}`", name.name),
        )
        .with_primary_label("no such field");

        if let Ty::Named(def) = looked_through
            && let Some(Nominal::Record { fields }) = self.types.nominal(def)
        {
            let available: Vec<&str> = fields.iter().map(|field| field.name.as_str()).collect();
            diagnostic = diagnostic.with_note(format!("it has {}", list(&available)));
        }

        self.emit(diagnostic);
        Ty::Unknown
    }

    fn infer_call(&mut self, callee: &Expr, args: &[Expr], span: Span) -> Ty {
        let callee_def = match callee {
            Expr::Ident(ident) => self.def_of(ident),
            Expr::Field { name, .. } => self.resolutions.resolution(name.span),
            _ => None,
        };

        // `ok` and `err` are the two constructors the language provides. Each
        // says nothing about the other side, which is why the unknown type has
        // to absorb rather than unify.
        if let Some(def) = callee_def
            && self.resolutions.def(def).kind == DefKind::Builtin
        {
            let name = self.resolutions.def(def).name.clone();
            if name == "ok" || name == "err" {
                let mut types: Vec<Ty> = args.iter().map(|arg| self.infer(arg)).collect();
                if types.len() != 1 {
                    self.emit(
                        Diagnostic::error(
                            codes::WRONG_ARITY,
                            self.file,
                            span,
                            format!(
                                "`{name}` takes one argument, but {} were given",
                                types.len()
                            ),
                        )
                        .with_primary_label("wrong number of arguments"),
                    );
                    return Ty::Unknown;
                }
                let carried = types.remove(0);
                return if name == "ok" {
                    Ty::Result(Box::new(carried), Box::new(Ty::Unknown))
                } else {
                    Ty::Result(Box::new(Ty::Unknown), Box::new(carried))
                };
            }
        }

        // A direct call to a declared function, where the parameter spans are
        // available and a mismatch can point at the declaration.
        if let Some(def) = callee_def
            && let Some(signature) = self.signatures.get(&def).cloned()
        {
            let name = self.types.name_of(def).to_string();
            if args.len() != signature.params.len() {
                self.emit(
                    Diagnostic::error(
                        codes::WRONG_ARITY,
                        self.file,
                        span,
                        format!(
                            "`{name}` takes {} argument{}, but {} {} given",
                            signature.params.len(),
                            if signature.params.len() == 1 { "" } else { "s" },
                            args.len(),
                            if args.len() == 1 { "was" } else { "were" }
                        ),
                    )
                    .with_primary_label("wrong number of arguments")
                    .with_secondary(signature.span, "declared here"),
                );
            }
            for (index, arg) in args.iter().enumerate() {
                let actual = self.infer(arg);
                if let Some(param) = signature.params.get(index) {
                    let param_ty = param.ty.clone();
                    self.assign(
                        &actual,
                        &param_ty,
                        Some(arg),
                        arg.span(),
                        Some((param.span, "the parameter it is passed to".to_string())),
                    );
                }
            }
            return signature.ret;
        }

        let callee_ty = self.infer(callee);
        match callee_ty {
            Ty::Fn { params, ret } => {
                if args.len() != params.len() {
                    self.emit(
                        Diagnostic::error(
                            codes::WRONG_ARITY,
                            self.file,
                            span,
                            format!(
                                "this takes {} argument{}, but {} were given",
                                params.len(),
                                if params.len() == 1 { "" } else { "s" },
                                args.len()
                            ),
                        )
                        .with_primary_label("wrong number of arguments"),
                    );
                }
                for (index, arg) in args.iter().enumerate() {
                    let actual = self.infer(arg);
                    if let Some(param) = params.get(index) {
                        let param = param.clone();
                        self.assign(&actual, &param, Some(arg), arg.span(), None);
                    }
                }
                *ret
            }
            other if other.absorbs() => {
                for arg in args {
                    self.infer(arg);
                }
                Ty::Unknown
            }
            other => {
                let described = self.types.describe(&other);
                self.emit(
                    Diagnostic::error(
                        codes::NOT_CALLABLE,
                        self.file,
                        callee.span(),
                        format!("{described} is not a function"),
                    )
                    .with_primary_label("not callable"),
                );
                for arg in args {
                    self.infer(arg);
                }
                Ty::Unknown
            }
        }
    }

    fn infer_struct_lit(&mut self, path: &Expr, fields: &[FieldInit], span: Span) -> Ty {
        let ctor = match path {
            Expr::Ident(ident) => self.def_of(ident),
            Expr::Field { name, .. } => self.resolutions.resolution(name.span),
            _ => None,
        };

        if let Some(def) = ctor {
            match self.resolutions.def(def).kind {
                DefKind::Record => {
                    if let Some(Nominal::Record { fields: declared }) = self.types.nominal(def) {
                        let declared = declared.clone();
                        let name = self.types.name_of(def).to_string();
                        self.check_literal_fields(&declared, fields, span, &name);
                        return Ty::Named(def);
                    }
                }
                DefKind::Variant => {
                    let parent = self.resolutions.def(def).parent;
                    let declared = parent
                        .and_then(|parent| self.types.nominal(parent))
                        .and_then(|nominal| match nominal {
                            Nominal::Choice { variants } => {
                                variants.iter().find(|variant| variant.def == def).cloned()
                            }
                            _ => None,
                        });
                    if let Some(variant) = declared {
                        let name = variant.name.clone();
                        self.check_literal_fields(
                            variant.fields.as_deref().unwrap_or(&[]),
                            fields,
                            span,
                            &name,
                        );
                    }
                    return self.variant_ty(def);
                }
                _ => {}
            }
        }

        let path_ty = self.infer(path);
        if !path_ty.absorbs() {
            let described = self.types.describe(&path_ty);
            self.emit(
                Diagnostic::error(
                    codes::NOT_A_CONSTRUCTOR,
                    self.file,
                    path.span(),
                    format!("{described} is not a record or a variant"),
                )
                .with_primary_label("cannot be built with a literal"),
            );
        }
        for field in fields {
            match &field.value {
                Some(value) => {
                    self.infer(value);
                }
                None => {
                    self.ident_ty(&field.name);
                }
            }
        }
        Ty::Unknown
    }

    fn check_literal_fields(
        &mut self,
        declared: &[FieldTy],
        fields: &[FieldInit],
        span: Span,
        what: &str,
    ) {
        let mut seen: HashSet<String> = HashSet::new();

        for init in fields {
            match declared
                .iter()
                .find(|field| field.name == init.name.name)
                .cloned()
            {
                Some(field) => {
                    seen.insert(field.name.clone());
                    let actual = match &init.value {
                        Some(value) => self.infer(value),
                        // Shorthand: the label is also the value.
                        None => self.ident_ty(&init.name),
                    };
                    let value_span = init
                        .value
                        .as_ref()
                        .map(Expr::span)
                        .unwrap_or(init.name.span);
                    self.assign(
                        &actual,
                        &field.ty,
                        init.value.as_ref(),
                        value_span,
                        Some((field.span, "the field it is assigned to".to_string())),
                    );
                }
                None => {
                    let available: Vec<&str> =
                        declared.iter().map(|field| field.name.as_str()).collect();
                    self.emit(
                        Diagnostic::error(
                            codes::UNKNOWN_FIELD,
                            self.file,
                            init.name.span,
                            format!("`{what}` has no field `{}`", init.name.name),
                        )
                        .with_primary_label("no such field")
                        .with_note(format!("it has {}", list(&available))),
                    );
                    if let Some(value) = &init.value {
                        self.infer(value);
                    }
                }
            }
        }

        let missing: Vec<&str> = declared
            .iter()
            .map(|field| field.name.as_str())
            .filter(|name| !seen.contains(*name))
            .collect();

        if !missing.is_empty() {
            self.emit(
                Diagnostic::error(
                    codes::MISSING_FIELDS,
                    self.file,
                    span,
                    format!("`{what}` is missing {}", list(&missing)),
                )
                .with_primary_label("incomplete literal")
                .with_note(
                    "every field has to be given, because a partially built value is not a value",
                ),
            );
        }
    }

    fn infer_match(&mut self, scrutinee: &Expr, arms: &[MatchArm], span: Span) -> Ty {
        let scrutinee_ty = self.infer(scrutinee);

        let mut result: Option<Ty> = None;
        for arm in arms {
            self.bind_pattern(&arm.pattern, &scrutinee_ty);
            let arm_ty = self.infer(&arm.body);
            match &result {
                None => result = Some(arm_ty),
                Some(expected) if expected.absorbs() => result = Some(arm_ty),
                Some(expected) => {
                    let expected = expected.clone();
                    self.assign(
                        &arm_ty,
                        &expected,
                        Some(&arm.body),
                        arm.body.span(),
                        Some((arms[0].span, "the first arm".to_string())),
                    );
                }
            }
        }

        self.check_exhaustive(&scrutinee_ty, arms, span);
        result.unwrap_or(Ty::Never)
    }

    /// A choice is worth nothing if a case can be forgotten, so this is not
    /// optional. It is also worth nothing if a case can be swallowed, which is
    /// why a catch-all arm over a choice is rejected rather than accepted as
    /// coverage.
    fn check_exhaustive(&mut self, scrutinee: &Ty, arms: &[MatchArm], span: Span) {
        if matches!(self.widen(scrutinee), Ty::Result(_, _)) {
            self.check_result_exhaustive(arms, span);
            return;
        }

        let Ty::Named(def) = self.widen(scrutinee) else {
            return;
        };
        let Some(Nominal::Choice { variants }) = self.types.nominal(def).cloned() else {
            return;
        };

        let mut covered: HashSet<DefId> = HashSet::new();
        let mut catch_all: Option<Span> = None;

        for arm in arms {
            match &arm.pattern {
                Pattern::Wildcard(span) => {
                    catch_all.get_or_insert(*span);
                }
                Pattern::Path { segments, .. } => {
                    if let Some(last) = segments.last() {
                        match self.resolutions.resolution(last.span) {
                            Some(def) if self.resolutions.def(def).kind == DefKind::Variant => {
                                covered.insert(def);
                            }
                            // A bare binding matches every variant.
                            _ => {
                                catch_all.get_or_insert(arm.pattern.span());
                            }
                        }
                    }
                }
                Pattern::Tuple { path, .. } | Pattern::Record { path, .. } => {
                    if let Some(last) = path.last()
                        && let Some(def) = self.resolutions.resolution(last.span)
                    {
                        covered.insert(def);
                    }
                }
                _ => {}
            }
        }

        let name = self.types.name_of(def).to_string();

        if let Some(catch_all) = catch_all {
            self.emit(
                Diagnostic::error(
                    codes::CATCH_ALL_ON_CHOICE,
                    self.file,
                    catch_all,
                    format!("this arm matches every variant of `{name}`"),
                )
                .with_primary_label("catches everything")
                .with_secondary(span, "in this match")
                .with_note(format!(
                    "list the variants instead: adding one to `{name}` should break every match that has to care, and a catch-all arm is what stops that from happening"
                )),
            );
            return;
        }

        let missing: Vec<&str> = variants
            .iter()
            .filter(|variant| !covered.contains(&variant.def))
            .map(|variant| variant.name.as_str())
            .collect();

        if !missing.is_empty() {
            self.emit(
                Diagnostic::error(
                    codes::NON_EXHAUSTIVE_MATCH,
                    self.file,
                    span,
                    format!("this match does not cover {}", list(&missing)),
                )
                .with_primary_label("not exhaustive")
                .with_note(format!(
                    "every variant of `{name}` needs an arm, and there is no wildcard to fall back on"
                )),
            );
        }
    }

    /// A `Result` has two cases and the same rules apply: both have to be
    /// handled, and neither can be swallowed by a catch-all.
    fn check_result_exhaustive(&mut self, arms: &[MatchArm], span: Span) {
        let mut covered: HashSet<&'static str> = HashSet::new();
        let mut catch_all: Option<Span> = None;

        for arm in arms {
            match &arm.pattern {
                Pattern::Wildcard(span) => {
                    catch_all.get_or_insert(*span);
                }
                Pattern::Path { .. } => {
                    catch_all.get_or_insert(arm.pattern.span());
                }
                Pattern::Tuple { path, .. } => {
                    let name = path.last().and_then(|last| {
                        let def = self.resolutions.resolution(last.span)?;
                        Some(self.resolutions.def(def).name.clone())
                    });
                    match name.as_deref() {
                        Some("ok") => {
                            covered.insert("ok");
                        }
                        Some("err") => {
                            covered.insert("err");
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        if let Some(catch_all) = catch_all {
            self.emit(
                Diagnostic::error(
                    codes::CATCH_ALL_ON_CHOICE,
                    self.file,
                    catch_all,
                    "this arm matches both cases of the `Result`",
                )
                .with_primary_label("catches everything")
                .with_secondary(span, "in this match")
                .with_note("write `ok(...)` and `err(...)` instead, so the failure case cannot be handled by accident"),
            );
            return;
        }

        let missing: Vec<&str> = ["ok", "err"]
            .into_iter()
            .filter(|case| !covered.contains(case))
            .collect();

        if !missing.is_empty() {
            self.emit(
                Diagnostic::error(
                    codes::NON_EXHAUSTIVE_MATCH,
                    self.file,
                    span,
                    format!("this match does not cover {}", list(&missing)),
                )
                .with_primary_label("not exhaustive")
                .with_note("a `Result` has two cases and both need an arm"),
            );
        }
    }

    fn bind_pattern(&mut self, pattern: &Pattern, ty: &Ty) {
        match pattern {
            Pattern::Wildcard(_)
            | Pattern::Int { .. }
            | Pattern::Str { .. }
            | Pattern::Bool { .. }
            | Pattern::Error(_) => {}

            Pattern::Path { segments, .. } => {
                if let Some(only) = segments.first()
                    && segments.len() == 1
                    && let Some(def) = self.def_of(only)
                    && self.resolutions.def(def).kind == DefKind::Local
                {
                    self.def_types.insert(def, ty.clone());
                }
            }

            Pattern::Tuple {
                path,
                elements,
                span,
            } => self.bind_result_pattern(path, elements, *span, ty),

            Pattern::Record { path, fields, .. } => {
                let variant_fields = path
                    .last()
                    .and_then(|last| self.resolutions.resolution(last.span))
                    .and_then(|def| self.variant_fields(def));

                for field in fields {
                    let field_ty = variant_fields
                        .as_ref()
                        .and_then(|fields| fields.iter().find(|f| f.name == field.name.name))
                        .map(|f| f.ty.clone())
                        .unwrap_or(Ty::Unknown);

                    match &field.pattern {
                        Some(pattern) => self.bind_pattern(pattern, &field_ty),
                        None => {
                            if let Some(def) = self.def_of(&field.name) {
                                self.def_types.insert(def, field_ty);
                            }
                        }
                    }
                }
            }
        }
    }

    /// `ok(x)` and `err(e)`, the only patterns that carry a value positionally.
    ///
    /// Variants have named fields, so anything else in this shape can never
    /// match, and a pattern that can never match should say so rather than
    /// quietly falling through.
    fn bind_result_pattern(&mut self, path: &[Ident], elements: &[Pattern], span: Span, ty: &Ty) {
        let head = path.last().and_then(|last| {
            let def = self.resolutions.resolution(last.span)?;
            (self.resolutions.def(def).kind == DefKind::Builtin)
                .then(|| self.resolutions.def(def).name.clone())
        });

        let inner = match (head.as_deref(), self.widen(ty)) {
            (Some("ok"), Ty::Result(ok, _)) => *ok,
            (Some("err"), Ty::Result(_, err)) => *err,
            (Some(name @ ("ok" | "err")), other) => {
                if !other.absorbs() {
                    let described = self.types.describe(&other);
                    self.emit(
                        Diagnostic::error(
                            codes::PATTERN_MISMATCH,
                            self.file,
                            span,
                            format!("`{name}(...)` matches a `Result`, and this is {described}"),
                        )
                        .with_primary_label("cannot match"),
                    );
                }
                Ty::Unknown
            }
            _ => {
                self.emit(
                    Diagnostic::error(
                        codes::PATTERN_MISMATCH,
                        self.file,
                        span,
                        "only `ok` and `err` carry a value in a pattern",
                    )
                    .with_primary_label("not a pattern that can match")
                    .with_note(
                        "variants have named fields, so they are matched with `Variant { field }`",
                    ),
                );
                Ty::Unknown
            }
        };

        if elements.len() != 1 {
            self.emit(
                Diagnostic::error(
                    codes::PATTERN_MISMATCH,
                    self.file,
                    span,
                    format!(
                        "this pattern binds {} values, and it should bind one",
                        elements.len()
                    ),
                )
                .with_primary_label("wrong number of bindings"),
            );
        }

        for element in elements {
            self.bind_pattern(element, &inner);
        }
    }

    fn variant_fields(&self, variant: DefId) -> Option<Vec<FieldTy>> {
        let parent = self.resolutions.def(variant).parent?;
        match self.types.nominal(parent)? {
            Nominal::Choice { variants } => variants
                .iter()
                .find(|candidate| candidate.def == variant)
                .and_then(|candidate| candidate.fields.clone()),
            _ => None,
        }
    }

    fn binary_ty(&mut self, op: BinaryOp, left: &Ty, right: &Ty, lhs: &Expr, rhs: &Expr) -> Ty {
        use BinaryOp::*;

        // If either side is unknown, the operator cannot say anything useful
        // and complaining about the other side would be guessing.
        let unknown = left.absorbs() || right.absorbs();

        match op {
            Add | Sub | Mul | Div | Rem => {
                if unknown {
                    return Ty::Unknown;
                }
                self.assign(left, &Ty::Int, Some(lhs), lhs.span(), None);
                self.assign(right, &Ty::Int, Some(rhs), rhs.span(), None);
                Ty::Int
            }
            Lt | Le | Gt | Ge => {
                if !unknown {
                    // Ordering is not tied to a trait yet, so this only insists
                    // the two sides agree. See the open questions in 02-syntax.
                    self.assign(
                        right,
                        left,
                        Some(rhs),
                        rhs.span(),
                        Some((lhs.span(), "compared with this".to_string())),
                    );
                }
                Ty::Bool
            }
            Eq | Ne => {
                if !unknown {
                    self.assign(
                        right,
                        left,
                        Some(rhs),
                        rhs.span(),
                        Some((lhs.span(), "compared with this".to_string())),
                    );
                }
                Ty::Bool
            }
            And | Or => {
                if !unknown {
                    self.assign(left, &Ty::Bool, Some(lhs), lhs.span(), None);
                    self.assign(right, &Ty::Bool, Some(rhs), rhs.span(), None);
                }
                Ty::Bool
            }
        }
    }
}

/// `a`, `a` and `b`, `a`, `b` and `c`.
fn list(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [only] => format!("`{only}`"),
        [rest @ .., last] => {
            let rest: Vec<String> = rest.iter().map(|item| format!("`{item}`")).collect();
            format!("{} and `{last}`", rest.join(", "))
        }
    }
}

// -- constant evaluation ---------------------------------------------------

/// The sliver of evaluation the compiler can do at check time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Constant {
    Int(i64),
    Bool(bool),
}

fn constant(expr: &Expr) -> Option<Constant> {
    match expr {
        Expr::Int { value, .. } => Some(Constant::Int(*value)),
        Expr::Bool { value, .. } => Some(Constant::Bool(*value)),
        Expr::Unary {
            op: UnaryOp::Neg,
            operand,
            ..
        } => match constant(operand)? {
            Constant::Int(value) => value.checked_neg().map(Constant::Int),
            Constant::Bool(_) => None,
        },
        _ => None,
    }
}

/// Evaluates a refinement predicate with `value` bound.
///
/// Returns `None` for anything it cannot decide, which is most things. That is
/// the honest answer and the caller treats it as such.
fn evaluate(predicate: &Expr, value: Constant) -> Option<Constant> {
    match predicate {
        Expr::Int { .. } | Expr::Bool { .. } => constant(predicate),
        Expr::Ident(ident) if ident.name == "value" => Some(value),
        Expr::Unary { op, operand, .. } => match (op, evaluate(operand, value)?) {
            (UnaryOp::Neg, Constant::Int(v)) => v.checked_neg().map(Constant::Int),
            (UnaryOp::Not, Constant::Bool(v)) => Some(Constant::Bool(!v)),
            _ => None,
        },
        Expr::Binary { op, lhs, rhs, .. } => {
            let left = evaluate(lhs, value)?;
            let right = evaluate(rhs, value)?;
            apply(*op, left, right)
        }
        _ => None,
    }
}

fn apply(op: BinaryOp, left: Constant, right: Constant) -> Option<Constant> {
    use BinaryOp::*;
    use Constant::*;

    match (left, right) {
        (Int(a), Int(b)) => match op {
            Add => a.checked_add(b).map(Int),
            Sub => a.checked_sub(b).map(Int),
            Mul => a.checked_mul(b).map(Int),
            Div => a.checked_div(b).map(Int),
            Rem => a.checked_rem(b).map(Int),
            Lt => Some(Bool(a < b)),
            Le => Some(Bool(a <= b)),
            Gt => Some(Bool(a > b)),
            Ge => Some(Bool(a >= b)),
            Eq => Some(Bool(a == b)),
            Ne => Some(Bool(a != b)),
            And | Or => None,
        },
        (Bool(a), Bool(b)) => match op {
            And => Some(Bool(a && b)),
            Or => Some(Bool(a || b)),
            Eq => Some(Bool(a == b)),
            Ne => Some(Bool(a != b)),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::list;

    #[test]
    fn lists_read_like_english() {
        assert_eq!(list(&[]), "");
        assert_eq!(list(&["a"]), "`a`");
        assert_eq!(list(&["a", "b"]), "`a` and `b`");
        assert_eq!(list(&["a", "b", "c"]), "`a`, `b` and `c`");
    }
}
