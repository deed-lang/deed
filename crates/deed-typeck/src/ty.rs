//! Types, and the table the checker fills in.

use std::collections::HashMap;
use std::rc::Rc;

use deed_diagnostics::Span;
use deed_resolve::{DefId, RowEntry};

/// What a function value is allowed to perform.
///
/// A row is part of a function type rather than something attached to it. Two
/// function types with different rows are different types, in the same way
/// that two with different parameters are, because the whole point of a row is
/// that a caller can read it off the signature.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FnRow {
    /// Written down, normalised so that two spellings of one row are one
    /// value.
    ///
    /// An empty list is a row rather than the absence of one: `Fn(Int) -> Int`
    /// promises to perform nothing at all.
    Declared(Vec<RowEntry>),
    /// A closure written on the spot, whose row this pass does not work out.
    ///
    /// Fits wherever it is put, and the effects pass settles whether it really
    /// did. The alternative is teaching this pass to walk a body looking for
    /// effects, which is the other pass in a worse place.
    Inferred,
}

impl FnRow {
    /// Whether a value with this row may be used where `expected` was wanted.
    ///
    /// Containment rather than equality, and this is the one place in the
    /// checker where a type fits another without being it. It gives way in the
    /// direction that is safe: a function that performs less than it was given
    /// room for breaks nothing, and one that performs more is the mistake the
    /// row was written to catch.
    pub fn within(&self, expected: &FnRow) -> bool {
        let (FnRow::Declared(actual), FnRow::Declared(expected)) = (self, expected) else {
            return true;
        };
        // A row variable stands for whatever the value performs, so a type
        // that carries one has made room for anything. Which row it turned out
        // to be, and whether the caller declared it, is settled by the pass
        // that knows about rows.
        if expected.iter().any(|entry| entry.variable) {
            return true;
        }
        actual.iter().all(|entry| {
            expected.iter().any(|allowed| {
                allowed == entry
                    // Naming a whole effect covers every operation on it, the
                    // same way naming one in a contract does.
                    || (allowed.operation.is_none()
                        && allowed.module == entry.module
                        && allowed.effect == entry.effect)
            })
        })
    }
}

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
    /// A record, a choice, or a refinement declared in this module, with
    /// whatever type arguments it was applied to.
    ///
    /// `args` is empty for the ordinary case. When it is not, two of these are
    /// the same type only when the head and every argument match, which is the
    /// same componentwise comparison `Result` and `List` already get.
    Named {
        def: DefId,
        args: Vec<Ty>,
    },
    /// The same, declared in another module.
    ///
    /// A `DefId` is an index into one module's resolution table, so it cannot
    /// name anything outside it. Identity here is the module path and the name
    /// together, which needs no shared numbering and so does not make one
    /// module's types depend on how another module happened to be resolved.
    External {
        module: Rc<str>,
        name: Rc<str>,
        args: Vec<Ty>,
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
    /// A type parameter of the generic function being checked, such as the `T`
    /// in `fn first<T>(items: List<T>) -> Result<T, String>`.
    ///
    /// Identified by where it sits in the declaration's list rather than by a
    /// `DefId`, for the same reason an imported type is identified by module
    /// path and name: a `DefId` is an index into one module's table, and a
    /// generic function is callable from another module. The name is carried
    /// so a diagnostic can say `T` rather than say nothing.
    ///
    /// It appears in exactly two places: inside the body of the function that
    /// declared it, where it is compared against itself and against nothing
    /// else, and inside that function's signature, where every call site
    /// substitutes it away.
    Param {
        index: usize,
        name: Rc<str>,
    },
    Fn {
        params: Vec<Ty>,
        /// What calling it is allowed to perform. See [`FnRow`].
        row: FnRow,
        ret: Box<Ty>,
    },
}

impl Ty {
    /// Whether this type agrees with anything, because nothing is known about
    /// it or because control never gets here.
    pub fn absorbs(&self) -> bool {
        matches!(self, Ty::Unknown | Ty::Never)
    }

