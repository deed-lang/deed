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
    Accumulator, BinaryOp, Block, Ensures, Expr, FieldInit, FnDecl, HandlerDecl, Ident, Item,
    MatchArm, Module, Outcome, Pattern, Stmt, Type, TypeAlias, UnaryOp,
};
use vow_diagnostics::{Diagnostic, FileId, Span};
use vow_resolve::{DefId, DefKind, Resolutions, RowLowering};

use crate::codes;
use crate::facts::{self, Facts, Guarantee, Promise, Range, Truth};
use crate::surface::{PRELUDE_MODULE, SurfaceItem, World};
use crate::ty::{FieldTy, FnRow, Nominal, Obligation, Tier, Ty, Types, VariantTy, bindings_for};

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
        rows: RowLowering::of(module),
        types: Types::default(),
        diagnostics: Vec::new(),
        def_types: HashMap::new(),
        type_params: HashMap::new(),
        nominal_generics: HashMap::new(),
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

/// What is known about a value nothing in the source names.
///
/// There is one such value: the number inside the `ok` of a `Result` that came
/// back from a call. No expression stands for it, so what it is worth and where
/// it lives have to travel next to the `Result` that contains it.
#[derive(Clone, Copy, Default)]
struct Carried {
    /// The range a promise said it lands in.
    range: Option<Range>,
    /// Whether it is one level inside the expression being checked.
    inside_ok: bool,
}

#[derive(Clone)]
struct Signature {
    params: Vec<ParamTy>,
    ret: Ty,
    span: Span,
    /// The type parameters this was declared with, in order, and where each
    /// was written. Empty for almost everything.
    generics: Vec<(String, Span)>,
    /// What the contract says a call performs, as a function type would write
    /// it. Named directly rather than called, a function is a value of that
    /// type, so this is what it has to fit.
    row: FnRow,
    /// What a call to this is known to hand back, from the declared return
    /// type and from the `ensures` clause. Promises nothing when nothing is.
    guarantee: Guarantee,
}

