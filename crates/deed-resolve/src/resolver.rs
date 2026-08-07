//! Name resolution.
//!
//! This is the first pass that asks what a program means rather than whether it
//! is well formed. It answers exactly one question, "what does this name refer
//! to", and leaves everything about types alone.
//!
//! It is also where the parser's deliberate ambiguities get settled. `a.b` is
//! module qualification or field access depending on what `a` turns out to be,
//! and a single segment pattern is a binding or a variant depending on the
//! same kind of question. The parser could not know. This pass can.

use std::collections::{HashMap, HashSet};

use deed_ast::{
    Block, ChoiceDecl, DeprecateDecl, EffectDecl, EffectRef, Expr, FnDecl, HandlerDecl, Ident,
    Item, Module, Pattern, RecordDecl, Stmt, Type, TypeAlias,
};
use deed_diagnostics::{Applicability, Diagnostic, FileId, Span};

use crate::codes;
use crate::defs::{DefData, DefId, DefKind, Deprecation, Dot, Resolutions};
use crate::exports::{ExportKind, Universe};

/// Names the language provides without anyone importing them.
///
/// Deliberately tiny. Every entry is a name that cannot be looked up in any
/// file, which is exactly the kind of thing P2 is a budget for.
///
/// `Result`, `ok` and `err` are here rather than in a library because a
/// language where you cannot write a failing function without an import is not
/// finished. Errors as values and `?` are core to the design.
///
/// `System`, `Console`, `Clock` and `Dir` are capability types. They have to be
/// built in for the same reason there is no ambient authority: if a program
/// could declare its own `Console` and conjure one, none of the rest would mean
/// anything.
///
/// `length` is here because a `String` you cannot measure is a `String` you
/// cannot check. It measures a `List` too, for the same reason.
///
/// `List` is built in rather than declared, which is a shortcut and was named
/// as one from the start. A `record` and a `choice` may carry type parameters
/// now, so it could be declared; what holds it here is `[1, 2, 3]`, which is a
/// literal with syntax of its own. Nothing else in the language can hold more
/// than one of
/// something, and a language where the only way to have two of a thing is to
/// name two variables is not one anybody can write a program in. `at` and
/// `push` come with it: a list you cannot read out of and cannot extend is a
/// literal, not a collection. `at` hands back a `Result` rather than the
/// element, because an index that is not there is not a bug in the caller and
/// nothing in this language traps.
///
/// `split`, `join`, `to_string` and `to_int` are two pairs of inverses, and
/// they are here because until them a program could hold text and a number and
/// get from neither to the other. Nothing could take input apart, put output
/// together, or print a count, which is most of what a program does.
///
/// `trim` is here on a narrower argument: it is the one text operation that
/// cannot be written in the language. `contains` is `length(split(a, b)) > 1`
/// and `replace` is `join(split(a, from), to)`, but deciding what whitespace
/// is needs to look at characters and dropping it off the ends needs a walk
/// that stops early, which a fold does not do.
///
/// `repeat` is here on the same argument and it was a real program that found
/// it. A `for` walks a list that already exists, so having something a number
/// of times has nothing to hand it, and the only way left is a function that
/// calls itself. That makes padding a column declare `Diverge`, and it spreads
/// to everything that builds a line, which is the outcome `design/02-syntax.md`
/// gives as the reason iteration exists at all. One name closes it, and `at`
/// turns the list back into the count it came from.
///
/// The prelude is a place names go to become unavailable to everyone else, so
/// it stays small and every addition to it is argued for rather than assumed.
///
/// `hash` is the newest and the argument for it is that it is one of the few
/// things this language cannot say about itself: taking a value of any shape
/// apart needs reflection or a trait to dispatch on, and there is neither. See
/// `design/decisions/2026-08-05-a-hash-is-the-equality-walk.md`.
pub const PRELUDE: &[&str] = &[
    "Int",
    "String",
    "Bool",
    "Result",
    "ok",
    "err",
    "length",
    "hash",
    "List",
    "at",
    "push",
    "repeat",
    "split",
    "join",
    "trim",
    "upper",
    "lower",
    "to_string",
    "to_int",
    "System",
    "Console",
    "Clock",
    "Dir",
    "Net",
];

/// Operations of the built-in `Io` effect.
///
/// Each takes the capability it acts on. The row says what kind of operation,
/// the argument says which resource, which is the split `04-capabilities.md`
/// describes.
///
/// `save` writes a file and `write` writes to a console, and by that split they
/// should be one name with the resource in the argument. They are two because
/// a signature here is one list of types per name and there is no overloading,
/// so the limitation shows through the design rather than the other way round.
///
/// `write` and `line` are the `read`/`save` split applied to the console. The
/// capability says which terminal; the row says which direction, and a
/// function handed a `Console` still cannot read what somebody typed unless it
/// says so. That is the same sentence `list` earns below, and it is the reason
/// there is no second capability for input.
///
/// `args` is the odd one. It hands back data rather than doing anything, and
/// it takes the whole `System` rather than a narrower capability, so it reads
/// like it does not belong. It goes in the row anyway, because how a program
/// was invoked is input from outside and every other way of getting input from
/// outside says so in a signature. A program that reads its arguments behaves
/// differently depending on them, and that is worth writing down.
///
/// `env` is `args` with one difference that matters: the arguments were typed
/// on the line that started the program, and the environment is whatever the
/// machine happens to be carrying, which routinely includes credentials nobody
/// meant to hand over. So it is granted by name at the call site rather than
/// read wholesale, and a variable the runner was not told about is not there
/// as far as the program is concerned. That is the `--allow` shape rather than
/// the `--dir` one: a list of what may be seen, not a place to see inside of.
///
/// `list` is the one that tests the whole model. Holding a `Dir` and declaring
/// `read` means you may read the file somebody told you about; declaring
/// `list` means you may find out what is there, which is strictly more. The
/// row is what separates them, and that is the same split that already stops a
/// reader from writing.
///
/// `now` and `epoch` are that split again, about something other than
/// authority. `now` counts calls, because P8 says the default is
/// deterministic and a wall clock would make every run different. `epoch`
/// reads the machine's clock, which is the right answer for a program that
/// needs the actual time and the wrong one for anything that has to give the
/// same answer twice. Holding a `Clock` says nothing about which of the two a
/// function may do, so the row does, and a signature saying `uses Io.epoch` is
/// a function whose output can change between two runs of the same program.
///
/// `reach`, `fetch` and `send` are the `Dir` split applied to the network.
/// `reach` narrows a `Net` to one host and is the `open` of this group: what
/// comes back reaches a subset of what went in and there is no way to widen
/// one. `fetch` reads and `send` writes, and they are two entries rather than
/// one for the reason `read` and `save` are: holding the capability says
/// nothing about which of them a function may do, and sending a request that
/// changes something on the other end is not the same permission as asking a
/// question. All three also carry what `epoch` carries, that the answer can
/// differ between two runs, and more so: the other end is a machine nobody
/// here controls.
pub const IO_OPERATIONS: &[&str] = &[
    "write", "line", "now", "epoch", "open", "read", "save", "remove", "make", "list", "args",
    "env", "reach", "fetch", "send",
];

/// The effects the language provides, available in every module without an
/// import.
///
/// A row naming one of these travels between modules under
/// [`PRELUDE_MODULE`], because the module that declared it is nobody's module
/// and every module can name it.
pub const PRELUDE_EFFECTS: &[&str] = &["Io", "Diverge"];

/// The module path builtins are named under.
///
/// Not a real module and not writable in source, so nothing can collide with
/// it. There is exactly one `Io` and every module has to agree about that,
/// which it would not if each one named it after itself.
pub const PRELUDE_MODULE: &str = "<prelude>";

