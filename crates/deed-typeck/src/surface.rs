//! What one module's declarations look like from another module.
//!
//! The type checker works in one module at a time and every `DefId` it sees is
//! an index into that module's resolution table. Nothing in that table can name
//! anything outside it, which is why an imported name used to have no type at
//! all.
//!
//! A surface is the same declarations lowered so they can be read from
//! anywhere: every type in it is either primitive or a [`Ty::External`], which
//! is identified by a module path and a name rather than by an index. Lowering
//! one module never asks about another, so the order they are visited in does
//! not matter and an import cycle still resolves.
//!
//! One step does need all of them, and it is the last one. A transparent alias
//! is a name for a type rather than a type, so a signature written with one
//! has to cross as what it names, and a module that imported the alias cannot
//! see what that is. [`World::of`] writes them out once every module is in,
//! which is why that is the only way to build a world with anything in it.
//!
//! What is deliberately not carried across: refinement predicates. An exported
//! `type Positive = Int where value > 0` arrives as an opaque named type rather
//! than as a proof obligation. Carrying the predicate means carrying the
//! expression it is written in, which means carrying that module's scope, and
//! that is a much larger thing than this.
//!
//! What does cross is what a function promises about its result: the range it
//! lands in, and the range of `result - argument` for each argument a clause
//! ties it to. That is the difference between exporting a proof and exporting
//! the conclusion of one. A caller gets what it needs to reason, and nothing
//! about how the callee decided it.
//!
//! A `where` clause crosses whole, which looks like the opposite decision and
//! is not. A precondition is not a proof the callee did, it is a question the
//! caller has to answer, and the caller is the only one who can. So the clause
//! travels as written, along with [`SurfaceRequires::names`], which says what
//! every identifier in it refers to. That part is resolved here, in the module
//! that declared the function, because a `DefId` belongs to the resolution
//! that made it and a span means nothing in another file.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use deed_ast::{BinaryOp, Expr, FieldDecl, Ident, Item, Module, Outcome, Type};
use deed_diagnostics::{FileId, Span};
use deed_resolve::{DefKind, Resolutions, RowLowering};

use crate::facts::{self, Guarantee, Range};
use crate::ty::{FnRow, Ty};

/// The module path builtin types are named under.
///
/// Defined by the resolver, which is where the prelude is, and re-exported
/// here because this is the file that explains why a module path is the
/// identity of a type at all.
pub use deed_resolve::PRELUDE_MODULE;

/// Where a declaration is written.
///
/// Carried across the boundary because a diagnostic about a call can now point
/// into the file that declares what was called. Before a `Label` had a file of
/// its own there was nowhere to put this, so the checker filled the span with
/// `Span::at(0)` and then declined to draw it.
#[derive(Clone, Copy, Debug)]
pub struct Declared {
    pub file: FileId,
    /// The whole declaration, which is what "declared here" underlines.
    pub span: Span,
}

/// What an identifier written inside a `where` clause refers to.
///
/// Only the two things a call site can do anything with. Anything else in a
/// clause is left unresolved on the far side, which makes the clause unknown
/// rather than false, which is the safe direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClauseName {
    /// The parameter at this position in the declaration.
    Param(usize),
    /// The `length` the language provides.
    Length,
}

/// A function's preconditions, readable from another module.
#[derive(Debug)]
pub struct SurfaceRequires {
    /// The clauses as written. A call site on the far side reads them with its
    /// own facts, which is exactly what a call site at home does.
    pub clauses: Vec<Expr>,
    /// How many parameters the declaration has, so an argument list can be
    /// lined up against it.
    pub arity: usize,
    /// What each identifier inside the clauses refers to, by where it is
    /// written. A list rather than a map because a clause holds a handful of
    /// names and looking one up is not worth a hash.
    pub names: Vec<(Span, ClauseName)>,
}