struct Checker<'a> {
    file: FileId,
    resolutions: &'a Resolutions,
    /// The lowered declarations of every other module being compiled.
    world: &'a World,
    /// How a row written in this module reads from anywhere else. The same
    /// lowering the exports table uses, so a row in a type and a row in a
    /// contract cannot mean two different things.
    rows: RowLowering,
    types: Types,
    diagnostics: Vec<Diagnostic>,
    /// Types of parameters, locals and handler state.
    def_types: HashMap<DefId, Ty>,
    /// Where each type parameter sits in the list its function declared.
    ///
    /// Keyed by definition, so two functions that both call theirs `T` never
    /// collide and the whole map can be one table rather than a stack of
    /// scopes.
    type_params: HashMap<DefId, (usize, Rc<str>)>,
    /// What each record and choice declared its type parameters as.
    ///
    /// Kept as names rather than a count, so a message about the wrong number
    /// of arguments can say which ones were wanted.
    nominal_generics: HashMap<DefId, Vec<String>>,
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
        // bug in the caller. So can `save`, for the same reason and for every
        // reason a disk has.
        let io_error = |ok: Ty| Ty::Result(Box::new(ok), Box::new(Ty::Str));

        let operations: [(&str, Vec<Ty>, Ty); 7] = [
            ("write", vec![console, Ty::Str], Ty::Unit),
            ("now", vec![clock], Ty::Int),
            ("open", vec![dir.clone(), Ty::Str], io_error(dir.clone())),
            ("read", vec![dir.clone(), Ty::Str], io_error(Ty::Str)),
            ("save", vec![dir, Ty::Str, Ty::Str], io_error(Ty::Unit)),
            // Enumerating rather than naming, which is why it takes the
            // directory and nothing else: there is no name to give, and
            // finding out what the names are is the whole operation.
            (
                "list",
                vec![capability("Dir")],
                io_error(Ty::List(Box::new(Ty::Str))),
            ),
            // The arguments a program was invoked with, which cannot fail to
            // exist: a program with none was given an empty list.
            (
                "args",
                vec![capability("System")],
                Ty::List(Box::new(Ty::Str)),
            ),
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
                    generics: Vec::new(),
                    // An operation performs its own effect and nothing else,
                    // so naming one where a function type was wanted is only
                    // allowed if that type made room for it.
                    row: FnRow::Declared(vec![vow_resolve::RowEntry {
                        module: PRELUDE_MODULE.to_string(),
                        effect: "Io".to_string(),
                        operation: Some(name.to_string()),
                        variable: false,
                    }]),
                    guarantee: Guarantee::any(),
                },
            );
        }

        // The prelude's own functions, which are not constructors and not
        // operations of an effect.
        //
        // `length` is the only one that promises anything beyond its type: a
        // length is never negative, and saying so here is what lets
        // `where length(name) > 0` land in the Proven tier instead of becoming
        // a runtime check.
        //
        // `split` and `join` are inverses, and so are `to_string` and
        // `to_int`. Each pair is here because a program that cannot take its
        // input apart or put its output together cannot read or print
        // anything, which is most of what a program does.
        //
        // `trim` is the one text operation that cannot be written in the
        // language, which is the test for whether something belongs in the
        // prelude at all.
        let lines = Ty::List(Box::new(Ty::Str));
        let functions: [(&str, Vec<Ty>, Ty, Guarantee); 6] = [
            (
                "length",
                vec![Ty::Str],
                Ty::Int,
                Guarantee::of(Range::between(0, i64::MAX)),
            ),
            (
                "split",
                vec![Ty::Str, Ty::Str],
                lines.clone(),
                Guarantee::any(),
            ),
            ("join", vec![lines, Ty::Str], Ty::Str, Guarantee::any()),
            ("trim", vec![Ty::Str], Ty::Str, Guarantee::any()),
            ("to_string", vec![Ty::Int], Ty::Str, Guarantee::any()),
            (
                "to_int",
                vec![Ty::Str],
                Ty::Result(Box::new(Ty::Int), Box::new(Ty::Str)),
                Guarantee::any(),
            ),
        ];

        for (name, params, ret, guarantee) in functions {
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
                    generics: Vec::new(),
                    row: FnRow::Declared(Vec::new()),
                    guarantee,
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
                    self.declare_type_params(def, &record.generics);
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
                    self.declare_type_params(def, &choice.generics);
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
                Item::Handler(handler) => {
                    let Some(def) = self.def_of(&handler.name) else {
                        continue;
                    };
                    let state = self.lower_fields(&handler.state);
                    self.types.set_nominal(
                        def,
                        handler.name.name.clone(),
                        Nominal::Handler { state },
                    );
                }
                Item::Function(function) => {
                    let Some(def) = self.def_of(&function.sig.name) else {
                        continue;
                    };
                    self.rows.declaring(&function.sig.rows);
                    let mut signature = self.lower_signature(&function.sig);
                    // A function named rather than called is a value, and
                    // what that value performs is what its contract says.
                    signature.row = FnRow::Declared(self.rows.normalised(&function.contract.uses));
                    signature.guarantee = signature
                        .guarantee
                        .meet(promised_by(&function.contract.ensures, &function.sig));
                    self.check_type_params_are_determined(&signature);
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

    /// Notes what a declaration's type parameters are called and where each
    /// sits, before anything that could name one is lowered.
    fn declare_type_params_of(&mut self, generics: &[Ident]) {
        for (index, parameter) in generics.iter().enumerate() {
            if let Some(def) = self.def_of(parameter) {
                self.type_params
                    .insert(def, (index, Rc::from(parameter.name.as_str())));
                self.types.set_name(def, parameter.name.clone());
            }
        }
    }

    /// The same, for a record or a choice, which also has to remember how many
    /// arguments a use of it owes.
    fn declare_type_params(&mut self, def: DefId, generics: &[Ident]) {
        self.declare_type_params_of(generics);
        self.nominal_generics.insert(
            def,
            generics
                .iter()
                .map(|parameter| parameter.name.clone())
                .collect(),
        );
    }

    fn lower_signature(&mut self, sig: &vow_ast::FnSig) -> Signature {
        self.declare_type_params_of(&sig.generics);

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
            generics: sig
                .generics
                .iter()
                .map(|parameter| (parameter.name.clone(), parameter.span))
                .collect(),
            // Overwritten by the caller for a function with a contract. A
            // signature on its own says nothing about effects, and an effect
            // operation performs only its own.
            row: FnRow::Declared(Vec::new()),
            guarantee,
        }
    }

    /// Checks that every type parameter can be worked out from a call.
    ///
    /// The only place a call site can look is the parameter types, so one that
    /// appears nowhere in them is one nothing can ever determine. Reported at
    /// the declaration rather than at every call, because the declaration is
    /// where the mistake is and a caller cannot fix it from where they are.
    fn check_type_params_are_determined(&mut self, signature: &Signature) {
        for (index, (name, span)) in signature.generics.iter().enumerate() {
            if signature
                .params
                .iter()
                .any(|param| param.ty.mentions(index))
            {
                continue;
            }

            let note = if signature.ret.mentions(index) {
                "it appears in the return type, and a return type is what a call produces rather than something it can be worked out from"
            } else {
                "a type parameter is worked out by matching the parameter types against the arguments, so one that appears in none of them has nothing to match"
            };

            self.emit(
                Diagnostic::error(
                    codes::UNDETERMINED_TYPE_PARAM,
                    self.file,
                    *span,
                    format!("nothing at a call site says what `{name}` is"),
                )
                .with_primary_label("appears in no parameter's type")
                .with_note(note),
            );
        }
    }

    /// A signature with its type parameters worked out from the arguments.
    ///
    /// Hands back the signature unchanged when nothing in it is generic, which
    /// is almost every call.
    fn instantiate(&self, signature: &Signature, actual: &[Ty]) -> Signature {
        if signature.generics.is_empty() {
            return signature.clone();
        }

        let mut bindings = HashMap::new();
        for (param, actual) in signature.params.iter().zip(actual) {
            param.ty.bind(actual, &mut bindings);
        }

        Signature {
            params: signature
                .params
                .iter()
                .map(|param| ParamTy {
                    ty: param.ty.substitute(&bindings),
                    span: param.span,
                })
                .collect(),
            ret: signature.ret.substitute(&bindings),
            span: signature.span,
            generics: Vec::new(),
            row: signature.row.clone(),
            guarantee: signature.guarantee.clone(),
        }
    }

    /// What another module's function looks like as a signature.
    ///
    /// `None` when the name is not a function there, which is every other kind
    /// of import and is somebody else's question.
    fn imported_function_signature(&self, def: DefId) -> Option<Signature> {
        let module = self.resolutions.import_module(def)?;
        let name = &self.resolutions.def(def).name;
        let Some(SurfaceItem::Function {
            params,
            ret,
            generics,
            row,
            guarantee,
        }) = self.world.get(module, name)
        else {
            return None;
        };

        Some(Signature {
            // Nowhere to point at. The declaration is in a file this
            // diagnostic cannot draw, so a mismatch lands on the argument and
            // says what was wanted rather than pointing across the boundary.
            params: params
                .iter()
                .map(|ty| ParamTy {
                    ty: ty.clone(),
                    span: Span::at(0),
                })
                .collect(),
            ret: ret.clone(),
            span: Span::at(0),
            generics: generics
                .iter()
                .map(|name| (name.clone(), Span::at(0)))
                .collect(),
            row: row.clone(),
            guarantee: guarantee.clone(),
        })
    }

    /// Checks a call against a signature, working out its type parameters
    /// first.
    ///
    /// Every argument is inferred before anything is checked. A generic
    /// signature is not a signature until the arguments have said what its
    /// type parameters are, and it cannot say what the first parameter wanted
    /// without having looked at all of them.
    ///
    /// `name` is what to call it in an arity message, when there is a name
    /// worth using.
    fn check_call_against(
        &mut self,
        signature: &Signature,
        callee: &'a Expr,
        args: &'a [Expr],
        span: Span,
        name: Option<String>,
    ) -> Ty {
        if args.len() != signature.params.len() {
            let what = match &name {
                Some(name) => format!("`{name}` takes"),
                None => "this takes".to_string(),
            };
            let mut diagnostic = Diagnostic::error(
                codes::WRONG_ARITY,
                self.file,
                span,
                format!(
                    "{what} {} argument{}, but {} {} given",
                    signature.params.len(),
                    if signature.params.len() == 1 { "" } else { "s" },
                    args.len(),
                    if args.len() == 1 { "was" } else { "were" }
                ),
            )
            .with_primary_label("wrong number of arguments");
            if !signature.span.is_empty() {
                diagnostic = diagnostic.with_secondary(signature.span, "declared here");
            }
            self.emit(diagnostic);
        }

        let actual: Vec<Ty> = args.iter().map(|arg| self.infer(arg)).collect();
        let signature = self.instantiate(signature, &actual);

        // What the name in callee position turned out to mean, which for a
        // generic function is the type it was called at rather than the type
        // it was declared with. Recorded rather than inferred, because
        // inferring it would be a second complaint about a generic function
        // named where a value belongs, and this is a call.
        self.types.record_expr(
            callee.span(),
            Ty::Fn {
                params: signature.params.iter().map(|p| p.ty.clone()).collect(),
                row: signature.row.clone(),
                ret: Box::new(signature.ret.clone()),
            },
        );

        for (index, arg) in args.iter().enumerate() {
            let Some(param) = signature.params.get(index) else {
                continue;
            };
            let param_ty = param.ty.clone();
            let because = (!param.span.is_empty())
                .then(|| (param.span, "the parameter it is passed to".to_string()));
            self.assign(&actual[index], &param_ty, Some(arg), arg.span(), because);
        }
        signature.ret
    }

    /// What a return type alone promises about the value coming back.
    ///
    /// A `Result` promises nothing usable here: the call site holds the
    /// wrapper, not the payload, and giving a range to a `Result` would be
    /// answering a question nobody asked.
    fn guarantee_of(&mut self, ret: &Ty) -> Guarantee {
        match ret {
            Ty::Named { def, .. } => Guarantee::of(self.refinement_range(*def)),
            _ => Guarantee::any(),
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
                        "List" => {
                            if lowered_args.len() == 1 {
                                return Ty::List(Box::new(lowered_args[0].clone()));
                            }
                            self.emit(
                                Diagnostic::error(
                                    codes::NOT_GENERIC,
                                    self.file,
                                    *span,
                                    format!(
                                        "`List` takes exactly one type argument, and {} were given",
                                        lowered_args.len()
                                    ),
                                )
                                .with_primary_label("wrong number of type arguments")
                                .with_note("it is written `List<Element>`"),
                            );
                            return Ty::Unknown;
                        }
                        _ => Ty::Unknown,
                    },
                    // A name from another module. It has a type now, and the
                    // identity is the module path and the name rather than a
                    // `DefId`, which cannot mean anything outside the table it
                    // came from.
                    DefKind::Import => return self.imported_ty(def, name, &lowered_args, *span),
                    DefKind::Record | DefKind::Choice => {
                        let arity = self.nominal_generics.get(&def).map_or(0, Vec::len);
                        if !self.check_type_arity(&name.name, arity, lowered_args.len(), *span) {
                            return Ty::Unknown;
                        }
                        return Ty::Named {
                            def,
                            args: lowered_args,
                        };
                    }
                    DefKind::Type => self.alias_ty(def),
                    // A type parameter of the function being checked. It only
                    // means anything inside that declaration, and every call
                    // site substitutes it away.
                    DefKind::TypeParam => match self.type_params.get(&def) {
                        Some((index, name)) => Ty::Param {
                            index: *index,
                            name: Rc::clone(name),
                        },
                        None => Ty::Unknown,
                    },
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
                        .with_note("a function, a record and a choice may be generic; an alias and an effect may not"),
                    );
                }

                base
            }

            Type::Fn {
                params, row, ret, ..
            } => Ty::Fn {
                params: params.iter().map(|param| self.lower_type(param)).collect(),
                row: FnRow::Declared(self.rows.normalised(row)),
                ret: Box::new(self.lower_type(ret)),
            },
        }
    }

    /// Whether a type was applied to as many arguments as it was declared
    /// with.
    ///
    /// Exactly as many, in both directions. A signature is complete, so a
    /// `Pair` written with no arguments is as much a hole in one as a
    /// parameter with no type is, and filling it in with unknowns would make
    /// every use of it agree with everything.
    fn check_type_arity(&mut self, name: &str, wanted: usize, given: usize, span: Span) -> bool {
        if wanted == given {
            return true;
        }

        let written = if wanted == 0 {
            format!("`{name}` takes no type arguments")
        } else {
            format!(
                "`{name}` takes {wanted} type argument{}",
                if wanted == 1 { "" } else { "s" }
            )
        };
        self.emit(
            Diagnostic::error(
                codes::NOT_GENERIC,
                self.file,
                span,
                format!(
                    "{written}, and {given} {} given",
                    if given == 1 { "was" } else { "were" }
                ),
            )
            .with_primary_label("wrong number of type arguments")
            .with_note(
                "a type argument is written out rather than left to be worked out, because a type is a signature's business and a signature is complete",
            ),
        );
        false
    }

    /// The type an imported name denotes when used in type position.
    ///
    /// A transparent alias is expanded, because it was not a distinct type
    /// where it was declared and crossing a module boundary does not make it
    /// one. A refinement stays nominal for the same reason it does at home.
    fn imported_ty(&mut self, def: DefId, name: &Ident, args: &[Ty], span: Span) -> Ty {
        let Some(module) = self.resolutions.import_module(def) else {
            return Ty::Unknown;
        };

        let arity = match self.world.get(module, &name.name) {
            Some(SurfaceItem::Record { generics, .. } | SurfaceItem::Choice { generics, .. }) => {
                generics.len()
            }
            _ => 0,
        };
        let external = Ty::External {
            module: Rc::from(module),
            name: Rc::from(name.name.as_str()),
            args: args.to_vec(),
        };

        match self.world.get(module, &name.name) {
            // An alias is expanded, so it is whatever it was declared as and
            // takes nothing of its own. A parameter on an alias is a different
            // question about what a refinement's predicate may say, and it is
            // not answered yet.
            Some(SurfaceItem::Alias { target }) => {
                let target = target.clone();
                if !self.check_type_arity(&name.name, 0, args.len(), span) {
                    return Ty::Unknown;
                }
                target
            }
            Some(
                SurfaceItem::Record { .. }
                | SurfaceItem::Choice { .. }
                | SurfaceItem::Refinement { .. },
            ) => {
                if !self.check_type_arity(&name.name, arity, args.len(), span) {
                    return Ty::Unknown;
                }
                external
            }
            // A function or a handler is not a type, and the resolver already
            // said the name exists, so this is the only place to say what is
            // wrong with using it here.
            Some(other) => {
                let what = match other {
                    SurfaceItem::Function { .. } => "a function",
                    SurfaceItem::Effect { .. } => "an effect",
                    SurfaceItem::Handler { .. } => "a handler",
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
        let Ty::External { module, name, .. } = ty else {
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
            let ty = Ty::Named {
                def,
                args: Vec::new(),
            };
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
        if let Ty::Named { def, .. } = ty
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
            // Componentwise for the same reason, which is what makes `[]`
            // work: it produces `List<unknown>`, and unknown agrees with
            // whatever the expected element type turns out to be.
            (Ty::List(actual), Ty::List(expected)) => self.compatible(actual, expected),
            // A declared generic type, compared the same way. The head has to
            // be the same declaration, which is what keeps a `Pair` from being
            // a `Box`, and the arguments are compared one by one, which is
            // what lets a bare `None` be an `Option<Int>`: nothing said what
            // its argument was, so it is unknown, and unknown absorbs.
            (
                Ty::Named { def: a_def, args },
                Ty::Named {
                    def: e_def,
                    args: expected,
                },
            ) => {
                a_def == e_def
                    && args.len() == expected.len()
                    && args
                        .iter()
                        .zip(expected)
                        .all(|(a, e)| self.compatible(a, e))
            }
            (
                Ty::External {
                    module: a_module,
                    name: a_name,
                    args,
                },
                Ty::External {
                    module: e_module,
                    name: e_name,
                    args: expected,
                },
            ) => {
                a_module == e_module
                    && a_name == e_name
                    && args.len() == expected.len()
                    && args
                        .iter()
                        .zip(expected)
                        .all(|(a, e)| self.compatible(a, e))
            }
            // Componentwise, for the same reason `Result` is: a closure with a
            // parameter the checker gave up on should not be a second error on
            // top of the first. Exactly matching otherwise, so a refined
            // parameter is a different function type from an unrefined one and
            // says so, rather than being quietly accepted in one direction.
            //
            // The row is the exception, and the only place in this checker
            // where one type fits another without being it. See [`FnRow`].
            (
                Ty::Fn {
                    params: a_params,
                    row: a_row,
                    ret: a_ret,
                },
                Ty::Fn {
                    params: e_params,
                    row: e_row,
                    ret: e_ret,
                },
            ) => {
                a_params.len() == e_params.len()
                    && a_params
                        .iter()
                        .zip(e_params)
                        .all(|(a, e)| self.compatible(a, e))
                    && a_row.within(e_row)
                    && self.compatible(a_ret, e_ret)
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
        self.assign_carrying(actual, expected, expr, Carried::default(), span, because);
    }

    /// The same, for a value that nothing in the source names.
    ///
    /// The case this exists for is the number inside the `ok` of a call that
    /// can fail: the expression at hand is the `Result`, the promise was about
    /// the payload, and without this there is nowhere to put the promise on the
    /// way past and no way to tell the runtime what to check.
    fn assign_carrying(
        &mut self,
        actual: &Ty,
        expected: &Ty,
        expr: Option<&Expr>,
        carried: Carried,
        span: Span,
        because: Option<(Span, String)>,
    ) {
        // A function type says what a value of it performs, and whether a
        // value keeps that promise is a question for the pass that knows about
        // rows. Which values owe which row is this pass's question, so it
        // answers it here, before anything short circuits.
        //
        // A row variable is left alone. It has made room for anything, and
        // what it turned out to be is settled at the call site rather than
        // here.
        if let Ty::Fn {
            row: FnRow::Declared(allowed),
            ..
        } = expected
            && !allowed.iter().any(|entry| entry.variable)
        {
            self.types
                .require_row(expr.map(Expr::span).unwrap_or(span), allowed.clone());
        }

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

            // A `Result` that came back from a call rather than from an `ok`
            // written here. Nothing names the payload, so its range is what
            // goes down instead of an expression, and the obligation has to
            // say that it is one level inside what it points at.
            let payload = match (ok_expr, expr) {
                (None, Some(expr)) => Carried {
                    range: Some(self.ok_range_of(expr)),
                    inside_ok: true,
                },
                _ => Carried::default(),
            };

            self.assign_carrying(a_ok, e_ok, ok_expr, payload, ok_span, because.clone());
            self.assign(a_err, e_err, err_expr, err_span, because);
            return;
        }

        // `[1, 2]` where the element type is refined. Same reasoning as the
        // `ok` above: the list has no range and nothing to check, the elements
        // do, so the obligation belongs on each of them. Only for a literal
        // written here, because that is the only case where anything names the
        // elements; a list that came back from a call falls through to the
        // mismatch below rather than being quietly accepted.
        if let (Ty::List(a_el), Ty::List(e_el)) = (actual, expected)
            && let Some(Expr::List { elements, .. }) = expr
            && !elements.is_empty()
        {
            let (a_el, e_el) = ((**a_el).clone(), (**e_el).clone());
            for element in elements {
                self.assign(&a_el, &e_el, Some(element), element.span(), because.clone());
            }
            return;
        }

        // Narrowing into a refinement is the interesting direction. The value
        // may already be refined by something else: going sideways is widening
        // out of one and back into the other, and the predicate that arrives is
        // often enough to discharge the predicate that is wanted.
        if let Ty::Named { def, .. } = expected
            && let Some(Nominal::Refinement { base, .. }) = self.types.nominal(*def)
        {
            let base = base.clone();
            if self.widen(actual) == base || actual.absorbs() {
                let subject = match expr {
                    Some(expr) => Some(self.range_of(expr)),
                    None => carried.range,
                };
                self.discharge(*def, subject, expr, span, carried.inside_ok);
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
        let Some(Ty::Named {
            def: refinement, ..
        }) = self.def_types.get(&def)
        else {
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

    /// What a call to `callee` is promised to hand back.
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
    ///
    /// A callee that can fail promises things about the number inside the
    /// `ok`, and the caller is holding the `Result`, so which of the two the
    /// promise is about is decided here and read back where the payload is
    /// taken out.
    fn call_promise(&self, callee: &Expr) -> Promise {
        let def = match callee {
            Expr::Ident(ident) => self.def_of(ident),
            Expr::Field { name, .. } => self.resolutions.resolution(name.span),
            _ => None,
        };
        let Some(def) = def else {
            return Promise::any();
        };

        let promise = |ret: &Ty, guarantee: Guarantee| match ret {
            Ty::Result(_, _) => Promise::ok(guarantee),
            _ => Promise::value(guarantee),
        };

        if let Some(signature) = self.signatures.get(&def) {
            return promise(&signature.ret, signature.guarantee.clone());
        }

        // An imported function. Its refinement is opaque out here, which is
        // why bounds travel rather than the predicate: a range and a set of
        // differences say what the caller needs without exporting how the
        // callee decided any of it.
        let Some(module) = self.resolutions.import_module(def) else {
            return Promise::any();
        };
        match self.world.get(module, &self.resolutions.def(def).name) {
            Some(SurfaceItem::Function { ret, guarantee, .. }) => promise(ret, guarantee.clone()),
            _ => Promise::any(),
        }
    }

    /// What the fact machinery is allowed to look up while checking this body.
    fn env(
        &self,
    ) -> (
        impl Fn(&Expr) -> Option<DefId> + use<'a>,
        impl Fn(&Expr) -> Promise + '_,
    ) {
        (self.resolver(), |callee: &Expr| self.call_promise(callee))
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

    /// The range the value inside the `ok` of `expr` lands in.
    fn ok_range_of(&self, expr: &Expr) -> Range {
        let (def_of, call) = self.env();
        let env = facts::Env {
            def_of: &def_of,
            call: &call,
        };
        facts::ok_range_of(expr, &self.facts, &env)
    }

    /// Where the arithmetic in `expr` can have no answer, if anywhere.
    fn overflowing(&self, expr: &Expr) -> Option<Span> {
        let (def_of, call) = self.env();
        let env = facts::Env {
            def_of: &def_of,
            call: &call,
        };
        facts::overflowing(expr, &self.facts, &env)
    }

    /// Whether the facts in scope settle a refinement predicate for a value.
    ///
    /// The value is a range rather than an expression, because the interesting
    /// case is the one where nothing in the source names it: the number inside
    /// the `ok` of a call that can fail.
    fn proves(&self, predicate: &Expr, subject: Option<Range>) -> Truth {
        let Some(subject) = subject else {
            return Truth::Unknown;
        };
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
    /// The Proven tier is interval reasoning with a range for the difference
    /// between two names next to it: what is known about the integers in scope,
    /// from the contract, from the types, and from the conditions above this
    /// point. It cannot relate two names through anything but adding and
    /// subtracting, and it cannot see through a call, which is written down in
    /// `facts.rs` and in `design/02-syntax.md` rather than left to be
    /// discovered.
    ///
    /// `inside_ok` says the value is the number inside the `ok` of whatever is
    /// at `span`, which happens when a `Result` came back from a call and
    /// nothing here names its payload. The runtime has to know, or it runs the
    /// predicate against the `Result`.
    ///
    /// `expr` is only for the diagnostic. A proof that failed because the
    /// arithmetic in it has no answer looks like weak reasoning, and saying
    /// which operation is what tells the two apart.
    fn discharge(
        &mut self,
        refinement: DefId,
        subject: Option<Range>,
        expr: Option<&Expr>,
        span: Span,
        inside_ok: bool,
    ) {
        let predicate = self
            .aliases
            .get(&refinement)
            .and_then(|alias| alias.refinement.as_ref());

        let outcome = match predicate {
            Some(predicate) => self.proves(predicate, subject),
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
                inside_ok,
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

                // A proof that failed because the arithmetic has no answer
                // looks exactly like weak reasoning, and it is not the same
                // thing at all. `n + 1` where `n` is positive is not provably
                // positive because there is no sum to prove anything about
                // when `n` is the largest integer there is.
                if let Some(at) = expr.and_then(|expr| self.overflowing(expr)) {
                    diagnostic = diagnostic
                        .with_secondary(at, "this can have no answer")
                        .with_note(
                            "an operation with no answer produces no value, so nothing about the result follows; bounding what goes into it is what settles this",
                        );
                }

                self.emit(diagnostic);
                self.types.push_obligation(Obligation {
                    span,
                    refinement,
                    tier: Tier::Guarded,
                    inside_ok,
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
                        let declared = self.operation_signature(handler, operation);
                        self.check_fn_against(operation, declared);
                    }
                }
                Item::Test(test) => {
                    self.check_block(&test.body);
                }
                _ => {}
            }
        }
    }

    /// What the effect says a handler operation's signature is.
    ///
    /// A handler operation writes no types because the effect already declared
    /// them, so this is where they come from. Without it every parameter in
    /// every handler body was the unknown type, and unknown agrees with
    /// everything, so the piece of code holding the state and talking to the
    /// outside world was the least checked in the language.
    ///
    /// `None` when the effect cannot be found at all, in which case the
    /// resolver has already said so and inventing a complaint here would be
    /// piling on.
    fn operation_signature(
        &mut self,
        handler: &'a HandlerDecl,
        operation: &'a FnDecl,
    ) -> Option<(Vec<Ty>, Ty)> {
        let name = operation.sig.name.name.as_str();
        let effect = self.def_of(&handler.effect)?;

        let found = match self.resolutions.def(effect).kind {
            DefKind::Import => self.imported_operation(effect, name),
            _ => self.local_operation(effect, name),
        };

        let Some((params, ret)) = found else {
            let effect_name = self.types.name_of(effect).to_string();
            self.emit(
                Diagnostic::error(
                    codes::OPERATION_MISMATCH,
                    self.file,
                    operation.sig.name.span,
                    format!("`{effect_name}` does not declare an operation called `{name}`"),
                )
                .with_primary_label("not part of the effect")
                .with_secondary(handler.effect.span, "the effect this handler implements"),
            );
            return None;
        };

        if params.len() != operation.sig.params.len() {
            let effect_name = self.types.name_of(effect).to_string();
            self.emit(
                Diagnostic::error(
                    codes::OPERATION_MISMATCH,
                    self.file,
                    operation.sig.name.span,
                    format!(
                        "`{effect_name}.{name}` takes {} argument{}, and this takes {}",
                        params.len(),
                        if params.len() == 1 { "" } else { "s" },
                        operation.sig.params.len()
                    ),
                )
                .with_primary_label("does not match the effect")
                .with_secondary(handler.effect.span, "the effect this handler implements")
                .with_note("a handler operation writes no types because the effect declares them, so the shape has to line up"),
            );
            return None;
        }

        Some((params, ret))
    }

    /// An operation of an effect declared in this module.
    fn local_operation(&self, effect: DefId, name: &str) -> Option<(Vec<Ty>, Ty)> {
        let operation = self.resolutions.defs().find_map(|(def, data)| {
            (data.kind == DefKind::EffectOp && data.parent == Some(effect) && data.name == name)
                .then_some(def)
        })?;
        let signature = self.signatures.get(&operation)?;
        Some((
            signature.params.iter().map(|p| p.ty.clone()).collect(),
            signature.ret.clone(),
        ))
    }

    /// An operation of an effect from another module.
    fn imported_operation(&self, effect: DefId, name: &str) -> Option<(Vec<Ty>, Ty)> {
        let module = self.resolutions.import_module(effect)?;
        let effect_name = &self.resolutions.def(effect).name;
        let Some(SurfaceItem::Effect { operations }) = self.world.get(module, effect_name) else {
            return None;
        };
        operations.get(name).cloned()
    }

    fn check_fn(&mut self, function: &'a FnDecl) {
        self.check_fn_against(function, None);
    }

    fn check_fn_against(&mut self, function: &'a FnDecl, declared: Option<(Vec<Ty>, Ty)>) {
        // A row variable means nothing outside the signature that declared it,
        // so what the rows in this body may name is set before anything in it
        // is lowered.
        self.rows.declaring(&function.sig.rows);

        // Reuse the signature computed during collection where there is one.
        // Lowering the same annotation twice would report anything wrong with
        // it twice, which is a cascade from a single mistake.
        let signature = self
            .def_of(&function.sig.name)
            .and_then(|def| self.signatures.get(&def).cloned());

        let (param_types, ret) = match (&signature, declared) {
            // A handler operation. The effect said what the types are, and
            // anything written here as well would be a second source of truth.
            (_, Some((params, ret))) => (params, ret),
            (Some(signature), None) => (
                signature.params.iter().map(|p| p.ty.clone()).collect(),
                signature.ret.clone(),
            ),
            (None, None) => {
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
            Expr::Match {
                scrutinee,
                arms,
                span,
            } => self.check_match(scrutinee, arms, *span, Some((expected.clone(), because))),
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

                // What was known about the old value is not known about the new
                // one. Handler state is the only thing in the language that can
                // be assigned twice, so this is the only place a fact can
                // outlive what it was about, and it did: after
                // `if count > 0 { count = 0 }` the checker still believed
                // `count` was positive and proved refinements with it.
                let range = self.range_of(value).meet(self.declared_range(def));
                self.facts.set(def, range);
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
        }
    }

    /// `for n in numbers with sum = 0 { ... }`
    ///
    /// A fold, checked as one. The element type comes out of the list, the
    /// accumulator's type comes from what it starts as, and the body has to
    /// produce that same type because its value is what the next turn starts
    /// with. Leaving `with` off means an accumulator of `()`, so the body has
    /// to produce `()` and the loop is there for its effects.
    ///
    /// Nothing here is assigned. `sum` is a fresh binding on every turn, which
    /// is what lets the language have iteration without having a second
    /// mutable thing in it.
    fn check_for(
        &mut self,
        binder: &'a Ident,
        iterable: &'a Expr,
        accumulator: Option<&'a Accumulator>,
        body: &'a Block,
        span: Span,
    ) -> Ty {
        let iterable_ty = self.infer(iterable);
        let element = match self.widen(&iterable_ty) {
            Ty::List(element) => *element,
            other if other.absorbs() => Ty::Unknown,
            other => {
                let described = self.types.describe(&other);
                self.emit(
                    Diagnostic::error(
                        codes::NOT_A_LIST,
                        self.file,
                        iterable.span(),
                        format!("`for` walks a list, and this is {described}"),
                    )
                    .with_primary_label("not a list")
                    .with_note(
                        "there is one thing to walk in this language, and a `for` over \
                         anything else would need a way to say what walking means",
                    ),
                );
                Ty::Unknown
            }
        };

        if let Some(def) = self.def_of(binder) {
            self.def_types.insert(def, element);
        }

        // What the accumulator starts as is worked out before the loop, so it
        // is checked with the loop's own names still out of scope.
        let carried = match accumulator {
            Some(accumulator) => {
                let ty = self.infer(&accumulator.init);
                if let Some(def) = self.def_of(&accumulator.name) {
                    self.def_types.insert(def, ty.clone());
                }
                ty
            }
            None => Ty::Unit,
        };

        // The body may run no times at all, so what is known afterwards is
        // what was known before it or what is known after it, and nothing
        // stronger. Same reasoning as an `if` with no `else`.
        let outer = self.facts.clone();
        let because = match accumulator {
            Some(accumulator) => Some((
                accumulator.span,
                "the accumulator this has to produce again".to_string(),
            )),
            None => Some((
                span,
                "a `for` with no `with` produces `()` on every turn".to_string(),
            )),
        };
        self.check_block_against(body, &carried, because);
        self.facts = outer.join(&self.facts.clone());

        carried
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

            Expr::List { elements, .. } => self.infer_list(elements),

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

            Expr::Binary {
                op, lhs, rhs, span, ..
            } => {
                let left = self.infer(lhs);
                let right = self.infer(rhs);
                let left = self.widen(&left);
                let right = self.widen(&right);
                self.binary_ty(*op, &left, &right, lhs, rhs, *span)
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

            Expr::For {
                binder,
                iterable,
                accumulator,
                body,
                span,
            } => self.check_for(binder, iterable, accumulator.as_ref(), body, *span),

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
                    row: FnRow::Inferred,
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
                // One expression has one type here, and a generic function
                // named rather than called has as many as there are ways to
                // call it. Making this work needs a polymorphic value, which
                // is a much larger thing than substituting at a call site.
                Some(signature) if !signature.generics.is_empty() => {
                    let names: Vec<&str> = signature
                        .generics
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect();
                    let names = names.join(", ");
                    self.emit(
                        Diagnostic::error(
                            codes::GENERIC_AS_VALUE,
                            self.file,
                            ident.span,
                            format!(
                                "`{}` is generic, so naming it does not say what it is",
                                ident.name
                            ),
                        )
                        .with_primary_label(format!("nothing here says what `{names}` is"))
                        .with_note(
                            "call it, or write a closure that calls it at the type you want",
                        ),
                    );
                    Ty::Unknown
                }
                Some(signature) => Ty::Fn {
                    params: signature.params.iter().map(|p| p.ty.clone()).collect(),
                    row: signature.row.clone(),
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
            // A builtin with a signature is a function, so it has a type like
            // any other. The rest of the prelude is type names, and naming one
            // where a value belongs is the mistake `VOW4019` exists for.
            DefKind::Builtin => match self.signatures.get(&def) {
                Some(signature) => Ty::Fn {
                    params: signature.params.iter().map(|p| p.ty.clone()).collect(),
                    row: signature.row.clone(),
                    ret: Box::new(signature.ret.clone()),
                },
                None if matches!(ident.name.as_str(), "ok" | "err" | "at" | "push") => Ty::Unknown,
                None => {
                    self.not_a_value(ident, "a type");
                    Ty::Unknown
                }
            },
            // A handler names itself in a `with` block. It is not a value
            // anybody can do anything with, but it is a name for something,
            // and a name with no type is a name nothing is checked against.
            DefKind::Handler => Ty::Named {
                def,
                args: Vec::new(),
            },
            _ => Ty::Unknown,
        }
    }

    /// The type an imported name denotes when used as a value.
    fn imported_value_ty(&mut self, def: DefId, ident: &Ident) -> Ty {
        let Some(module) = self.resolutions.import_module(def) else {
            return Ty::Unknown;
        };
        match self.world.get(module, &ident.name) {
            // Generic, so naming it says as little here as it does at home.
            // The message is the same one a local generic function gets,
            // because it is the same mistake and a module boundary does not
            // make it a different one.
            Some(SurfaceItem::Function { generics, .. }) if !generics.is_empty() => {
                let names = generics.join(", ");
                self.emit(
                    Diagnostic::error(
                        codes::GENERIC_AS_VALUE,
                        self.file,
                        ident.span,
                        format!(
                            "`{}` is generic, so naming it does not say what it is",
                            ident.name
                        ),
                    )
                    .with_primary_label(format!("nothing here says what `{names}` is"))
                    .with_note("call it, or write a closure that calls it at the type you want"),
                );
                Ty::Unknown
            }
            Some(SurfaceItem::Function {
                params, ret, row, ..
            }) => Ty::Fn {
                params: params.clone(),
                row: row.clone(),
                ret: Box::new(ret.clone()),
            },
            // A variant is a value of its choice. One with a payload written
            // bare is still that type; the struct literal path is what checks
            // the payload, and it is reached from somewhere else.
            Some(SurfaceItem::Variant {
                choice, generics, ..
            }) => Ty::External {
                module: Rc::from(module),
                name: Rc::clone(choice),
                args: vec![Ty::Unknown; generics.len()],
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
            // here. It is not a value, but it is a name for something, and
            // leaving it as unknown would leave a hole in the one file that
            // could tell us about it.
            Some(SurfaceItem::Handler { .. }) => Ty::External {
                module: Rc::from(module),
                name: Rc::from(ident.name.as_str()),
                args: Vec::new(),
            },
            None => Ty::Unknown,
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
    ///
    /// Written bare, so nothing says what its type arguments are. A choice
    /// with parameters comes back applied to unknowns, which absorb, and that
    /// is the same answer `[]` gets for its element type: nothing was said, so
    /// nothing is claimed.
    fn variant_ty(&self, variant: DefId) -> Ty {
        match self.resolutions.def(variant).parent {
            Some(def) => Ty::Named {
                def,
                args: vec![Ty::Unknown; self.nominal_generics.get(&def).map_or(0, Vec::len)],
            },
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
        if let Ty::Named { def, args } = &looked_through
            && let Some(Nominal::Record { fields }) = self.types.nominal(*def)
            && let Some(field) = fields.iter().find(|field| field.name == name.name)
        {
            // The field type as this use of the record sees it. `left` on a
            // `Pair<Int, String>` is an `Int` rather than the `A` the
            // declaration wrote.
            return field.ty.substitute(&bindings_for(args));
        }
        if let Ty::External { args, .. } = &looked_through
            && let Some(SurfaceItem::Record { fields, .. }) = self.external_item(&looked_through)
            && let Some((_, ty)) = fields.iter().find(|(field, _)| *field == name.name)
        {
            return ty.substitute(&bindings_for(args));
        }

        let described = self.types.describe(receiver);
        let mut diagnostic = Diagnostic::error(
            codes::NO_SUCH_FIELD,
            self.file,
            name.span,
            format!("{described} has no field `{}`", name.name),
        )
        .with_primary_label("no such field");

        if let Ty::Named { def, .. } = looked_through
            && let Some(Nominal::Record { fields }) = self.types.nominal(def)
        {
            let available: Vec<&str> = fields.iter().map(|field| field.name.as_str()).collect();
            diagnostic = diagnostic.with_note(format!("it has {}", list(&available)));
        }
        if let Some(SurfaceItem::Record { fields, .. }) = self.external_item(&looked_through) {
            let available: Vec<&str> = fields.iter().map(|(name, _)| name.as_str()).collect();
            diagnostic = diagnostic.with_note(format!("it has {}", list(&available)));
        }

        self.emit(diagnostic);
        Ty::Unknown
    }

    /// `[1, 2, 3]`
    ///
    /// The first element decides the element type and every other element is
    /// checked against it. There is no unification anywhere in this checker,
    /// so there is nothing to meet two candidate types with, and "the first
    /// one decides" is the only rule that fits in a sentence.
    ///
    /// `[]` is `List<unknown>`, which is what lets an empty list go where any
    /// list was wanted without an annotation on the literal itself.
    fn infer_list(&mut self, elements: &'a [Expr]) -> Ty {
        let mut element = Ty::Unknown;
        for (index, expr) in elements.iter().enumerate() {
            let ty = self.infer(expr);
            if index == 0 {
                element = ty;
                continue;
            }
            let expected = element.clone();
            self.assign(
                &ty,
                &expected,
                Some(expr),
                expr.span(),
                Some((
                    elements[0].span(),
                    "the first element, which decides the element type".to_string(),
                )),
            );
        }
        Ty::List(Box::new(element))
    }

    /// `length`, `at` and `push`.
    ///
    /// Typed here rather than through a [`Signature`], because a signature is
    /// a list of concrete types and none of these has one: each is polymorphic
    /// in the element type. The same reasoning that keeps `ok` and `err` out
    /// of the signature table keeps these out of it, and the unknown type
    /// absorbing is what stands in for the unification there is none of.
    fn infer_prelude_call(&mut self, name: &str, args: &'a [Expr], span: Span) -> Ty {
        let types: Vec<Ty> = args.iter().map(|arg| self.infer(arg)).collect();

        let wanted = if name == "length" { 1 } else { 2 };
        if types.len() != wanted {
            self.emit(
                Diagnostic::error(
                    codes::WRONG_ARITY,
                    self.file,
                    span,
                    format!(
                        "`{name}` takes {wanted} argument{}, but {} {} given",
                        if wanted == 1 { "" } else { "s" },
                        types.len(),
                        if types.len() == 1 { "was" } else { "were" }
                    ),
                )
                .with_primary_label("wrong number of arguments"),
            );
            return Ty::Unknown;
        }

        let receiver = self.widen(&types[0]);

        // `length` predates lists and a `String` still has one. It is the same
        // question about a different thing, so it stayed one name.
        if name == "length" {
            if !receiver.absorbs() && !matches!(receiver, Ty::Str | Ty::List(_)) {
                let described = self.types.describe(&types[0]);
                self.emit(
                    Diagnostic::error(
                        codes::NOT_A_LIST,
                        self.file,
                        args[0].span(),
                        format!("`length` needs something with a length, and this is {described}"),
                    )
                    .with_primary_label("nothing to measure")
                    .with_note("`length` measures a `String` or a `List`"),
                );
            }
            return Ty::Int;
        }

        let Ty::List(element) = receiver else {
            // An unknown receiver already produced a diagnostic, or came from
            // a module that was not loaded. Either way a second complaint here
            // would be about the same mistake.
            if !receiver.absorbs() {
                let described = self.types.describe(&types[0]);
                self.emit(
                    Diagnostic::error(
                        codes::NOT_A_LIST,
                        self.file,
                        args[0].span(),
                        format!("`{name}` needs a list, and this is {described}"),
                    )
                    .with_primary_label("not a list"),
                );
            }
            return Ty::Unknown;
        };
        let element = *element;

        if name == "at" {
            self.assign(&types[1], &Ty::Int, Some(&args[1]), args[1].span(), None);
            // An index that is not there is not a mistake in the caller, and
            // nothing in this language stops a program, so it is an error
            // value like every other thing that can fail.
            return Ty::Result(Box::new(element), Box::new(Ty::Str));
        }

        self.assign(
            &types[1],
            &element,
            Some(&args[1]),
            args[1].span(),
            Some((args[0].span(), "the list it is pushed onto".to_string())),
        );
        Ty::List(Box::new(element))
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
            if matches!(name.as_str(), "length" | "at" | "push") {
                return self.infer_prelude_call(&name, args, span);
            }
        }

        // An operation of an effect from another module. Its signature is in
        // that module's surface, so the call is checked the same way a local
        // one is. Without this the whole call had no type, so the arguments,
        // the arity and the result were all unchecked, which is the same hole
        // an imported name has had three times now.
        if let Some(def) = callee_def
            && !self.signatures.contains_key(&def)
            && let Some(signature) = self.imported_operation_signature(def)
        {
            let name = self.resolutions.def(def).name.clone();
            let (params, ret) = signature;
            if args.len() != params.len() {
                self.emit(
                    Diagnostic::error(
                        codes::WRONG_ARITY,
                        self.file,
                        span,
                        format!(
                            "`{name}` takes {} argument{}, but {} {} given",
                            params.len(),
                            if params.len() == 1 { "" } else { "s" },
                            args.len(),
                            if args.len() == 1 { "was" } else { "were" }
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
            return ret;
        }

        // A call to a function in another module. Its surface carries its type
        // parameters, so a generic one is worked out here exactly the way a
        // local one is. It cannot go through the function type path below,
        // because a function type never carries type parameters of its own.
        if let Some(def) = callee_def
            && self.resolutions.def(def).kind == DefKind::Import
            && let Some(signature) = self.imported_function_signature(def)
        {
            return self.check_call_against(&signature, callee, args, span, None);
        }

        // A direct call to a declared function, where the parameter spans are
        // available and a mismatch can point at the declaration.
        if let Some(def) = callee_def
            && let Some(signature) = self.signatures.get(&def).cloned()
        {
            let name = self.types.name_of(def).to_string();
            return self.check_call_against(&signature, callee, args, span, Some(name));
        }

        let callee_ty = self.infer(callee);
        match callee_ty {
            Ty::Fn { params, ret, .. } => {
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

                // No instantiation here, and that is the point. A function
                // type never carries type parameters of its own: the `A` and
                // `B` in a `f: Fn(A) -> B` parameter belong to the function
                // that declared `f`, and inside that body they are settled
                // already. Substituting them would turn `f(x)` into an unknown
                // in the one place the answer was never in doubt.
                //
                // An imported generic function does not arrive here. It is
                // handled above, through its surface, where its parameters are
                // its own.
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
                        let arity = self.nominal_generics.get(&def).map_or(0, Vec::len);
                        let args = self.check_literal_fields(&declared, fields, span, &name, arity);
                        return Ty::Named { def, args };
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
                    let arity = parent
                        .and_then(|parent| self.nominal_generics.get(&parent))
                        .map_or(0, Vec::len);
                    let mut args = vec![Ty::Unknown; arity];
                    if let Some(variant) = declared {
                        let name = variant.name.clone();
                        args = self.check_literal_fields(
                            variant.fields.as_deref().unwrap_or(&[]),
                            fields,
                            span,
                            &name,
                            arity,
                        );
                    }
                    return match parent {
                        Some(def) => Ty::Named { def, args },
                        None => Ty::Unknown,
                    };
                }
                // A handler is not a value, but the literal that installs one
                // is checked like a record's. Without this the whole literal
                // had no type, so `with InMemory { count: "hello" }` was
                // accepted and a missing field became a runtime failure.
                DefKind::Handler => {
                    if let Some(Nominal::Handler { state }) = self.types.nominal(def) {
                        let state = state.clone();
                        let name = self.types.name_of(def).to_string();
                        self.check_literal_fields(&state, fields, span, &name, 0);
                        return Ty::Named {
                            def,
                            args: Vec::new(),
                        };
                    }
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

    /// An operation of an effect from another module.
    ///
    /// The resolver makes a definition for one on demand when a row or a call
    /// names it, and its parent is the import the effect came in through. That
    /// is the only handle this module has on it, so it is what the surface is
    /// looked up by.
    fn imported_operation_signature(&self, def: DefId) -> Option<(Vec<Ty>, Ty)> {
        if self.resolutions.def(def).kind != DefKind::EffectOp {
            return None;
        }
        let effect = self.resolutions.def(def).parent?;
        if self.resolutions.def(effect).kind != DefKind::Import {
            return None;
        }
        self.imported_operation(effect, &self.resolutions.def(def).name)
    }

    /// A literal of a record or variant declared in another module.
    fn imported_literal(&mut self, def: DefId, fields: &'a [FieldInit], span: Span) -> Option<Ty> {
        let module: Rc<str> = Rc::from(self.resolutions.import_module(def)?);
        let name = self.resolutions.def(def).name.clone();

        match self.world.get(&module, &name)? {
            SurfaceItem::Record {
                fields: declared,
                generics,
            } => {
                let arity = generics.len();
                let declared = external_fields(declared);
                let args = self.check_literal_fields(&declared, fields, span, &name, arity);
                Some(Ty::External {
                    module,
                    name: Rc::from(name.as_str()),
                    args,
                })
            }
            SurfaceItem::Variant {
                choice,
                fields: declared,
                generics,
            } => {
                let choice = Rc::clone(choice);
                let arity = generics.len();
                let declared = declared.as_deref().map(external_fields).unwrap_or_default();
                let args = self.check_literal_fields(&declared, fields, span, &name, arity);
                Some(Ty::External {
                    module,
                    name: choice,
                    args,
                })
            }
            SurfaceItem::Handler { state } => {
                let declared = external_fields(state);
                self.check_literal_fields(&declared, fields, span, &name, 0);
                Some(Ty::External {
                    module,
                    name: Rc::from(name.as_str()),
                    args: Vec::new(),
                })
            }
            // Not a constructor, which the fallthrough below reports the same
            // way it does for a local name.
            _ => None,
        }
    }

    /// Checks a literal against the fields it was declared with, and works out
    /// what its type arguments are while doing it.
    ///
    /// Every value is inferred before anything is checked, for the same reason
    /// a call infers every argument first: the declared field types are not
    /// types yet until the values have said what the type parameters are, and
    /// the first field cannot say what it wanted without the rest having been
    /// looked at.
    ///
    /// The answer is `arity` types long. A parameter no field mentioned comes
    /// back as unknown, which absorbs, and that is the same answer `[]` gets
    /// for its element type and `ok(x)` gets for its error type. A third
    /// answer to the same question would be a third thing to explain.
    fn check_literal_fields(
        &mut self,
        declared: &[FieldTy],
        fields: &'a [FieldInit],
        span: Span,
        what: &str,
        arity: usize,
    ) -> Vec<Ty> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut given: Vec<(FieldTy, Ty, &FieldInit, Span)> = Vec::new();

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
                    given.push((field, actual, init, value_span));
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

        let mut bindings = HashMap::new();
        if arity > 0 {
            for (field, actual, _, _) in &given {
                field.ty.bind(actual, &mut bindings);
            }
        }

        for (field, actual, init, value_span) in &given {
            self.assign(
                actual,
                &field.ty.substitute(&bindings),
                init.value.as_ref(),
                *value_span,
                // An empty span means the declaration is in another file, so
                // there is nothing here to point at.
                (field.span != Span::at(0))
                    .then(|| (field.span, "the field it is assigned to".to_string())),
            );
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

        (0..arity)
            .map(|index| bindings.get(&index).cloned().unwrap_or(Ty::Unknown))
            .collect()
    }

    fn infer_match(&mut self, scrutinee: &'a Expr, arms: &'a [MatchArm], span: Span) -> Ty {
        self.check_match(scrutinee, arms, span, None)
    }

    /// A `match`, with each arm checked knowing what its pattern bound.
    ///
    /// `wanted` is the type the whole expression has to produce, when there is
    /// one, and it is pushed into the arms for the same reason it is pushed
    /// into the branches of an `if`: an arm compared against the answer after
    /// the fact is no longer the arm being checked, and every refinement
    /// underneath it falls back to a runtime guard.
    fn check_match(
        &mut self,
        scrutinee: &'a Expr,
        arms: &'a [MatchArm],
        span: Span,
        wanted: Option<(Ty, Option<(Span, String)>)>,
    ) -> Ty {
        let scrutinee_ty = self.infer(scrutinee);
        let payload = self.ok_range_of(scrutinee);

        let outer = self.facts.clone();
        let mut after: Option<Facts> = None;
        let mut result: Option<Ty> = None;

        for arm in arms {
            // One arm's bindings are not another's, and nor are the facts
            // about them.
            self.facts = outer.clone();
            self.bind_pattern(&arm.pattern, &scrutinee_ty);

            // `match f() { ok(n) => ..` is the other way the payload of a
            // fallible call comes out, and the name bound here is the only
            // thing that ever stands for it.
            if let Some(def) = self.ok_binding(&arm.pattern) {
                let range = payload.meet(self.declared_range(def));
                self.facts.set(def, range);
            }

            let arm_ty = match &wanted {
                Some((expected, because)) => {
                    self.check_against(&arm.body, expected, because.clone())
                }
                None => self.infer(&arm.body),
            };

            // Only an arm that can fall through says anything about what is
            // known past the `match`.
            if !arm_ty.absorbs() {
                after = Some(match after {
                    Some(existing) => existing.join(&self.facts),
                    None => self.facts.clone(),
                });
            }

            match &result {
                None => result = Some(arm_ty),
                Some(expected) if expected.absorbs() => result = Some(arm_ty),
                Some(expected) => {
                    // Already checked against what was wanted, so comparing the
                    // arms again would report one mistake twice.
                    if wanted.is_none() {
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
        }

        self.facts = after.unwrap_or(outer);
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
        if let Some(SurfaceItem::Choice { variants, .. }) = self.external_item(&widened) {
            let names: Vec<String> = variants.iter().map(|v| v.name.clone()).collect();
            let choice = self.types.describe(&widened);
            self.check_named_exhaustive(&names, &choice, arms, span);
            return;
        }

        let Ty::Named { def, .. } = widened else {
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

    /// The name an `ok(n)` pattern binds, when it binds exactly one.
    fn ok_binding(&self, pattern: &Pattern) -> Option<DefId> {
        let Pattern::Tuple { path, elements, .. } = pattern else {
            return None;
        };
        let head = path.last()?;
        let head_def = self.resolutions.resolution(head.span)?;
        if self.resolutions.def(head_def).kind != DefKind::Builtin
            || self.resolutions.def(head_def).name != "ok"
        {
            return None;
        }

        let [Pattern::Path { segments, .. }] = elements.as_slice() else {
            return None;
        };
        let [only] = segments.as_slice() else {
            return None;
        };
        let def = self.def_of(only)?;
        (self.resolutions.def(def).kind == DefKind::Local).then_some(def)
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

                // What the scrutinee was applied to, so a binder reads at the
                // type the value actually has. `Some { value }` on an
                // `Option<Int>` binds an `Int` and not the `T` the choice was
                // declared with, and a binder that stayed a type parameter
                // would be a parameter of a declaration nobody is inside.
                let bindings = match self.widen(ty) {
                    Ty::Named { args, .. } | Ty::External { args, .. } => bindings_for(&args),
                    _ => HashMap::new(),
                };

                for field in fields {
                    let field_ty = variant_fields
                        .as_ref()
                        .and_then(|fields| fields.iter().find(|f| f.name == field.name.name))
                        .map(|f| f.ty.substitute(&bindings))
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

    fn binary_ty(
        &mut self,
        op: BinaryOp,
        left: &Ty,
        right: &Ty,
        lhs: &Expr,
        rhs: &Expr,
        span: Span,
    ) -> Ty {
        use BinaryOp::*;

        // If either side is unknown, the operator cannot say anything useful
        // and complaining about the other side would be guessing.
        let unknown = left.absorbs() || right.absorbs();

        match op {
            // `+` is the one operator with two meanings. Joining strings is
            // common enough that spelling it any other way would be a tax on
            // the most ordinary thing a program does, and the two meanings
            // never overlap: a `String` is not an `Int` and there is no
            // conversion between them, so no expression is ambiguous.
            Add if !unknown && (left == &Ty::Str || right == &Ty::Str) => {
                self.assign(
                    right,
                    &Ty::Str,
                    Some(rhs),
                    rhs.span(),
                    Some((lhs.span(), "joined with this".to_string())),
                );
                self.assign(
                    left,
                    &Ty::Str,
                    Some(lhs),
                    lhs.span(),
                    Some((rhs.span(), "joined with this".to_string())),
                );
                Ty::Str
            }
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
                    // Both sides have to agree, and then the thing they agree
                    // on has to be something with an order. Without the second
                    // half, comparing two records passed here and failed at
                    // runtime, which put the blame on the interpreter for
                    // something the type checker let through.
                    let before = self.diagnostics.len();
                    self.assign(
                        right,
                        left,
                        Some(rhs),
                        rhs.span(),
                        Some((lhs.span(), "compared with this".to_string())),
                    );
                    // Only when the sides agreed. Telling someone their two
                    // types do not match and then that the type they do not
                    // have has no ordering is two diagnostics for one mistake.
                    if self.diagnostics.len() == before {
                        self.require_order(left, span, op);
                    }
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

    /// Insists that `ty` is something `<` could mean anything about.
    ///
    /// `Int` and `String` and nothing else. There is no trait system, so a
    /// record has no ordering anyone could define, and accepting the comparison
    /// on the grounds that it might mean something one day is how a type
    /// checker ends up not checking.
    fn require_order(&mut self, ty: &Ty, at: Span, op: BinaryOp) {
        if matches!(self.widen(ty), Ty::Int | Ty::Str | Ty::Never) {
            return;
        }

        let described = self.types.describe(ty);
        let operator = op.as_str();
        self.emit(
            Diagnostic::error(
                codes::NOT_ORDERED,
                self.file,
                at,
                format!("`{operator}` needs an order, and there is none on {described}"),
            )
            .with_primary_label(format!("cannot be compared with `{operator}`"))
            .with_note(
                "`Int` and `String` are ordered; everything else can be compared with `==` but not ranked",
            ),
        );
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
        args: Vec::new(),
    }
}

/// What an `ensures` block promises about the returned value.
///
/// Only the `ok` outcome. A clause about the failure case says nothing about
/// the value a successful call hands back, which is the only thing a call site
/// is holding.
fn promised_by(ensures: &[Ensures], sig: &vow_ast::FnSig) -> Guarantee {
    let params: Vec<&str> = sig
        .params
        .iter()
        .map(|param| param.name.name.as_str())
        .collect();

    ensures
        .iter()
        .filter(|clause| clause.outcome == Outcome::Ok)
        .fold(Guarantee::any(), |promise, clause| {
            promise.meet(facts::promised_by(&clause.condition, "result", &params))
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
