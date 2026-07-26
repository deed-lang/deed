//! What one module's declarations look like from another module.
//!
//! The type checker works in one module at a time and every `DefId` it sees is
//! an index into that module's resolution table. Nothing in that table can name
//! anything outside it, which is why an imported name used to have no type at
//! all.
//!
//! A surface is the same declarations lowered so they can be read from
//! anywhere: every type in it is either primitive or a [`Ty::External`], which
//! is identified by a module path and a name rather than by an index. Nothing
//! here needs another module's surface to have been built first, so the order
//! modules are visited in does not matter and an import cycle still resolves.
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

use std::collections::BTreeMap;
use std::rc::Rc;

use vow_ast::{Expr, FieldDecl, Ident, Item, Module, Outcome, Type};
use vow_resolve::{DefKind, Resolutions};

use crate::facts::{self, Guarantee, Range};
use crate::ty::Ty;

/// The module path builtin types are named under.
///
/// Defined by the resolver, which is where the prelude is, and re-exported
/// here because this is the file that explains why a module path is the
/// identity of a type at all.
pub use vow_resolve::PRELUDE_MODULE;

/// One exported declaration, with its types readable from outside.
#[derive(Clone, Debug)]
pub enum SurfaceItem {
    Function {
        params: Vec<Ty>,
        ret: Ty,
        /// What a call is promised to hand back. See the note at the top about
        /// why the bounds cross and the predicate does not.
        guarantee: Guarantee,
    },
    Record {
        fields: Vec<(String, Ty)>,
    },
    Choice {
        variants: Vec<SurfaceVariant>,
    },
    /// A variant, exported in its own right, remembering what it constructs.
    Variant {
        choice: Rc<str>,
        fields: Option<Vec<(String, Ty)>>,
    },
    /// A refinement, opaque from outside. See the note at the top of the file.
    Refinement {
        base: Ty,
    },
    /// A transparent alias, which is just its target.
    Alias {
        target: Ty,
    },
    Effect {
        operations: BTreeMap<String, (Vec<Ty>, Ty)>,
    },
    /// A handler, and the state a `with` block has to initialise.
    ///
    /// The state crosses because installing a handler from another module is
    /// still writing a literal, and a literal nobody checks is a literal that
    /// can put a `String` where an `Int` was declared.
    Handler {
        state: Vec<(String, Ty)>,
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, path: impl Into<String>, surface: Surface) {
        self.modules.insert(path.into(), surface);
    }

    pub fn get(&self, module: &str, name: &str) -> Option<&SurfaceItem> {
        self.modules.get(module)?.get(name)
    }
}

/// Lowers one module's declarations into something other modules can read.
pub fn surface(module: &Module, resolutions: &Resolutions) -> Surface {
    let Some(path) = module.name.as_ref().map(|name| name.to_string_path()) else {
        // A file with no `module` line cannot be imported, so its surface is
        // nobody's business.
        return Surface::default();
    };

    let lowerer = Lowerer {
        here: Rc::from(path.as_str()),
        resolutions,
    };

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
    for item in &module.items {
        match item {
            Item::Function(decl) => {
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
                        ret: match &decl.sig.ret {
                            Some(ty) => lowerer.ty(ty),
                            None => Ty::Unit,
                        },
                        guarantee: Guarantee::of(declared).meet(promised),
                    },
                );
            }
            Item::Record(decl) => {
                items.insert(
                    decl.name.name.clone(),
                    SurfaceItem::Record {
                        fields: lowerer.fields(&decl.fields),
                    },
                );
            }
            Item::Choice(decl) => {
                let choice: Rc<str> = Rc::from(decl.name.name.as_str());
                let mut variants = Vec::new();
                for variant in &decl.variants {
                    let fields = variant.fields.as_ref().map(|f| lowerer.fields(f));
                    items.insert(
                        variant.name.name.clone(),
                        SurfaceItem::Variant {
                            choice: Rc::clone(&choice),
                            fields: fields.clone(),
                        },
                    );
                    variants.push(SurfaceVariant {
                        name: variant.name.name.clone(),
                        fields,
                    });
                }
                items.insert(decl.name.name.clone(), SurfaceItem::Choice { variants });
            }
            Item::TypeAlias(decl) => {
                let base = lowerer.ty(&decl.ty);
                items.insert(
                    decl.name.name.clone(),
                    match decl.refinement {
                        // A predicate makes it a distinct type, so it has to be
                        // nominal from outside as well as inside.
                        Some(_) => SurfaceItem::Refinement { base },
                        None => SurfaceItem::Alias { target: base },
                    },
                );
            }
            Item::Effect(decl) => {
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
                items.insert(decl.name.name.clone(), SurfaceItem::Effect { operations });
            }
            Item::Handler(decl) => {
                items.insert(
                    decl.name.name.clone(),
                    SurfaceItem::Handler {
                        state: lowerer.fields(&decl.state),
                    },
                );
            }
            Item::Test(_) => {}
        }
    }

    Surface { items }
}

struct Lowerer<'a> {
    here: Rc<str>,
    resolutions: &'a Resolutions,
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
            Type::Fn { params, ret, .. } => Ty::Fn {
                params: params.iter().map(|param| self.ty(param)).collect(),
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
                        "System" | "Console" | "Clock" | "Dir" => {
                            self.external(&Rc::from(PRELUDE_MODULE), name)
                        }
                        _ => Ty::Unknown,
                    },
                    // Already from somewhere else, and it keeps that identity
                    // rather than picking up this module's path on the way
                    // through.
                    DefKind::Import => match self.resolutions.import_module(def) {
                        Some(module) => self.external(&Rc::from(module), name),
                        None => Ty::Unknown,
                    },
                    DefKind::Type | DefKind::Record | DefKind::Choice => {
                        self.external(&Rc::clone(&self.here), name)
                    }
                    _ => Ty::Unknown,
                }
            }
        }
    }

    fn external(&self, module: &Rc<str>, name: &Ident) -> Ty {
        Ty::External {
            module: Rc::clone(module),
            name: Rc::from(name.name.as_str()),
        }
    }
}