impl SurfaceRequires {
    /// What the identifier at `span` refers to, or `None` for anything this
    /// side cannot say.
    pub fn name_at(&self, span: Span) -> Option<ClauseName> {
        self.names
            .iter()
            .find(|(at, _)| *at == span)
            .map(|(_, name)| *name)
    }
}

/// One exported declaration, with its types readable from outside.
#[derive(Clone, Debug)]
pub enum SurfaceItem {
    Function {
        params: Vec<Ty>,
        /// Where each parameter is written, in the same order.
        param_spans: Vec<Span>,
        ret: Ty,
        /// The type parameters it was declared with, in order.
        ///
        /// Carried by name rather than by count, so a caller on the far side
        /// can say which one it could not work out. The types themselves refer
        /// to them by position, which is what makes them portable at all.
        generics: Vec<String>,
        /// What its contract says a call performs, so that naming it as a
        /// value across a module boundary is checked the same way naming one
        /// at home is.
        row: FnRow,
        /// What a call is promised to hand back. See the note at the top about
        /// why the bounds cross and the predicate does not.
        guarantee: Guarantee,
        /// What a caller has to answer for. `None` for a function with no
        /// `where` clause, which is most of them.
        ///
        /// Shared rather than cloned, because expanding aliases rebuilds every
        /// item and a clause has no types in it to expand.
        requires: Option<Rc<SurfaceRequires>>,
        declared: Declared,
    },
    Record {
        fields: Vec<(String, Ty)>,
        /// Where each field is written, in the same order.
        field_spans: Vec<Span>,
        /// The type parameters it was declared with. A use of it owes exactly
        /// this many arguments.
        generics: Vec<String>,
        declared: Declared,
    },
    Choice {
        variants: Vec<SurfaceVariant>,
        generics: Vec<String>,
    },
    /// A variant, exported in its own right, remembering what it constructs.
    Variant {
        choice: Rc<str>,
        fields: Option<Vec<(String, Ty)>>,
        /// Where each field is written, in the same order.
        field_spans: Option<Vec<Span>>,
        /// The type parameters of the choice, not of the variant. A variant
        /// has none of its own.
        generics: Vec<String>,
        declared: Declared,
    },
    /// A refinement, opaque from outside. See the note at the top of the file.
    Refinement { base: Ty },
    /// A transparent alias, which is just its target.
    ///
    /// The parameters travel with it because the substitution happens where
    /// it is used, and a use on the far side of a boundary is a use like any
    /// other.
    Alias { target: Ty, generics: Vec<String> },
    Effect {
        operations: BTreeMap<String, (Vec<Ty>, Ty)>,
        declared: Declared,
    },
    /// A handler, and the state a `with` block has to initialise.
    ///
    /// The state crosses because installing a handler from another module is
    /// still writing a literal, and a literal nobody checks is a literal that
    /// can put a `String` where an `Int` was declared.
    Handler {
        state: Vec<(String, Ty)>,
        /// Where each state field is written, in the same order.
        state_spans: Vec<Span>,
        declared: Declared,
    },
}

#[derive(Clone, Debug)]
pub struct SurfaceVariant {
    pub name: String,
    pub fields: Option<Vec<(String, Ty)>>,
}

/// Everything one module offers, lowered.
#[derive(Clone, Debug, Default)]
pub struct Surface {
    items: BTreeMap<String, SurfaceItem>,
    /// The operators this module bound, and the function each one means.
    ///
    /// The binding travels with the type rather than staying at home. A module
    /// that imports `Ratio` imports what `+` means on one, because the
    /// alternative is a type whose arithmetic only works in the file that
    /// declared it. Where a binding may be *written* is still narrow, and that
    /// is what keeps the meaning decided in one place.
    operators: Vec<(BinaryOp, String)>,
}

impl Surface {
    pub fn get(&self, name: &str) -> Option<&SurfaceItem> {
        self.items.get(name)
    }
}

