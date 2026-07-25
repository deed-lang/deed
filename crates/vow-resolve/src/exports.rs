//! What one module lets another see.
//!
//! A module is named by its own `module` declaration, not by where it sits on
//! disk. That makes the unit of compilation the set of files you handed the
//! compiler, and it means `use payments/ledger` is answered by looking for the
//! file that says `module payments/ledger` rather than by a path lookup with
//! its own rules about roots, extensions and case.
//!
//! Exports are computed from the syntax alone. Nothing here needs another
//! module to have been resolved first, which is what keeps the whole thing a
//! single pass and, as it turns out, makes import cycles a non-problem.

use std::collections::BTreeMap;

use vow_ast::{Item, Module};

/// What kind of thing a name refers to on the other side of an import.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExportKind {
    Type,
    Record,
    Choice,
    /// A variant of a choice, exported in its own right.
    Variant,
    Effect,
    Handler,
    Function,
}

impl ExportKind {
    pub fn describe(self) -> &'static str {
        match self {
            ExportKind::Type => "a type",
            ExportKind::Record => "a record",
            ExportKind::Choice => "a choice",
            ExportKind::Variant => "a variant",
            ExportKind::Effect => "an effect",
            ExportKind::Handler => "a handler",
            ExportKind::Function => "a function",
        }
    }
}

/// One exported name.
#[derive(Clone, Debug)]
pub struct Export {
    pub kind: ExportKind,
    /// The variants of a choice, the operations of an effect, or the one
    /// effect a handler implements.
    ///
    /// Empty for everything else. This is what lets `Ledger.post` be checked
    /// when `Ledger` came from another file, and what lets a `with` block
    /// naming an imported handler discharge that handler's effect and no more.
    pub members: Vec<String>,
}

/// Everything one module offers.
///
/// There is no visibility modifier in Vow, so every item is exported. That is
/// a decision rather than an omission: a language with no wildcard imports
/// already makes the reader of a file see every name it pulled in, and adding
/// `pub` on top of that buys a second, weaker version of the same guarantee.
#[derive(Clone, Debug, Default)]
pub struct Exports {
    names: BTreeMap<String, Export>,
}

impl Exports {
    pub fn of(module: &Module) -> Self {
        let mut names = BTreeMap::new();

        for item in &module.items {
            // A choice's variants are usable unqualified inside the module
            // that declares them, which is what makes `err(NotFound { .. })`
            // read the way it does. They are exported in their own right so
            // that reads the same from outside, and importing one has to be
            // written down, because a `use` that quietly brought in six more
            // names would be the wildcard import this language does not have.
            if let Item::Choice(decl) = item {
                for variant in &decl.variants {
                    names.insert(
                        variant.name.name.clone(),
                        Export {
                            kind: ExportKind::Variant,
                            members: Vec::new(),
                        },
                    );
                }
            }

            let (name, kind, members) = match item {
                Item::TypeAlias(decl) => (&decl.name, ExportKind::Type, Vec::new()),
                Item::Record(decl) => (&decl.name, ExportKind::Record, Vec::new()),
                Item::Choice(decl) => (
                    &decl.name,
                    ExportKind::Choice,
                    decl.variants.iter().map(|v| v.name.name.clone()).collect(),
                ),
                Item::Effect(decl) => (
                    &decl.name,
                    ExportKind::Effect,
                    decl.operations
                        .iter()
                        .map(|o| o.name.name.clone())
                        .collect(),
                ),
                Item::Handler(decl) => (
                    &decl.name,
                    ExportKind::Handler,
                    vec![decl.effect.name.clone()],
                ),
                Item::Function(decl) => (&decl.sig.name, ExportKind::Function, Vec::new()),
                // A `test` is not part of the surface, and an error node is
                // not part of anything.
                Item::Test(_) | Item::Error(_) => continue,
            };

            names.insert(name.name.clone(), Export { kind, members });
        }

        Self { names }
    }

    pub fn get(&self, name: &str) -> Option<&Export> {
        self.names.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.keys().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Every module being compiled together.
///
/// Empty means a single file was handed to the compiler on its own, and an
/// import in that file has nowhere to point. Reporting that is better than the
/// alternative, which is what the compiler used to do: accept the import,
/// give the name no type, and let `Unknown` absorb every mistake made with it.
#[derive(Clone, Debug, Default)]
pub struct Universe {
    modules: BTreeMap<String, Exports>,
}

impl Universe {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a module. Returns the path it was already known by, if any.
    pub fn insert(&mut self, path: impl Into<String>, exports: Exports) -> Option<Exports> {
        self.modules.insert(path.into(), exports)
    }

    /// Adds a module under the name it declares for itself.
    ///
    /// A file with no `module` line exports nothing to anyone, since there is
    /// no name to import it by.
    pub fn add(&mut self, module: &Module) -> Option<Exports> {
        let path = module.name.as_ref()?.to_string_path();
        self.insert(path, Exports::of(module))
    }

    pub fn get(&self, path: &str) -> Option<&Exports> {
        self.modules.get(path)
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.modules.keys().map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }
}