/// How many "did you mean" suggestions one file gets.
///
/// See [`Resolver::suggest`] for why there is a limit at all. The number is
/// chosen so that a file being edited, which has a handful of unresolved names
/// while something is half typed, never notices it.
const SUGGESTION_BUDGET: usize = 24;

pub struct Resolved {
    pub resolutions: Resolutions,
    pub diagnostics: Vec<Diagnostic>,
}

impl Resolved {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Resolves one module. Always succeeds, possibly with diagnostics.
///
/// `universe` is every other module being compiled alongside this one. An
/// empty universe means a single file on its own, and any `use` in it has
/// nowhere to point, which is now reported rather than assumed away.
pub fn resolve(file: FileId, module: &Module, universe: &Universe) -> Resolved {
    let mut resolver = Resolver {
        file,
        universe,
        resolutions: Resolutions::default(),
        scopes: Vec::new(),
        diagnostics: Vec::new(),
        used: HashSet::new(),
        named: Vec::new(),
        suggestions: SUGGESTION_BUDGET,
        turn_names: Vec::new(),
        walked: Vec::new(),
    };

    resolver.push_scope(ScopeKind::Prelude);
    for name in PRELUDE {
        let def = resolver.resolutions.add_def(DefData {
            kind: DefKind::Builtin,
            name: (*name).to_string(),
            span: Span::at(0),
            parent: None,
        });
        resolver.insert(name, def);
        resolver.resolutions.record_builtin(name, def);
        // Builtins are never "unused".
        resolver.used.insert(def);
    }

    // The one effect the language provides. It is an effect rather than a set
    // of free functions so that writing to a console still has to be declared
    // in a `uses` clause like anything else.
    let io = resolver.resolutions.add_def(DefData {
        kind: DefKind::Effect,
        name: "Io".to_string(),
        span: Span::at(0),
        parent: None,
    });
    resolver.insert("Io", io);
    resolver.resolutions.record_builtin("Io", io);
    resolver.used.insert(io);

    // Not returning is something a function can do, so it goes in the row like
    // anything else a function can do. It has no operations: there is nothing
    // to call, only something to admit to. Built in for the same reason `Io`
    // is, since a program that could declare its own would be a program that
    // could opt out of saying it might not finish.
    let diverge = resolver.resolutions.add_def(DefData {
        kind: DefKind::Effect,
        name: "Diverge".to_string(),
        span: Span::at(0),
        parent: None,
    });
    resolver.insert("Diverge", diverge);
    resolver.resolutions.record_builtin("Diverge", diverge);
    resolver.used.insert(diverge);

    for operation in IO_OPERATIONS {
        let def = resolver.resolutions.add_def(DefData {
            kind: DefKind::EffectOp,
            name: (*operation).to_string(),
            span: Span::at(0),
            parent: Some(io),
        });
        resolver.resolutions.record_builtin(operation, def);
        resolver.used.insert(def);
    }

    resolver.push_scope(ScopeKind::Module);
    resolver.collect(module);
    resolver.resolve_module(module);
    resolver.report_unused_imports();
    resolver.report_unused_bindings();

    Resolved {
        resolutions: resolver.resolutions,
        diagnostics: resolver.diagnostics,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Prelude,
    Module,
    Local,
}

struct Scope {
    kind: ScopeKind,
    names: HashMap<String, DefId>,
}

struct Resolver<'a> {
    file: FileId,
    universe: &'a Universe,
    resolutions: Resolutions,
    scopes: Vec<Scope>,
    diagnostics: Vec<Diagnostic>,
    used: HashSet<DefId>,
    /// Every `let` that binds one plain name, in the order they were written.
    /// See [`Resolver::report_unused_bindings`] for why only those.
    named: Vec<DefId>,
    /// How many more "did you mean" suggestions this file gets. See
    /// [`Resolver::suggest`].
    suggestions: usize,
    /// The element and index of the `for` whose `while` is being resolved.
    ///
    /// Empty everywhere else. A name in that one condition matching one of
    /// these is the one unresolved name in the language that is not a
    /// misspelling, and telling somebody a correctly spelled word cannot be
    /// found sends them looking for a mistake that is not there.
    turn_names: Vec<String>,
    /// Accumulators of walks already left behind, by name, where the name was
    /// written, and where the walk was.
    ///
    /// A walk's accumulator is in scope in its body and nowhere else, and the
    /// value of the walk is what it ended up as. Somebody who read it as a
    /// variable writes the walk and then names the accumulator underneath,
    /// which is an unresolved name that is spelled exactly right.
    walked: Vec<(String, Span, Span)>,
}

impl Resolver<'_> {
    // -- scopes ------------------------------------------------------------