/// Every module's surface, keyed by module path.
#[derive(Clone, Debug, Default)]
pub struct World {
    modules: BTreeMap<String, Surface>,
}

impl World {
    /// A world with nothing in it, for a file that imports nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every module at once.
    ///
    /// All of them together rather than one at a time, because a signature can
    /// mention a transparent alias that a third module declared, and what that
    /// alias names cannot be worked out until the third module is here.
    /// Lowering a module deliberately does not ask about another module, so
    /// that the order they are visited in does not matter and an import cycle
    /// still resolves; this is the one step that needs the whole set, and it
    /// is the only way to build a world that has anything in it.
    pub fn of(modules: impl IntoIterator<Item = (String, Surface)>) -> Self {
        let mut world = World {
            modules: modules.into_iter().collect(),
        };
        world.expand_aliases();
        world
    }

    pub fn get(&self, module: &str, name: &str) -> Option<&SurfaceItem> {
        self.modules.get(module)?.get(name)
    }

    /// Every operator every module in this world bound, with the module that
    /// bound it.
    pub fn operators(&self) -> impl Iterator<Item = (&str, BinaryOp, &str)> {
        self.modules.iter().flat_map(|(path, surface)| {
            surface
                .operators
                .iter()
                .map(move |(op, function)| (path.as_str(), *op, function.as_str()))
        })
    }

    /// Replaces every mention of a transparent alias with what it names.
    ///
    /// A module lowering its own signatures expands the aliases it declared,
    /// because it can see them. It cannot expand one it imported, so an
    /// exported signature written with a third module's alias crossed as a
    /// bare name and the far side had nothing to compare it against. That is
    /// the same failure the alias expansion was written to fix, one module
    /// further out: `size(counts)` was refused as a mismatch between a type
    /// and its own definition, where `size` took the alias written out and
    /// `counts` came back from a function that named it.
    ///
    /// Refinements are not aliases here and do not move. A predicate makes a
    /// distinct type wherever it is read from.
    fn expand_aliases(&mut self) {
        let aliases: BTreeMap<(&str, &str), &Ty> = self
            .modules
            .iter()
            .flat_map(|(path, surface)| {
                surface
                    .items
                    .iter()
                    .filter_map(move |(name, item)| match item {
                        SurfaceItem::Alias { target, .. } => {
                            Some(((path.as_str(), name.as_str()), target))
                        }
                        _ => None,
                    })
            })
            .collect();

        if aliases.is_empty() {
            return;
        }

        let expanded: BTreeMap<String, Surface> = self
            .modules
            .iter()
            .map(|(path, surface)| {
                let items = surface
                    .items
                    .iter()
                    .map(|(name, item)| (name.clone(), expand_item(item, &aliases)))
                    .collect();
                (
                    path.clone(),
                    Surface {
                        items,
                        operators: surface.operators.clone(),
                    },
                )
            })
            .collect();

        self.modules = expanded;
    }
}

