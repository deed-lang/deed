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
use std::rc::Rc;

use vow_ast::{
    BinaryOp, Block, Ensures, Expr, FieldInit, FnDecl, Ident, Item, MatchArm, Module, Outcome,
    Pattern, Stmt, Type, TypeAlias, UnaryOp,
};
use vow_diagnostics::{Diagnostic, FileId, Span};
use vow_resolve::{DefId, DefKind, Resolutions};

use crate::codes;
use crate::facts::{self, Facts, Range, Truth};
use crate::surface::{PRELUDE_MODULE, SurfaceItem, World};
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
pub fn check(file: FileId, module: &Module, resolutions: &Resolutions, world: &World) -> Checked {
    let mut checker = Checker {
        file,
        resolutions,
        world,
        types: Types::default(),
        diagnostics: Vec::new(),
        def_types: HashMap::new(),
        signatures: HashMap::new(),
        aliases: HashMap::new(),
        alias_targets: HashMap::new(),
        alias_stack: Vec::new(),
        returns: Vec::new(),
        facts: Facts::new(),
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
    /// The range a call to this is known to land in, from the declared return
    /// type and from the `ensures` clause. `ANY` when nothing is promised.
    guarantee: Range,
}

struct Checker<'a> {
    file: FileId,
    resolutions: &'a Resolutions,
    /// The lowered declarations of every other module being compiled.
    world: &'a World,
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
    /// What is known about the integers in scope, at the point being checked.
    ///
    /// This is the Proven tier. Without it, a refinement can only be
    /// discharged against a literal, which is a small enough slice of the tier
    /// that almost every refinement in real code became a runtime check.
    facts: Facts,
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
        // Every `Io` operation takes the capability it acts on as its first
        // argument. The row says what kind of thing is happening, the argument
        // says which resource it happens to, and neither is enough alone.
        let console = capability("Console");
        let clock = capability("Clock");
        let dir = capability("Dir");

        // `open` hands back a narrower `Dir` and `read` hands back the file's
        // contents. Both can fail, because a path that is not there is not a
        // bug in the caller.
        let io_error = |ok: Ty| Ty::Result(Box::new(ok), Box::new(Ty::Str));

        let operations: [(&str, Vec<Ty>, Ty); 4] = [
            ("write", vec![console, Ty::Str], Ty::Unit),
            ("now", vec![clock], Ty::Int),
            ("open", vec![dir.clone(), Ty::Str], io_error(dir.clone())),
            ("read", vec![dir, Ty::Str], io_error(Ty::Str)),
        ];

        for (name, params, ret) in operations {
            let Some(def) = self.resolutions.builtin(name) else {
                continue;
            };
            self.types.set_name(def, name.to_string());
            self.signatures.insert(
                def,
                Signature {
                    params: params
                        .into_iter()
                        .map(|ty| ParamTy {
                            ty,
                            span: Span::at(0),
                        })
                        .collect(),
                    ret,
                    span: Span::at(0),
                    guarantee: Range::ANY,
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
                    let mut signature = self.lower_signature(&function.sig);
                    signature.guarantee = signature
                        .guarantee
                        .meet(promised_by(&function.contract.ensures));
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

        let guarantee = self.guarantee_of(&ret);

        Signature {
            params,
            ret,
            span: sig.span,
            guarantee,
        }
    }

    /// What a return type alone promises about the value coming back.
    ///
    /// A `Result` promises nothing usable here: the call site holds the
    /// wrapper, not the payload, and giving a range to a `Result` would be
    /// answering a question nobody asked.
    fn guarantee_of(&mut self, ret: &Ty) -> Range {
        match ret {
            Ty::Named(def) => self.refinement_range(*def),
            _ => Range::ANY,
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
                        // about one except that you were handed it, and there
                        // is exactly one `Console`, so it is named under the
                        // prelude rather than under this module.
                        "System" | "Console" | "Clock" | "Dir" => capability(&name.name),
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
                    // A name from another module. It has a type now, and the
                    // identity is the module path and the name rather than a
                    // `DefId`, which cannot mean anything outside the table it
                    // came from.
                    DefKind::Import => self.imported_ty(def, name),
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

    /// The type an imported name denotes when used in type position.
    ///
    /// A transparent alias is expanded, because it was not a distinct type
    /// where it was declared and crossing a module boundary does not make it
    /// one. A refinement stays nominal for the same reason it does at home.
    fn imported_ty(&mut self, def: DefId, name: &Ident) -> Ty {
        let Some(module) = self.resolutions.import_module(def) else {
            return Ty::Unknown;
        };
        let external = Ty::External {
            module: Rc::from(module),
            name: Rc::from(name.name.as_str()),
        };

        match self.world.get(module, &name.name) {
            Some(SurfaceItem::Alias { target }) => target.clone(),
            Some(
                SurfaceItem::Record { .. }
                | SurfaceItem::Choice { .. }
                | SurfaceItem::Refinement { .. },
            ) => external,
            // A function or a handler is not a type, and the resolver already
            // said the name exists, so this is the only place to say what is
            // wrong with using it here.
            Some(other) => {
                let what = match other {
                    SurfaceItem::Function { .. } => "a function",
                    SurfaceItem::Effect { .. } => "an effect",
                    SurfaceItem::Handler => "a handler",
                    SurfaceItem::Variant { .. } => "a variant",
                    _ => "not a type",
                };
                self.emit(
                    Diagnostic::error(
                        codes::NOT_A_TYPE,
                        self.file,
                        name.span,
                        format!("`{}` is {what}, not a type", name.name),
                    )
                    .with_primary_label("not a type")
                    .with_secondary(name.span, format!("declared in `{module}`")),
                );
                Ty::Unknown
            }
            // The module was not among the files being compiled, which the
            // resolver already reported. Nothing more to add.
            None => Ty::Unknown,
        }
    }

    /// The declaration behind an external type, if it can be found.
    fn external_item(&self, ty: &Ty) -> Option<&'a SurfaceItem> {
        let Ty::External { module, name } = ty else {
            return None;
        };
        self.world.get(module, name)
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
        if let Some(SurfaceItem::Refinement { base }) = self.external_item(ty) {
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

    /// The value inside `ok(x)` or `err(x)`, when the expression is one.
    ///
    /// Returned separately so blame lands on the value rather than on the
    /// constructor around it, and so a refinement on the success type is
    /// discharged against `x` rather than against a `Result` that has no
    /// range.
    fn result_parts<'e>(&self, expr: Option<&'e Expr>) -> (Option<&'e Expr>, Option<&'e Expr>) {
        let Some(Expr::Call { callee, args, .. }) = expr else {
            return (None, None);
        };
        let Some(def) = (match &**callee {
            Expr::Ident(ident) => self.def_of(ident),
            _ => None,
        }) else {
            return (None, None);
        };
        if self.resolutions.def(def).kind != DefKind::Builtin {
            return (None, None);
        }
        let inner = args.first();
        match self.resolutions.def(def).name.as_str() {
            "ok" => (inner, None),
            "err" => (None, inner),
            _ => (None, None),
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

        // `ok(n)` where the success type is refined. Without this, a guard
        // that establishes a refinement and then returns it fails to type
        // check, which is the ordinary shape for establishing one at all.
        if let (Ty::Result(a_ok, a_err), Ty::Result(e_ok, e_err)) = (actual, expected) {
            let (ok_expr, err_expr) = self.result_parts(expr);
            let ok_span = ok_expr.map(Expr::span).unwrap_or(span);
            let err_span = err_expr.map(Expr::span).unwrap_or(span);
            self.assign(a_ok, e_ok, ok_expr, ok_span, because.clone());
            self.assign(a_err, e_err, err_expr, err_span, because);
            return;
        }

        // Narrowing into a refinement is the interesting direction. The value
        // may already be refined by something else: going sideways is widening
        // out of one and back into the other, and the predicate that arrives is
        // often enough to discharge the predicate that is wanted.
        if let Ty::Named(def) = expected
            && let Some(Nominal::Refinement { base, .. }) = self.types.nominal(*def)
        {
            let base = base.clone();
            if self.widen(actual) == base || actual.absorbs() {
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
    /// The range a definition's declared type admits, if it is refined.
    ///
    /// A parameter of type `Positive` is already known to be positive, and
    /// making the author write `where n > 0` next to it would be asking them
    /// to repeat the type in prose.
    fn declared_range(&self, def: DefId) -> Range {
        let Some(Ty::Named(refinement)) = self.def_types.get(&def) else {
            return Range::ANY;
        };
        self.refinement_range(*refinement)
    }

    /// The range a refinement admits, when its predicate is simple enough.
    fn refinement_range(&self, refinement: DefId) -> Range {
        let Some(alias) = self.aliases.get(&refinement) else {
            return Range::ANY;
        };
        match &alias.refinement {
            Some(predicate) => facts::range_admitted_by(predicate),
            None => Range::ANY,
        }
    }

    /// Turns an identifier into what it refers to, for the fact machinery.
    ///
    /// Captures the resolution table rather than the checker, so the result
    /// can be alive while the checker is being mutated.
    fn resolver(&self) -> impl Fn(&Expr) -> Option<DefId> + use<'a> {
        let resolutions = self.resolutions;
        move |expr: &Expr| match expr {
            Expr::Ident(ident) => resolutions.resolution(ident.span),
            _ => None,
        }
    }

    /// The range a call to `callee` is promised to land in.
    ///
    /// Reading a promise is only honest if the promise is kept, and both of
    /// these are. An `ensures` clause is evaluated on the way out of every
    /// call, whatever tier it landed in, and a refined return type is checked
    /// against its predicate at the same point. So a caller holding the
    /// returned value is holding something that already passed. The tier on
    /// the callee's own obligation says how much was settled ahead of time; it
    /// does not say whether the check happens.
    ///
    /// This is the one place the reasoning leaves the function being checked,
    /// so it is also the one place a broken contract could launder itself into
    /// a proof somewhere else. That is why it reads what the callee declared
    /// and never what its body happens to do.
    fn call_range(&self, callee: &Expr) -> Range {
        let def = match callee {
            Expr::Ident(ident) => self.def_of(ident),
            Expr::Field { name, .. } => self.resolutions.resolution(name.span),
            _ => None,
        };
        let Some(def) = def else {
            return Range::ANY;
        };

        if let Some(signature) = self.signatures.get(&def) {
            return signature.guarantee;
        }

        // An imported function. Its refinement is opaque out here, which is
        // why the range travels rather than the predicate: a pair of bounds
        // says what the caller needs without exporting how it was decided.
        let Some(module) = self.resolutions.import_module(def) else {
            return Range::ANY;
        };
        match self.world.get(module, &self.resolutions.def(def).name) {
            Some(SurfaceItem::Function { guarantee, .. }) => *guarantee,
            _ => Range::ANY,
        }
    }

    /// What the fact machinery is allowed to look up while checking this body.
    fn env(
        &self,
    ) -> (
        impl Fn(&Expr) -> Option<DefId> + use<'a>,
        impl Fn(&Expr) -> Range + '_,
    ) {
        (self.resolver(), |callee: &Expr| self.call_range(callee))
    }

    fn narrowed_by(&self, condition: &Expr, when_true: bool) -> Facts {
        self.narrowed_from(&self.facts, condition, when_true)
    }

    fn narrowed_from(&self, base: &Facts, condition: &Expr, when_true: bool) -> Facts {
        let (def_of, call) = self.env();
        let env = facts::Env {
            def_of: &def_of,
            call: &call,
        };
        facts::narrowed(condition, base, &env, when_true)
    }

    fn range_of(&self, expr: &Expr) -> Range {
        let (def_of, call) = self.env();
        let env = facts::Env {
            def_of: &def_of,
            call: &call,
        };
        facts::range_of(expr, &self.facts, &env)
    }

    /// Whether the facts in scope settle a refinement predicate for `expr`.
    fn proves(&self, predicate: &Expr, expr: Option<&Expr>) -> Truth {
        let Some(expr) = expr else {
            return Truth::Unknown;
        };
        let subject = self.range_of(expr);
        let with_subject = self.facts.with_subject(subject);
        let (def_of, call) = self.env();
        let env = facts::Env {
            def_of: &def_of,
            call: &call,
        };
        facts::holds(predicate, &with_subject, &env)
    }

    /// Records the tier an obligation landed in, and says so when it is not
    /// the one the author probably wanted.
    ///
    /// The Proven tier is interval reasoning: what is known about the integers
    /// in scope, from the contract, from the types, and from the conditions
    /// above this point. It cannot relate two variables to each other and it
    /// cannot see through a call, which is written down in `facts.rs` and in
    /// `design/02-syntax.md` rather than left to be discovered.
    fn discharge(&mut self, refinement: DefId, expr: Option<&Expr>, span: Span) {
        let predicate = self
            .aliases
            .get(&refinement)
            .and_then(|alias| alias.refinement.as_ref());

        let outcome = match predicate {
            Some(predicate) => self.proves(predicate, expr),
            None => Truth::Unknown,
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
            Truth::Always => self.types.push_obligation(Obligation {
                span,
                refinement,
                tier: Tier::Proven,
            }),
            Truth::Never => {
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

    fn check_module(&mut self, module: &'a Module) {
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

    fn check_fn(&mut self, function: &'a FnDecl) {
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

        // A fresh set of facts per function. Nothing a caller knows survives
        // the call, because the body is checked against its own contract and
        // nothing else, which is P1.
        self.facts = Facts::new();
        for param in &function.sig.params {
            if let Some(def) = self.def_of(&param.name) {
                let range = self.declared_range(def);
                if !range.is_any() {
                    self.facts.set(def, range);
                }
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

        // A `where` clause is a fact about the body, not just a check on the
        // caller. This is where most of the Proven tier comes from: the
        // precondition usually says exactly what a refinement below it needs.
        for requirement in &function.contract.requires {
            self.facts = self.narrowed_by(requirement, true);
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
        self.check_block_against(
            &function.body,
            &ret,
            Some((ret_span, "the declared return type".to_string())),
        );
        self.returns.pop();
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

    /// Checks an expression against the type that was wanted.
    ///
    /// Local bidirectional checking, and the reason it exists is refinements.
    /// Inferring a body and then comparing the answer against the return type
    /// loses where each part of it was written, so the branch that established
    /// a fact is no longer the branch being checked and every refinement in a
    /// conditional falls back to a runtime guard.
    fn check_against(
        &mut self,
        expr: &'a Expr,
        expected: &Ty,
        because: Option<(Span, String)>,
    ) -> Ty {
        match expr {
            Expr::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => self.check_if(
                condition,
                then_branch,
                else_branch.as_deref(),
                Some((expected.clone(), because)),
            ),
            Expr::Block(block) => self.check_block_against(block, expected, because),
            other => {
                let ty = self.infer(other);
                self.assign(&ty, expected, Some(other), other.span(), because);
                ty
            }
        }
    }

    fn check_block_against(
        &mut self,
        block: &'a Block,
        expected: &Ty,
        because: Option<(Span, String)>,
    ) -> Ty {
        let mut diverges = false;
        for stmt in &block.stmts {
            self.check_stmt(stmt);
            if matches!(stmt, Stmt::Return { .. }) {
                diverges = true;
            }
        }

        let ty = match &block.tail {
            Some(tail) => self.check_against(tail, expected, because),
            None if diverges => Ty::Never,
            None => {
                self.assign(&Ty::Unit, expected, None, block.span, because);
                Ty::Unit
            }
        };
        self.types.record_expr(block.span, ty.clone());
        ty
    }

    /// An `if`, with each branch checked knowing what its condition settled.
    ///
    /// `wanted` is the type the whole expression has to produce, when there is
    /// one. Without it the branches are only compared against each other,
    /// which is enough for types and not enough for refinements.
    fn check_if(
        &mut self,
        condition: &'a Expr,
        then_branch: &'a Block,
        else_branch: Option<&'a Expr>,
        wanted: Option<(Ty, Option<(Span, String)>)>,
    ) -> Ty {
        let condition_ty = self.infer(condition);
        self.assign(
            &condition_ty,
            &Ty::Bool,
            Some(condition),
            condition.span(),
            None,
        );

        // This is the other half of the Proven tier. A guard above a value is
        // the ordinary way anyone establishes a refinement, and it only counts
        // if the branch is checked while the fact is still in scope.
        let outer = self.facts.clone();
        self.facts = self.narrowed_by(condition, true);

        let then_ty = match &wanted {
            Some((expected, because)) => {
                self.check_block_against(then_branch, expected, because.clone())
            }
            None => self.check_block(then_branch),
        };
        let after_then = self.facts.clone();

        match else_branch {
            Some(else_branch) => {
                self.facts = self.narrowed_from(&outer, condition, false);
                let else_ty = match &wanted {
                    Some((expected, because)) => {
                        self.check_against(else_branch, expected, because.clone())
                    }
                    None => self.infer(else_branch),
                };
                let after_else = self.facts.clone();

                // Past the `if`, only what both branches agree on survives,
                // unless one of them cannot fall through.
                self.facts = match (then_ty.absorbs(), else_ty.absorbs()) {
                    (true, false) => after_else,
                    (false, true) => after_then,
                    _ => after_then.join(&after_else),
                };

                if then_ty.absorbs() {
                    else_ty
                } else {
                    // Already checked against what was wanted, so comparing
                    // the branches again would report one mistake twice.
                    if wanted.is_none() {
                        self.assign(
                            &else_ty,
                            &then_ty,
                            Some(else_branch),
                            else_branch.span(),
                            Some((then_branch.span, "the other branch".to_string())),
                        );
                    }
                    then_ty
                }
            }
            None => {
                // A guard that leaves settles the condition for everything
                // below it: after `if n <= 0 { return .. }` the rest of the
                // body knows `n` is at least one.
                self.facts = if then_ty.absorbs() {
                    self.narrowed_from(&outer, condition, false)
                } else {
                    outer.clone()
                };
                if wanted.is_none() {
                    self.assign(&then_ty, &Ty::Unit, None, then_branch.span, None);
                }
                Ty::Unit
            }
        }
    }

    fn check_block(&mut self, block: &'a Block) -> Ty {
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

    fn check_stmt(&mut self, stmt: &'a Stmt) {
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

                // Naming a value should not lose what was known about it.
                // Without this, `let n = f()` throws away the contract the
                // call site just read, and every proof would have to be
                // written as one long expression.
                if let Pattern::Path { segments, .. } = pattern
                    && let [ident] = segments.as_slice()
                    && let Some(def) = self.def_of(ident)
                {
                    let range = self.range_of(init).meet(self.declared_range(def));
                    self.facts.set(def, range);
                }
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

    fn infer(&mut self, expr: &'a Expr) -> Ty {
        let ty = self.infer_inner(expr);
        self.types.record_expr(expr.span(), ty.clone());
        ty
    }

    fn infer_inner(&mut self, expr: &'a Expr) -> Ty {
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
            } => self.check_if(condition, then_branch, else_branch.as_deref(), None),

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
            DefKind::Import => self.imported_value_ty(def, ident),
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

    /// The type an imported name denotes when used as a value.
    fn imported_value_ty(&mut self, def: DefId, ident: &Ident) -> Ty {
        let Some(module) = self.resolutions.import_module(def) else {
            return Ty::Unknown;
        };
        match self.world.get(module, &ident.name) {
            Some(SurfaceItem::Function { params, ret, .. }) => Ty::Fn {
                params: params.clone(),
                ret: Box::new(ret.clone()),
            },
            // A variant is a value of its choice. One with a payload written
            // bare is still that type; the struct literal path is what checks
            // the payload, and it is reached from somewhere else.
            Some(SurfaceItem::Variant { choice, .. }) => Ty::External {
                module: Rc::from(module),
                name: Rc::clone(choice),
            },
            Some(SurfaceItem::Record { .. } | SurfaceItem::Choice { .. }) => {
                self.not_a_value(ident, "a type");
                Ty::Unknown
            }
            Some(SurfaceItem::Refinement { .. } | SurfaceItem::Alias { .. }) => {
                self.not_a_value(ident, "a type");
                Ty::Unknown
            }
            Some(SurfaceItem::Effect { .. }) => {
                // `Counter.bump` where `Counter` was imported does not resolve
                // to an operation yet, so the checker sees the effect name in
                // what looks like value position. Complaining here would be a
                // message about the wrong thing, and the effects pass already
                // says the row cannot be verified.
                Ty::Unknown
            }
            // A handler names itself in a `with` block, which goes through
            // here, so it is not an error the way the others are.
            Some(SurfaceItem::Handler) | None => Ty::Unknown,
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

        if matches!(name.as_str(), "System" | "Console" | "Clock" | "Dir") {
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
        if receiver == &capability("System") {
            let narrower = match name.name.as_str() {
                "console" => Some("Console"),
                "clock" => Some("Clock"),
                "files" => Some("Dir"),
                _ => None,
            };
            return match narrower {
                Some(narrower) => capability(narrower),
                None => {
                    self.emit(
                        Diagnostic::error(
                            codes::NO_SUCH_FIELD,
                            self.file,
                            name.span,
                            format!("`System` carries no `{}`", name.name),
                        )
                        .with_primary_label("no such capability")
                        .with_note("it carries `console`, `clock` and `files`"),
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
        if let Some(SurfaceItem::Record { fields }) = self.external_item(&looked_through)
            && let Some((_, ty)) = fields.iter().find(|(field, _)| *field == name.name)
        {
            return ty.clone();
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
        if let Some(SurfaceItem::Record { fields }) = self.external_item(&looked_through) {
            let available: Vec<&str> = fields.iter().map(|(name, _)| name.as_str()).collect();
            diagnostic = diagnostic.with_note(format!("it has {}", list(&available)));
        }

        self.emit(diagnostic);
        Ty::Unknown
    }

    fn infer_call(&mut self, callee: &'a Expr, args: &'a [Expr], span: Span) -> Ty {
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

    fn infer_struct_lit(&mut self, path: &'a Expr, fields: &'a [FieldInit], span: Span) -> Ty {
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
                // A record or a variant from another module is built the same
                // way. The field types came across in that module's surface,
                // so the check is the same one, only the blame span for the
                // declaration is missing because it is in a file this pass is
                // not looking at.
                DefKind::Import => {
                    if let Some(ty) = self.imported_literal(def, fields, span) {
                        return ty;
                    }
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

    /// A literal of a record or variant declared in another module.
    fn imported_literal(&mut self, def: DefId, fields: &'a [FieldInit], span: Span) -> Option<Ty> {
        let module: Rc<str> = Rc::from(self.resolutions.import_module(def)?);
        let name = self.resolutions.def(def).name.clone();

        match self.world.get(&module, &name)? {
            SurfaceItem::Record { fields: declared } => {
                let declared = external_fields(declared);
                self.check_literal_fields(&declared, fields, span, &name);
                Some(Ty::External {
                    module,
                    name: Rc::from(name.as_str()),
                })
            }
            SurfaceItem::Variant {
                choice,
                fields: declared,
            } => {
                let choice = Rc::clone(choice);
                let declared = declared.as_deref().map(external_fields).unwrap_or_default();
                self.check_literal_fields(&declared, fields, span, &name);
                Some(Ty::External {
                    module,
                    name: choice,
                })
            }
            // Not a constructor, which the fallthrough below reports the same
            // way it does for a local name.
            _ => None,
        }
    }

    fn check_literal_fields(
        &mut self,
        declared: &[FieldTy],
        fields: &'a [FieldInit],
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
                        // An empty span means the declaration is in another
                        // file, so there is nothing here to point at.
                        (field.span != Span::at(0))
                            .then(|| (field.span, "the field it is assigned to".to_string())),
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

    fn infer_match(&mut self, scrutinee: &'a Expr, arms: &'a [MatchArm], span: Span) -> Ty {
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

        let widened = self.widen(scrutinee);
        if let Some(SurfaceItem::Choice { variants }) = self.external_item(&widened) {
            let names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
            let choice = self.types.describe(&widened);
            self.check_named_exhaustive(&names, &choice, arms, span);
            return;
        }

        let Ty::Named(def) = widened else {
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

    /// Exhaustiveness for a choice declared in another module.
    ///
    /// The same two rules, matched by name rather than by `DefId`, because a
    /// variant from elsewhere has a local import def that says nothing about
    /// which variant of which choice it is.
    fn check_named_exhaustive(
        &mut self,
        variants: &[String],
        choice: &str,
        arms: &[MatchArm],
        span: Span,
    ) {
        let mut covered: HashSet<String> = HashSet::new();
        let mut catch_all: Option<Span> = None;

        for arm in arms {
            match &arm.pattern {
                Pattern::Wildcard(span) => {
                    catch_all.get_or_insert(*span);
                }
                Pattern::Path { segments, .. } => match segments.last() {
                    Some(last) if variants.contains(&last.name) => {
                        covered.insert(last.name.clone());
                    }
                    // A bare binding matches every variant.
                    _ => {
                        catch_all.get_or_insert(arm.pattern.span());
                    }
                },
                Pattern::Tuple { path, .. } | Pattern::Record { path, .. } => {
                    if let Some(last) = path.last() {
                        covered.insert(last.name.clone());
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
                    format!("this arm matches every variant of {choice}"),
                )
                .with_primary_label("catches everything")
                .with_secondary(span, "in this match")
                .with_note(
                    "list the variants instead: adding one to a choice should break every \
                     match that has to care, and that is as true across a module boundary \
                     as inside one",
                ),
            );
            return;
        }

        let missing: Vec<&str> = variants
            .iter()
            .filter(|name| !covered.contains(*name))
            .map(String::as_str)
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
                    "every variant of {choice} needs an arm, and there is no wildcard to fall back on"
                )),
            );
        }
    }

    /// A `Result` has two cases and the same rules apply: both have to be
    /// handled, and neither can be swallowed by a catch-all.
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
/// A builtin capability type.
///
/// Named under the prelude rather than under whichever module mentioned it,
/// because there is exactly one `Console` and every module has to agree about
/// that. Naming it after the module would make the same capability compare
/// unequal to itself across a file boundary, which a test caught.
fn capability(name: &str) -> Ty {
    Ty::External {
        module: Rc::from(PRELUDE_MODULE),
        name: Rc::from(name),
    }
}

/// The range an `ensures` block pins the returned value to.
///
/// Only the `ok` outcome, and only what it says about `result` itself. A clause
/// that relates `result` to an argument says something true and useful that an
/// interval cannot hold, so it contributes nothing rather than something wrong.
fn promised_by(ensures: &[Ensures]) -> Range {
    ensures
        .iter()
        .filter(|clause| clause.outcome == Outcome::Ok)
        .fold(Range::ANY, |range, clause| {
            range.meet(facts::range_of_subject(&clause.condition, "result"))
        })
}

/// Fields from another module's surface, as the checker's own field type.
///
/// The span is empty because the declaration is in a file this pass is not
/// looking at. Diagnostics that would point at it fall back to saying nothing
/// rather than pointing at the wrong place.
fn external_fields(fields: &[(String, Ty)]) -> Vec<FieldTy> {
    fields
        .iter()
        .map(|(name, ty)| FieldTy {
            name: name.clone(),
            ty: ty.clone(),
            span: Span::at(0),
        })
        .collect()
}

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