    /// Whether a type parameter appears anywhere inside this type.
    ///
    /// What makes a signature generic, rather than a count kept beside it.
    /// Keeping the answer in the type means an imported generic function needs
    /// nothing extra to cross a module boundary: its parameters arrive with
    /// the parameters still in them and the call site does the same work it
    /// does at home.
    pub fn is_generic(&self) -> bool {
        match self {
            Ty::Param { .. } => true,
            Ty::Result(ok, err) => ok.is_generic() || err.is_generic(),
            Ty::List(element) => element.is_generic(),
            Ty::Named { args, .. } | Ty::External { args, .. } => args.iter().any(Ty::is_generic),
            Ty::Fn { params, ret, .. } => params.iter().any(Ty::is_generic) || ret.is_generic(),
            _ => false,
        }
    }

    /// Whether the type parameter at `index` appears anywhere inside.
    pub fn mentions(&self, index: usize) -> bool {
        match self {
            Ty::Param { index: found, .. } => *found == index,
            Ty::Result(ok, err) => ok.mentions(index) || err.mentions(index),
            Ty::List(element) => element.mentions(index),
            Ty::Named { args, .. } | Ty::External { args, .. } => {
                args.iter().any(|arg| arg.mentions(index))
            }
            Ty::Fn { params, ret, .. } => {
                params.iter().any(|param| param.mentions(index)) || ret.mentions(index)
            }
            _ => false,
        }
    }

    /// Works out what the type parameters have to be for `self` to describe
    /// `actual`, and writes the answers into `bindings`.
    ///
    /// A walk down two types in step, not a solver. The first answer for a
    /// parameter wins: `fn pair<T>(a: T, b: T)` called with two different
    /// types binds `T` from the first and then reports an ordinary mismatch on
    /// the second, which is a better message than anything a unifier would
    /// produce about a variable the caller never wrote.
    ///
    /// An actual type that absorbs binds nothing. It agrees with whatever the
    /// parameter turns out to be, so treating it as an answer would let one
    /// argument the checker gave up on decide the type of every other.
    pub fn bind(&self, actual: &Ty, bindings: &mut HashMap<usize, Ty>) {
        if actual.absorbs() {
            return;
        }
        match (self, actual) {
            (Ty::Param { index, .. }, _) => {
                bindings.entry(*index).or_insert_with(|| actual.clone());
            }
            (Ty::Result(ok, err), Ty::Result(a_ok, a_err)) => {
                ok.bind(a_ok, bindings);
                err.bind(a_err, bindings);
            }
            (Ty::List(element), Ty::List(actual)) => element.bind(actual, bindings),
            (
                Ty::Named { args, .. },
                Ty::Named {
                    args: actual_args, ..
                },
            )
            | (
                Ty::External { args, .. },
                Ty::External {
                    args: actual_args, ..
                },
            ) => {
                for (arg, actual) in args.iter().zip(actual_args) {
                    arg.bind(actual, bindings);
                }
            }
            (
                Ty::Fn { params, ret, .. },
                Ty::Fn {
                    params: a_params,
                    ret: a_ret,
                    ..
                },
            ) => {
                for (param, actual) in params.iter().zip(a_params) {
                    param.bind(actual, bindings);
                }
                ret.bind(a_ret, bindings);
            }
            _ => {}
        }
    }

    /// The same type with every parameter replaced by what it was bound to.
    ///
    /// A parameter with no binding becomes unknown rather than staying a
    /// parameter. It is unbound because nothing in the call said what it is,
    /// which is exactly the thing unknown means, and leaving it as a parameter
    /// would put one function's `T` into another function's types.
    pub fn substitute(&self, bindings: &HashMap<usize, Ty>) -> Ty {
        match self {
            Ty::Param { index, .. } => bindings.get(index).cloned().unwrap_or(Ty::Unknown),
            Ty::Result(ok, err) => Ty::Result(
                Box::new(ok.substitute(bindings)),
                Box::new(err.substitute(bindings)),
            ),
            Ty::List(element) => Ty::List(Box::new(element.substitute(bindings))),
            Ty::Named { def, args } => Ty::Named {
                def: *def,
                args: args.iter().map(|arg| arg.substitute(bindings)).collect(),
            },
            Ty::External { module, name, args } => Ty::External {
                module: Rc::clone(module),
                name: Rc::clone(name),
                args: args.iter().map(|arg| arg.substitute(bindings)).collect(),
            },
            Ty::Fn { params, row, ret } => Ty::Fn {
                params: params.iter().map(|p| p.substitute(bindings)).collect(),
                row: row.clone(),
                ret: Box::new(ret.substitute(bindings)),
            },
            other => other.clone(),
        }
    }
}