/// One exported declaration with the aliases in its types written out.
fn expand_item(item: &SurfaceItem, aliases: &BTreeMap<(&str, &str), &Ty>) -> SurfaceItem {
    let one = |ty: &Ty| expand_ty(ty, aliases, &mut Vec::new());
    let named = |fields: &Vec<(String, Ty)>| {
        fields
            .iter()
            .map(|(name, ty)| (name.clone(), one(ty)))
            .collect()
    };

    match item {
        SurfaceItem::Function {
            params,
            param_spans,
            ret,
            generics,
            row,
            guarantee,
            requires,
            declared,
        } => SurfaceItem::Function {
            params: params.iter().map(one).collect(),
            param_spans: param_spans.clone(),
            ret: one(ret),
            generics: generics.clone(),
            row: row.clone(),
            guarantee: guarantee.clone(),
            requires: requires.clone(),
            declared: *declared,
        },
        SurfaceItem::Record {
            fields,
            field_spans,
            generics,
            declared,
        } => SurfaceItem::Record {
            fields: named(fields),
            field_spans: field_spans.clone(),
            generics: generics.clone(),
            declared: *declared,
        },
        SurfaceItem::Choice { variants, generics } => SurfaceItem::Choice {
            variants: variants
                .iter()
                .map(|variant| SurfaceVariant {
                    name: variant.name.clone(),
                    fields: variant.fields.as_ref().map(named),
                })
                .collect(),
            generics: generics.clone(),
        },
        SurfaceItem::Variant {
            choice,
            fields,
            field_spans,
            generics,
            declared,
        } => SurfaceItem::Variant {
            choice: Rc::clone(choice),
            fields: fields.as_ref().map(named),
            field_spans: field_spans.clone(),
            generics: generics.clone(),
            declared: *declared,
        },
        SurfaceItem::Refinement { base } => SurfaceItem::Refinement { base: one(base) },
        SurfaceItem::Alias { target, generics } => SurfaceItem::Alias {
            target: one(target),
            generics: generics.clone(),
        },
        SurfaceItem::Effect {
            operations,
            declared,
        } => SurfaceItem::Effect {
            operations: operations
                .iter()
                .map(|(name, (params, ret))| {
                    (name.clone(), (params.iter().map(one).collect(), one(ret)))
                })
                .collect(),
            declared: *declared,
        },
        SurfaceItem::Handler {
            state,
            state_spans,
            declared,
        } => SurfaceItem::Handler {
            state: named(state),
            state_spans: state_spans.clone(),
            declared: *declared,
        },
    }
}

/// One type with the aliases in it written out.
///
/// `open` is which aliases are being written out right now. A chain of them
/// can lead back to where it started, and this runs over whatever files it was
/// handed rather than over files something else already accepted, so it cannot
/// rely on anybody having refused the cycle first.
fn expand_ty(
    ty: &Ty,
    aliases: &BTreeMap<(&str, &str), &Ty>,
    open: &mut Vec<(Rc<str>, Rc<str>)>,
) -> Ty {
    match ty {
        Ty::Result(ok, err) => Ty::Result(
            Box::new(expand_ty(ok, aliases, open)),
            Box::new(expand_ty(err, aliases, open)),
        ),
        Ty::List(element) => Ty::List(Box::new(expand_ty(element, aliases, open))),
        Ty::Fn { params, row, ret } => Ty::Fn {
            params: params
                .iter()
                .map(|param| expand_ty(param, aliases, open))
                .collect(),
            row: row.clone(),
            ret: Box::new(expand_ty(ret, aliases, open)),
        },
        Ty::Named { def, args } => Ty::Named {
            def: *def,
            args: args
                .iter()
                .map(|arg| expand_ty(arg, aliases, open))
                .collect(),
        },
        Ty::External { module, name, args } => {
            let args: Vec<Ty> = args
                .iter()
                .map(|arg| expand_ty(arg, aliases, open))
                .collect();
            let key = (Rc::clone(module), Rc::clone(name));
            let Some(target) = aliases.get(&(module.as_ref(), name.as_ref())) else {
                return Ty::External {
                    module: Rc::clone(module),
                    name: Rc::clone(name),
                    args,
                };
            };
            if open.contains(&key) {
                return Ty::Unknown;
            }

            open.push(key);
            let target = expand_ty(target, aliases, open);
            open.pop();

            if args.is_empty() {
                return target;
            }
            target.substitute(&crate::ty::bindings_for(&args))
        }
        other => other.clone(),
    }
}

