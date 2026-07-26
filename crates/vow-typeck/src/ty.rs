//! Types, and the table the checker fills in.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use vow_diagnostics::Span;
use vow_resolve::DefId;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Ty {
    /// Nothing useful is known.
    ///
    /// Two things produce this, and keeping them as one type is deliberate.
    /// The first is a name that came from a module the compiler has not
    /// loaded, where saying anything would be a guess. The second is an
    /// expression that already produced a diagnostic, where saying anything
    /// more would be a second complaint about one mistake.
    ///
    /// `Unknown` is compatible with everything, in both directions. That makes
    /// the checker useful before the language is finished, and it means the
    /// number of real checks grows as more of the language lands rather than
    /// the number of false ones.
    Unknown,
    /// The type of an expression that does not return, such as a block ending
    /// in `return`. Compatible with everything, for the opposite reason.
    Never,
    Unit,
    Int,
    Str,
    Bool,
    /// A record, a choice, or a refinement declared in this module.
    Named(DefId),
    /// The same, declared in another module.
    ///
    /// A `DefId` is an index into one module's resolution table, so it cannot
    /// name anything outside it. Identity here is the module path and the name
    /// together, which needs no shared numbering and so does not make one
    /// module's types depend on how another module happened to be resolved.
    External {
        module: Rc<str>,
        name: Rc<str>,
    },
    /// `Result<T, E>`, which the language provides.
    ///
    /// Not a `Named` type over a builtin definition, because the arguments have
    /// to be compared componentwise and an unknown on either side has to
    /// absorb. That is what lets `ok(x)` produce `Result<T, unknown>` and still
    /// fit where a `Result<T, E>` was wanted, with no unification anywhere.
    Result(Box<Ty>, Box<Ty>),
    /// `List<T>`, which the language provides.
    ///
    /// Built in for the same reason `Result` is, and compared the same way:
    /// componentwise, with an unknown element type absorbing. That is what
    /// lets `[]` be a `List<unknown>` and still fit where a `List<Int>` was
    /// wanted, with no unification anywhere.
    List(Box<Ty>),
    Fn {
        params: Vec<Ty>,
        ret: Box<Ty>,
    },
}

impl Ty {
    /// Whether this type agrees with anything, because nothing is known about
    /// it or because control never gets here.
    pub fn absorbs(&self) -> bool {
        matches!(self, Ty::Unknown | Ty::Never)
    }
}

#[derive(Clone, Debug)]
pub struct FieldTy {
    pub name: String,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct VariantTy {
    pub def: DefId,
    pub name: String,
    /// `None` for a variant with no payload.
    pub fields: Option<Vec<FieldTy>>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Nominal {
    Record {
        fields: Vec<FieldTy>,
    },
    Choice {
        variants: Vec<VariantTy>,
    },
    /// `type Positive = Int where value > 0`
    ///
    /// A refinement is a distinct type rather than an alias. An alias with no
    /// predicate adds nothing and is expanded away; one with a predicate
    /// carries a proof obligation, and an obligation attached to a transparent
    /// alias would mean nothing at all.
    Refinement {
        base: Ty,
        predicate: Span,
    },
    /// A handler, and the state a `with` block has to initialise.
    ///
    /// A handler is not a value anyone can do anything with, but the literal
    /// that installs one is checked like a record's, and a type with no fields
    /// recorded is a literal nobody checks. `with InMemory { count: "hello" }`
    /// used to be accepted.
    Handler {
        state: Vec<FieldTy>,
    },
}

/// How an obligation was discharged.
///
/// `design/02-syntax.md` promises that a contract never quietly degrades into a
/// runtime check, so every obligation records which tier it landed in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// Discharged statically.
    Proven,
    /// Exercised by property tests generated from the contract.
    Tested,
    /// Not provable here, so it becomes a check at the boundary.
    Guarded,
}

#[derive(Clone, Debug)]
pub struct Obligation {
    /// The expression that has to satisfy the refinement.
    pub span: Span,
    pub refinement: DefId,
    pub tier: Tier,
    /// Whether the value is the number inside the `ok` rather than the
    /// expression itself.
    ///
    /// A `Result` that came back from a call has nothing naming its payload,
    /// so the obligation lands on the whole expression and has to say that the
    /// value it is about is one level in. Without this the runtime check ran
    /// the predicate against the `Result`, which fails whatever is inside it.
    pub inside_ok: bool,
}

/// Everything the type checker worked out.
#[derive(Default)]
pub struct Types {
    nominals: HashMap<DefId, Nominal>,
    names: HashMap<DefId, String>,
    exprs: HashMap<Span, Ty>,
    obligations: Vec<Obligation>,
    pure_required: HashSet<Span>,
}

impl Types {
    pub(crate) fn set_nominal(&mut self, def: DefId, name: String, nominal: Nominal) {
        self.names.insert(def, name);
        self.nominals.insert(def, nominal);
    }

