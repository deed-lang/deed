//! What names resolve to, and the table that records it.

use std::collections::HashMap;

use vow_diagnostics::Span;

use crate::exports::Export;

/// Handle to a declaration.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DefId(u32);

impl DefId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// Fabricates an id with no declaration behind it.
    ///
    /// For a table of facts about names that have none. `facts::promised_by`
    /// reads an `ensures` clause by inventing an id for `result` and one per
    /// parameter, so the narrowing a body gets can be run over a contract.
    ///
    /// An id made this way must not be handed to [`Resolutions::def`], which
    /// will panic or answer about the wrong thing. Nothing that holds one has
    /// a `Resolutions` to hand it to.
    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DefKind {
    /// Provided by the language, with no source location.
    Builtin,
    Type,
    Record,
    Choice,
    /// A variant of a `choice`, whose parent is that choice.
    Variant,
    Effect,
    /// An operation of an `effect`, whose parent is that effect.
    EffectOp,
    Handler,
    Function,
    Param,
    /// A type parameter of a generic function, such as the `T` in
    /// `fn first<T>(items: List<T>) -> Result<T, String>`.
    ///
    /// In scope for the whole declaration, which is the signature, the
    /// contract and the body, and nowhere else.
    TypeParam,
    /// A row variable of a generic function, such as the `r` in
    /// `fn map<A, B, uses r>(items: List<A>, step: Fn(A) uses r -> B)`.
    ///
    /// Stands for whatever the callback it is attached to performs, so that
    /// the function can pass that through to its own row rather than naming
    /// one effect and being useful for that effect only.
    RowParam,
    /// A `state` field of a handler. The only mutable thing in the language.
    State,
    /// A `let` binding, a pattern binding, or a closure parameter.
    Local,
    /// A name brought in by `use`. What it refers to is in another module,
    /// which the compiler cannot see yet.
    Import,
}

impl DefKind {
    pub fn describe(self) -> &'static str {
        match self {
            DefKind::Builtin => "builtin",
            DefKind::Type => "type",
            DefKind::Record => "record",
            DefKind::Choice => "choice",
            DefKind::Variant => "variant",
            DefKind::Effect => "effect",
            DefKind::EffectOp => "effect operation",
            DefKind::Handler => "handler",
            DefKind::Function => "function",
            DefKind::Param => "parameter",
            DefKind::TypeParam => "type parameter",
            DefKind::RowParam => "row variable",
            DefKind::State => "handler state",
            DefKind::Local => "binding",
            DefKind::Import => "import",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DefData {
    pub kind: DefKind,
    pub name: String,
    /// Where the declaration is written. Builtins have an empty span.
    pub span: Span,
    /// The choice a variant belongs to, or the effect an operation belongs to.
    pub parent: Option<DefId>,
}

/// How a `.` was classified.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dot {
    /// A field of a value. Which field is a question for the type checker.
    Field,
    /// A member of a module that has not been loaded, so nothing can be said
    /// about it yet. Not an error, just not knowable.
    Foreign,
}

/// The result of resolving one module.
///
/// References are keyed by the span of the identifier that mentions them.
/// Two distinct occurrences of a name cannot share a source range, so the span
/// is a sound key and the AST does not need node identities threaded through it
/// to support this pass.
#[derive(Default)]
pub struct Resolutions {
    defs: Vec<DefData>,
    names: HashMap<Span, DefId>,
    dots: HashMap<Span, Dot>,
    builtins: HashMap<String, DefId>,
    /// What each `use`d name turned out to be in the module it came from.
    ///
    /// Absent when the module was not among the files being compiled, which
    /// is already an error, so the rest of the pipeline treats it the way it
    /// used to treat every import.
    imports: HashMap<DefId, Export>,
    /// Which module each `use`d name came from.
    ///
    /// Recorded even when the module was missing, because a diagnostic about
    /// the name reads better with the module in it either way.
    import_modules: HashMap<DefId, String>,
}

impl Resolutions {
    pub(crate) fn add_def(&mut self, def: DefData) -> DefId {
        let id = DefId(self.defs.len() as u32);
        self.defs.push(def);
        id
    }

    pub(crate) fn record_name(&mut self, span: Span, def: DefId) {
        self.names.insert(span, def);
    }

    pub(crate) fn record_dot(&mut self, span: Span, dot: Dot) {
        self.dots.insert(span, dot);
    }

    pub(crate) fn record_builtin(&mut self, name: &str, def: DefId) {
        self.builtins.insert(name.to_string(), def);
    }

    pub(crate) fn record_export(&mut self, def: DefId, export: Export) {
        self.imports.insert(def, export);
    }

    pub(crate) fn record_import_module(&mut self, def: DefId, module: &str) {
        self.import_modules.insert(def, module.to_string());
    }

    /// What an imported name is, on the other side of the import.
    pub fn import(&self, def: DefId) -> Option<&Export> {
        self.imports.get(&def)
    }

    /// Which module an imported name came from.
    pub fn import_module(&self, def: DefId) -> Option<&str> {
        self.import_modules.get(&def).map(String::as_str)
    }

    /// A name the language provides, such as `Console` or the `Io` effect.
    ///
    /// Later passes need these by name, since nothing in the source declares
    /// them and there is no span to look them up by.
    pub fn builtin(&self, name: &str) -> Option<DefId> {
        self.builtins.get(name).copied()
    }

    /// # Panics
    ///
    /// Panics if the id came from a different resolution, which is a bug.
    pub fn def(&self, id: DefId) -> &DefData {
        &self.defs[id.index()]
    }

    pub fn defs(&self) -> impl Iterator<Item = (DefId, &DefData)> {
        self.defs
            .iter()
            .enumerate()
            .map(|(index, def)| (DefId(index as u32), def))
    }

    /// What the identifier at `span` refers to.
    pub fn resolution(&self, span: Span) -> Option<DefId> {
        self.names.get(&span).copied()
    }

    /// Every reference, as a pair of mention site and declaration.
    pub fn names(&self) -> impl Iterator<Item = (Span, DefId)> {
        self.names.iter().map(|(span, def)| (*span, *def))
    }

    pub fn dot(&self, span: Span) -> Option<Dot> {
        self.dots.get(&span).copied()
    }
}