/// A function's `where` clauses, with every name in them resolved.
///
/// `None` when there is no clause, so a function without a contract costs
/// nothing.
///
/// The resolution happens here rather than at the call site because this is
/// the only place it can happen. On the far side of the boundary the clause's
/// spans are offsets into a file nobody has, and asking that file's resolution
/// about them would not merely fail: it would answer about whatever the
/// importing module happens to have written at the same byte offset.
fn requires_of(decl: &deed_ast::FnDecl, resolutions: &Resolutions) -> Option<Rc<SurfaceRequires>> {
    if decl.contract.requires.is_empty() {
        return None;
    }

    let length = resolutions.builtin("length");
    let params: Vec<Option<deed_resolve::DefId>> = decl
        .sig
        .params
        .iter()
        .map(|param| resolutions.resolution(param.name.span))
        .collect();

    let inside = |span: Span| {
        decl.contract
            .requires
            .iter()
            .any(|clause| span.start >= clause.span().start && span.end <= clause.span().end)
    };

    let mut names: Vec<(Span, ClauseName)> = resolutions
        .names()
        .filter(|(span, _)| inside(*span))
        .filter_map(|(span, def)| {
            if let Some(index) = params.iter().position(|param| *param == Some(def)) {
                return Some((span, ClauseName::Param(index)));
            }
            (Some(def) == length).then_some((span, ClauseName::Length))
        })
        .collect();
    // Sorted so that two runs of the compiler over the same file produce the
    // same surface. The resolution table is a hash map and iterating it is not
    // ordered.
    names.sort_by_key(|(span, _)| *span);

    Some(Rc::new(SurfaceRequires {
        clauses: decl.contract.requires.clone(),
        arity: decl.sig.params.len(),
        names,
    }))
}