    pub(crate) fn set_name(&mut self, def: DefId, name: String) {
        self.names.insert(def, name);
    }

    pub(crate) fn record_expr(&mut self, span: Span, ty: Ty) {
        self.exprs.insert(span, ty);
    }

    pub(crate) fn push_obligation(&mut self, obligation: Obligation) {
        self.obligations.push(obligation);
    }

    /// Notes that the value at `span` has to perform no effects.
    ///
    /// A `Fn(Int) -> Int` promises that, and whether a value keeps the promise
    /// is a question about rows, which this pass has no answer for. Which
    /// values have to keep it is a question about types, which it does. So the
    /// question is recorded here and settled by the pass that can settle it.
    pub(crate) fn require_pure(&mut self, span: Span) {
        self.pure_required.insert(span);
    }

    pub fn pure_required(&self) -> &HashSet<Span> {
        &self.pure_required
    }

    pub fn nominal(&self, def: DefId) -> Option<&Nominal> {
        self.nominals.get(&def)
    }

    /// The type worked out for the expression at `span`.
    ///
    /// Best effort, and intended for tooling and tests. Spans are unique for
    /// distinct source ranges but a few nodes share one with a child, so this
    /// is not a general purpose index.
    pub fn type_of(&self, span: Span) -> Option<&Ty> {
        self.exprs.get(&span)
    }

    pub fn obligations(&self) -> &[Obligation] {
        &self.obligations
    }

    /// Every expression whose type the checker never worked out.
    ///
    /// In a file that checks cleanly this should be empty, and that is a real
    /// invariant rather than a nice-to-have. `Unknown` agrees with everything,
    /// so an expression that has one is an expression nothing done with it will
    /// be checked against. Three separate holes in this language have been
    /// exactly that: a type name in expression position, a function parameter
    /// with no type, and a handler operation's parameters. Each was found by
    /// accident. This is how the next one gets found on purpose.
    pub fn unknowns(&self) -> impl Iterator<Item = Span> {
        self.exprs
            .iter()
            .filter(|(_, ty)| matches!(ty, Ty::Unknown))
            .map(|(span, _)| *span)
    }

    pub fn obligations_at(&self, tier: Tier) -> usize {
        self.obligations.iter().filter(|o| o.tier == tier).count()
    }

    /// A name for a type, suitable for a diagnostic.
    pub fn describe(&self, ty: &Ty) -> String {
        match ty {
            Ty::Unknown => "an unknown type".to_string(),
            Ty::Never => "no value".to_string(),
            Ty::Unit => "`()`".to_string(),
            Ty::Int => "`Int`".to_string(),
            Ty::Str => "`String`".to_string(),
            Ty::Bool => "`Bool`".to_string(),
            Ty::Named(def) => match self.names.get(def) {
                Some(name) => format!("`{name}`"),
                None => "an unnamed type".to_string(),
            },
            Ty::External { module, name } => {
                // A capability is not "from" anywhere a reader could go and
                // look, so saying so would be noise in every message about one.
                if &**module == "<prelude>" {
                    format!("`{name}`")
                } else {
                    format!("`{name}` from `{module}`")
                }
            }
            Ty::Result(ok, err) => format!("`Result<{}, {}>`", self.bare(ok), self.bare(err)),
            Ty::List(element) => format!("`List<{}>`", self.bare(element)),
            Ty::Fn { params, ret } => {
                let params: Vec<String> = params.iter().map(|p| self.describe(p)).collect();
                format!(
                    "a function of {} returning {}",
                    params.len(),
                    self.describe(ret)
                )
            }
        }
    }

    pub fn name_of(&self, def: DefId) -> &str {
        self.names.get(&def).map(String::as_str).unwrap_or("?")
    }

    /// A type name without the surrounding backticks, for nesting.
    fn bare(&self, ty: &Ty) -> String {
        match ty {
            Ty::Unknown => "_".to_string(),
            Ty::Never => "!".to_string(),
            Ty::Unit => "()".to_string(),
            Ty::Int => "Int".to_string(),
            Ty::Str => "String".to_string(),
            Ty::Bool => "Bool".to_string(),
            Ty::Named(def) => self.name_of(*def).to_string(),
            Ty::External { name, .. } => name.to_string(),
            Ty::Result(ok, err) => format!("Result<{}, {}>", self.bare(ok), self.bare(err)),
            Ty::List(element) => format!("List<{}>", self.bare(element)),
            Ty::Fn { params, ret } => {
                format!("Fn({}) -> {}", params.len(), self.bare(ret))
            }
        }
    }
}