/// The bindings a type's arguments stand for, by position.
///
/// A declaration's parameters are numbered from zero in the order they were
/// written, so its arguments line up with them and nothing else is needed to
/// read a field type at the type it was applied to.
pub fn bindings_for(args: &[Ty]) -> HashMap<usize, Ty> {
    args.iter()
        .enumerate()
        .map(|(index, arg)| (index, arg.clone()))
        .collect()
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

/// A `where` clause, answered for where the call was written.
///
/// Separate from [`Obligation`] because there is no refinement behind it and
/// nothing new for the runtime to do: the callee already checks its own
/// preconditions on every call. What this records is how much the call site
/// settled, which is the difference between a contract that was read and one
/// that was only written down.
#[derive(Clone, Debug)]
pub struct Precondition {
    /// The call, not the clause. The clause is in the callee and the mistake
    /// is here.
    pub span: Span,
    pub callee: String,
    pub tier: Tier,
}

/// Everything the type checker worked out.
#[derive(Default)]
pub struct Types {
    nominals: HashMap<DefId, Nominal>,
    names: HashMap<DefId, String>,
    exprs: HashMap<Span, Ty>,
    obligations: Vec<Obligation>,
    preconditions: Vec<Precondition>,
    row_required: HashMap<Span, Vec<RowEntry>>,
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

    pub(crate) fn push_precondition(&mut self, precondition: Precondition) {
        self.preconditions.push(precondition);
    }

    /// Every `where` clause answered for at a call, in the order they were
    /// reached.
    pub fn preconditions(&self) -> &[Precondition] {
        &self.preconditions
    }

    /// Notes that the value at `span` may perform no more than `allowed`.
    ///
    /// A function type says what a value of it performs, and whether a value
    /// keeps that promise is a question about rows, which this pass has no
    /// answer for. Which values have to keep it, and which row each one owes,
    /// are questions about types, which it does. So the question is recorded
    /// here and settled by the pass that can settle it.
    ///
    /// Two expectations on one expression both have to hold, so what is kept
    /// is what they agree on rather than whichever arrived last.
    pub(crate) fn require_row(&mut self, span: Span, allowed: Vec<RowEntry>) {
        self.row_required
            .entry(span)
            .and_modify(|existing| existing.retain(|entry| allowed.contains(entry)))
            .or_insert(allowed);
    }

    pub fn row_required(&self) -> &HashMap<Span, Vec<RowEntry>> {
        &self.row_required
    }

    /// The row of every expression whose type is a function type that wrote
    /// one down.
    ///
    /// The other half of the same handoff as [`Types::row_required`], in the
    /// other direction. That one says what a value crossing into a function
    /// type is allowed to perform; this one says what calling a value performs,
    /// which is the caller's problem rather than the value's.
    ///
    /// It exists because a row is part of a type, so anything that can work out
    /// a type can work out a row, and the pass that works out types is this
    /// one. The effect checker used to derive this from the shape of the
    /// expression instead, which meant a function value arriving as the result
    /// of a call, a branch of an `if`, an element of a list or a field of a
    /// record performed nothing as far as it was concerned. An empty row is a
    /// claim, not an absence, so each of those was a claim nobody checked.
    pub fn function_rows(&self) -> HashMap<Span, Vec<RowEntry>> {
        self.exprs
            .iter()
            .filter_map(|(span, ty)| match ty {
                Ty::Fn {
                    row: FnRow::Declared(entries),
                    ..
                } if !entries.is_empty() => Some((*span, entries.clone())),
                _ => None,
            })
            .collect()
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

    /// The narrowest expression covering an offset, and what it turned out to
    /// be.
    ///
    /// Narrowest because an offset inside `f(n)` is inside the call and inside
    /// the argument, and the argument is the thing under the cursor. This is
    /// what a hover asks, and asking it by scanning is fine: the number of
    /// expressions in a file is small and the answer is wanted once per
    /// keystroke rather than once per node.
    pub fn at(&self, offset: u32) -> Option<(Span, &Ty)> {
        self.exprs
            .iter()
            .filter(|(span, _)| span.start <= offset && offset < span.end)
            .min_by_key(|(span, _)| (span.end - span.start, span.start))
            .map(|(span, ty)| (*span, ty))
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
            Ty::Named { .. } | Ty::External { .. } => {
                let written = self.bare(ty);
                match ty {
                    // A capability is not "from" anywhere a reader could go
                    // and look, so saying so would be noise in every message
                    // about one.
                    Ty::External { module, .. } if &**module != "<prelude>" => {
                        format!("`{written}` from `{module}`")
                    }
                    _ if written == "?" => "an unnamed type".to_string(),
                    _ => format!("`{written}`"),
                }
            }
            Ty::Result(ok, err) => format!("`Result<{}, {}>`", self.bare(ok), self.bare(err)),
            Ty::List(element) => format!("`List<{}>`", self.bare(element)),
            Ty::Param { name, .. } => format!("`{name}`, a type parameter"),
            Ty::Fn { .. } => format!("`{}`", self.bare(ty)),
        }
    }

    /// What can be written after a `.` on a value of this type.
    ///
    /// The fields of a record, with what each one is, and nothing for anything
    /// else. A capability has no readable fields except `System`'s, which is
    /// the root of all authority and is where every program starts.
    ///
    /// Best effort and for tooling, like [`Self::at`]. A type from another
    /// module has its fields in that module's surface rather than here, so
    /// this answers nothing for one, and answering nothing is better than
    /// answering the wrong module's fields.
    pub fn members_of(&self, ty: &Ty) -> Vec<(String, String)> {
        let looked_through = match ty {
            Ty::Named { def, .. } => match self.nominal(*def) {
                Some(Nominal::Refinement { base, .. }) => base.clone(),
                _ => ty.clone(),
            },
            other => other.clone(),
        };

        match &looked_through {
            Ty::Named { def, args } => match self.nominal(*def) {
                Some(Nominal::Record { fields }) => fields
                    .iter()
                    .map(|field| {
                        (
                            field.name.clone(),
                            self.bare(&field.ty.substitute(&bindings_for(args))),
                        )
                    })
                    .collect(),
                Some(Nominal::Handler { state }) => state
                    .iter()
                    .map(|field| (field.name.clone(), self.bare(&field.ty)))
                    .collect(),
                _ => Vec::new(),
            },
            // The one capability with anything inside it. `sys.console` is
            // where a program gets permission to print, and a reader of one
            // has to start there.
            Ty::External { module, name, .. }
                if &**module == "<prelude>" && &**name == "System" =>
            {
                vec![
                    ("console".to_string(), "Console".to_string()),
                    ("files".to_string(), "Dir".to_string()),
                    ("clock".to_string(), "Clock".to_string()),
                ]
            }
            _ => Vec::new(),
        }
    }

    pub fn name_of(&self, def: DefId) -> &str {
        self.names.get(&def).map(String::as_str).unwrap_or("?")
    }

    /// `Pair<Int, String>`, or just `Pair` when it was applied to nothing.
    fn applied(&self, name: &str, args: &[Ty]) -> String {
        if args.is_empty() {
            return name.to_string();
        }
        let written: Vec<String> = args.iter().map(|arg| self.bare(arg)).collect();
        format!("{name}<{}>", written.join(", "))
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
            Ty::Named { def, args } => self.applied(self.name_of(*def), args),
            Ty::External { name, args, .. } => self.applied(name, args),
            Ty::Result(ok, err) => format!("Result<{}, {}>", self.bare(ok), self.bare(err)),
            Ty::List(element) => format!("List<{}>", self.bare(element)),
            Ty::Param { name, .. } => name.to_string(),
            // Written the way the signature that declared it is written. The
            // arity alone used to be all this said, which reads as a riddle in
            // a message about two function types not matching, and reads as
            // nothing at all in a hover.
            Ty::Fn { params, row, ret } => {
                let params: Vec<String> = params.iter().map(|p| self.bare(p)).collect();
                let row = match row {
                    FnRow::Declared(entries) if !entries.is_empty() => {
                        let named: Vec<String> = entries
                            .iter()
                            .map(|entry| match &entry.operation {
                                Some(operation) => format!("{}.{operation}", entry.effect),
                                None => entry.effect.clone(),
                            })
                            .collect();
                        format!(" uses {}", named.join(", "))
                    }
                    // A closure's row is not this pass's to state, and
                    // inventing one for a hover would be inventing it
                    // everywhere.
                    FnRow::Declared(_) | FnRow::Inferred => String::new(),
                };
                format!("Fn({}){row} -> {}", params.join(", "), self.bare(ret))
            }
        }
    }
}
