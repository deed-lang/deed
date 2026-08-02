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

use deed_ast::{
    Accumulator, BinaryOp, Block, Ensures, Expr, FieldInit, FnDecl, HandlerDecl, Ident, Item,
    MatchArm, Module, Outcome, Pattern, Stmt, Type, TypeAlias, UnaryOp,
};
use deed_diagnostics::{Applicability, Diagnostic, FileId, Span};
use deed_resolve::{DefId, DefKind, Resolutions, RowLowering};

use crate::codes;
use crate::facts::{self, Facts, Guarantee, Promise, Range, Truth};
use crate::surface::{ClauseName, PRELUDE_MODULE, SurfaceItem, SurfaceRequires, World};
use crate::ty::{
    FieldTy, FnRow, Nominal, Obligation, Precondition, Tier, Ty, Types, VariantTy, bindings_for,
};

/// The prelude names that work on any type, so there is no one signature to
/// give them. Named in one place because the refusal and the note explaining it
/// have to agree: they did not, and a test that dropped one from the refusal
/// still passed on the strength of the other.
const GENERIC_BUILTINS: &[&str] = &["ok", "err", "at", "push", "repeat"];

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
        refuting: false,
        in_closure: 0,
        walking: Vec::new(),
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

/// Where the expected type was written, for the label on a mismatch.
///
/// The file is `None` for the file being checked, which is everything except a
/// signature that came across a module boundary.
type Because = Option<(Option<FileId>, Span, String)>;

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

/// The pieces of a `for`, so checking one takes an argument rather than seven.
struct Walk<'a> {
    binder: &'a Ident,
    index: Option<&'a Ident>,
    iterable: &'a Expr,
    accumulator: Option<&'a Accumulator>,
    keep: Option<&'a Expr>,
    body: &'a Block,
    span: Span,
}

/// What a caller has to guarantee, and the names the clauses talk about.
///
/// The clauses are kept as written rather than lowered to anything, because a
/// call site settles them by reading them with its own facts, and lowering
/// would mean deciding ahead of time which shapes are worth keeping.
#[derive(Debug)]
struct Requires {
    /// The definition of each parameter, in order, so a clause naming one can
    /// be told what was passed there.
    params: Vec<Option<DefId>>,
    origin: Origin,
}

/// Which module wrote the clauses, which decides how a name in one is read.
#[derive(Debug)]
enum Origin {
    /// This one. Every name in a clause resolves the way every other name in
    /// the file does.
    Here { clauses: Vec<Expr> },
    /// Another one, whose resolution this side does not have. The names were
    /// worked out where they were written and crossed as roles; see
    /// [`imported_name`] for the ids they stand for here.
    Elsewhere {
        /// The file the clauses are written in, so a label about one is drawn
        /// against the right bytes.
        file: FileId,
        declared: Rc<SurfaceRequires>,
    },
}

impl Requires {
    fn clauses(&self) -> &[Expr] {
        match &self.origin {
            Origin::Here { clauses } => clauses,
            Origin::Elsewhere { declared, .. } => &declared.clauses,
        }
    }
}

/// The id a name in an imported clause stands for while it is being read.
///
/// Invented rather than resolved, and safe to invent because nothing real ever
/// enters the table these are keys into. A call site building facts for an
/// imported clause writes them under exactly these ids and reads them back
/// under exactly these ids, so the numbers only have to agree with themselves.
/// The alternative, resolving the clause's spans against this module, would
/// not merely fail: it would answer about whatever this file happens to have
/// written at the same byte offset.
fn imported_name(name: ClauseName) -> DefId {
    match name {
        ClauseName::Length => DefId::from_raw(0),
        ClauseName::Param(index) => DefId::from_raw(index as u32 + 1),
    }
}