    fn push_scope(&mut self, kind: ScopeKind) {
        self.scopes.push(Scope {
            kind,
            names: HashMap::new(),
        });
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn insert(&mut self, name: &str, def: DefId) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.names.insert(name.to_string(), def);
        }
    }

    fn lookup(&self, name: &str) -> Option<(DefId, ScopeKind)> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.names.get(name).map(|def| (*def, scope.kind)))
    }

    fn member_of(&self, container: DefId, name: &str) -> Option<DefId> {
        self.resolutions
            .defs()
            .find(|(_, def)| def.parent == Some(container) && def.name == name)
            .map(|(id, _)| id)
    }

    // -- declaring ---------------------------------------------------------

    /// Declares something at module level, reporting a clash with whatever was
    /// already there.
    fn declare_item(&mut self, ident: &Ident, kind: DefKind, parent: Option<DefId>) -> DefId {
        if let Some(scope) = self.scopes.last()
            && let Some(previous) = scope.names.get(&ident.name).copied()
        {
            let previous_span = self.resolutions.def(previous).span;
            let previous_kind = self.resolutions.def(previous).kind;
            self.diagnostics.push(
                Diagnostic::error(
                    codes::DUPLICATE_DEFINITION,
                    self.file,
                    ident.span,
                    format!("`{}` is declared twice in this module", ident.name),
                )
                .with_primary_label(format!("redeclared as a {}", kind.describe()))
                .with_secondary(
                    previous_span,
                    format!("first declared as a {} here", previous_kind.describe()),
                ),
            );
        } else if PRELUDE.contains(&ident.name.as_str())
            || ident.name == "Io"
            || ident.name == "Diverge"
        {
            // Silently shadowing a builtin would put everything that depends on
            // it quietly back to being unchecked, which is the worst way for
            // this to go wrong.
            self.diagnostics.push(
                Diagnostic::warning(
                    codes::SHADOWED_DECLARATION,
                    self.file,
                    ident.span,
                    format!("`{}` hides a name the language provides", ident.name),
                )
                .with_primary_label("hides a builtin")
                .with_note(
                    "the builtin becomes unreachable in this file, and anything that relied on it stops being checked",
                ),
            );
        }

        let def = self.resolutions.add_def(DefData {
            kind,
            name: ident.name.clone(),
            span: ident.span,
            parent,
        });
        self.insert(&ident.name, def);
        self.resolutions.record_name(ident.span, def);
        def
    }

    /// Declares a member that is only reachable through its container, such as
    /// an effect operation. It does not enter any scope.
    fn declare_member(&mut self, ident: &Ident, kind: DefKind, parent: DefId) -> DefId {
        let def = self.resolutions.add_def(DefData {
            kind,
            name: ident.name.clone(),
            span: ident.span,
            parent: Some(parent),
        });
        self.resolutions.record_name(ident.span, def);
        def
    }

    /// Declares a parameter or a local binding.
    ///
    /// Shadowing another binding is an error. Shadowing a declaration is a
    /// warning. The reasoning is in `design/02-syntax.md`: a reader should be
    /// able to point at a name and know what it means without tracking where
    /// they are in the function.
    fn declare_local(&mut self, ident: &Ident, kind: DefKind) -> DefId {
        if let Some((previous, scope_kind)) = self.lookup(&ident.name) {
            let previous_data = self.resolutions.def(previous);
            let previous_span = previous_data.span;
            let previous_kind = previous_data.kind;

            match scope_kind {
                ScopeKind::Local => self.diagnostics.push(
                    Diagnostic::error(
                        codes::SHADOWED_BINDING,
                        self.file,
                        ident.span,
                        format!("`{}` is already bound", ident.name),
                    )
                    .with_primary_label("shadows an existing binding")
                    .with_secondary(previous_span, format!("the {} bound here", previous_kind.describe()))
                    .with_note("Deed does not allow shadowing, so that a name means one thing for the whole function"),
                ),
                ScopeKind::Module | ScopeKind::Prelude => {
                    let mut diagnostic = Diagnostic::warning(
                        codes::SHADOWED_DECLARATION,
                        self.file,
                        ident.span,
                        format!("`{}` hides a {}", ident.name, previous_kind.describe()),
                    )
                    .with_primary_label("hides a declaration");
                    // A builtin is declared nowhere, and its span says so by
                    // being empty. This used to point at it anyway: the offsets
                    // clamp, so `fn f(length: Int)` drew "declared here" under
                    // the first byte of the file, which is the `module` line
                    // and is not where `length` comes from.
                    if !previous_span.is_empty() {
                        diagnostic = diagnostic.with_secondary(previous_span, "declared here");
                    }
                    self.diagnostics.push(diagnostic);
                }
            }
        }

        let def = self.resolutions.add_def(DefData {
            kind,
            name: ident.name.clone(),
            span: ident.span,
            parent: None,
        });
        self.insert(&ident.name, def);
        self.resolutions.record_name(ident.span, def);
        def
    }

    // -- using -------------------------------------------------------------

    fn use_name(&mut self, ident: &Ident) -> Option<DefId> {
        // The parser produces empty identifiers as placeholders after an error.
        // Reporting them would be reporting the same mistake twice.
        if ident.name.is_empty() {
            return None;
        }

        if let Some((def, _)) = self.lookup(&ident.name) {
            self.used.insert(def);
            self.resolutions.record_name(ident.span, def);
            self.warn_if_deprecated(ident, def);
            return Some(def);
        }

        let suggestion = self.suggest(&ident.name);
        let mut diagnostic = Diagnostic::error(
            codes::UNKNOWN_NAME,
            self.file,
            ident.span,
            format!("cannot find `{}` in this scope", ident.name),
        )
        .with_primary_label("not found");

        // The one unresolved name that is spelled right. A `while` is read
        // before the turn starts, so the element it would be deciding about
        // does not exist yet, and a suggestion here would send somebody to
        // whatever name happens to look similar.
        if self.turn_names.contains(&ident.name) {
            self.diagnostics.push(
                diagnostic
                    .with_primary_label("not in scope yet")
                    .with_note(format!(
                        "`{}` belongs to the turn this condition decides whether to take, \
                         so it is not in scope until that turn starts",
                        ident.name
                    ))
                    .with_note(
                        "a walk that stops on something about the element has to work that \
                         out in the body and put it in the accumulator, which `while` reads \
                         on the turn after",
                    ),
            );
            return None;
        }

        // The accumulator of a walk that has already finished. It is spelled
        // right and it was declared, one scope up and one statement ago, so
        // the suggester has nothing useful to say and the shipped library has
        // an import to offer that would be about a different function
        // entirely: `sum` is in `std/list`.
        //
        // Measured. A model wrote `for n in ns with sum = 0 { sum = sum + n }`
        // and then `sum` underneath, and got four diagnostics for one
        // misunderstanding, two of them carrying repairs that make the program
        // worse.
        if let Some((_, declared, walk)) = self
            .walked
            .iter()
            .rev()
            .find(|(name, _, _)| *name == ident.name)
        {
            self.diagnostics.push(
                diagnostic
                    .with_primary_label("not in scope here")
                    .with_secondary(*declared, "the accumulator of a walk, in scope in its body")
                    .with_secondary(*walk, "and this is what it ended up as")
                    .with_note(format!(
                        "a walk is an expression whose value is the accumulator it finished \
                         with, so what reads it is `let {} = for ... {{ ... }}`",
                        ident.name
                    )),
            );
            return None;
        }

        // A name that means something in another language and nothing here.
        // The suggester works on edit distance, so `null` used to be answered
        // with whatever short name happened to be nearby, which is worse than
        // a typo hint on a typo: it is a confident answer to a question that
        // was never asked.
        if let Some(elsewhere) = name_from_elsewhere(&ident.name) {
            self.diagnostics.push(match elsewhere {
                Elsewhere::Operator(op) => diagnostic
                    .with_note(format!("this is spelled `{op}`"))
                    .with_fix(
                        format!("write `{op}`"),
                        ident.span,
                        op,
                        Applicability::MachineApplicable,
                    ),
                Elsewhere::Absent(reason) => diagnostic.with_note(reason),
            });
            return None;
        }

        if let Some(candidate) = suggestion {
            diagnostic = diagnostic.with_fix(
                format!("there is a `{candidate}` in scope"),
                ident.span,
                candidate,
                Applicability::MachineApplicable,
            );
        }

        self.diagnostics.push(diagnostic);
        None
    }

    fn warn_if_deprecated(&mut self, ident: &Ident, def: DefId) {
        let Some(deprecation) = self.resolutions.deprecation(def).cloned() else {
            return;
        };
        let data = self.resolutions.def(def);
        let mut diagnostic = Diagnostic::warning(
            codes::DEPRECATED_DECLARATION,
            self.file,
            ident.span,
            format!(
                "`{}` is deprecated; use `{}` instead",
                data.name, deprecation.replacement
            ),
        )
        .with_primary_label("deprecated declaration");
        if !data.span.is_empty() {
            diagnostic = diagnostic.with_secondary(data.span, "deprecated here");
        }
        if self.lookup(&deprecation.replacement).is_some() {
            diagnostic = diagnostic.with_fix(
                format!("write `{}`", deprecation.replacement),
                ident.span,
                &deprecation.replacement,
                Applicability::MachineApplicable,
            );
        }
        self.diagnostics.push(diagnostic);
    }

    /// Resolves `container.name`, classifying the `.` on the way.
    fn resolve_member(&mut self, container: DefId, ident: &Ident) -> Option<DefId> {
        match self.resolutions.def(container).kind {
            // The name lives in another module. An effect's operations and a
            // choice's variants are part of its declaration, so those cross a
            // file boundary as syntax and get definitions of their own here.
            // Anything else stays foreign, because a record's fields are the
            // type checker's business and it reads them from that module's
            // surface rather than from a name.
            DefKind::Import => {
                let Some(export) = self.resolutions.import(container).cloned() else {
                    self.resolutions.record_dot(ident.span, Dot::Foreign);
                    return None;
                };
                if !matches!(export.kind, ExportKind::Effect | ExportKind::Choice) {
                    self.resolutions.record_dot(ident.span, Dot::Foreign);
                    return None;
                }

                if export.members.contains(&ident.name) {
                    let kind = match export.kind {
                        ExportKind::Effect => DefKind::EffectOp,
                        _ => DefKind::Variant,
                    };
                    let member = self.member_of(container, &ident.name).unwrap_or_else(|| {
                        // Created on demand, once per name, so two mentions of
                        // the same operation are the same definition.
                        self.resolutions.add_def(DefData {
                            kind,
                            name: ident.name.clone(),
                            span: ident.span,
                            parent: Some(container),
                        })
                    });
                    self.used.insert(member);
                    self.resolutions.record_name(ident.span, member);
                    self.warn_if_deprecated(ident, member);
                    return Some(member);
                }

                self.resolutions.record_dot(ident.span, Dot::Foreign);
                let suggestion = closest(&ident.name, export.members.iter().map(String::as_str));
                let container_data = self.resolutions.def(container);
                let container_name = container_data.name.clone();
                let what = export.kind.describe();

                let mut diagnostic = Diagnostic::error(
                    codes::UNKNOWN_MEMBER,
                    self.file,
                    ident.span,
                    format!("`{container_name}` is {what} with no `{}`", ident.name),
                )
                .with_primary_label("no such member");
                if let Some(candidate) = suggestion {
                    diagnostic = diagnostic.with_fix(
                        format!("there is a `{candidate}`"),
                        ident.span,
                        candidate,
                        Applicability::MaybeIncorrect,
                    );
                }
                self.diagnostics.push(diagnostic);
                None
            }
            DefKind::Choice | DefKind::Effect => match self.member_of(container, &ident.name) {
                Some(member) => {
                    self.used.insert(member);
                    self.resolutions.record_name(ident.span, member);
                    self.warn_if_deprecated(ident, member);
                    Some(member)
                }
                None => {
                    let container_data = self.resolutions.def(container);
                    let container_name = container_data.name.clone();
                    let container_kind = container_data.kind.describe();
                    self.diagnostics.push(
                        Diagnostic::error(
                            codes::UNKNOWN_MEMBER,
                            self.file,
                            ident.span,
                            format!(
                                "the {container_kind} `{container_name}` has no member `{}`",
                                ident.name
                            ),
                        )
                        .with_primary_label("no such member"),
                    );
                    None
                }
            },
            // A value. Which field this is, and whether it exists, is a
            // question the type checker gets to answer.
            _ => {
                self.resolutions.record_dot(ident.span, Dot::Field);
                None
            }
        }
    }

    /// Suggests a name, while there is budget for it.
    ///
    /// Finding the nearest name costs a pass over everything in scope, so doing
    /// it once per unresolved name is quadratic in the size of the file. A file
    /// full of unresolved names is the normal state of a file being edited,
    /// which is exactly when P9's latency claim is about something, so there is
    /// a budget and it is small.
    ///
    /// Spending it in source order is what makes the output the same every
    /// time. Cutting off by count rather than by cost would depend on how fast
    /// the machine was, and a diagnostic that changes with the weather is not
    /// an API.
    ///
    /// Past the budget the diagnostic still points at the name and still says
    /// it was not found. Only the "did you mean" goes, and a file with two
    /// dozen unresolved names has a problem that a typo hint does not solve.
    fn suggest(&mut self, name: &str) -> Option<String> {
        if self.suggestions == 0 {
            return None;
        }
        self.suggestions -= 1;

        let mut candidates = Vec::new();
        for scope in &self.scopes {
            candidates.extend(scope.names.keys().map(String::as_str));
        }
        closest(name, candidates.into_iter())
    }

    fn report_unused_imports(&mut self) {
        let unused: Vec<(String, Span)> = self
            .resolutions
            .defs()
            .filter(|(id, def)| def.kind == DefKind::Import && !self.used.contains(id))
            .map(|(_, def)| (def.name.clone(), def.span))
            .collect();

        for (name, span) in unused {
            self.diagnostics.push(
                Diagnostic::warning(
                    codes::UNUSED_IMPORT,
                    self.file,
                    span,
                    format!("`{name}` is imported but never used"),
                )
                .with_primary_label("unused import")
                .with_note("imports are explicit and there is no wildcard form, so an unused one is only noise"),
            );
        }
    }

    /// Reports every `let` that names a value nobody reads.
    ///
    /// A `let` exists to give a value a name so that something else can use
    /// it. The name is the whole reason the form was written, so one that no
    /// expression mentions leaves a statement doing nothing a statement could
    /// not have done on its own. That case already has a warning, and a name
    /// is exactly what silences it: `twice(n)` on its own line is a value
    /// nobody reads, and `let b = twice(n)` is the same value with somewhere
    /// to put it that nobody looks in.
    ///
    /// Only a `let` binding one plain name. Every other binder is part of a
    /// shape something outside the binding chose. A pattern is there to match,
    /// so `err(why)` names what it is not going to look at and reads better
    /// for it. A parameter's shape is the signature, and a handler's signature
    /// belongs to the effect it implements. A `for` binder walks whether or
    /// not the element is wanted. Asking those to be spelled differently
    /// trades a name for a hole and says nothing that was not already visible.
    ///
    /// `_name` is how a `let` says it meant to keep the name and not the
    /// value. So is `let _ = ...`, which binds nothing at all.
    fn report_unused_bindings(&mut self) {
        let unused: Vec<(String, Span)> = self
            .named
            .iter()
            .filter(|id| !self.used.contains(id))
            .map(|id| {
                let def = self.resolutions.def(*id);
                (def.name.clone(), def.span)
            })
            .filter(|(name, _)| !name.starts_with('_'))
            .collect();

        for (name, span) in unused {
            self.diagnostics.push(
                Diagnostic::warning(
                    codes::UNUSED_BINDING,
                    self.file,
                    span,
                    format!("nothing reads `{name}`"),
                )
                .with_primary_label("this name is never used")
                .with_note(format!(
                    "a name cannot be shadowed, so this one is read nowhere; write `_{name}` if the value is meant to be dropped"
                ))
                // A guess, and the guess is about intent rather than about
                // spelling. The other answer is that something was supposed to
                // read this and reads the wrong thing instead, which is a bug
                // that renaming would bury. So an editor offers it and
                // `deed fix` leaves it alone.
                .with_fix(
                    format!("call it `_{name}`"),
                    span,
                    format!("_{name}"),
                    Applicability::MaybeIncorrect,
                ),
            );
        }
    }

    // -- walking -----------------------------------------------------------

    /// Collects every module level name before resolving any body, so that
    /// declaration order does not matter.
    fn collect(&mut self, module: &Module) {
        for import in &module.uses {
            let path = import.path.to_string_path();
            let exports = self.universe.get(&path);

            if exports.is_none() {
                let suggestion = closest(&path, self.universe.paths());
                let mut diagnostic = Diagnostic::error(
                    codes::UNKNOWN_MODULE,
                    self.file,
                    import.path.span,
                    format!("no module `{path}` among the files being compiled"),
                )
                .with_primary_label("not found")
                .with_note(
                    "a module is named by its own `module` line, and only the files handed \
                     to the compiler are looked at",
                );
                if let Some(candidate) = suggestion {
                    diagnostic = diagnostic.with_fix(
                        format!("there is a module `{candidate}`"),
                        import.path.span,
                        candidate,
                        Applicability::MaybeIncorrect,
                    );
                }
                self.diagnostics.push(diagnostic);
            }

            for name in &import.names {
                // Declared whatever happened above, so one missing module
                // produces one diagnostic rather than one per name plus a
                // cascade of unresolved uses further down the file.
                let def = self.declare_item(name, DefKind::Import, None);
                self.resolutions.record_import_module(def, &path);

                let Some(exports) = exports else { continue };
                match exports.get(&name.name) {
                    Some(export) => {
                        self.resolutions.record_export(def, export.clone());
                        if let Some(replacement) = &export.deprecated {
                            self.resolutions.record_deprecation(
                                def,
                                Deprecation {
                                    replacement: replacement.clone(),
                                },
                            );
                        }
                    }
                    None => {
                        let suggestion = closest(&name.name, exports.names());
                        let mut diagnostic = Diagnostic::error(
                            codes::UNKNOWN_EXPORT,
                            self.file,
                            name.span,
                            format!("`{path}` declares no `{}`", name.name),
                        )
                        .with_primary_label("not declared there");
                        if let Some(candidate) = suggestion {
                            diagnostic = diagnostic.with_fix(
                                format!("`{path}` declares a `{candidate}`"),
                                name.span,
                                candidate,
                                Applicability::MaybeIncorrect,
                            );
                        }
                        self.diagnostics.push(diagnostic);
                    }
                }
            }
        }

        for item in &module.items {
            match item {
                Item::Deprecate(_) | Item::Operator(_) => {}
                Item::TypeAlias(alias) => {
                    self.declare_item(&alias.name, DefKind::Type, None);
                }
                Item::Record(record) => {
                    self.declare_item(&record.name, DefKind::Record, None);
                }
                Item::Choice(choice) => {
                    let id = self.declare_item(&choice.name, DefKind::Choice, None);
                    // Variants are usable unqualified, which is what makes
                    // `err(InsufficientFunds { .. })` read the way it does.
                    for variant in &choice.variants {
                        self.declare_item(&variant.name, DefKind::Variant, Some(id));
                    }
                }
                Item::Effect(effect) => {
                    let id = self.declare_item(&effect.name, DefKind::Effect, None);
                    // A row variable belongs to the effect rather than to any
                    // one operation, because a handler's state has to be able
                    // to name the same one. Declared here, once, so that the
                    // effect's signatures and the handler's state are talking
                    // about the same variable and not two of the same name.
                    for variable in &effect.rows {
                        let def = self.declare_member(variable, DefKind::RowParam, id);
                        self.used.insert(def);
                    }
                    // Operations are reachable only through the effect.
                    for operation in &effect.operations {
                        self.declare_member(&operation.name, DefKind::EffectOp, id);
                    }
                }
                Item::Handler(handler) => {
                    self.declare_item(&handler.name, DefKind::Handler, None);
                }
                Item::Function(function) => {
                    self.declare_item(&function.sig.name, DefKind::Function, None);
                }
                Item::Test(_) => {}
            }
        }

        for item in &module.items {
            let Item::Deprecate(deprecate) = item else {
                continue;
            };
            self.apply_deprecation(deprecate);
        }
    }

    fn resolve_module(&mut self, module: &Module) {
        for item in &module.items {
            match item {
                Item::Deprecate(_) => {}
                // The name is looked up like any other, so a binding to a
                // function that is not there is the ordinary message about a
                // name, and binding one is a use of it.
                Item::Operator(decl) => {
                    self.use_name(&decl.function);
                }
                Item::TypeAlias(alias) => self.resolve_type_alias(alias),
                Item::Record(record) => self.resolve_record(record),
                Item::Choice(choice) => self.resolve_choice(choice),
                Item::Effect(effect) => self.resolve_effect(effect),
                Item::Handler(handler) => self.resolve_handler(handler),
                Item::Function(function) => self.resolve_fn(function),
                Item::Test(test) => self.resolve_block(&test.body),
            }
        }
    }

    fn apply_deprecation(&mut self, deprecate: &DeprecateDecl) {
        if deprecate.old.name.is_empty() {
            return;
        }
        if deprecate.new.name.is_empty() {
            return;
        }
        let Some((old, _)) = self.lookup(&deprecate.old.name) else {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::UNKNOWN_NAME,
                    self.file,
                    deprecate.old.span,
                    format!("cannot find `{}` in this scope", deprecate.old.name),
                )
                .with_primary_label("deprecated declaration not found"),
            );
            return;
        };
        let Some((new, _)) = self.lookup(&deprecate.new.name) else {
            self.diagnostics.push(
                Diagnostic::error(
                    codes::UNKNOWN_NAME,
                    self.file,
                    deprecate.new.span,
                    format!("cannot find `{}` in this scope", deprecate.new.name),
                )
                .with_primary_label("replacement not found"),
            );
            return;
        };
        if old == new {
            return;
        }
        self.resolutions.record_deprecation(
            old,
            Deprecation {
                replacement: deprecate.new.name.clone(),
            },
        );
    }

    fn resolve_type_alias(&mut self, alias: &TypeAlias) {
        self.push_scope(ScopeKind::Local);
        self.declare_type_params(&alias.generics);
        self.resolve_type(&alias.ty);
        self.pop_scope();

        if let Some(refinement) = &alias.refinement {
            // `value` is the thing being refined. It is the only name the
            // language introduces implicitly, and it exists because a
            // refinement has nothing else to talk about.
            self.push_scope(ScopeKind::Local);
            let def = self.resolutions.add_def(DefData {
                kind: DefKind::Local,
                name: "value".to_string(),
                span: alias.name.span,
                parent: None,
            });
            self.insert("value", def);
            self.used.insert(def);
            self.resolve_expr(refinement);
            self.pop_scope();
        }
    }

    fn resolve_record(&mut self, record: &RecordDecl) {
        self.push_scope(ScopeKind::Local);
        self.declare_type_params(&record.generics);
        for field in &record.fields {
            self.resolve_type(&field.ty);
        }
        self.pop_scope();
    }

    fn resolve_choice(&mut self, choice: &ChoiceDecl) {
        self.push_scope(ScopeKind::Local);
        self.declare_type_params(&choice.generics);
        for variant in &choice.variants {
            for field in variant.fields.iter().flatten() {
                self.resolve_type(&field.ty);
            }
        }
        self.pop_scope();
    }

    /// Puts a declaration's type parameters in scope for the rest of it.
    ///
    /// Marked used on the spot. Whether one is ever mentioned is a question
    /// the type checker answers with a better message than "unused", because
    /// the rule is about where it appears rather than whether it does.
    fn declare_type_params(&mut self, generics: &[Ident]) {
        for parameter in generics {
            let def = self.declare_local(parameter, DefKind::TypeParam);
            self.used.insert(def);
        }
    }

    fn resolve_effect(&mut self, effect: &EffectDecl) {
        self.push_scope(ScopeKind::Local);
        self.bring_row_params_into_scope(self.resolutions.resolution(effect.name.span));
        for operation in &effect.operations {
            for param in &operation.params {
                if let Some(ty) = &param.ty {
                    self.resolve_type(ty);
                }
            }
            if let Some(ret) = &operation.ret {
                self.resolve_type(ret);
            }
        }
        self.pop_scope();
    }

    /// Puts an effect's row variables in scope, by name.
    ///
    /// The definitions are the effect's own, so an operation signature and a
    /// handler's state that both write `r` resolve to one variable rather than
    /// to two that happen to be spelled alike. That is the whole reason this
    /// is not a fresh declaration in each place.
    fn bring_row_params_into_scope(&mut self, effect: Option<DefId>) {
        let Some(effect) = effect else {
            return;
        };
        if self.resolutions.def(effect).kind != DefKind::Effect {
            return;
        }
        let variables: Vec<(String, DefId)> = self
            .resolutions
            .defs()
            .filter(|(_, data)| data.parent == Some(effect) && data.kind == DefKind::RowParam)
            .map(|(id, data)| (data.name.clone(), id))
            .collect();
        for (name, def) in variables {
            self.insert(&name, def);
        }
    }

    fn resolve_handler(&mut self, handler: &HandlerDecl) {
        self.use_name(&handler.effect);

        self.push_scope(ScopeKind::Local);
        self.bring_row_params_into_scope(self.resolutions.resolution(handler.effect.span));
        for field in &handler.state {
            self.resolve_type(&field.ty);
            self.declare_local(&field.name, DefKind::State);
        }
        for operation in &handler.operations {
            self.resolve_fn(operation);
        }
        // `finally` is inside the handler's scope and can see state names,
        // the same as an operation body. Unlike a closure written inside an
        // operation, a `finally` block is structural and only ever runs from
        // the `with` block that installed the handler, so it is part of the
        // handler in the same way an operation is.
        if let Some(finally) = &handler.finally {
            self.resolve_block(finally);
        }
        self.pop_scope();
    }

    fn resolve_fn(&mut self, function: &FnDecl) {
        self.push_scope(ScopeKind::Local);

        // A walk's accumulator is a fact about this declaration, so what one
        // function walked says nothing about the next one.
        self.walked.clear();

        // Before anything else, because a parameter's type, the return type
        // and the body can all name one and none of them can be resolved
        // without it.
        self.declare_type_params(&function.sig.generics);
        for variable in &function.sig.rows {
            let def = self.declare_local(variable, DefKind::RowParam);
            self.used.insert(def);
        }

        for param in &function.sig.params {
            if let Some(ty) = &param.ty {
                self.resolve_type(ty);
            }
            self.declare_local(&param.name, DefKind::Param);
        }
        if let Some(ret) = &function.sig.ret {
            self.resolve_type(ret);
        }

        for requirement in &function.contract.requires {
            self.resolve_expr(requirement);
        }
        for effect in &function.contract.uses {
            self.resolve_effect_ref(effect);
        }
        for obligation in &function.contract.ensures {
            // `result` is what the function produced. It is bound per
            // obligation rather than once, because an `ok` clause and an `err`
            // clause see different things and therefore different types.
            self.push_scope(ScopeKind::Local);
            let def = self.resolutions.add_def(DefData {
                kind: DefKind::Local,
                name: "result".to_string(),
                span: obligation.outcome_span,
                parent: None,
            });
            self.insert("result", def);
            self.used.insert(def);
            self.resolve_expr(&obligation.condition);
            self.pop_scope();
        }

        self.resolve_block(&function.body);
        self.pop_scope();
    }

    fn resolve_effect_ref(&mut self, effect: &EffectRef) {
        let Some(def) = self.use_name(&effect.effect) else {
            return;
        };
        if let Some(operation) = &effect.operation {
            self.resolve_member(def, operation);
        }
    }

    fn resolve_type(&mut self, ty: &Type) {
        match ty {
            Type::Named { name, args, .. } => {
                self.use_name(name);
                for arg in args {
                    self.resolve_type(arg);
                }
            }
            Type::Fn {
                params, row, ret, ..
            } => {
                for param in params {
                    self.resolve_type(param);
                }
                // A row inside a type names effects the same way a contract
                // does, so naming one here counts as using the import and
                // naming something that is not an effect is an error here too.
                for effect in row {
                    self.resolve_effect_ref(effect);
                }
                self.resolve_type(ret);
            }
            Type::Unit(_) | Type::Error(_) => {}
        }
    }

    fn resolve_block(&mut self, block: &Block) {
        self.push_scope(ScopeKind::Local);
        for stmt in &block.stmts {
            self.resolve_stmt(stmt);
        }
        if let Some(tail) = &block.tail {
            self.resolve_expr(tail);
        }
        self.pop_scope();
    }

    fn resolve_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let {
                pattern, ty, init, ..
            } => {
                // The initialiser is resolved first, so `let x = x` reads the
                // outer `x` rather than itself.
                self.resolve_expr(init);
                if let Some(ty) = ty {
                    self.resolve_type(ty);
                }
                self.bind_pattern(pattern);

                // A `let` binding one plain name is the only binder whose
                // whole reason for existing is the name.
                if let Pattern::Path { segments, .. } = pattern
                    && let [only] = segments.as_slice()
                    && !starts_upper(&only.name)
                    && let Some((def, _)) = self.lookup(&only.name)
                {
                    self.named.push(def);
                }
            }
            Stmt::Return { value, .. } => {
                if let Some(value) = value {
                    self.resolve_expr(value);
                }
            }
            Stmt::Assign { target, value, .. } => {
                self.resolve_expr(value);
                self.use_name(target);
            }
            Stmt::Assert { condition, .. } => self.resolve_expr(condition),
            Stmt::Refuses { subject, .. } => self.resolve_expr(subject),
            Stmt::Abandon { .. } => {}
            Stmt::Expr(expr) => self.resolve_expr(expr),
        }
    }

    fn resolve_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Int { .. }
            | Expr::Str { .. }
            | Expr::Bool { .. }
            | Expr::Unit(_)
            | Expr::Error(_) => {}

            Expr::Ident(ident) => {
                self.use_name(ident);
            }

            Expr::Field { receiver, name, .. } => self.resolve_field(receiver, name),

            Expr::Call { callee, args, .. } => {
                self.resolve_expr(callee);
                for arg in args {
                    self.resolve_expr(arg);
                }
            }

            Expr::List { elements, .. } => {
                for element in elements {
                    self.resolve_expr(element);
                }
            }

            Expr::StructLit { path, fields, .. } => {
                self.resolve_expr(path);
                for field in fields {
                    match &field.value {
                        Some(value) => self.resolve_expr(value),
                        // Shorthand: `Receipt { from }` means `from: from`, so
                        // the label is also a reference.
                        None => {
                            self.use_name(&field.name);
                        }
                    }
                }
            }

            Expr::Unary { operand, .. } => self.resolve_expr(operand),
            Expr::Binary { lhs, rhs, .. } => {
                self.resolve_expr(lhs);
                self.resolve_expr(rhs);
            }
            Expr::Try { operand, .. } => self.resolve_expr(operand),

            Expr::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.resolve_expr(condition);
                self.resolve_block(then_branch);
                if let Some(else_branch) = else_branch {
                    self.resolve_expr(else_branch);
                }
            }

            Expr::Match {
                scrutinee, arms, ..
            } => {
                self.resolve_expr(scrutinee);
                for arm in arms {
                    self.push_scope(ScopeKind::Local);
                    self.bind_pattern(&arm.pattern);
                    self.resolve_expr(&arm.body);
                    self.pop_scope();
                }
            }

            Expr::For {
                binder,
                index,
                iterable,
                accumulator,
                keep,
                body,
                span: span_of_for,
                ..
            } => {
                let span_of_for = *span_of_for;
                self.resolve_expr(iterable);
                // What the accumulator starts as is worked out before the loop
                // exists, so the accumulator is not in scope in it. `with sum =
                // sum` is a name that does not resolve rather than a value
                // that refers to itself.
                if let Some(accumulator) = accumulator {
                    self.resolve_expr(&accumulator.init);
                }

                self.push_scope(ScopeKind::Local);
                if let Some(accumulator) = accumulator {
                    self.declare_local(&accumulator.name, DefKind::Local);
                    self.walked.push((
                        accumulator.name.name.clone(),
                        accumulator.name.span,
                        span_of_for,
                    ));
                }
                // Before the element and the index, because the condition
                // decides whether to take the turn those belong to. `while
                // item` is a name that does not resolve rather than a value
                // from a turn that has not happened.
                if let Some(keep) = keep {
                    // Named while this one expression is resolved, so that the
                    // failure can say why rather than offering a spelling.
                    self.turn_names = std::iter::once(binder.name.to_string())
                        .chain(index.iter().map(|index| index.name.to_string()))
                        .collect();
                    self.resolve_expr(keep);
                    self.turn_names.clear();
                }
                self.declare_local(binder, DefKind::Local);
                if let Some(index) = index {
                    self.declare_local(index, DefKind::Local);
                }
                self.resolve_block(body);
                self.pop_scope();
            }

            Expr::Block(block) => self.resolve_block(block),

            Expr::Closure { params, body, .. } => {
                self.push_scope(ScopeKind::Local);
                for param in params {
                    if let Some(ty) = &param.ty {
                        self.resolve_type(ty);
                    }
                    self.declare_local(&param.name, DefKind::Param);
                }
                self.resolve_expr(body);
                self.pop_scope();
            }

            Expr::Old { expr, .. } => self.resolve_expr(expr),
            Expr::Unchanged { effect, .. } => self.resolve_effect_ref(effect),

            Expr::With { handlers, body, .. } => {
                for handler in handlers {
                    self.resolve_expr(handler);
                }
                self.resolve_block(body);
            }
        }
    }

    fn resolve_field(&mut self, receiver: &Expr, name: &Ident) {
        // A path only means qualification when it starts at a name. Anything
        // else on the left is a value, and `.name` is a field of it.
        let container = match receiver {
            Expr::Ident(ident) => self.use_name(ident),
            other => {
                self.resolve_expr(other);
                None
            }
        };

        match container {
            Some(container) => {
                self.resolve_member(container, name);
            }
            None => self.resolutions.record_dot(name.span, Dot::Field),
        }
    }

    // -- patterns ----------------------------------------------------------

    fn bind_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Wildcard(_)
            | Pattern::Int { .. }
            | Pattern::Str { .. }
            | Pattern::Bool { .. }
            | Pattern::Error(_) => {}

            Pattern::Path { segments, .. } => {
                match segments.split_first() {
                    None => {}
                    Some((first, [])) => {
                        // The one place the language leans on capitalisation.
                        // Without it a mistyped variant silently becomes a
                        // binding that matches everything, which is a bug the
                        // compiler would never mention.
                        if starts_upper(&first.name) {
                            self.use_name(first);
                        } else {
                            self.declare_local(first, DefKind::Local);
                        }
                    }
                    Some(_) => self.resolve_path(segments),
                }
            }

            Pattern::Tuple { path, elements, .. } => {
                self.resolve_path(path);
                for element in elements {
                    self.bind_pattern(element);
                }
            }

            Pattern::Record { path, fields, .. } => {
                self.resolve_path(path);
                for field in fields {
                    match &field.pattern {
                        Some(pattern) => self.bind_pattern(pattern),
                        None => {
                            self.declare_local(&field.name, DefKind::Local);
                        }
                    }
                }
            }

            Pattern::OneOf { alternatives, .. } => {
                for alternative in alternatives {
                    if let Some(span) = binder_in(alternative) {
                        self.diagnostics.push(
                            Diagnostic::error(
                                codes::BINDING_IN_AN_ALTERNATIVE,
                                self.file,
                                span,
                                "an alternative cannot bind a name",
                            )
                            .with_primary_label("this would only be bound when this side matched")
                            .with_note(
                                "the body of the arm runs whichever alternative matched, so a name bound by one of them would not be there for the others",
                            )
                            .with_note(
                                "a variant with fields can be named on its own, without the braces",
                            ),
                        );
                    }
                    // Bound anyway, including when it was just refused. The
                    // body was written expecting the name, and letting it fail
                    // to resolve as well would answer one mistake with a
                    // second complaint about a line that is not wrong.
                    self.bind_pattern(alternative);
                }
            }
        }
    }

    fn resolve_path(&mut self, segments: &[Ident]) {
        let Some((first, rest)) = segments.split_first() else {
            return;
        };
        let mut container = self.use_name(first);
        for segment in rest {
            container = match container {
                Some(id) => self.resolve_member(id, segment),
                None => {
                    self.resolutions.record_dot(segment.span, Dot::Field);
                    None
                }
            };
        }
    }
}