/// Lowers one module's declarations into something other modules can read.
pub fn surface(file: FileId, module: &Module, resolutions: &Resolutions) -> Surface {
    let Some(path) = module.name.as_ref().map(|name| name.to_string_path()) else {
        // A file with no `module` line cannot be imported, so its surface is
        // nobody's business.
        return Surface::default();
    };

    let mut lowerer = Lowerer {
        here: Rc::from(path.as_str()),
        resolutions,
        rows: RowLowering::of(module),
        type_params: RefCell::new(BTreeMap::new()),
        aliases: BTreeMap::new(),
        expanding: RefCell::new(Vec::new()),
    };

    // Transparent aliases, so a signature written with one crosses as what it
    // names. An alias is a name for a type rather than a type, and a boundary
    // does not make it one: leaving `Table<K, V>` in an exported signature
    // sent the far side a head it had nothing to compare against, and every
    // call through it was a mismatch between a type and its own definition.
    //
    // Refinements are not here on purpose. A predicate makes a distinct type
    // and it stays nominal across the boundary for the same reason it does at
    // home.
    for item in &module.items {
        if let Item::TypeAlias(decl) = item
            && decl.refinement.is_none()
            && let Some(def) = resolutions.resolution(decl.name.span)
        {
            lowerer
                .aliases
                .insert(def, (&decl.ty, positions(&decl.generics, resolutions)));
        }
    }

    // Refinement predicates, by the name they are declared under. Needed here
    // and nowhere else: a function returning `Positive` promises a positive
    // number, and nobody on the far side of the boundary can look the
    // predicate up, so the bounds have to be worked out on this side.
    let mut predicates: BTreeMap<&str, &Expr> = BTreeMap::new();
    for item in &module.items {
        if let Item::TypeAlias(decl) = item
            && let Some(predicate) = &decl.refinement
        {
            predicates.insert(decl.name.name.as_str(), predicate);
        }
    }

    let mut items = BTreeMap::new();
    // A handler's state is typed by the effect it implements, row variables
    // and all, so lowering it needs to know which names those are.
    let effect_rows: BTreeMap<&str, &[Ident]> = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Effect(decl) => Some((decl.name.name.as_str(), decl.rows.as_slice())),
            _ => None,
        })
        .collect();
    for item in &module.items {
        match item {
            Item::Deprecate(_) | Item::Operator(_) => {}
            Item::Function(decl) => {
                // What this module's own checker calls them, so an imported
                // generic function arrives with its parameters still in it and
                // the call site does the same substitution it does at home.
                *lowerer.type_params.borrow_mut() = positions(&decl.sig.generics, resolutions);
                lowerer.rows.declaring(&decl.sig.rows);

                let declared = match &decl.sig.ret {
                    Some(Type::Named { name, .. }) => predicates
                        .get(name.name.as_str())
                        .map(|predicate| facts::range_admitted_by(predicate))
                        .unwrap_or(Range::ANY),
                    _ => Range::ANY,
                };
                let names: Vec<&str> = decl
                    .sig
                    .params
                    .iter()
                    .map(|param| param.name.name.as_str())
                    .collect();
                let promised = decl
                    .contract
                    .ensures
                    .iter()
                    .filter(|clause| clause.outcome == Outcome::Ok)
                    .fold(Guarantee::any(), |promise, clause| {
                        promise.meet(facts::promised_by(&clause.condition, "result", &names))
                    });

                items.insert(
                    decl.sig.name.name.clone(),
                    SurfaceItem::Function {
                        params: decl
                            .sig
                            .params
                            .iter()
                            .map(|param| match &param.ty {
                                Some(ty) => lowerer.ty(ty),
                                None => Ty::Unknown,
                            })
                            .collect(),
                        param_spans: decl.sig.params.iter().map(|param| param.span).collect(),
                        ret: match &decl.sig.ret {
                            Some(ty) => lowerer.ty(ty),
                            None => Ty::Unit,
                        },
                        row: FnRow::Declared(lowerer.rows.normalised(&decl.contract.uses)),
                        generics: decl
                            .sig
                            .generics
                            .iter()
                            .map(|parameter| parameter.name.clone())
                            .collect(),
                        guarantee: Guarantee::of(declared).meet(promised),
                        requires: requires_of(decl, resolutions),
                        declared: Declared {
                            file,
                            span: decl.sig.span,
                        },
                    },
                );
            }
            Item::Record(decl) => {
                *lowerer.type_params.borrow_mut() = positions(&decl.generics, resolutions);
                items.insert(
                    decl.name.name.clone(),
                    SurfaceItem::Record {
                        fields: lowerer.fields(&decl.fields),
                        field_spans: decl.fields.iter().map(|field| field.span).collect(),
                        generics: named(&decl.generics),
                        declared: Declared {
                            file,
                            span: decl.name.span,
                        },
                    },
                );
            }
            Item::Choice(decl) => {
                *lowerer.type_params.borrow_mut() = positions(&decl.generics, resolutions);
                let choice: Rc<str> = Rc::from(decl.name.name.as_str());
                let mut variants = Vec::new();
                for variant in &decl.variants {
                    let fields = variant.fields.as_ref().map(|f| lowerer.fields(f));
                    items.insert(
                        variant.name.name.clone(),
                        SurfaceItem::Variant {
                            choice: Rc::clone(&choice),
                            fields: fields.clone(),
                            field_spans: variant
                                .fields
                                .as_ref()
                                .map(|f| f.iter().map(|field| field.span).collect()),
                            generics: named(&decl.generics),
                            declared: Declared {
                                file,
                                span: variant.name.span,
                            },
                        },
                    );
                    variants.push(SurfaceVariant {
                        name: variant.name.name.clone(),
                        fields,
                    });
                }
                items.insert(
                    decl.name.name.clone(),
                    SurfaceItem::Choice {
                        variants,
                        generics: named(&decl.generics),
                    },
                );
            }
            Item::TypeAlias(decl) => {
                *lowerer.type_params.borrow_mut() = positions(&decl.generics, resolutions);
                let base = lowerer.ty(&decl.ty);
                items.insert(
                    decl.name.name.clone(),
                    match decl.refinement {
                        // A predicate makes it a distinct type, so it has to be
                        // nominal from outside as well as inside.
                        Some(_) => SurfaceItem::Refinement { base },
                        None => SurfaceItem::Alias {
                            target: base,
                            generics: named(&decl.generics),
                        },
                    },
                );
            }
            Item::Effect(decl) => {
                // The effect's own row variables, so `Fn() uses r -> ()`
                // crosses as a type with a variable in its row rather than
                // one naming an effect called `r` that the far side would go
                // looking for and not find.
                lowerer.rows.declaring(&decl.rows);
                let mut operations = BTreeMap::new();
                for op in &decl.operations {
                    operations.insert(
                        op.name.name.clone(),
                        (
                            op.params
                                .iter()
                                .map(|param| match &param.ty {
                                    Some(ty) => lowerer.ty(ty),
                                    None => Ty::Unknown,
                                })
                                .collect(),
                            match &op.ret {
                                Some(ty) => lowerer.ty(ty),
                                None => Ty::Unit,
                            },
                        ),
                    );
                }
                items.insert(
                    decl.name.name.clone(),
                    SurfaceItem::Effect {
                        operations,
                        declared: Declared {
                            file,
                            span: decl.name.span,
                        },
                    },
                );
            }
            Item::Handler(decl) => {
                lowerer.rows.declaring(
                    effect_rows
                        .get(decl.effect.name.as_str())
                        .copied()
                        .unwrap_or(&[]),
                );
                items.insert(
                    decl.name.name.clone(),
                    SurfaceItem::Handler {
                        state: lowerer.fields(&decl.state),
                        state_spans: decl.state.iter().map(|field| field.span).collect(),
                        declared: Declared {
                            file,
                            span: decl.name.span,
                        },
                    },
                );
            }
            Item::Test(_) => {}
        }
    }

    let operators = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Operator(decl) if decl.op.is_bindable() => {
                Some((decl.op, decl.function.name.clone()))
            }
            _ => None,
        })
        .collect();

    Surface { items, operators }
}

