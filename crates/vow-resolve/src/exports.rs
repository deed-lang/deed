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

use std::collections::{BTreeMap, BTreeSet};

use vow_ast::{Item, Module};

use crate::resolver::{PRELUDE_EFFECTS, PRELUDE_MODULE};

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

/// One entry of an exported function's effect row.
///
/// An entry cannot travel as a `DefId`, for the same reason a type could not:
/// a `DefId` is an index into one module's table and means nothing outside it.
/// It travels as the module the effect was declared in, its name there, and the
/// operation, which is the same identity the interpreter uses for an effect at
/// runtime.
///
/// The declaring module knows the path from its own syntax and nothing else:
/// either the effect is declared in it, or it is in its `use` list. That is
/// what keeps exports computable without resolving anything first.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct RowEntry {
    pub module: String,
    pub effect: String,
    /// `None` means every operation of the effect.
    pub operation: Option<String>,
    /// Whether this is a row variable rather than an effect.
    ///
    /// A variable stands for whatever a callback performs, so it names nothing
    /// on its own and a reader on the far side has to be told that rather than
    /// left to look for an effect that is not there.
    pub variable: bool,
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
    /// What a function declares it does, for the caller to inherit.
    ///
    /// Empty for everything that is not a function, and for a function that is
    /// pure. Without this a call into another module was free: the effect
    /// system, which is the pass this language exists for, stopped at the
    /// module boundary.
    pub row: Vec<RowEntry>,
    /// Whether that row is the whole story.
    ///
    /// False when the declaration contains something no caller could read,
    /// which today means `sys.*`. Exporting an empty row in that case would
    /// make every caller look pure, so the loophole would move rather than
    /// close.
    pub row_complete: bool,
    /// Which parameters' rows flow into this function's own.
    ///
    /// Positions rather than names, for the same reason a type parameter
    /// crosses as a position: a `DefId` means nothing outside the table it
    /// came from and a name would need one to be looked up. A caller charges
    /// itself with whatever it passed at each of these, which is what makes a
    /// row variable mean anything at a call site.
    pub row_from: Vec<usize>,
}

/// Which parameters carry a row variable that the declaration passes through.
///
/// Syntax alone, like everything else in this file. A parameter counts when
/// its type is a function type whose row names one of the declaration's row
/// variables, and the declaration's own row names it too. Both halves are
/// needed: a callback whose row goes nowhere is one the function may not call.
pub fn row_sources(sig: &vow_ast::FnSig, contract: &vow_ast::Contract) -> Vec<usize> {
    let passed: Vec<&str> = sig
        .rows
        .iter()
        .map(|variable| variable.name.as_str())
        .filter(|name| {
            contract
                .uses
                .iter()
                .any(|entry| entry.effect.name == *name && entry.operation.is_none())
        })
        .collect();

    sig.params
        .iter()
        .enumerate()
        .filter(|(_, param)| match &param.ty {
            Some(vow_ast::Type::Fn { row, .. }) => row
                .iter()
                .any(|entry| passed.contains(&entry.effect.name.as_str())),
            _ => false,
        })
        .map(|(index, _)| index)
        .collect()
}

/// Turns a `uses` clause into the portable form, from syntax alone.
///
/// Exports need this, and so does a row written inside a function type, and
/// two implementations that agree today are two implementations that stop
/// agreeing the first time one of them is edited. Nothing here is resolved:
/// either the effect is declared in this module or it is on a `use` line, and
/// both are visible in the syntax.
pub struct RowLowering {
    here: String,
    origins: BTreeMap<String, String>,
    /// The row variables of the declaration being lowered, and nothing else.
    /// Replaced per declaration, because one means nothing outside the
    /// signature that declared it.
    variables: BTreeSet<String>,
}

impl RowLowering {
    pub fn of(module: &Module) -> Self {
        let here = module
            .name
            .as_ref()
            .map(|name| name.to_string_path())
            .unwrap_or_default();

        let mut origins = BTreeMap::new();
        for entry in &module.uses {
            let path = entry.path.to_string_path();
            for name in &entry.names {
                origins.insert(name.name.clone(), path.clone());
            }
        }

        Self {
            here,
            origins,
            variables: BTreeSet::new(),
        }
    }

