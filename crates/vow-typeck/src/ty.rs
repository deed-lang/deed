//! Types, and the table the checker fills in.

use std::collections::HashMap;

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
    /// `Result<T, E>`, which the language provides.
    ///
    /// Not a `Named` type over a builtin definition, because the arguments have
    /// to be compared componentwise and an unknown on either side has to
    /// absorb. That is what lets `ok(x)` produce `Result<T, unknown>` and still
    /// fit where a `Result<T, E>` was wanted, with no unification anywhere.
    Result(Box<Ty>, Box<Ty>),
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
}

/// How an obligation was discharged.
///
/// `design/02-syntax.md` promises that a contract never quietly degrades into a
/// runtime check, so every obligation records which tier it landed in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// Discharged statically.
    Proven,
    /// Not provable here, so it becomes a check at the boundary.
    Guarded,
}

#[derive(Clone, Debug)]
pub struct Obligation {
    /// The expression that has to satisfy the refinement.
    pub span: Span,
    pub refinement: DefId,
    pub tier: Tier,
}

/// Everything the type checker worked out.
#[derive(Default)]
pub struct Types {
    nominals: HashMap<DefId, Nominal>,
    names: HashMap<DefId, String>,
    exprs: HashMap<Span, Ty>,
    obligations: Vec<Obligation>,
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
            Ty::Result(ok, err) => format!("`Result<{}, {}>`", self.bare(ok), self.bare(err)),
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
            Ty::Result(ok, err) => format!("Result<{}, {}>", self.bare(ok), self.bare(err)),
            Ty::Fn { params, ret } => {
                format!("Fn({}) -> {}", params.len(), self.bare(ret))
            }
        }
    }
}