/// Where each of a declaration's type parameters sits in its list.
///
/// A position rather than a name is what makes a parameter portable: the far
/// side has no `DefId` to compare against and no need for one, because the
/// arguments it applies line up by order.
fn positions(
    generics: &[Ident],
    resolutions: &Resolutions,
) -> BTreeMap<deed_resolve::DefId, (usize, Rc<str>)> {
    generics
        .iter()
        .enumerate()
        .filter_map(|(index, parameter)| {
            let def = resolutions.resolution(parameter.span)?;
            Some((def, (index, Rc::from(parameter.name.as_str()))))
        })
        .collect()
}

fn named(generics: &[Ident]) -> Vec<String> {
    generics
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect()
}

/// Where each type parameter of one declaration sits, by the name it was
/// declared under.
type Params = BTreeMap<deed_resolve::DefId, (usize, Rc<str>)>;

struct Lowerer<'a> {
    here: Rc<str>,
    resolutions: &'a Resolutions,
    /// How a row written in this module reads from anywhere else. Shared with
    /// the exports table so the two cannot drift apart.
    rows: RowLowering,
    /// The type parameters of the declaration being lowered, and nothing else.
    /// Replaced per item, because they mean nothing outside the one that
    /// declared them, and swapped again while an alias is expanded so its
    /// target reads its own parameters rather than the caller's.
    type_params: RefCell<Params>,
    /// What each transparent alias in this module names, with its own
    /// parameters. Collected once, expanded wherever one is mentioned.
    aliases: BTreeMap<deed_resolve::DefId, (&'a Type, Params)>,
    /// Which aliases are being expanded right now.
    ///
    /// `type A = List<A>` is a cycle the checker reports at the declaration,
    /// but this runs whether or not the file checked, so it has to stop by
    /// itself rather than by trusting that somebody else already refused.
    expanding: RefCell<Vec<deed_resolve::DefId>>,
}