fn starts_upper(name: &str) -> bool {
    name.chars().next().is_some_and(char::is_uppercase)
}

/// Where `pattern` would bind a name, if it would.
///
/// The same capitalisation rule [`Resolver::bind_pattern`] uses, asked ahead
/// of time rather than acted on, because an alternative that binds is refused
/// rather than resolved and the answer has to come before anything is
/// declared.
fn binder_in(pattern: &Pattern) -> Option<Span> {
    match pattern {
        Pattern::Wildcard(_)
        | Pattern::Int { .. }
        | Pattern::Str { .. }
        | Pattern::Bool { .. }
        | Pattern::Error(_) => None,

        Pattern::Path { segments, span } => match segments.split_first() {
            Some((first, [])) if !starts_upper(&first.name) => Some(*span),
            _ => None,
        },

        Pattern::Tuple { elements, .. } => elements.iter().find_map(binder_in),

        Pattern::Record { fields, .. } => fields.iter().find_map(|field| match &field.pattern {
            Some(pattern) => binder_in(pattern),
            None => Some(field.span),
        }),

        Pattern::OneOf { alternatives, .. } => alternatives.iter().find_map(binder_in),
    }
}

/// What a name that is not here means somewhere else.
enum Elsewhere {
    /// The same thing under a different spelling, and the spelling is a
    /// straight substitution for the name.
    Operator(&'static str),
    /// Not in the language at all, with the reason.
    Absent(&'static str),
}

/// A name only a language other than this one would have had.
///
/// Nothing here can shadow anything: a name that resolves never reaches this
/// point, so somebody who declared a function called `and` still has it.
fn name_from_elsewhere(name: &str) -> Option<Elsewhere> {
    Some(match name {
        "and" => Elsewhere::Operator("&&"),
        "or" => Elsewhere::Operator("||"),
        "not" => Elsewhere::Operator("!"),
        "null" | "nil" | "undefined" | "None" | "none" => Elsewhere::Absent(
            "there is no empty value: something that might be absent is a `Result`, or a \
             `choice` with a variant for the absence, which is how `Option` is declared \
             when a program wants one",
        ),
        "self" | "this" => Elsewhere::Absent(
            "there are no methods, so a function takes the thing it works on as an argument \
             like anything else",
        ),
        // Measured rather than guessed: these are the words three benchmark
        // runs reached for and got nothing back but "cannot find" and a
        // suggestion from the edit-distance table, which is the answer to a
        // typo and these are not typos.
        "perform" => Elsewhere::Absent(
            "an effect operation is called like any other function, `Log.note(text)`, and the \
             `uses` clause in the signature is what says the call performs it",
        ),
        "state" => Elsewhere::Absent(
            "a handler's state is named on its own, `count`, declared `state count: Int` \
             inside the handler and given its value where the handler is installed, \
             `with H { count: 0 } { .. }`",
        ),
        "append" => Elsewhere::Absent(
            "one element onto the end of a list is `push(list, element)`, and two lists \
             joined is `concat` from `std/list`",
        ),
        "rest" => Elsewhere::Absent(
            "everything but the first element is `drop(list, 1)` from `std/list`, and the \
             first on its own is `first`",
        ),
        "update" | "mutate" | "change" | "put" => Elsewhere::Absent(
            "nothing is changed in place: a value is built and bound once, and the only name \
             that can be assigned is a handler's `state`, inside one of its operations",
        ),
        // `sum` used to be here too, saying to write the walk out. `std/list`
        // has it now, so the answer is an import and the driver writes it.
        "max" | "min" => Elsewhere::Absent(
            "the largest by an order you pass is `largest` from `std/list`, and handing it \
             the opposite comparator gives the other end",
        ),
        // The prelude has `length` and nothing else that reduces a list, and
        // `for` already carries an accumulator, so the rest of these are the
        // walk rather than a name.
        "product" | "average" | "reduce" => Elsewhere::Absent(
            "a value folded out of a list is written as the walk that folds it, \
             `for n in numbers with total = 0 { total + n }`, or with `fold` from `std/list`",
        ),
        _ => return None,
    })
}

/// The nearest candidate to `name`, when there is an unambiguous one.
///
/// Only candidates within the threshold are considered at all, which matters
/// more than it looks. This runs once per unresolved name and a file being
/// edited is a file full of unresolved names, so an edit distance against every
/// name in scope is quadratic in the size of the file. A scaling test caught
/// exactly that: ten times the broken input cost ninety times the time.
fn closest<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<String> {
    // One edit for short names, proportionally more for long ones. A
    // suggestion that is not obviously right is worse than none, because a
    // machine-applicable fix gets applied.
    let length = name.chars().count();
    let threshold = (length / 3).max(1);

    let mut best: Option<(usize, &str)> = None;
    let mut ambiguous = false;

    for candidate in candidates {
        // An edit distance is at least the difference in length, so this rules
        // most candidates out without looking at a single character.
        if candidate.chars().count().abs_diff(length) > threshold {
            continue;
        }
        let Some(distance) = edit_distance_within(name, candidate, threshold) else {
            continue;
        };
        match best {
            Some((best_distance, _)) if distance < best_distance => {
                best = Some((distance, candidate));
                ambiguous = false;
            }
            Some((best_distance, _)) if distance == best_distance => ambiguous = true,
            None => best = Some((distance, candidate)),
            _ => {}
        }
    }

    let (_, candidate) = best?;
    (!ambiguous).then(|| candidate.to_string())
}

/// Edit distance, or `None` when it is greater than `limit`.
///
/// Two letters written in the wrong order cost one edit rather than two. That
/// is the difference between this and plain Levenshtein and it is the whole
/// reason for it: the threshold in [`closest`] is one for any name of five
/// characters or fewer, a transposition is two edits under Levenshtein, and
/// short names are most of the names anybody writes. `psuh`, `tirm`, `totla`
/// and `spilt` all got no suggestion at all, while `lenght` got one only
/// because `length` is long enough to have earned a threshold of two.
///
/// Only the band of cells within `limit` of the diagonal can hold a value that
/// small, so the rest are never computed. With a limit of one or two that is a
/// handful of cells per row rather than the whole table. The transposition
/// reads one row further back, which is still inside the band it was computed
/// in.
fn edit_distance_within(a: &str, b: &str, limit: usize) -> Option<usize> {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();

    if a.len().abs_diff(b.len()) > limit {
        return None;
    }
    if a.is_empty() {
        return (b.len() <= limit).then_some(b.len());
    }

    let too_far = limit + 1;
    if b.is_empty() {
        return (a.len() <= limit).then_some(a.len());
    }

    // Three rows rather than two, because a transposition is the only rule
    // that looks past the row before this one.
    let mut before_previous = vec![too_far; b.len() + 1];
    let mut previous: Vec<usize> = (0..=b.len()).map(|j| j.min(too_far)).collect();
    let mut current = vec![too_far; b.len() + 1];

    for (i, &ca) in a.iter().enumerate() {
        // Cell (i + 1, j + 1) can only be within the limit when the row and
        // the column are within the limit of each other, so the rest of the
        // table is never computed.
        let first = i.saturating_sub(limit);
        let last = (i + limit).min(b.len() - 1);

        current[0] = (i + 1).min(too_far);
        for cell in current.iter_mut().skip(1) {
            *cell = too_far;
        }

        for j in first..=last {
            let cost = usize::from(ca != b[j]);
            let mut best = (current[j] + 1)
                .min(previous[j + 1] + 1)
                .min(previous[j] + cost)
                .min(too_far);

            // The two letters this pair is about are each other's, one place
            // over. Swapping them back is one edit, not the two that deleting
            // and reinserting would cost.
            if i > 0 && j > 0 && ca == b[j - 1] && a[i - 1] == b[j] {
                best = best.min(before_previous[j - 1] + 1).min(too_far);
            }

            current[j + 1] = best;
        }

        if current[first..=last + 1]
            .iter()
            .all(|cell| *cell >= too_far)
            && current[0] >= too_far
        {
            return None;
        }
        // before_previous <- previous <- current, with the oldest row becoming
        // the scratch space the next turn overwrites.
        std::mem::swap(&mut before_previous, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }

    let distance = previous[b.len()];
    (distance <= limit).then_some(distance)
}

#[cfg(test)]
mod tests {
    use super::edit_distance_within;

    /// The distance when it is wanted regardless of how large it is.
    fn distance(a: &str, b: &str) -> usize {
        let limit = a.chars().count().max(b.chars().count());
        edit_distance_within(a, b, limit).expect("the limit cannot be exceeded")
    }

    #[test]
    fn edit_distance_is_symmetric_and_zero_on_equal() {
        assert_eq!(distance("balance", "balance"), 0);
        assert_eq!(distance("balance", "balanse"), 1);
        assert_eq!(distance("balanse", "balance"), 1);
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("abc", ""), 3);
        assert_eq!(distance("kitten", "sitting"), 3);
    }

    /// The reason this is not plain Levenshtein. Every one of these costs two
    /// edits under that metric and gets no suggestion, because a name this
    /// short earns a threshold of one.
    #[test]
    fn two_letters_in_the_wrong_order_cost_one_edit() {
        for (typo, meant) in [
            ("psuh", "push"),
            ("tirm", "trim"),
            ("totla", "total"),
            ("spilt", "split"),
            ("lenght", "length"),
            ("ta", "at"),
        ] {
            assert_eq!(distance(typo, meant), 1, "`{typo}` for `{meant}`");
            assert_eq!(
                edit_distance_within(typo, meant, 1),
                Some(1),
                "`{typo}` for `{meant}` should be reachable at a limit of one"
            );
        }
    }

    /// Two separate transpositions are still two edits, and a transposition
    /// with something written between the letters is not one at all. Neither
    /// is a typo somebody makes on the way to the name next to it.
    #[test]
    fn it_is_the_restricted_metric_and_not_more() {
        assert_eq!(distance("abcd", "badc"), 2);
        assert_eq!(edit_distance_within("abcd", "badc", 1), None);
        // `ca` -> `abc` is a transposition with an insertion through it, which
        // the restricted metric charges separately and so does this.
        assert!(distance("ca", "abc") > 1);
    }

    #[test]
    fn a_distance_past_the_limit_is_not_computed() {
        assert_eq!(edit_distance_within("balance", "balanse", 1), Some(1));
        assert_eq!(edit_distance_within("balance", "sitting", 2), None);
        assert_eq!(edit_distance_within("a", "abcdefg", 2), None);
        assert_eq!(edit_distance_within("kitten", "sitting", 3), Some(3));
        assert_eq!(edit_distance_within("kitten", "sitting", 2), None);
    }

    #[test]
    fn the_band_agrees_with_the_full_table() {
        // The banded version is an optimisation, so it has to answer the same
        // thing wherever it answers at all. The transposed pairs are here
        // because that rule reads a row further back than the others, which is
        // the one place the band could have been too narrow.
        let words = [
            "", "a", "ab", "ba", "abc", "bac", "acb", "balance", "balanse", "kitten", "sitting",
            "counter", "count", "psuh", "push", "spilt", "split", "lenght", "length",
        ];
        for a in words {
            for b in words {
                let full = distance(a, b);
                for limit in 0..8 {
                    let banded = edit_distance_within(a, b, limit);
                    if full <= limit {
                        assert_eq!(banded, Some(full), "`{a}` vs `{b}` at limit {limit}");
                    } else {
                        assert_eq!(banded, None, "`{a}` vs `{b}` at limit {limit}");
                    }
                }
            }
        }
    }
}