#[derive(Clone)]
struct Signature {
    params: Vec<ParamTy>,
    ret: Ty,
    span: Span,
    /// Which file `span` and the parameter spans are offsets into. `None` is
    /// the file being checked, which is every signature declared at home.
    file: Option<FileId>,
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
    /// The `where` clauses, for the call site to answer for.
    ///
    /// `None` rather than an empty list for the common case, so a signature
    /// with no contract costs no allocation. Shared, because a signature is
    /// cloned to instantiate it at a call and the clauses do not change.
    requires: Option<Rc<Requires>>,
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
    /// Whether the expression being checked is one an `assert refuses` says
    /// will break its contract.
    ///
    /// A contract the checker can see will be broken is that statement being
    /// right rather than a mistake, so the two diagnostics that would report
    /// it are silent inside one, and nothing is recorded as discharged.
    refuting: bool,
    /// How many closure bodies deep the expression being checked is.
    ///
    /// A closure captures the frame by value, so every name it reads is a copy
    /// taken where the closure was written. Handler state is the one name that
    /// is not a frame binding, and reading it through a closure is `DEED4030`.
    /// A depth rather than a flag because a closure can be written inside a
    /// closure, and nothing else nests: a declaration cannot appear inside an
    /// expression, so the count is only ever unwound by the closure that
    /// raised it.
    in_closure: usize,
    /// The binder of each `for` whose body is being checked, innermost last.
    ///
    /// An assignment is refused wherever it appears, but inside a walk the
    /// reader is almost always building something up, and this language
    /// spells that with the walk's own accumulator rather than with a
    /// mutable name. Knowing there is a walk around the assignment is what
    /// lets the message say the shape instead of only the rule.
    walking: Vec<String>,
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
        for (name, params, ret) in io_signatures() {
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
                    file: None,
                    generics: Vec::new(),
                    // An operation performs its own effect and nothing else,
                    // so naming one where a function type was wanted is only
                    // allowed if that type made room for it.
                    row: FnRow::Declared(vec![deed_resolve::RowEntry {
                        module: PRELUDE_MODULE.to_string(),
                        effect: "Io".to_string(),
                        operation: Some(name.to_string()),
                        variable: false,
                    }]),
                    guarantee: Guarantee::any(),
                    requires: None,
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
        let functions: [(&str, Vec<Ty>, Ty, Guarantee); 8] = [
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
            ("upper", vec![Ty::Str], Ty::Str, Guarantee::any()),
            ("lower", vec![Ty::Str], Ty::Str, Guarantee::any()),
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
                    file: None,
                    generics: Vec::new(),
                    row: FnRow::Declared(Vec::new()),
                    guarantee,
                    requires: None,
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
                        // A predicate over a value whose type is not known yet
                        // is a different question from expanding a name, and
                        // it is not one this language has answered. What could
                        // `length(value) > 0` mean about a `T`? So the
                        // parameters are refused rather than accepted and
                        // quietly ignored.
                        if let Some(parameter) = alias.generics.first() {
                            self.emit(
                                Diagnostic::error(
                                    codes::REFINEMENT_TYPE_PARAM,
                                    self.file,
                                    parameter.span,
                                    format!(
                                        "`{}` has a predicate, so it cannot take `{}`",
                                        alias.name.name, parameter.name
                                    ),
                                )
                                .with_primary_label("a refinement takes no type parameters")
                                .with_secondary(predicate.span(), "the predicate it carries")
                                .with_note(
                                    "an alias with no predicate expands to what it names, so \
                                     parameters on one are the same substitution a `record` \
                                     does",
                                )
                                .with_note(
                                    "a predicate about a value whose type is not decided yet \
                                     has nothing it can say, and deciding what it could is a \
                                     larger question than this",
                                ),
                            );
                        }
                        let base = self.lower_type(&alias.ty);
                        self.types.set_nominal(
                            def,
                            alias.name.name.clone(),
                            Nominal::Refinement {
                                base: base.clone(),
                                predicate: predicate.span(),
                            },
                        );

                        // Read the predicate as the expression it is, with
                        // `value` standing for what it is about. Nothing here
                        // needs the answer: what this leaves behind is a type
                        // per span, and without those the compiled backend has
                        // no way to turn the predicate into the runtime check
                        // the checker says it becomes.
                        let subject = self
                            .resolutions
                            .defs()
                            .find(|(_, data)| {
                                data.kind == DefKind::Local
                                    && data.name == "value"
                                    && data.span == alias.name.span
                            })
                            .map(|(id, _)| id);
                        if let Some(subject) = subject {
                            self.def_types.insert(subject, base);
                        }
                        self.infer(predicate);
                    } else {
                        self.declare_type_params(def, &alias.generics);
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
                    if !function.contract.requires.is_empty() {
                        signature.requires = Some(Rc::new(Requires {
                            params: function
                                .sig
                                .params
                                .iter()
                                .map(|param| self.def_of(&param.name))
                                .collect(),
                            origin: Origin::Here {
                                clauses: function.contract.requires.clone(),
                            },
                        }));
                    }
                    self.check_type_params_are_determined(&signature);
                    self.signatures.insert(def, signature);
                }
                _ => {}
            }
        }
    }

    fn lower_fields(&mut self, fields: &[deed_ast::FieldDecl]) -> Vec<FieldTy> {
        fields
            .iter()
            .map(|field| FieldTy {
                name: field.name.name.clone(),
                ty: self.lower_type(&field.ty),
                span: field.span,
                file: None,
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

    fn lower_signature(&mut self, sig: &deed_ast::FnSig) -> Signature {
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
            file: None,
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
            requires: None,
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
            file: signature.file,
            generics: Vec::new(),
            row: signature.row.clone(),
            guarantee: signature.guarantee.clone(),
            // The clauses talk about parameters by name, and substituting a
            // type parameter does not change which name is which.
            requires: signature.requires.clone(),
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
            param_spans,
            ret,
            generics,
            row,
            guarantee,
            requires,
            declared,
        }) = self.world.get(module, name)
        else {
            return None;
        };

        Some(Signature {
            params: params
                .iter()
                .zip(param_spans)
                .map(|(ty, span)| ParamTy {
                    ty: ty.clone(),
                    span: *span,
                })
                .collect(),
            ret: ret.clone(),
            span: declared.span,
            file: Some(declared.file),
            generics: generics
                .iter()
                .map(|name| (name.clone(), Span::at(0)))
                .collect(),
            row: row.clone(),
            guarantee: guarantee.clone(),
            // A precondition crosses whole. It is not a proof the callee did,
            // it is a question the caller has to answer, and the caller is the
            // only one who can: `halve(0 - 5)` against `where n >= 0` used to
            // pass in silence purely because `halve` was written next door.
            // What still does not cross is a refinement predicate, for the
            // reason given at the top of `surface.rs`.
            requires: requires.as_ref().map(|declared_requires| {
                Rc::new(Requires {
                    params: (0..declared_requires.arity)
                        .map(|index| Some(imported_name(ClauseName::Param(index))))
                        .collect(),
                    origin: Origin::Elsewhere {
                        file: declared.file,
                        declared: Rc::clone(declared_requires),
                    },
                })
            }),
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
                diagnostic = match signature.file {
                    Some(other) => {
                        diagnostic.with_secondary_in(other, signature.span, "declared here")
                    }
                    None => diagnostic.with_secondary(signature.span, "declared here"),
                };
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
            let because = (!param.span.is_empty()).then(|| {
                (
                    signature.file,
                    param.span,
                    "the parameter it is passed to".to_string(),
                )
            });
            self.assign(&actual[index], &param_ty, Some(arg), arg.span(), because);
        }

        if let Some(requires) = signature.requires.clone() {
            self.check_preconditions(&requires, args, span, name);
        }
        signature.ret
    }

    /// Whether the call site can answer for what the callee requires.
    ///
    /// `design/02-syntax.md` has always said a precondition is checked at the
    /// call site when it can be proven there. It was not: a `where` clause was
    /// a fact for the callee's body and a check inside the callee at runtime,
    /// and nothing looked at it from where the call was written. So
    /// `halve(0 - 5)` against `where n >= 0` passed the checker in silence and
    /// failed when it ran.
    ///
    /// The runtime check stays whatever happens here, the same way an
    /// `ensures` clause is evaluated on every call whatever tier it landed in.
    /// What this adds is a call that provably breaks the contract being a
    /// mistake at check time rather than at run time, and a tier that says
    /// which calls were settled.
    fn check_preconditions(
        &mut self,
        requires: &Requires,
        args: &'a [Expr],
        span: Span,
        name: Option<String>,
    ) {
        let facts = self.facts_for_call(requires, args);
        let called = match &name {
            Some(name) => format!("`{name}`"),
            None => "this".to_string(),
        };

        for clause in requires.clauses() {
            let outcome = self.clause_holds(requires, clause, &facts);

            // Inside an `assert refuses`, a clause that will not hold is the
            // statement being right, so nothing is reported and no tier is
            // recorded: a precondition that is meant to fail is not an
            // obligation anybody discharged, and counting one would say the
            // corpus is less proven than it is. What is recorded is that
            // somebody is aiming at this contract. The compiled backend drops
            // a callee's check when every recorded call proved the clause,
            // and this call is the one that needs the check to still be there.
            if self.refuting {
                if let Some(name) = &name {
                    self.types.push_refuted(name.clone());
                }
                continue;
            }

            let (tier, reason) = match outcome {
                Truth::Always => (Tier::Proven, None),
                Truth::Never => {
                    let diagnostic = Diagnostic::error(
                        codes::BROKEN_PRECONDITION,
                        self.file,
                        span,
                        format!("this call does not satisfy what {called} requires"),
                    )
                    .with_primary_label("the precondition does not hold here");
                    let diagnostic = match &requires.origin {
                        Origin::Here { .. } => {
                            diagnostic.with_secondary(clause.span(), "the clause it has to satisfy")
                        }
                        Origin::Elsewhere { file, .. } => diagnostic.with_secondary_in(
                            *file,
                            clause.span(),
                            "the clause it has to satisfy",
                        ),
                    };
                    self.emit(diagnostic.with_note(
                        "a precondition failure is a mistake in the caller, so it is reported here rather than inside the function",
                    ));
                    (Tier::Guarded, None)
                }
                Truth::Unknown(reason) => (Tier::Guarded, Some(reason)),
            };

            self.types.push_precondition(Precondition {
                span,
                tier,
                callee: name.clone().unwrap_or_default(),
                reason,
            });
        }
    }

    /// Whether one clause holds, read the way the module that wrote it meant.
    ///
    /// A clause written here is read with this module's resolver. One that
    /// crossed a boundary is read against the roles its own module worked out,
    /// and against nothing else: a call inside it names a function this side
    /// cannot look up, so it promises nothing rather than promising whatever a
    /// function of the same name here would.
    fn clause_holds(&self, requires: &Requires, clause: &Expr, facts: &Facts) -> Truth {
        match &requires.origin {
            Origin::Here { .. } => {
                let (def_of, call) = self.env();
                let env = facts::Env {
                    def_of: &def_of,
                    length: self.resolutions.builtin("length"),
                    call: &call,
                };
                facts::holds(clause, facts, &env)
            }
            Origin::Elsewhere { declared, .. } => {
                let def_of = |expr: &Expr| match expr {
                    Expr::Ident(ident) => declared.name_at(ident.span).map(imported_name),
                    _ => None,
                };
                let env = facts::Env {
                    def_of: &def_of,
                    length: Some(imported_name(ClauseName::Length)),
                    call: &|_| Promise::any(),
                };
                let outcome = facts::holds(clause, facts, &env);
                facts::thinned_by_boundary(clause, &env, outcome)
            }
        }
    }

    /// The caller's facts, said in the callee's parameter names.
    ///
    /// A clause talks about parameters and the facts are about arguments, so
    /// one has to be translated into the other. Each parameter gets the range
    /// of what was passed there, and where the argument is itself a term the
    /// differences between arguments come across too, which is what lets
    /// `where index < length(items)` be settled by a caller that checked the
    /// length.
    ///
    /// Nothing else crosses. A fact about a caller's local is not a fact about
    /// anything the clause can name.
    fn facts_for_call(&mut self, requires: &Requires, args: &'a [Expr]) -> Facts {
        let mut mapped = Facts::new();
        let mut pairs: Vec<(facts::Term, facts::Term)> = Vec::new();

        for (index, arg) in args.iter().enumerate() {
            let Some(Some(param)) = requires.params.get(index).copied() else {
                continue;
            };
            let range = self.range_of(arg);
            mapped.set(param, range);

            // How long the argument is, said about the parameter, so a clause
            // written as `index < length(items)` has something to compare
            // against. A name brings whatever the caller knows and a literal
            // brings its own size.
            let length = {
                let (def_of, call) = self.env();
                let env = facts::Env {
                    def_of: &def_of,
                    length: self.resolutions.builtin("length"),
                    call: &call,
                };
                facts::length_of(arg, &self.facts, &env)
            };
            mapped.narrow(facts::Term::Length(param), length);

            if let Some(term) = self.term_of(arg) {
                pairs.push((facts::Term::Name(param), term));
                if let facts::Term::Name(passed) = term {
                    pairs.push((facts::Term::Length(param), facts::Term::Length(passed)));
                }
            }
        }

        for (left, from_left) in &pairs {
            for (right, from_right) in &pairs {
                let known = self.facts.difference(*from_left, *from_right);
                if !known.is_any() {
                    mapped.narrow_difference(*left, *right, known);
                }
            }
        }
        mapped
    }

    /// What a fact could be attached to, for an expression in this body.
    fn term_of(&self, expr: &Expr) -> Option<facts::Term> {
        let (def_of, call) = self.env();
        let env = facts::Env {
            def_of: &def_of,
            length: self.resolutions.builtin("length"),
            call: &call,
        };
        facts::term_of(expr, &env)
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
                        let declared_here = self.resolutions.def(def).span;
                        if !self.check_type_arity(
                            &name.name,
                            arity,
                            lowered_args.len(),
                            *span,
                            Some((None, declared_here)),
                        ) {
                            return Ty::Unknown;
                        }
                        return Ty::Named {
                            def,
                            args: lowered_args,
                        };
                    }
                    DefKind::Type => {
                        // An alias expands, so its parameters are substituted
                        // here rather than carried. `Table<String, Int>` is
                        // `List<Entry<String, Int>>` and there is no `Table`
                        // afterwards, which is what makes it a name for a type
                        // rather than a type.
                        let arity = self.nominal_generics.get(&def).map_or(0, Vec::len);
                        let declared_here = self.resolutions.def(def).span;
                        if !self.check_type_arity(
                            &name.name,
                            arity,
                            lowered_args.len(),
                            *span,
                            Some((None, declared_here)),
                        ) {
                            return Ty::Unknown;
                        }
                        let target = self.alias_ty(def);
                        if lowered_args.is_empty() {
                            return target;
                        }
                        let bindings: HashMap<usize, Ty> =
                            lowered_args.iter().cloned().enumerate().collect();
                        return target.substitute(&bindings);
                    }
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
                        let mut diagnostic = Diagnostic::error(
                            codes::NOT_A_TYPE,
                            self.file,
                            name.span,
                            format!("`{}` is a {}, not a type", name.name, other.describe()),
                        )
                        .with_primary_label("not a type");
                        // Same pin the imported path already has: the name is a
                        // real declaration of the wrong kind, and "declared here"
                        // is where that kind is written.
                        let at = self.resolutions.def(def).span;
                        if !at.is_empty() {
                            diagnostic = diagnostic.with_secondary(at, "declared here");
                        }
                        self.emit(diagnostic);
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
    fn check_type_arity(
        &mut self,
        name: &str,
        wanted: usize,
        given: usize,
        span: Span,
        declared_here: Option<(Option<FileId>, Span)>,
    ) -> bool {
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
        let mut diagnostic = Diagnostic::error(
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
        );
        // The arity lives on the declaration. Without this pin the reader is
        // left counting angle brackets against a name that is already known.
        if let Some((file, at)) = declared_here {
            if !at.is_empty() {
                diagnostic = match file {
                    Some(other) => diagnostic.with_secondary_in(other, at, "declared here"),
                    None => diagnostic.with_secondary(at, "declared here"),
                };
            }
        }
        self.emit(diagnostic);
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
            Some(
                SurfaceItem::Record { generics, .. }
                | SurfaceItem::Choice { generics, .. }
                | SurfaceItem::Alias { generics, .. },
            ) => generics.len(),
            _ => 0,
        };
        let external = Ty::External {
            module: Rc::from(module),
            name: Rc::from(name.name.as_str()),
            args: args.to_vec(),
        };

        match self.world.get(module, &name.name) {
            // An alias is expanded, so it is whatever it was declared as and
            // takes nothing of its own beyond the parameters it wrote down. A
            // parameter on a refinement is a different question about what a
            // predicate may say, and `DEED4028` refuses it at the declaration.
            // The surface does not carry a declaration span for aliases yet.
            Some(SurfaceItem::Alias { target, .. }) => {
                let target = target.clone();
                if !self.check_type_arity(&name.name, arity, args.len(), span, None) {
                    return Ty::Unknown;
                }
                if args.is_empty() {
                    return target;
                }
                let bindings: HashMap<usize, Ty> = args.iter().cloned().enumerate().collect();
                target.substitute(&bindings)
            }
            Some(SurfaceItem::Record { declared, .. }) => {
                let pin = Some((Some(declared.file), declared.span));
                if !self.check_type_arity(&name.name, arity, args.len(), span, pin) {
                    return Ty::Unknown;
                }
                external
            }
            Some(SurfaceItem::Choice { .. } | SurfaceItem::Refinement { .. }) => {
                // Choice and refinement surfaces do not carry Declared today.
                if !self.check_type_arity(&name.name, arity, args.len(), span, None) {
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
                // Point at the real declaration when the surface carries one.
                // The old secondary used the use-site span and said "declared
                // in `module`", which underlined the mistake twice and never
                // the place the name was written.
                let declared = match other {
                    SurfaceItem::Function { declared, .. }
                    | SurfaceItem::Handler { declared, .. }
                    | SurfaceItem::Variant { declared, .. }
                    | SurfaceItem::Effect { declared, .. } => Some(*declared),
                    _ => None,
                };
                let mut diagnostic = Diagnostic::error(
                    codes::NOT_A_TYPE,
                    self.file,
                    name.span,
                    format!("`{}` is {what}, not a type", name.name),
                )
                .with_primary_label("not a type");
                diagnostic = match declared {
                    Some(at) => diagnostic.with_secondary_in(
                        at.file,
                        at.span,
                        format!("declared in `{module}`"),
                    ),
                    None => diagnostic.with_secondary(name.span, format!("declared in `{module}`")),
                };
                self.emit(diagnostic);
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
            // The row is the exception, and the only give in this function.
            // Refinements widen to their base, which is the checker's other
            // one, but that happens in `assign_carrying` after this has
            // already said no. See [`FnRow`].
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
        because: Because,
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
        because: Because,
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
            // The same question the base type would have been asked, rather
            // than equality. `[]` is a `List<unknown>` and fits a `List<Int>`,
            // so it fits the base of a refinement over one, and the thing
            // wrong with `first_of([])` is the predicate rather than the type.
            // Asking for equality here said "expected NonEmptyList, found
            // List<_>", which is a true sentence about the wrong problem.
            if self.compatible(&self.widen(actual), &base) {
                let subject = match expr {
                    Some(expr) => Some(self.subject_of(expr)),
                    None => carried.range.map(facts::Subject::of),
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

        if let Some((file, where_span, why)) = because {
            diagnostic = match file {
                Some(other) => diagnostic.with_secondary_in(other, where_span, why),
                None => diagnostic.with_secondary(where_span, why),
            };
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
        self.declared_facts(def).range
    }

    /// Everything a definition's declared type admits, if it is refined.
    fn declared_facts(&self, def: DefId) -> facts::Subject {
        let Some(Ty::Named {
            def: refinement, ..
        }) = self.def_types.get(&def)
        else {
            return facts::Subject::of(Range::ANY);
        };
        self.refinement_facts(*refinement)
    }

    /// The range a refinement admits, when its predicate is simple enough.
    fn refinement_range(&self, refinement: DefId) -> Range {
        self.refinement_facts(refinement).range
    }

    /// The same, for everything the predicate pins down.
    fn refinement_facts(&self, refinement: DefId) -> facts::Subject {
        let Some(alias) = self.aliases.get(&refinement) else {
            return facts::Subject::of(Range::ANY);
        };
        match &alias.refinement {
            Some(predicate) => {
                let (def_of, call) = self.env();
                let env = facts::Env {
                    def_of: &def_of,
                    length: self.resolutions.builtin("length"),
                    call: &call,
                };
                facts::admitted_by(predicate, &env)
            }
            None => facts::Subject::of(Range::ANY),
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
            length: self.resolutions.builtin("length"),
            call: &call,
        };
        facts::narrowed(condition, base, &env, when_true)
    }

    fn range_of(&self, expr: &Expr) -> Range {
        let (def_of, call) = self.env();
        let env = facts::Env {
            def_of: &def_of,
            length: self.resolutions.builtin("length"),
            call: &call,
        };
        facts::range_of(expr, &self.facts, &env)
    }

    /// The range the value inside the `ok` of `expr` lands in.
    fn ok_range_of(&self, expr: &Expr) -> Range {
        let (def_of, call) = self.env();
        let env = facts::Env {
            def_of: &def_of,
            length: self.resolutions.builtin("length"),
            call: &call,
        };
        facts::ok_range_of(expr, &self.facts, &env)
    }

    /// What is known about how long an expression is.
    fn length_of(&self, expr: &Expr) -> Range {
        let (def_of, call) = self.env();
        let env = facts::Env {
            def_of: &def_of,
            length: self.resolutions.builtin("length"),
            call: &call,
        };
        facts::length_of(expr, &self.facts, &env)
    }

    /// Where the arithmetic in `expr` can have no answer, if anywhere.
    fn overflowing(&self, expr: &Expr) -> Option<Span> {
        let (def_of, call) = self.env();
        let env = facts::Env {
            def_of: &def_of,
            length: self.resolutions.builtin("length"),
            call: &call,
        };
        facts::overflowing(expr, &self.facts, &env)
    }

    /// Whether the facts in scope settle a refinement predicate for a value.
    ///
    /// The value is a [`facts::Subject`] rather than an expression, because
    /// the interesting case is the one where nothing in the source names it:
    /// the number inside the `ok` of a call that can fail.
    fn proves(&self, predicate: &Expr, subject: Option<facts::Subject>) -> Truth {
        let Some(subject) = subject else {
            return Truth::Unknown(facts::Reason::NothingNamesThisValue);
        };
        let with_subject = self.facts.with_subject(subject);
        let (def_of, call) = self.env();
        let env = facts::Env {
            def_of: &def_of,
            length: self.resolutions.builtin("length"),
            call: &call,
        };
        facts::holds(predicate, &with_subject, &env)
    }

    /// Everything known about a value being checked against a refinement.
    ///
    /// A range answers `value > 0`. `length(value) > 0` is a question about
    /// how long the thing is, and it has two better answers than the default:
    /// a name is a term the body has been narrowing, and a literal says its
    /// own length out loud. The name only for a bare one, because `f(x)`
    /// produces a value nothing names and calling it the subject would mean
    /// two calls looked like one thing.
    fn subject_of(&self, expr: &Expr) -> facts::Subject {
        let name = match expr {
            Expr::Ident(_) => self.resolver()(expr),
            _ => None,
        };
        facts::Subject::of(self.range_of(expr))
            .with_length(self.length_of(expr))
            .with_name(name)
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
        subject: Option<facts::Subject>,
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
            // Not a value nothing narrowed: there is no predicate at all to
            // check it against, which is not a shape this checker reasons
            // about any more than a condition it does not recognise is.
            None => Truth::Unknown(facts::Reason::NotAShapeTheCheckerReasonsAbout),
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
                reason: None,
            }),
            // Inside an `assert refuses`, a value the checker can see will not
            // satisfy the refinement is the statement being right.
            Truth::Never if self.refuting => {}
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
            Truth::Unknown(reason) => {
                let mut diagnostic = Diagnostic::warning(
                    codes::UNPROVEN_REFINEMENT,
                    self.file,
                    span,
                    format!("cannot prove this satisfies `{name}`, so it becomes a runtime check"),
                )
                .with_primary_label("checked at runtime")
                .with_note(format!(
                    "obligations are Proven, Tested or Guarded, and this one is Guarded because {}; see design/02-syntax.md",
                    reason.text()
                ));
                if let Some(predicate) = predicate_text {
                    diagnostic =
                        diagnostic.with_secondary(predicate, "the predicate it has to satisfy");
                }

                // What gets through. "cannot prove" covers two situations that
                // read the same and are not the same: nothing is known about
                // the value, and enough is known to name a value that fails.
                // A converter guarded by `n >= 0` for a `value > 0` refinement
                // is the second, and that is a mistake rather than a limit of
                // the checker, so the number it lets past is worth saying.
                if let Some(witness) = predicate
                    .map(facts::range_admitted_by)
                    .zip(subject.map(|subject| subject.range))
                    .and_then(|(admitted, known)| escapes(known, admitted))
                {
                    diagnostic = diagnostic.with_note(format!(
                        "when this is {witness} it does not satisfy `{name}`, and what is known about it here does not rule that out"
                    ));
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
                    reason: Some(reason),
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
                    // `finally` is part of the handler and can read and write
                    // state. Its value is discarded, so it is checked like a
                    // test body: any type is fine.
                    if let Some(finally) = &handler.finally {
                        self.check_block(finally);
                    }
                    self.check_handler_is_whole(handler);
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
            let effect_name = self.resolutions.def(effect).name.clone();
            let mut diagnostic = Diagnostic::error(
                codes::OPERATION_MISMATCH,
                self.file,
                operation.sig.name.span,
                format!("`{effect_name}` does not declare an operation called `{name}`"),
            )
            .with_primary_label("not part of the effect");
            diagnostic = self.effect_secondary(
                diagnostic,
                effect,
                handler.effect.span,
                "the effect this handler implements",
            );
            self.emit(diagnostic);
            return None;
        };

        if params.len() != operation.sig.params.len() {
            let effect_name = self.resolutions.def(effect).name.clone();
            let mut diagnostic = Diagnostic::error(
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
            .with_note("a handler operation writes no types because the effect declares them, so the shape has to line up");
            diagnostic = self.effect_secondary(
                diagnostic,
                effect,
                handler.effect.span,
                "the effect this handler implements",
            );
            self.emit(diagnostic);
            return None;
        }

        Some((params, ret))
    }

    /// Points a secondary label at the effect a handler implements.
    ///
    /// Local effects sit on the use-site name already carried by
    /// `handler.effect.span`. Imported ones have a real declaration in another
    /// file now that the surface carries [`Declared`] for effects; without
    /// that the secondary only underlined the import name.
    fn effect_secondary(
        &self,
        diagnostic: Diagnostic,
        effect: DefId,
        fallback: Span,
        why: &str,
    ) -> Diagnostic {
        match self.resolutions.def(effect).kind {
            DefKind::Import => {
                let Some(module) = self.resolutions.import_module(effect) else {
                    return diagnostic.with_secondary(fallback, why);
                };
                let effect_name = &self.resolutions.def(effect).name;
                match self.world.get(module, effect_name) {
                    Some(SurfaceItem::Effect { declared, .. }) => {
                        diagnostic.with_secondary_in(declared.file, declared.span, why)
                    }
                    _ => diagnostic.with_secondary(fallback, why),
                }
            }
            _ => {
                let span = self.resolutions.def(effect).span;
                if span.is_empty() {
                    diagnostic.with_secondary(fallback, why)
                } else {
                    diagnostic.with_secondary(span, why)
                }
            }
        }
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
        let Some(SurfaceItem::Effect { operations, .. }) = self.world.get(module, effect_name)
        else {
            return None;
        };
        operations.get(name).cloned()
    }

    /// Checks that a handler implements every operation its effect declares.
    ///
    /// The other direction has been checked all along, in
    /// [`Checker::operation_signature`]: writing an operation the effect does
    /// not declare is DEED4021. Leaving one out was not checked anywhere, and
    /// the two are the same claim read from opposite ends.
    ///
    /// It matters because a `with` block discharges the effect and not the
    /// operations written inside the handler. So a caller declaring
    /// `uses Counter.total`, which is the caller doing everything right, could
    /// be handed a handler with no `total` and find out when the call reached
    /// it. Reporting it here means the handler is wrong where the handler is
    /// written, rather than at whichever call happened to need the gap.
    fn check_handler_is_whole(&mut self, handler: &'a HandlerDecl) {
        let Some(effect) = self.def_of(&handler.effect) else {
            return;
        };
        let declared = match self.resolutions.def(effect).kind {
            DefKind::Import => self.imported_operation_names(effect),
            DefKind::Effect => self.local_operation_names(effect),
            // Not an effect at all. The resolver has already said so, and a
            // list of operations it does not have would be piling on.
            _ => return,
        };

        let written: HashSet<&str> = handler
            .operations
            .iter()
            .map(|operation| operation.sig.name.name.as_str())
            .collect();
        let missing: Vec<String> = declared
            .into_iter()
            .filter(|name| !written.contains(name.as_str()))
            .collect();
        if missing.is_empty() {
            return;
        }

        // The import's own name, not `name_of` on the import DefId (that is the
        // local alias table and is not the effect's spelling across the boundary).
        let effect_name = self.resolutions.def(effect).name.clone();
        let handler_name = &handler.name.name;
        let listed = missing
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let counted = if missing.len() == 1 {
            "one operation".to_string()
        } else {
            format!("{} operations", missing.len())
        };

        let mut diagnostic = Diagnostic::error(
            codes::HANDLER_MISSING_OPERATION,
            self.file,
            handler.name.span,
            format!("`{handler_name}` does not implement {listed}"),
        )
        .with_primary_label(format!("{counted} still to write"))
        .with_note(
            "a `with` block discharges the effect rather than the operations written inside the handler, so installing one is a claim that every call underneath has somewhere to go",
        );
        diagnostic = self.effect_secondary(
            diagnostic,
            effect,
            handler.effect.span,
            &format!("`{effect_name}` declares them"),
        );
        self.emit(diagnostic);
    }

    /// The operations of an effect declared in this module, in declaration
    /// order.
    fn local_operation_names(&self, effect: DefId) -> Vec<String> {
        self.resolutions
            .defs()
            .filter(|(_, data)| data.kind == DefKind::EffectOp && data.parent == Some(effect))
            .map(|(_, data)| data.name.clone())
            .collect()
    }

    /// The operations of an effect from another module.
    fn imported_operation_names(&self, effect: DefId) -> Vec<String> {
        let Some(module) = self.resolutions.import_module(effect) else {
            return Vec::new();
        };
        let effect_name = &self.resolutions.def(effect).name;
        let Some(SurfaceItem::Effect { operations, .. }) = self.world.get(module, effect_name)
        else {
            return Vec::new();
        };
        operations.keys().cloned().collect()
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
                let admitted = self.declared_facts(def);
                if !admitted.range.is_any() {
                    self.facts.set(def, admitted.range);
                }
                // A refinement about how long something is is as much a fact
                // as one about what it is worth, and it belongs on the term
                // everything else reads. Without this a `NonEmptyList`
                // parameter knew nothing about its own length inside the body
                // that declared it.
                if !admitted.length.is_any() {
                    self.facts.note(facts::Term::Length(def), admitted.length);
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
            Some((None, ret_span, "the declared return type".to_string())),
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
    fn check_against(&mut self, expr: &'a Expr, expected: &Ty, because: Because) -> Ty {
        let ty = self.check_against_inner(expr, expected, because);
        // The bidirectional path skips `infer`, which is where every other
        // expression gets its type written down. Without this an `if` or a
        // `match` in tail position is the one shape the compiler knows the
        // type of and never says, which the backend then cannot lower.
        self.types.record_expr(expr.span(), ty.clone());
        ty
    }

    fn check_against_inner(&mut self, expr: &'a Expr, expected: &Ty, because: Because) -> Ty {
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
            Expr::For {
                binder,
                index,
                iterable,
                accumulator,
                keep,
                body,
                span,
            } => self.check_for_against(
                Walk {
                    binder,
                    index: index.as_ref(),
                    iterable,
                    accumulator: accumulator.as_ref(),
                    keep: keep.as_deref(),
                    body,
                    span: *span,
                },
                Some(expected.clone()),
            ),
            Expr::Block(block) => self.check_block_against(block, expected, because),
            other => {
                let ty = self.infer(other);
                self.assign(&ty, expected, Some(other), other.span(), because);
                ty
            }
        }
    }

    fn check_block_against(&mut self, block: &'a Block, expected: &Ty, because: Because) -> Ty {
        let mut diverges = false;
        for stmt in &block.stmts {
            self.check_stmt(stmt);
            if matches!(stmt, Stmt::Return { .. } | Stmt::Abandon { .. }) {
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
        wanted: Option<(Ty, Because)>,
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
                            Some((None, then_branch.span, "the other branch".to_string())),
                        );
                    }
                    // The branch that knows more decides. Taking the first one
                    // meant `if c { kept } else { push(kept, one) }` reported
                    // whatever `kept` started as, so writing the unchanged case
                    // first gave a list of unknown and writing it second did
                    // not. Two spellings of one filter should not typecheck
                    // differently.
                    settled(&then_ty, &else_ty)
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
            if matches!(stmt, Stmt::Return { .. } | Stmt::Abandon { .. }) {
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
                            Some((None, annotation.span(), "declared here".to_string())),
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
                    let mut diagnostic = Diagnostic::error(
                        codes::NOT_ASSIGNABLE,
                        self.file,
                        *span,
                        format!("`{}` is a {}, not handler state", target.name, kind.describe()),
                    )
                    .with_primary_label("cannot be assigned to")
                    .with_secondary(declared, "declared here")
                    .with_note(
                        "handler state is the only mutable thing in Deed, which is what lets an empty effect row mean a function cannot change anything",
                    );
                    // Inside a walk this is almost always somebody building a
                    // value up, and the rule alone leaves them looking for a
                    // mutable name they will not find.
                    if let Some(item) = self.walking.last() {
                        diagnostic = diagnostic.with_note(format!(
                            "a `for` carries what it is building: \
                             `for {item} in ... with {name} = ... {{ ... }}`, and the value \
                             of the block is `{name}` on the next turn",
                            name = target.name
                        ));
                    }
                    self.emit(diagnostic);
                    return;
                }

                self.closed_over_state(target, def, "assigned to inside a closure");
                if self.in_closure > 0 {
                    return;
                }

                let declared = self.def_types.get(&def).cloned().unwrap_or(Ty::Unknown);
                let field_span = self.resolutions.def(def).span;
                self.assign(
                    &actual,
                    &declared,
                    Some(value),
                    value.span(),
                    Some((None, field_span, "the state it is assigned to".to_string())),
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
                    Some((None, ret_span, "the declared return type".to_string())),
                );
            }
            Stmt::Assert { condition, .. } => {
                let ty = self.infer(condition);
                self.assign(&ty, &Ty::Bool, Some(condition), condition.span(), None);
            }
            // The claim is that evaluating this breaks a contract, so the type
            // of the value it would have produced is not the question and any
            // type is fine.
            //
            // What is checked here is turned off while it runs. A contract the
            // checker can see will be broken is this statement agreeing with
            // it, not a mistake, so `DEED4025` and a violated refinement are
            // both silent inside one. Nothing is recorded either: a
            // precondition that is meant to fail is not an obligation anybody
            // discharged.
            Stmt::Refuses { subject, .. } => {
                let outer = self.refuting;
                self.refuting = true;
                self.infer(subject);
                self.refuting = outer;
            }
            // `abandon` is a diverging statement, like `return`. No type to
            // check: it never produces a value and the block it lives in is
            // marked diverging by `check_block_against`.
            Stmt::Abandon { .. } => {}
            Stmt::Expr(expr) => {
                let ty = self.infer(expr);
                self.discarded(&ty, expr);
            }
        }
    }

    /// A statement that produces a value nobody reads.
    ///
    /// A block's value is its tail, so every other expression in it is there
    /// for what it does rather than for what it is. When it produces `()` that
    /// is the whole story. When it produces something else the value has
    /// nowhere to go, and the two ways to arrive here are both mistakes: a
    /// result that was meant to be looked at, or a line that was meant to
    /// belong to the one above it.
    ///
    /// The second one is why this is here. An expression ends at the end of a
    /// line, so `let a = 1` with `-2` under it is two statements rather than
    /// one, which is the honest reading and still leaves a line doing nothing.
    /// Saying so is the difference between a rule that is right and a rule
    /// that helps.
    ///
    /// A warning rather than an error, because `let _ = f()` is how you say
    /// you meant it and a program should not have to be rewritten to keep
    /// compiling. `Unknown` and `Never` say nothing: the first is a type the
    /// checker does not have and the second is an expression that never
    /// produced a value at all.
    ///
    /// `f()?` is exempt. The value it drops is the success case, and the case
    /// worth not losing is the error, which `?` returns rather than discards.
    /// A statement written as `?` is a statement about the failure, so the rest
    /// of it going nowhere is what it says.
    fn discarded(&mut self, ty: &Ty, expr: &Expr) {
        if matches!(ty, Ty::Unit | Ty::Unknown | Ty::Never) || matches!(expr, Expr::Try { .. }) {
            return;
        }

        let found = self.types.describe(ty);
        let advice = if matches!(ty, Ty::Result(..)) {
            // Worth its own sentence. Dropping an `Int` wastes a line and
            // dropping a `Result` loses the failure, which is the thing the
            // type was carrying and the reason it is a `Result` at all.
            "the failure case goes with it; use `?`, a `match`, or `let _ = ...` if that is what you meant"
        } else {
            "write `let _ = ...` if that is what you meant"
        };

        self.emit(
            Diagnostic::warning(
                codes::DISCARDED_VALUE,
                self.file,
                expr.span(),
                format!("this produces {found} and nothing reads it"),
            )
            .with_primary_label("the value goes nowhere")
            .with_note(advice)
            // The first fix the type checker has ever carried, and it is not
            // the shape the others are. A type that does not fit has no
            // obvious repair, which is why there were none. This one has
            // exactly one mechanical answer, and it is still a guess, because
            // the other way to arrive here is a value that was supposed to be
            // read and `let _ =` would bury that. An editor offers it and
            // `deed fix` leaves it alone.
            .with_fix(
                "say the value is being dropped",
                Span::new(expr.span().start, expr.span().start),
                "let _ = ",
                Applicability::MaybeIncorrect,
            ),
        );
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
    ///
    /// `while so_far` stops the walk early. It is read before each turn with
    /// the accumulator in scope, so it needs one: a condition that can only
    /// read things the walk never changes either stops it before it starts or
    /// never stops it at all, and neither is a thing anybody meant to write.
    fn check_for(&mut self, walk: Walk<'a>) -> Ty {
        self.check_for_against(walk, None)
    }

    /// The same, with what the whole walk has to produce when something above
    /// it said.
    ///
    /// The accumulator is the walk's value, so a walk in tail position of a
    /// function returning `Option<String>` starts with an accumulator of that
    /// type. Without it `with seen = None` is a variant of a generic choice
    /// with nothing saying what it holds, and every read of `seen` before the
    /// body settles it is a value the compiler knows nothing about.
    fn check_for_against(&mut self, walk: Walk<'a>, wanted: Option<Ty>) -> Ty {
        let Walk {
            binder,
            index,
            iterable,
            accumulator,
            keep,
            body,
            span,
        } = walk;
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

        // Where in the list the element was. Not negative, and below however
        // long the list is, which is the fact that makes it worth binding
        // rather than counting by hand: a walk that indexes something with it
        // can say so.
        if let Some(index) = index
            && let Some(def) = self.def_of(index)
        {
            self.def_types.insert(def, Ty::Int);
            self.facts.set(def, Range::between(0, i64::MAX));
            if let Some(walked) = self.term_of(iterable)
                && let facts::Term::Name(walked) = walked
            {
                self.facts.narrow_difference(
                    facts::Term::Name(def),
                    facts::Term::Length(walked),
                    Range::between(i64::MIN, -1),
                );
            }
        }

        // What the accumulator starts as is worked out before the loop, so it
        // is checked with the loop's own names still out of scope.
        let carried = match accumulator {
            Some(accumulator) => {
                let ty = match &wanted {
                    // What the walk has to produce is what the accumulator
                    // is, so an initialiser that says less than the context
                    // does is filled in from the context. `settled` rather
                    // than the context outright, and one level at a time, for
                    // the same reason it is used on what the body produced:
                    // where the initialiser knew, it wins.
                    Some(wanted) if !wanted.absorbs() => {
                        let got = self.check_against(
                            &accumulator.init,
                            wanted,
                            Some((
                                None,
                                accumulator.span,
                                "what this walk has to produce".to_string(),
                            )),
                        );
                        let ty = settled(&got, wanted);
                        self.types.record_expr(accumulator.init.span(), ty.clone());
                        ty
                    }
                    _ => self.infer(&accumulator.init),
                };
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

        if let Some(keep) = keep {
            if accumulator.is_none() {
                self.emit(
                    Diagnostic::error(
                        codes::WHILE_WITHOUT_ACCUMULATOR,
                        self.file,
                        keep.span(),
                        "a `while` on a `for` needs a `with`",
                    )
                    .with_primary_label("nothing here changes between turns")
                    .with_secondary(span, "this walk has no accumulator")
                    .with_note(
                        "the condition is about what the walk has worked out so far, and one \
                         that can only read what the walk never changes either stops it before \
                         it starts or never stops it at all",
                    ),
                );
            }
            let ty = self.infer(keep);
            self.assign(&ty, &Ty::Bool, Some(keep), keep.span(), None);
            // Read before each turn, so the body knows it held. What is known
            // after the loop does not, because the walk also ends by running
            // out of list.
            self.facts = self.narrowed_by(keep, true);
        }

        let because = match accumulator {
            Some(accumulator) => Some((
                None,
                accumulator.span,
                "the accumulator this has to produce again".to_string(),
            )),
            None => Some((
                None,
                span,
                "a `for` with no `with` produces `()` on every turn".to_string(),
            )),
        };
        self.walking.push(binder.name.to_string());
        let produced = self.check_block_against(body, &carried, because);
        self.walking.pop();
        self.facts = outer.join(&self.facts.clone());

        // What the accumulator ends up holding, rather than what it started
        // as. `[]` is a list of unknown until the body says what goes into it,
        // and reporting the initialiser's type handed a list of unknown to
        // whatever walked what the loop built. An unknown element agrees with
        // everything, so nothing done with those elements was checked.
        //
        // Only where the initialiser did not know. The loop may run no times
        // at all, so the answer has to accept what it started as, and where
        // that is a real type it wins: an accumulator that started as an `Int`
        // is an `Int` even if every turn happens to produce a `Positive`.
        let ty = settled(&carried, &produced);

        // Said about the initialiser too. `with seen = None` on an
        // `Option<String>` is the same expression that only the body settles,
        // and what it was recorded as is what the backend has to build it
        // from.
        if let Some(accumulator) = accumulator
            && carried != ty
        {
            self.types.record_expr(accumulator.init.span(), ty.clone());
            if let Some(def) = self.def_of(&accumulator.name) {
                self.def_types.insert(def, ty.clone());
            }
        }
        ty
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

            Expr::Field {
                receiver,
                name,
                span,
            } => {
                if let Some(ty) = self.limit_of_a_type(receiver, name, *span) {
                    return ty;
                }
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
                        Some((
                            None,
                            ret_span,
                            "the error type this function returns".to_string(),
                        )),
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
                index,
                iterable,
                accumulator,
                keep,
                body,
                span,
            } => self.check_for(Walk {
                binder,
                index: index.as_ref(),
                iterable,
                accumulator: accumulator.as_ref(),
                keep: keep.as_deref(),
                body,
                span: *span,
            }),

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
                self.in_closure += 1;
                let ret = self.infer(body);
                self.in_closure -= 1;
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
                    self.state_was_given(handler);
                }
                self.check_block(body)
            }
        }
    }

    /// A handler installed by name alone, with the state it declared left out.
    ///
    /// `with H { n: 0 }` is checked as a literal, so leaving a field out or
    /// naming one that is not there is already `DEED4002`. `with H { .. }`
    /// parses as the handler on its own followed by the block, the literal
    /// check never ran, and the missing field waited for the interpreter.
    /// Nothing about it needs waiting for: the state is declared here and
    /// whether a value was written is a fact about the source. See #820.
    fn state_was_given(&mut self, handler: &'a Expr) {
        let Expr::Ident(ident) = handler else {
            return;
        };
        let Some(def) = self.def_of(ident) else {
            return;
        };
        if self.resolutions.def(def).kind != DefKind::Handler {
            return;
        }
        let Some(Nominal::Handler { state }) = self.types.nominal(def) else {
            return;
        };
        let state = state.clone();
        let name = self.types.name_of(def).to_string();
        let declared_here = self.resolutions.def(def).span;
        self.check_literal_fields(
            &state,
            &[],
            ident.span,
            &name,
            0,
            Some((None, declared_here)),
        );
    }

    /// Refuses a closure that names the handler state around it.
    ///
    /// Handler state is the one mutable thing in the language and its lifetime
    /// is the `with` block that installed the handler. A closure's is not: it
    /// is a value, it leaves through a function type, and it can be called
    /// after the block has ended or underneath a different handler entirely.
    /// The interpreter used to answer such a read out of whichever handler was
    /// innermost when the call landed, which is a wrong number rather than a
    /// refusal whenever two handlers share a state name.
    ///
    /// Capturing the handler was the other way out and it is refused here
    /// instead, because the closure's type would not say it. `Fn() -> Int`
    /// says the value takes nothing and performs nothing, and one that is also
    /// a live window onto a particular handler's state carries an input and a
    /// lifetime through a signature that mentions neither. See
    /// `design/03-effects.md`.
    ///
    /// Lexical rather than about where the closure ends up. Working out
    /// whether a particular closure escapes is escape analysis, and the reason
    /// a closure's effects are charged to whoever wrote it is that this
    /// language does not want to have to answer that question.
    fn closed_over_state(&mut self, ident: &Ident, def: DefId, label: &str) {
        if self.in_closure == 0 || self.resolutions.def(def).kind != DefKind::State {
            return;
        }
        let declared = self.resolutions.def(def).span;
        let name = ident.name.clone();
        self.emit(
            Diagnostic::error(
                codes::CLOSURE_OVER_STATE,
                self.file,
                ident.span,
                format!("`{name}` is handler state, and this closure can outlive the handler"),
            )
            .with_primary_label(label)
            .with_secondary(declared, "the handler state it names")
            .with_note(
                "a closure captures the frame by value, so read the state into a local and let the closure carry that number",
            )
            .with_note(
                "a handler lives as long as the `with` block that installed it, and nothing in the closure's type says which handler it came from",
            ),
        );
    }

    fn ident_ty(&mut self, ident: &Ident) -> Ty {
        let Some(def) = self.def_of(ident) else {
            return Ty::Unknown;
        };
        self.closed_over_state(ident, def, "read inside a closure");
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
            // where a value belongs is the mistake `DEED4019` exists for.
            DefKind::Builtin => match self.signatures.get(&def) {
                Some(signature) => Ty::Fn {
                    params: signature.params.iter().map(|p| p.ty.clone()).collect(),
                    row: signature.row.clone(),
                    ret: Box::new(signature.ret.clone()),
                },
                // #818: these work on any type, so there is no one signature to
                // hand back. `Unknown` absorbs, so a bare one used to compare
                // equal to anything and reach the interpreter, which has no
                // value to give it either.
                None if GENERIC_BUILTINS.contains(&ident.name.as_str()) => {
                    self.not_a_value(ident, "a builtin that works on any type");
                    Ty::Unknown
                }
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

    /// `Int.max` and `Int.min`, which are names this language does not have.
    ///
    /// The answer used to be that `Int` is a type and not a value, which is
    /// true and is not the question. `Int` is a signed 64-bit integer and
    /// nothing in a program says so, and the place it comes up is a `where`
    /// clause that has to keep a sum inside the type, which is a real thing to
    /// want: overflow stops the program, so a property test finds the edge
    /// even when nothing else does.
    ///
    /// The number is what there is, so it is named and written. A function
    /// returning it would arrive at a clause as a call rather than as a
    /// number, and a bound nothing can read is not a bound.
    fn limit_of_a_type(&mut self, receiver: &Expr, name: &Ident, span: Span) -> Option<Ty> {
        let Expr::Ident(ident) = receiver else {
            return None;
        };
        if ident.name != "Int" || self.resolutions.resolution(name.span).is_some() {
            return None;
        }
        let (which, literal) = match name.name.as_str() {
            "max" => ("largest", i64::MAX.to_string()),
            // Written as a subtraction because the smallest `Int` has no
            // literal: negation is an operator, and the digits it would be
            // applied to are one past the largest.
            "min" => ("smallest", format!("0 - {} - 1", i64::MAX)),
            _ => return None,
        };
        self.emit(
            Diagnostic::error(
                codes::NO_LIMIT_NAME,
                self.file,
                span,
                format!("there is no name for the {which} `Int`"),
            )
            .with_primary_label(format!("write {literal}"))
            .with_note(
                "`Int` is a signed 64-bit integer, and the number is the only way to say so \
                 in a program",
            )
            .with_note(
                "a clause about a bound is read where it is written, so the bound has to be a \
                 number there rather than a call that would produce one",
            )
            .with_fix(
                format!("write the {which} `Int`"),
                span,
                literal,
                Applicability::MachineApplicable,
            ),
        );
        Some(Ty::Int)
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

        if GENERIC_BUILTINS.contains(&name.as_str()) {
            diagnostic = diagnostic.with_note(format!(
                "`{name}` works on any type, so it has no one type to be a value of; \
                 call it rather than naming it"
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
            let at = self.resolutions.def(def).span;
            if !at.is_empty() {
                diagnostic = diagnostic.with_secondary(at, "declared here");
            }
        }
        if let Some(SurfaceItem::Record {
            fields,
            declared: at,
            ..
        }) = self.external_item(&looked_through)
        {
            let available: Vec<&str> = fields.iter().map(|(name, _)| name.as_str()).collect();
            diagnostic = diagnostic.with_note(format!("it has {}", list(&available)));
            if !at.span.is_empty() {
                diagnostic = diagnostic.with_secondary_in(at.file, at.span, "declared here");
            }
        }

        // `xs.length()` is what somebody writes on their first day, because
        // every language they came from has methods. `length` is here, it is
        // just a function, and "no such field" sends them looking for a field
        // rather than telling them the call is spelled the other way round.
        if self.resolutions.builtin(&name.name).is_some() {
            diagnostic = diagnostic.with_note(format!(
                "there are no methods: `{name}` is a function and takes the value as its \
                 first argument, so this is written `{name}(x)` rather than `x.{name}()`",
                name = name.name
            ));
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
                    None,
                    elements[0].span(),
                    "the first element, which decides the element type".to_string(),
                )),
            );
        }
        Ty::List(Box::new(element))
    }

    /// `length`, `at`, `push` and `repeat`.
    ///
    /// Typed here rather than through a [`Signature`], because a signature is
    /// a list of concrete types and none of these has one.
    ///
    /// That is where the similarity ends, and the difference matters enough
    /// to write down. `at`, `push` and `repeat` are here for a mechanical
    /// reason only: they are polymorphic in the element and the table holds
    /// concrete types. Nothing stops anyone writing them, and `std/table`
    /// writes the `at`-shaped one. They are not exempt from anything.
    ///
    /// `ok` and `err`, typed further down, are the ones that are. They
    /// return an error type that appears nowhere in their arguments, which
    /// is what [`Self::check_type_params_are_determined`] refuses, so they
    /// are a rule being broken rather than a table being missed.
    ///
    /// `length` is a third case and the only ad-hoc one in the prelude: its
    /// signature says `String` and the check below also takes a list, so one
    /// name covers two receivers. There is no overloading and no way to say
    /// "anything with a length", so this is the one prelude entry a user
    /// could not have written.
    ///
    /// The unknown type absorbing is what stands in for the unification
    /// there is none of.
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

        // The one that builds a list rather than reading one, so its first
        // argument is the element and there is nothing to check about it: a
        // list of anything is a list.
        if name == "repeat" {
            self.assign(&types[1], &Ty::Int, Some(&args[1]), args[1].span(), None);
            return Ty::List(Box::new(types[0].clone()));
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
            Some((
                None,
                args[0].span(),
                "the list it is pushed onto".to_string(),
            )),
        );
        // What is in the list after this, rather than what was in it before.
        // `[]` is a list of unknown, so pushing onto one used to hand back
        // another list of unknown and the element type never got worked out at
        // all. The value going in is the only thing that knows.
        Ty::List(Box::new(settled(&element, &types[1])))
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
            if matches!(name.as_str(), "length" | "at" | "push" | "repeat") {
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
            let name = self.resolutions.def(def).name.clone();
            return self.check_call_against(&signature, callee, args, span, Some(name));
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
                        let declared_here = self.resolutions.def(def).span;
                        let args = self.check_literal_fields(
                            &declared,
                            fields,
                            span,
                            &name,
                            arity,
                            Some((None, declared_here)),
                        );
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
                            Some((None, variant.span)),
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
                        let declared_here = self.resolutions.def(def).span;
                        self.check_literal_fields(
                            &state,
                            fields,
                            span,
                            &name,
                            0,
                            Some((None, declared_here)),
                        );
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
            let mut diagnostic = Diagnostic::error(
                codes::NOT_A_CONSTRUCTOR,
                self.file,
                path.span(),
                format!("{described} is not a record or a variant"),
            )
            .with_primary_label("cannot be built with a literal");
            // A named head that is the wrong kind of declaration: point at it.
            // Values without a declaration (parameters, locals) stay primary-only.
            if let Some(def) = ctor {
                let at = self.resolutions.def(def).span;
                if !at.is_empty() {
                    diagnostic = match self.resolutions.def(def).kind {
                        DefKind::Import => {
                            // Prefer the other file when the import is a real
                            // export; fall back to the use-site name if not.
                            match self.resolutions.import_module(def) {
                                Some(module) => {
                                    let name = self.resolutions.def(def).name.clone();
                                    match self.world.get(module, &name) {
                                        Some(
                                            SurfaceItem::Function { declared, .. }
                                            | SurfaceItem::Handler { declared, .. }
                                            | SurfaceItem::Effect { declared, .. }
                                            | SurfaceItem::Variant { declared, .. }
                                            | SurfaceItem::Record { declared, .. },
                                        ) => diagnostic.with_secondary_in(
                                            declared.file,
                                            declared.span,
                                            "declared here",
                                        ),
                                        _ => diagnostic.with_secondary(at, "declared here"),
                                    }
                                }
                                None => diagnostic.with_secondary(at, "declared here"),
                            }
                        }
                        _ => diagnostic.with_secondary(at, "declared here"),
                    };
                }
            }
            self.emit(diagnostic);
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
                field_spans,
                generics,
                declared: at,
            } => {
                let arity = generics.len();
                let declared = external_fields(declared, Some((field_spans, at.file)));
                let args = self.check_literal_fields(
                    &declared,
                    fields,
                    span,
                    &name,
                    arity,
                    Some((Some(at.file), at.span)),
                );
                Some(Ty::External {
                    module,
                    name: Rc::from(name.as_str()),
                    args,
                })
            }
            SurfaceItem::Variant {
                choice,
                fields: declared,
                field_spans,
                generics,
                declared: at,
            } => {
                let choice = Rc::clone(choice);
                let arity = generics.len();
                let declared = declared
                    .as_deref()
                    .map(|fields| {
                        external_fields(
                            fields,
                            field_spans.as_deref().map(|spans| (spans, at.file)),
                        )
                    })
                    .unwrap_or_default();
                let args = self.check_literal_fields(
                    &declared,
                    fields,
                    span,
                    &name,
                    arity,
                    Some((Some(at.file), at.span)),
                );
                Some(Ty::External {
                    module,
                    name: choice,
                    args,
                })
            }
            SurfaceItem::Handler {
                state,
                state_spans,
                declared: at,
            } => {
                let declared = external_fields(state, Some((state_spans, at.file)));
                self.check_literal_fields(
                    &declared,
                    fields,
                    span,
                    &name,
                    0,
                    Some((Some(at.file), at.span)),
                );
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
        declared_here: Option<(Option<FileId>, Span)>,
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
                    let mut diagnostic = Diagnostic::error(
                        codes::UNKNOWN_FIELD,
                        self.file,
                        init.name.span,
                        format!("`{what}` has no field `{}`", init.name.name),
                    )
                    .with_primary_label("no such field")
                    .with_note(format!("it has {}", list(&available)));
                    // Same pin MISSING_FIELDS already has: the declaration is
                    // where the real fields live, and a wrong name is the other
                    // half of the same question.
                    if let Some((file, at)) = declared_here {
                        if !at.is_empty() {
                            diagnostic = match file {
                                Some(other) => {
                                    diagnostic.with_secondary_in(other, at, "declared here")
                                }
                                None => diagnostic.with_secondary(at, "declared here"),
                            };
                        }
                    }
                    self.emit(diagnostic);
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
                // An empty span means there is nothing to point at.
                (field.span != Span::at(0)).then(|| {
                    (
                        field.file,
                        field.span,
                        "the field it is assigned to".to_string(),
                    )
                }),
            );
        }

        let missing: Vec<&str> = declared
            .iter()
            .map(|field| field.name.as_str())
            .filter(|name| !seen.contains(*name))
            .collect();

        if !missing.is_empty() {
            let mut diagnostic = Diagnostic::error(
                codes::MISSING_FIELDS,
                self.file,
                span,
                format!("`{what}` is missing {}", list(&missing)),
            )
            .with_primary_label("incomplete literal")
            .with_note(
                "every field has to be given, because a partially built value is not a value",
            );
            if let Some((file, at)) = declared_here {
                if !at.is_empty() {
                    diagnostic = match file {
                        Some(other) => diagnostic.with_secondary_in(other, at, "declared here"),
                        None => diagnostic.with_secondary(at, "declared here"),
                    };
                }
            }
            self.emit(diagnostic);
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
        wanted: Option<(Ty, Because)>,
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
                            Some((None, arms[0].span, "the first arm".to_string())),
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
            for pattern in alternatives_of(&arm.pattern) {
                match pattern {
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
                                    catch_all.get_or_insert(pattern.span());
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
            for pattern in alternatives_of(&arm.pattern) {
                match pattern {
                    Pattern::Wildcard(span) => {
                        catch_all.get_or_insert(*span);
                    }
                    Pattern::Path { segments, .. } => match segments.last() {
                        Some(last) if variants.contains(&last.name) => {
                            covered.insert(last.name.clone());
                        }
                        // A bare binding matches every variant.
                        _ => {
                            catch_all.get_or_insert(pattern.span());
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
    fn check_result_exhaustive(&mut self, arms: &[MatchArm], span: Span) {
        let mut covered: HashSet<&'static str> = HashSet::new();
        let mut catch_all: Option<Span> = None;

        for arm in arms {
            for pattern in alternatives_of(&arm.pattern) {
                match pattern {
                    Pattern::Wildcard(span) => {
                        catch_all.get_or_insert(*span);
                    }
                    Pattern::Path { .. } => {
                        catch_all.get_or_insert(pattern.span());
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
            // An alternative binds nothing, which the resolver enforces, so
            // there is nothing here to give a type to.
            | Pattern::OneOf { .. }
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
            // `+` means two things, and so do the comparisons below. None of
            // them is ambiguous, for the same reason: a `String` is not an
            // `Int` and there is no conversion between them, so no expression
            // is unsure which meaning it wanted. What `+` has that they do not
            // is a result that changes shape with the meaning. Joining is
            // common enough that spelling it any other way would be a tax on
            // the most ordinary thing a program does, which is the argument
            // for this arm and not for theirs.
            Add if !unknown && (left == &Ty::Str || right == &Ty::Str) => {
                self.assign(
                    right,
                    &Ty::Str,
                    Some(rhs),
                    rhs.span(),
                    Some((None, lhs.span(), "joined with this".to_string())),
                );
                self.assign(
                    left,
                    &Ty::Str,
                    Some(lhs),
                    lhs.span(),
                    Some((None, rhs.span(), "joined with this".to_string())),
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
                        Some((None, lhs.span(), "compared with this".to_string())),
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
                        Some((None, lhs.span(), "compared with this".to_string())),
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
    ///
    /// `String` is here on purpose rather than by leftover, though it started
    /// as one: this rule replaced one that asked only that both sides agree,
    /// so text was already comparable and narrowing kept it. It stays because
    /// it is the only thing in the language that puts two pieces of text in an
    /// order and never ties two of them that differ. `length` and `to_int`
    /// both rank text and both tie, `ab` with `ba` and `007` with `7`, and
    /// `to_int` refuses everything that does not spell a number; `Io.list`
    /// sorts file names, which is a real order over text, but it wants a `Dir`,
    /// two entries in the row and a written file per comparison, and it ties
    /// whatever the filesystem does not tell apart. A record is refused and a
    /// caller who wants two ranked passes the comparison in, which asks that
    /// caller for nothing it does not already have to have, since the shape of
    /// a record does not say which field decides the order and some of those
    /// fields have no order of their own. Text is reachable as well:
    /// `split(s, "")` hands back the characters. What a comparator written
    /// over those cannot do is rank a character nobody typed into it, because
    /// there is no code point and `to_int` only speaks about text that spells
    /// a number, so the characters it turns into numbers are the ten digits
    /// and no letter. Refusing here would not make ordering text impossible,
    /// it would buy a hand-written alphabet per program that sorts names and a
    /// silent tie for everything outside it.
    ///
    /// `design/02-syntax.md` carries the argument and what it rules out, and
    /// the corpus carries the consequence nobody should meet by surprise,
    /// which is that `"10" < "9"`.
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

/// The patterns an arm actually tests against.
///
/// One, unless the arm names alternatives, in which case it is each of them.
/// Every exhaustiveness walk wants this and none of them wants to know the
/// difference: an arm reading `Plus | Times` covers both, exactly as two arms
/// would have, which is the point of the feature and the reason the rule about
/// catch-alls did not have to change to allow it.
fn alternatives_of(pattern: &Pattern) -> &[Pattern] {
    match pattern {
        Pattern::OneOf { alternatives, .. } => alternatives,
        other => std::slice::from_ref(other),
    }
}

/// A builtin capability type.
///
/// Named under the prelude rather than under whichever module mentioned it,/// because there is exactly one `Console` and every module has to agree about
/// that. Naming it after the module would make the same capability compare
/// unequal to itself across a file boundary, which a test caught.
fn capability(name: &str) -> Ty {
    Ty::External {
        module: Rc::from(PRELUDE_MODULE),
        name: Rc::from(name),
        args: Vec::new(),
    }
}

/// The signature of every `Io` operation.
///
/// Public because it is the only place the compiler writes down which
/// operations hand a capability back, and that is the claim the capability
/// argument rests on: authority narrows on the way down and there is no
/// operation that widens it. Two operations return a `Dir`, both of them
/// rooted inside the one they were given, and each has a test that climbing
/// out of what came back is refused. A third would need the same, and a
/// declaration nothing outside this file can read is a declaration nobody
/// checks that against.
pub fn io_signatures() -> Vec<(&'static str, Vec<Ty>, Ty)> {
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

    vec![
        ("write", vec![console, Ty::Str], Ty::Unit),
        ("now", vec![clock.clone()], Ty::Int),
        // The machine's clock, in milliseconds since 1970. A separate
        // entry in the row for the same reason `save` is separate from
        // `read`: holding a `Clock` says nothing about which of these a
        // function may do, and the difference between them is whether the
        // program gives the same answer twice.
        ("epoch", vec![clock], Ty::Int),
        ("open", vec![dir.clone(), Ty::Str], io_error(dir.clone())),
        ("read", vec![dir.clone(), Ty::Str], io_error(Ty::Str)),
        (
            "save",
            vec![dir.clone(), Ty::Str, Ty::Str],
            io_error(Ty::Unit),
        ),
        // Destroying rather than replacing. A separate entry in the row
        // for the same reason `save` is separate from `read`: what a
        // caller is handing over is which of these a function may do, and
        // holding the directory says nothing about that.
        ("remove", vec![dir.clone(), Ty::Str], io_error(Ty::Unit)),
        // Making a place rather than putting something in one. The `Dir`
        // it hands back is rooted inside the one it was given, so this is
        // `open` on a directory that did not exist yet rather than a way
        // to reach anything new.
        ("make", vec![dir.clone(), Ty::Str], io_error(dir)),
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
    ]
}

/// Whether `ty` is one of the four builtin capabilities.
///
/// The names live here rather than in a list somewhere, because this is the
/// function that builds them and a second copy is a second thing to keep in
/// step.
pub fn is_capability(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::External { module, name, .. }
            if &**module == PRELUDE_MODULE
                && matches!(&**name, "Console" | "Clock" | "Dir" | "System")
    )
}

/// `carried` with the parts it did not know filled in from `produced`.
///
/// A `for` used to report what its accumulator started as, and `[]` starts as
/// a list of unknown. So `let kept = for x in xs with out = [] { push(out, x) }`
/// produced a list of unknown, and a walk over `kept` bound an element nothing
/// was known about, which agrees with everything and is checked against
/// nothing. That is the shape this repository has been caught by three times,
/// and the invariant that catches it found this one in the first program of the
/// second phase.
///
/// Where the initialiser knows, it wins, because the loop may run no times at
/// all and the answer has to accept what it started as. An accumulator that
/// started as an `Int` is an `Int` even if every turn happens to hand back a
/// `Positive`. `Never` never wins: a body that always returns says nothing
/// about a loop that took no turns.
fn settled(carried: &Ty, produced: &Ty) -> Ty {
    match (carried, produced) {
        (Ty::Unknown, other) if !other.absorbs() => other.clone(),
        (Ty::List(started), Ty::List(ended)) => Ty::List(Box::new(settled(started, ended))),
        (Ty::Result(ok_started, err_started), Ty::Result(ok_ended, err_ended)) => Ty::Result(
            Box::new(settled(ok_started, ok_ended)),
            Box::new(settled(err_started, err_ended)),
        ),
        // A bare variant is its choice with unknown arguments, so a fold whose
        // accumulator starts at `None` and ends at `Some { value: 3 }` is the
        // same question one level in.
        (
            Ty::Named {
                def: started,
                args: started_args,
            },
            Ty::Named {
                def: ended,
                args: ended_args,
            },
        ) if started == ended && started_args.len() == ended_args.len() => Ty::Named {
            def: *started,
            args: settled_all(started_args, ended_args),
        },
        (
            Ty::External {
                module: started_module,
                name: started_name,
                args: started_args,
            },
            Ty::External {
                module: ended_module,
                name: ended_name,
                args: ended_args,
            },
        ) if started_module == ended_module
            && started_name == ended_name
            && started_args.len() == ended_args.len() =>
        {
            Ty::External {
                module: Rc::clone(started_module),
                name: Rc::clone(started_name),
                args: settled_all(started_args, ended_args),
            }
        }
        _ => carried.clone(),
    }
}

fn settled_all(started: &[Ty], ended: &[Ty]) -> Vec<Ty> {
    started
        .iter()
        .zip(ended)
        .map(|(started, ended)| settled(started, ended))
        .collect()
}

/// What an `ensures` block promises about the returned value.
///
/// Only the `ok` outcome. A clause about the failure case says nothing about
/// the value a successful call hands back, which is the only thing a call site
/// is holding.
fn promised_by(ensures: &[Ensures], sig: &deed_ast::FnSig) -> Guarantee {
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

/// A value the checker knows is possible and the refinement turns down.
///
/// `None` when there is nothing to say: when everything known is admitted, so
/// the proof failed for some other reason, and when nothing is known at all,
/// where any number would be an invention rather than a finding. The witness
/// is taken from the end of the known range that sticks out, so it is the
/// nearest miss rather than an extreme.
fn escapes(known: Range, admitted: Range) -> Option<i64> {
    let (
        Range::Bounded { low, high },
        Range::Bounded {
            low: ok,
            high: hi_ok,
        },
    ) = (known, admitted)
    else {
        return None;
    };
    if known.is_any() || admitted.is_any() {
        return None;
    }
    if low < ok {
        return Some(ok.checked_sub(1).unwrap_or(low).max(low));
    }
    if high > hi_ok {
        return Some(hi_ok.checked_add(1).unwrap_or(high).min(high));
    }
    None
}

/// Fields from another module's surface, as the checker's own field type.
///
/// `written` is where they are declared and which file that is. Records,
/// variants and handlers all carry those spans now; callers that still have
/// none pass `None` and the diagnostic points at nothing rather than at the
/// wrong place.
fn external_fields(fields: &[(String, Ty)], written: Option<(&[Span], FileId)>) -> Vec<FieldTy> {
    fields
        .iter()
        .enumerate()
        .map(|(index, (name, ty))| {
            let (span, file) = match written {
                Some((spans, file)) => (
                    spans.get(index).copied().unwrap_or_else(|| Span::at(0)),
                    Some(file),
                ),
                None => (Span::at(0), None),
            };
            FieldTy {
                name: name.clone(),
                ty: ty.clone(),
                span,
                file,
            }
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
    use super::{escapes, list};
    use crate::facts::Range;

    #[test]
    fn lists_read_like_english() {
        assert_eq!(list(&[]), "");
        assert_eq!(list(&["a"]), "`a`");
        assert_eq!(list(&["a", "b"]), "`a` and `b`");
        assert_eq!(list(&["a", "b", "c"]), "`a`, `b` and `c`");
    }

    /// The witness a `Guarded` refinement names, and when there is not one.
    ///
    /// The boundaries are the whole content: a range that reaches exactly as
    /// far as the predicate allows has nothing sticking out, and one that
    /// reaches one further has exactly one number to report. Reading `<` as
    /// `<=` would invent a number for the first, which is a diagnostic telling
    /// somebody they have a bug they do not have.
    #[test]
    fn a_range_that_fits_inside_the_predicate_escapes_nothing() {
        assert_eq!(escapes(Range::between(1, 10), Range::between(1, 10)), None);
        assert_eq!(escapes(Range::between(2, 9), Range::between(1, 10)), None);
        assert_eq!(escapes(Range::exactly(1), Range::between(1, 10)), None);
    }

    #[test]
    fn a_range_reaching_past_the_predicate_names_the_nearest_miss() {
        // `n >= 0` against `value > 0`, which is the converter the design
        // document names: the number that gets through is zero, not the
        // smallest integer there is.
        assert_eq!(
            escapes(Range::between(0, i64::MAX), Range::between(1, i64::MAX)),
            Some(0)
        );
        assert_eq!(
            escapes(Range::between(-50, 5), Range::between(1, 10)),
            Some(0)
        );
        // And from the other end.
        assert_eq!(
            escapes(Range::between(1, 11), Range::between(1, 10)),
            Some(11)
        );
    }

    #[test]
    fn nothing_known_and_nothing_required_name_no_number() {
        // Any number here would be an invention rather than a finding.
        assert_eq!(escapes(Range::ANY, Range::between(1, 10)), None);
        assert_eq!(escapes(Range::between(1, 10), Range::ANY), None);
        assert_eq!(escapes(Range::Empty, Range::between(1, 10)), None);
    }
}