impl Lowerer<'_> {
    fn fields(&self, fields: &[FieldDecl]) -> Vec<(String, Ty)> {
        fields
            .iter()
            .map(|field| (field.name.name.clone(), self.ty(&field.ty)))
            .collect()
    }

    fn ty(&self, ty: &Type) -> Ty {
        match ty {
            Type::Unit(_) => Ty::Unit,
            Type::Error(_) => Ty::Unknown,
            Type::Fn {
                params, row, ret, ..
            } => Ty::Fn {
                params: params.iter().map(|param| self.ty(param)).collect(),
                row: FnRow::Declared(self.rows.normalised(row)),
                ret: Box::new(self.ty(ret)),
            },
            Type::Named { name, args, .. } => {
                let lowered: Vec<Ty> = args.iter().map(|arg| self.ty(arg)).collect();
                let Some(def) = self.resolutions.resolution(name.span) else {
                    return Ty::Unknown;
                };

                match self.resolutions.def(def).kind {
                    DefKind::Builtin => match name.name.as_str() {
                        "Int" => Ty::Int,
                        "String" => Ty::Str,
                        "Bool" => Ty::Bool,
                        "Result" if lowered.len() == 2 => {
                            Ty::Result(Box::new(lowered[0].clone()), Box::new(lowered[1].clone()))
                        }
                        "List" if lowered.len() == 1 => Ty::List(Box::new(lowered[0].clone())),
                        // A capability crossing a module boundary as a
                        // parameter type is the whole point of one, and there
                        // is exactly one `Console`, so it is named under the
                        // prelude rather than under whichever module happened
                        // to mention it.
                        "System" | "Console" | "Clock" | "Dir" | "Net" => {
                            self.external(&Rc::from(PRELUDE_MODULE), name, Vec::new())
                        }
                        _ => Ty::Unknown,
                    },
                    // Already from somewhere else, and it keeps that identity
                    // rather than picking up this module's path on the way
                    // through.
                    DefKind::Import => match self.resolutions.import_module(def) {
                        Some(module) => self.external(&Rc::from(module), name, lowered),
                        None => Ty::Unknown,
                    },
                    DefKind::Type => self.alias(def, lowered, name),
                    DefKind::Record | DefKind::Choice => {
                        self.external(&Rc::clone(&self.here), name, lowered)
                    }
                    // A type parameter of the function being lowered. It
                    // crosses as a position rather than as a name, which is
                    // all a call site on the far side needs to substitute it.
                    DefKind::TypeParam => match self.type_params.borrow().get(&def) {
                        Some((index, name)) => Ty::Param {
                            index: *index,
                            name: Rc::clone(name),
                        },
                        None => Ty::Unknown,
                    },
                    _ => Ty::Unknown,
                }
            }
        }
    }

    fn external(&self, module: &Rc<str>, name: &Ident, args: Vec<Ty>) -> Ty {
        Ty::External {
            module: Rc::clone(module),
            name: Rc::from(name.name.as_str()),
            args,
        }
    }

    /// What a transparent alias names, with its arguments put in.
    ///
    /// A refinement never gets here: it is not in the table, so it falls
    /// through to the nominal answer, which is what it should cross as.
    fn alias(&self, def: deed_resolve::DefId, args: Vec<Ty>, name: &Ident) -> Ty {
        let Some((target, params)) = self.aliases.get(&def) else {
            return self.external(&Rc::clone(&self.here), name, args);
        };
        if self.expanding.borrow().contains(&def) {
            return Ty::Unknown;
        }

        self.expanding.borrow_mut().push(def);
        // The alias's own parameters, not the ones belonging to whatever
        // declaration mentioned it.
        let outer = std::mem::replace(&mut *self.type_params.borrow_mut(), params.clone());
        let expanded = self.ty(target);
        *self.type_params.borrow_mut() = outer;
        self.expanding.borrow_mut().pop();

        if args.is_empty() {
            return expanded;
        }
        let bindings: HashMap<usize, Ty> = args.into_iter().enumerate().collect();
        expanded.substitute(&bindings)
    }
}