    /// Points this at one declaration's row variables, for the rows inside it.
    pub fn declaring(&mut self, variables: &[vow_ast::Ident]) {
        self.variables = variables
            .iter()
            .map(|variable| variable.name.clone())
            .collect();
    }

    /// The entries, and whether they are the whole story.
    ///
    /// The flag is false when the clause contains something no caller could
    /// read, which today means `sys.*`: it grants everything a capability
    /// carries and there is no name for "everything" on the far side.
    pub fn row(&self, uses: &[vow_ast::EffectRef]) -> (Vec<RowEntry>, bool) {
        let complete = uses.iter().all(|entry| !entry.all);
        let entries = uses
            .iter()
            .filter(|entry| !entry.all && !entry.effect.name.is_empty())
            .map(|entry| RowEntry {
                module: match self.origins.get(entry.effect.name.as_str()) {
                    _ if self.variables.contains(&entry.effect.name) => self.here.clone(),
                    Some(path) => path.clone(),
                    // The language provides it, so every module can name it
                    // and no import is involved anywhere.
                    None if PRELUDE_EFFECTS.contains(&entry.effect.name.as_str()) => {
                        PRELUDE_MODULE.to_string()
                    }
                    None => self.here.clone(),
                },
                effect: entry.effect.name.clone(),
                operation: entry.operation.as_ref().map(|op| op.name.clone()),
                variable: self.variables.contains(&entry.effect.name),
            })
            .collect();
        (entries, complete)
    }

    /// The same, sorted and deduplicated, for a row that has to be compared.
    ///
    /// A row inside a type is compared for equality and for containment, and
    /// two spellings of one row have to be one value or `Fn(Int) uses A, B ->
    /// Int` and `Fn(Int) uses B, A -> Int` would be different types.
    pub fn normalised(&self, uses: &[vow_ast::EffectRef]) -> Vec<RowEntry> {
        let (mut entries, _) = self.row(uses);
        entries.sort();
        entries.dedup();
        entries
    }
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

        // Where each imported name came from, so a row naming an imported
        // effect can say which module declared it. Straight off the `use`
        // lines, so nothing has to be resolved first.
        let mut rows = RowLowering::of(module);

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
                            row: Vec::new(),
                            row_complete: true,
                            row_from: Vec::new(),
                        },
                    );
                }
            }

            let (name, kind, members, row, row_complete, row_from) = match item {
                Item::TypeAlias(decl) => (
                    &decl.name,
                    ExportKind::Type,
                    Vec::new(),
                    Vec::new(),
                    true,
                    Vec::new(),
                ),
                Item::Record(decl) => (
                    &decl.name,
                    ExportKind::Record,
                    Vec::new(),
                    Vec::new(),
                    true,
                    Vec::new(),
                ),
                Item::Choice(decl) => (
                    &decl.name,
                    ExportKind::Choice,
                    decl.variants.iter().map(|v| v.name.name.clone()).collect(),
                    Vec::new(),
                    true,
                    Vec::new(),
                ),
                Item::Effect(decl) => (
                    &decl.name,
                    ExportKind::Effect,
                    decl.operations
                        .iter()
                        .map(|o| o.name.name.clone())
                        .collect(),
                    Vec::new(),
                    true,
                    Vec::new(),
                ),
                Item::Handler(decl) => (
                    &decl.name,
                    ExportKind::Handler,
                    vec![decl.effect.name.clone()],
                    Vec::new(),
                    true,
                    Vec::new(),
                ),
                Item::Function(decl) => {
                    rows.declaring(&decl.sig.rows);
                    let (row, complete) = rows.row(&decl.contract.uses);
                    (
                        &decl.sig.name,
                        ExportKind::Function,
                        Vec::new(),
                        row,
                        complete,
                        row_sources(&decl.sig, &decl.contract),
                    )
                }
                // A `test` is not part of the surface, and an error node is
                // not part of anything.
                Item::Test(_) => continue,
            };

            names.insert(
                name.name.clone(),
                Export {
                    kind,
                    members,
                    row,
                    row_complete,
                    row_from,
                },
            );
        }

        Self { names }
    }

    pub fn get(&self, name: &str) -> Option<&Export> {
        self.names.get(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.keys().map(String::as_str)
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
}
