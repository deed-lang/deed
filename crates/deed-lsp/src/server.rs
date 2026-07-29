//! The server: what to do about each message, and nothing about how it
//! arrived.
//!
//! [`Server::handle`] takes one message and hands back the messages to send in
//! reply, which is what makes the whole protocol testable without a pipe, a
//! thread or an editor. The loop that owns the streams is in
//! [`crate::serve`] and is deliberately dull.
//!
//! A document is checked together with every other `.deed` file in the folders
//! the editor said it has open, with the text of anything open coming from the
//! editor's buffer rather than from disk, and with the modules that ship inside
//! the compiler after those. See [`crate::workspace`] for why the set is that
//! one and not another. An editor that names no folder gets the single file
//! behaviour, which is honest for a server that has been handed one file and
//! nothing else.

use std::collections::BTreeMap;
use std::path::PathBuf;

use deed_ast::Item;
use deed_diagnostics::{Applicability, Diagnostic, FileId, Severity, SourceMap, Span};
use deed_driver::{Checked, ObligationReport};
use deed_resolve::{DefId, DefKind};
use deed_typeck::ty::{FnRow, Ty};

use crate::json::Json;
use crate::position::{Lines, Position};
use crate::uri;
use crate::workspace::{Workspace, canonical};

/// JSON-RPC error codes this server can produce.
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_REQUEST: i64 = -32600;

/// The `SymbolKind` numbers an outline uses.
///
/// Spelled out rather than borrowed, for the same reason the JSON reader is
/// written here: the protocol is a handful of constants and a dependency for
/// them would be a larger thing to audit than the code it replaced.
mod kind {
    pub const CLASS: i64 = 5;
    pub const METHOD: i64 = 6;
    pub const FIELD: i64 = 8;
    pub const ENUM: i64 = 10;
    pub const INTERFACE: i64 = 11;
    pub const FUNCTION: i64 = 12;
    pub const ENUM_MEMBER: i64 = 22;
    pub const STRUCT: i64 = 23;
}

/// What the loop should do after a message.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Next {
    Continue,
    /// `exit` arrived. The process is expected to stop.
    Stop,
}

/// One open document, and the line index that goes with its text.
struct Document {
    text: String,
    lines: Lines,
}

impl Document {
    fn new(text: String) -> Self {
        let lines = Lines::of(&text);
        Self { text, lines }
    }
}

/// One file of the workspace, checked, and the URI it is known by.
struct Entry {
    /// `None` for a module that ships inside the compiler. It is checked so
    /// that a `use` naming it resolves, and there is no file behind it: no
    /// place to open, no place to edit, and nowhere to put a squiggle. Every
    /// answer that hands the editor a location has to say what it does with
    /// one of these, which is why this is an `Option` rather than a made up
    /// URI that fails when somebody clicks it.
    uri: Option<String>,
    checked: Checked,
}

/// Every place one name is written in one file.
///
/// Kept as spans rather than as answers, because references wants locations
/// and rename wants edits and the walk that finds them is the same walk.
struct Found {
    uri: String,
    file: deed_diagnostics::FileId,
    spans: Vec<Span>,
}

#[derive(Default)]
pub struct Server {
    /// Keyed by URI, ordered so that anything iterating them is reproducible.
    documents: BTreeMap<String, Document>,
    /// The folders the editor said it has open, empty until `initialize`.
    workspace: Workspace,
    initialized: bool,
    shutting_down: bool,
}

impl Server {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles one message, and says what to send and whether to carry on.
    pub fn handle(&mut self, message: &Json) -> (Vec<Json>, Next) {
        let method = message.at(&["method"]).and_then(Json::as_str);
        let id = message.at(&["id"]).filter(|id| !id.is_null()).cloned();

        match (method, id) {
            (Some("exit"), _) => (Vec::new(), Next::Stop),
            (Some(method), Some(id)) => (vec![self.request(method, &id, message)], Next::Continue),
            (Some(method), None) => (self.notification(method, message), Next::Continue),
            // A response to something this server asked for. It asks for
            // nothing, so there is nothing this could be an answer to.
            (None, _) => (Vec::new(), Next::Continue),
        }
    }

    fn request(&mut self, method: &str, id: &Json, message: &Json) -> Json {
        // Everything except `initialize` has to wait for it, and everything
        // after `shutdown` is refused. Without this a server answers questions
        // about state the editor has already torn down.
        if method != "initialize" && !self.initialized {
            return error(id, INVALID_REQUEST, "the server has not been initialized");
        }
        if self.shutting_down {
            return error(id, INVALID_REQUEST, "the server is shutting down");
        }

        match method {
            "initialize" => {
                self.initialized = true;
                self.workspace = Workspace::from_initialize(message);
                result(id, initialize_result())
            }
            "shutdown" => {
                self.shutting_down = true;
                result(id, Json::Null)
            }
            "textDocument/hover" => result(id, self.hover(message)),
            "textDocument/inlayHint" => result(id, self.inlay_hint(message)),
            "textDocument/definition" => result(id, self.definition(message)),
            "textDocument/references" => result(id, self.references(message)),
            "textDocument/prepareRename" => result(id, self.prepare_rename(message)),
            // Answers with the whole response rather than a value, because
            // refusing a rename has to be an error: an editor told "here is an
            // edit with nothing in it" reports success and changes nothing.
            "textDocument/rename" => self.rename(id, message),
            "textDocument/formatting" => result(id, self.formatting(message)),
            "textDocument/codeAction" => result(id, self.code_action(message)),
            "textDocument/documentSymbol" => result(id, self.document_symbol(message)),
            "textDocument/signatureHelp" => result(id, self.signature_help(message)),
            "workspace/symbol" => result(id, self.workspace_symbol(message)),
            "textDocument/completion" => result(id, self.completion(message)),
            _ => error(
                id,
                METHOD_NOT_FOUND,
                &format!("`{method}` is not something this server does"),
            ),
        }
    }

    /// The document a request is about, the byte offset it points at, and what
    /// the compiler makes of it.
    fn locate(&self, message: &Json) -> Option<(&Document, u32, Checked)> {
        let uri = text_document_uri(message)?;
        let document = self.documents.get(&uri)?;
        let position = position_of(message)?;
        let offset = document.lines.offset(&document.text, position);
        Some((document, offset, self.check_one(&uri)?))
    }

    /// What the cursor is on.
    ///
    /// The type of the narrowest expression covering it, and failing that, what
    /// the name under it refers to. Both, when the cursor is on a name that is
    /// also an expression, because "`total` is a function" and
    /// "`Fn(Int) -> Int`" answer different halves of the same question.
    ///
    /// Then the two halves of the contract, which is the part
    /// `design/02-syntax.md` promises is always visible and which used to be
    /// visible only in a terminal. On a function's name, what it requires,
    /// performs and guarantees, in the words it was written with:
    /// `crates/deed-ast/src/lib.rs` calls that the review surface. On anything
    /// an obligation covers, which tier that obligation landed in, including
    /// `proven`, because a reader shown nothing cannot tell a proof from a
    /// question nobody asked.
    ///
    /// The tiers are read off [`Checked::obligations`], which the check behind
    /// this hover has already built. Nothing here checks anything again.
    fn hover(&self, message: &Json) -> Json {
        let Some((document, offset, checked)) = self.locate(message) else {
            return Json::Null;
        };

        let mut lines: Vec<String> = Vec::new();
        let mut range = None;

        if let Some((span, ty)) = checked.types.at(offset) {
            lines.push(checked.types.describe(ty));
            range = Some(span);
        }

        if let Some((span, def)) = narrowest_name(&checked, offset) {
            let data = checked.resolutions.def(def);
            lines.push(format!("`{}`, {}", data.name, with_article(data.kind)));
            range = range.or(Some(span));
            if let Some(contract) = contract_declared_at(&checked, &document.text, data.span) {
                lines.push(contract);
            }
        }

        // In source order, which for nested spans puts the outer one first: a
        // call's precondition before the refinement on the argument inside it.
        // Every one of them is true of this position, and which one a reader
        // meant is not a thing to guess.
        let covering: Vec<&ObligationReport> = checked
            .obligations
            .iter()
            .filter(|report| report.span.contains(offset))
            .collect();
        for report in &covering {
            lines.push(format!("`{}`, {}", report.subject, report.tier.name()));
        }
        // An `ensures` clause is not an expression and names nothing the
        // resolver kept, so a cursor on one has no range yet. The narrowest
        // obligation covering it is the thing being hovered.
        if range.is_none() {
            range = covering
                .iter()
                .map(|report| report.span)
                .min_by_key(|span| (span.end - span.start, span.start));
        }

        let Some(range) = range else {
            // Nothing known about this position is not an error, and an editor
            // shows an empty tooltip rather than nothing when told so.
            return Json::Null;
        };

        Json::object(vec![
            (
                "contents",
                Json::object(vec![
                    ("kind", Json::string("markdown")),
                    ("value", Json::string(lines.join("\n\n"))),
                ]),
            ),
            ("range", self.range(document, range)),
        ])
    }

    /// The tier of every obligation in view, written where it was settled.
    ///
    /// A tier is the thing this language has that others do not, and until now
    /// it was visible one at a time, under a cursor. Somebody reading a screen
    /// of code to find out which of it was proven had to hover every call on
    /// it, which is the sort of answer nobody asks for twice.
    ///
    /// `proven` is shown along with the rest, for the same reason `hover` says
    /// it: a reader shown nothing where a proof succeeded cannot tell a proof
    /// from a question nobody asked.
    ///
    /// Read off [`Checked::obligations`], which the check behind this has
    /// already built. Nothing here checks anything again.
    fn inlay_hint(&self, message: &Json) -> Json {
        let Some(uri) = text_document_uri(message) else {
            return Json::Null;
        };
        let Some(document) = self.documents.get(&uri) else {
            return Json::Null;
        };
        let Some(checked) = self.check_one(&uri) else {
            return Json::Null;
        };

        let (start, end) = match range_of(message) {
            Some((start, end)) => (
                document.lines.offset(&document.text, start),
                document.lines.offset(&document.text, end),
            ),
            // The protocol says a range is required. An editor that sends none
            // is asking about the document rather than about nothing.
            None => (0, document.text.len() as u32),
        };

        // Grouped by where the hint goes, because two obligations can end at
        // the same place: a call whose precondition is proven and whose result
        // is a refinement leaves two, and two hints in one column would be
        // drawn as one word with no space in it.
        let mut tiers: BTreeMap<u32, Vec<&'static str>> = BTreeMap::new();
        for report in &checked.obligations {
            if !touches(report.span, start, end) {
                continue;
            }
            let at = tiers.entry(report.span.end).or_default();
            // Two obligations that landed in the same tier at the same place
            // are one thing to say, not two.
            if !at.contains(&report.tier.name()) {
                at.push(report.tier.name());
            }
        }

        Json::Array(
            tiers
                .into_iter()
                .map(|(offset, tiers)| {
                    Json::object(vec![
                        (
                            "position",
                            position(document.lines.position(&document.text, offset)),
                        ),
                        ("label", Json::string(tiers.join(" "))),
                        // `Type` rather than `Parameter`, which are the two the
                        // protocol has. A tier is something worked out about
                        // the code to its left, which is what a type hint is,
                        // rather than the name of a hole it goes into.
                        ("kind", Json::number(1)),
                        ("paddingLeft", Json::Bool(true)),
                    ])
                })
                .collect(),
        )
    }

    /// What could be written where the cursor is.
    ///
    /// Every other question this server answers is about a name that already
    /// exists. This one is about one that does not yet, and the document does
    /// not parse while somebody is typing into it, so the honest thing is to
    /// answer a narrower question: what is in scope here.
    ///
    /// Three shapes, and between them they are most of the value:
    ///
    /// - after a `.`, the fields of whatever is to the left of it
    /// - inside `use a/b.{ }`, what that module exports
    /// - anywhere else, what this file declared, what it imported, and the
    ///   prelude
    ///
    /// Nothing here inserts anything but a name. No snippets, no argument
    /// placeholders, no auto-import. Each of those is a decision about what
    /// somebody meant, and this server has been getting by on answering only
    /// what it was asked.
    fn completion(&self, message: &Json) -> Json {
        let Some(uri) = text_document_uri(message) else {
            return Json::Array(Vec::new());
        };
        let Some(document) = self.documents.get(&uri) else {
            return Json::Array(Vec::new());
        };
        let Some(position) = position_of(message) else {
            return Json::Array(Vec::new());
        };
        let offset = document.lines.offset(&document.text, position) as usize;
        let before = &document.text[..offset.min(document.text.len())];

        // A `use` line is answered from the exporting module rather than from
        // this one, so it takes the workspace path rather than the single file
        // one below. It needs no types, only the other module's names, but the
        // way to reach another module's names is the check that produced them.
        if let Some(module) = importing_from(before) {
            return self.exported_names(&module);
        }

        let Some(checked) = self.check_one(&uri) else {
            return Json::Array(Vec::new());
        };

        match receiver_of(before) {
            Some(receiver) => self.fields_of(&checked, receiver),
            None => self.names_in_scope(&checked, offset as u32),
        }
    }

    /// The fields of whatever ends at `receiver`.
    ///
    /// The receiver's type is what a hover already answers, so this is the
    /// hover machinery with a different question at the end.
    fn fields_of(&self, checked: &Checked, receiver: u32) -> Json {
        let Some((_, ty)) = checked.types.at(receiver) else {
            return Json::Array(Vec::new());
        };

        let mut items = Vec::new();
        for (name, described) in checked.types.members_of(ty) {
            items.push(item(&name, FIELD, &described));
        }
        Json::Array(items)
    }

    /// What the module at `path` offers, for a `use` line.
    ///
    /// The one place a name has to match another file exactly, and so the one
    /// most likely to be wrong when it is typed by hand.
    ///
    /// This is the most expensive completion there is, because the names it
    /// wants belong to another file and the way to have them is to have checked
    /// it. Reading them out of that file's text instead is what this used to
    /// do, and it meant a second, worse answer to the question `check_all`
    /// already answers.
    fn exported_names(&self, path: &str) -> Json {
        let (_, entries) = self.check_workspace();
        let Some(entry) = entries.iter().find(|entry| {
            entry
                .checked
                .module
                .name
                .as_ref()
                .is_some_and(|name| name.to_string_path() == path)
        }) else {
            return Json::Array(Vec::new());
        };

        let mut items = Vec::new();
        for (_, data) in entry.checked.resolutions.defs() {
            if !declares(data.kind) || data.span.is_empty() {
                continue;
            }
            items.push(item(&data.name, kind_of(data.kind), data.kind.describe()));
        }
        items.sort_by_key(label_of);
        items.dedup_by_key(|item| label_of(item));
        Json::Array(items)
    }

    /// Everything nameable at `offset`.
    ///
    /// Module level declarations and imports are in scope everywhere in the
    /// file. A local is offered when it was declared inside the item the
    /// cursor is in and before the cursor, which is an approximation of scope
    /// rather than the real thing: the resolver does not keep its scopes once
    /// it is done, and rebuilding them to rank a list would be a second
    /// resolver.
    fn names_in_scope(&self, checked: &Checked, offset: u32) -> Json {
        let enclosing = enclosing_item(checked, offset);
        let mut items = Vec::new();

        for (_, data) in checked.resolutions.defs() {
            if data.name.is_empty() {
                continue;
            }
            let visible = match data.kind {
                DefKind::Builtin => true,
                kind if declares(kind) => true,
                DefKind::Import => true,
                // A `let`, a parameter, a type parameter, handler state.
                _ => match enclosing {
                    Some(item) => item.contains(data.span.start) && data.span.start <= offset,
                    None => false,
                },
            };
            if !visible {
                continue;
            }
            items.push(item(&data.name, kind_of(data.kind), data.kind.describe()));
        }

        items.sort_by_key(label_of);
        items.dedup_by_key(|item| label_of(item));
        Json::Array(items)
    }

    /// Where the name under the cursor was declared.
    ///
    /// An imported name leads to the file that declares it rather than to the
    /// `use` that brought it in. Nothing carries a `DefId` across the
    /// boundary to do that, because a `DefId` is an index into one module's
    /// table and means nothing outside it. What crosses is what already
    /// crosses everywhere else in this compiler: the module path and the name.
    fn definition(&self, message: &Json) -> Json {
        let Some((document, offset, checked)) = self.locate(message) else {
            return Json::Null;
        };

        let Some((_, def)) = narrowest_name(&checked, offset) else {
            return Json::Null;
        };

        if checked.resolutions.def(def).kind == DefKind::Import
            && let Some(location) = self.imported_definition(&checked, def)
        {
            return location;
        }

        let declared = checked.resolutions.def(def).span;
        // Builtins are declared nowhere. There is no file to open and no line
        // to jump to, and inventing one would land the cursor on whatever
        // happens to be at the top of the file.
        if declared.is_empty() {
            return Json::Null;
        }

        let uri = text_document_uri(message).unwrap_or_default();
        Json::object(vec![
            ("uri", Json::string(uri)),
            ("range", self.range(document, declared)),
        ])
    }

    /// Where an imported name was declared, in the file that declares it.
    ///
    /// `None` when the module is not in the workspace, or does not declare the
    /// name, both of which are already errors the editor is showing. Falling
    /// back to the `use` line is better than answering nothing: it is where a
    /// reader can see what was imported and what the other file is called.
    fn imported_definition(&self, from: &Checked, def: DefId) -> Option<Json> {
        let module = from.resolutions.import_module(def)?.to_string();
        let name = from.resolutions.def(def).name.clone();

        let (sources, entries) = self.check_workspace();
        let entry = entries.iter().find(|entry| {
            entry
                .checked
                .module
                .name
                .as_ref()
                .is_some_and(|path| path.to_string_path() == module)
        })?;

        // Looked up by name in the other module's own table, which is the same
        // identity an export travels under. Only declarations: a `let` in some
        // function body may share the name and is not what was imported.
        let (_, declared) =
            entry.checked.resolutions.defs().find(|(_, data)| {
                data.name == name && declares(data.kind) && !data.span.is_empty()
            })?;

        // A module that ships inside the compiler is not a place to go. There
        // is no file to open, so the answer is the one for a module that is
        // not there at all: the `use` line, where a reader can at least see
        // what was imported and what it came from.
        let uri = entry.uri.as_ref()?;
        let text = sources.file(entry.checked.file).text();
        Some(Json::object(vec![
            ("uri", Json::string(uri)),
            ("range", range_in(text, declared.span)),
        ]))
    }

    /// Every place a name is used, across the workspace.
    ///
    /// The list rename edits, as a list. Sharing the walk rather than writing
    /// it twice is the point: two implementations of "everywhere this name is
    /// written" would agree today and stop agreeing on whichever file nobody
    /// tested.
    fn references(&self, message: &Json) -> Json {
        let wanted = message
            .at(&["params", "context", "includeDeclaration"])
            .map(|value| value == &Json::Bool(true))
            .unwrap_or(true);

        let Some((sources, found)) = self.occurrences(message, wanted) else {
            return Json::Array(Vec::new());
        };

        Json::Array(
            found
                .iter()
                .flat_map(|file| {
                    let text = sources.file(file.file).text();
                    file.spans.iter().map(move |span| {
                        Json::object(vec![
                            ("uri", Json::string(&file.uri)),
                            ("range", range_in(text, *span)),
                        ])
                    })
                })
                .collect(),
        )
    }

    /// Every place the name under the cursor is written, across the workspace.
    ///
    /// Two questions wearing one name. A parameter or a `let` cannot leave the
    /// module that declared it, so the answer is every span in this file that
    /// resolves to that one definition, and it is exact. A declaration another
    /// module can import has no single definition to compare against, because
    /// a `DefId` is an index into one module's table, so the answer is
    /// assembled the same way go to definition crosses the boundary: by the
    /// module path and the name.
    ///
    /// `None` when the question has no answer at all, which is a different
    /// thing from an answer with nothing in it and is what lets rename refuse
    /// rather than quietly do nothing.
    fn occurrences(
        &self,
        message: &Json,
        include_declaration: bool,
    ) -> Option<(SourceMap, Vec<Found>)> {
        let (_, offset, checked) = self.locate(message)?;
        let (_, def) = narrowest_name(&checked, offset)?;

        let data = checked.resolutions.def(def);
        // Declared nowhere and used everywhere. Listing every mention of
        // `length` in a workspace is not an answer to any question somebody
        // was asking, and renaming one is not a thing this workspace can do.
        if data.kind == DefKind::Builtin {
            return None;
        }

        let name = data.name.clone();
        let here = checked
            .module
            .name
            .as_ref()
            .map(|path| path.to_string_path());
        let home = match data.kind {
            DefKind::Import => checked.resolutions.import_module(def).map(str::to_string),
            // A declaration another module could import. A file with no
            // `module` line cannot be imported, so it has no name to be known
            // by and the question stays local.
            kind if declares(kind) => here,
            // A parameter, a `let`, a `state` field. It cannot be named
            // anywhere else, so nothing outside this module can be an answer.
            _ => None,
        };

        let (sources, entries) = self.check_workspace();
        let mut found = Vec::new();

        for entry in &entries {
            // A module that ships inside the compiler is not a place. Nothing
            // written in it is somewhere to jump to, and nothing written in it
            // is this workspace's to rewrite: renaming an export of
            // `std/list` here would edit the caller and leave the declaration
            // alone, in a file that is inside the binary.
            let Some(uri) = &entry.uri else {
                continue;
            };
            let here = entry
                .checked
                .module
                .name
                .as_ref()
                .map(|path| path.to_string_path());

            // Which definition in this file the question is about, if any.
            let local = match &home {
                // Not exportable, so only the file it was asked from can hold
                // an answer, and the definition is the one already in hand.
                None => (*uri == text_document_uri(message).unwrap_or_default()).then_some(def),
                Some(home) if here.as_deref() == Some(home.as_str()) => entry
                    .checked
                    .resolutions
                    .defs()
                    .find(|(_, data)| data.name == name && declares(data.kind))
                    .map(|(id, _)| id),
                // Somewhere else: the `use` that brought the name in, if it
                // brought this one in and not another module's name of the
                // same spelling.
                Some(home) => entry
                    .checked
                    .resolutions
                    .defs()
                    .find(|(id, data)| {
                        data.kind == DefKind::Import
                            && data.name == name
                            && entry.checked.resolutions.import_module(*id) == Some(home.as_str())
                    })
                    .map(|(id, _)| id),
            };
            let Some(local) = local else {
                continue;
            };

            let declared = entry.checked.resolutions.def(local).span;
            let mut spans: Vec<Span> = entry
                .checked
                .resolutions
                .names()
                .filter(|(span, id)| *id == local && *span != declared)
                .map(|(span, _)| span)
                .collect();
            if include_declaration && !declared.is_empty() {
                spans.push(declared);
            }
            // Source order, so an editor's list reads down the file rather
            // than in whatever order a hash map handed things back.
            spans.sort_by_key(|span| (span.start, span.end));

            if !spans.is_empty() {
                found.push(Found {
                    uri: uri.clone(),
                    file: entry.checked.file,
                    spans,
                });
            }
        }

        Some((sources, found))
    }

    /// Whether the thing under the cursor can be renamed, and where its name
    /// is written.
    ///
    /// An editor asks this before showing the box, so that a name nothing can
    /// be done about is greyed out rather than accepting a new spelling and
    /// then doing nothing with it.
    fn prepare_rename(&self, message: &Json) -> Json {
        let Some((document, offset, checked)) = self.locate(message) else {
            return Json::Null;
        };
        let Some((span, def)) = narrowest_name(&checked, offset) else {
            return Json::Null;
        };
        // The prelude belongs to the language rather than to this workspace.
        if checked.resolutions.def(def).kind == DefKind::Builtin {
            return Json::Null;
        }
        self.range(document, span)
    }

    /// One name, everywhere it is written, replaced.
    ///
    /// The same set references answers with, always including the declaration,
    /// which is the difference between the two questions. It crosses module
    /// boundaries, so renaming an exported function rewrites the `use` line
    /// that brought it in as well: a rename that edited one file and broke
    /// another would be worse than no rename at all.
    ///
    /// What this deliberately does not do is decide whether the new name is a
    /// good one. A name that collides with something already in scope, or that
    /// shadows a prelude entry, is something the checker already has a
    /// diagnostic for, and answering it twice is how the two answers come to
    /// disagree.
    fn rename(&self, id: &Json, message: &Json) -> Json {
        let Some(name) = message
            .at(&["params", "newName"])
            .and_then(Json::as_str)
            .map(str::to_string)
        else {
            return error(id, INVALID_REQUEST, "a rename needs a new name");
        };
        if !is_identifier(&name) {
            return error(
                id,
                INVALID_REQUEST,
                &format!("`{name}` is not a name this language can hold"),
            );
        }

        let Some((sources, found)) = self.occurrences(message, true) else {
            return error(
                id,
                INVALID_REQUEST,
                "there is nothing here that can be renamed",
            );
        };

        let changes: Vec<(String, Json)> = found
            .iter()
            .map(|file| {
                let text = sources.file(file.file).text();
                let edits = file
                    .spans
                    .iter()
                    .map(|span| {
                        Json::object(vec![
                            ("range", range_in(text, *span)),
                            ("newText", Json::string(&name)),
                        ])
                    })
                    .collect();
                (file.uri.clone(), Json::Array(edits))
            })
            .collect();

        result(
            id,
            Json::object(vec![(
                "changes",
                Json::Object(changes.into_iter().collect()),
            )]),
        )
    }

    /// The whole document, replaced by its canonical form.
    ///
    /// One edit rather than a minimal diff. The formatter answers with a whole
    /// file, working out which parts of it changed would be a second
    /// implementation of the same idea, and an editor collapses a replacement
    /// that changes nothing into nothing.
    fn formatting(&self, message: &Json) -> Json {
        let Some(uri) = text_document_uri(message) else {
            return Json::Null;
        };
        let Some(document) = self.documents.get(&uri) else {
            return Json::Null;
        };

        let mut sources = SourceMap::new();
        let file = sources.add(uri, document.text.clone());
        // A file that does not parse is left alone. Reshaping one is guessing
        // at what was meant, and the guess would land in the working tree the
        // moment the editor saves.
        let Ok(formatted) = deed_fmt::format(file, &document.text) else {
            return Json::Array(Vec::new());
        };
        if formatted == document.text {
            return Json::Array(Vec::new());
        }

        let whole = Span::new(0, document.text.len() as u32);
        Json::Array(vec![Json::object(vec![
            ("range", self.range(document, whole)),
            ("newText", Json::string(formatted)),
        ])])
    }

    /// The fixes the diagnostics over a range are carrying.
    ///
    /// P7 says a diagnostic carries an applicable patch where the repair is
    /// unambiguous, and the lexer, the parser, the resolver and the row
    /// diagnostics all write one. Until now the only thing that could reach
    /// them was `deed fix` on a command line, which applies the
    /// machine-applicable ones and skips the rest, so the guesses were data
    /// nothing could ever read and the certain ones were only offered to
    /// somebody who had already left the editor. "cannot find `lenght`, did
    /// you mean `length`" knew the answer and made the reader type it.
    ///
    /// Which diagnostics: the ones whose primary span touches the range the
    /// editor asked about. A zero width range is a cursor, and a cursor
    /// resting at either end of a span counts, because that is where a cursor
    /// sits when someone has just finished typing the word.
    ///
    /// The diagnostics come from re-checking rather than from
    /// `context.diagnostics`. The editor's copy is whatever it was last told,
    /// and a fix whose span was worked out against older text edits the wrong
    /// bytes.
    ///
    /// [`Applicability`] carries straight over. A machine-applicable fix is
    /// the one `deed fix` would apply without being asked, so it is the
    /// preferred action; a guess is offered and never preferred. The editor
    /// already has a word for that distinction and this server already had the
    /// distinction, so nothing new is being decided here.
    fn code_action(&self, message: &Json) -> Json {
        let Some(uri) = text_document_uri(message) else {
            return Json::Array(Vec::new());
        };
        let Some(document) = self.documents.get(&uri) else {
            return Json::Array(Vec::new());
        };
        if !wants_quickfixes(message) {
            return Json::Array(Vec::new());
        }
        let Some(range) = range_of(message) else {
            return Json::Array(Vec::new());
        };
        let start = document.lines.offset(&document.text, range.0);
        let end = document.lines.offset(&document.text, range.1);
        // The whole workspace rather than one file, out of the same single
        // check: an action carries the diagnostic it answers, and that
        // diagnostic can have a label about another file in it.
        let (sources, entries) = self.check_workspace();
        let Some(checked) = entries
            .iter()
            .find(|entry| entry.uri.as_deref() == Some(uri.as_str()))
            .map(|entry| &entry.checked)
        else {
            return Json::Array(Vec::new());
        };
        let elsewhere = reachable(&sources, &entries);

        let actions: Vec<Json> = checked
            .diagnostics
            .iter()
            // A diagnostic about another file can be reported here, and its
            // fix edits bytes this document does not have.
            .filter(|diagnostic| diagnostic.file == checked.file)
            .filter(|diagnostic| touches(diagnostic.primary.span, start, end))
            .filter_map(|diagnostic| {
                let fix = diagnostic.fix.as_ref()?;
                let edits: Vec<Json> = fix
                    .edits
                    .iter()
                    .map(|edit| {
                        Json::object(vec![
                            ("range", self.range(document, edit.span)),
                            ("newText", Json::string(&edit.replacement)),
                        ])
                    })
                    .collect();
                Some(Json::object(vec![
                    ("title", Json::string(&fix.message)),
                    ("kind", Json::string("quickfix")),
                    (
                        "diagnostics",
                        Json::Array(vec![self.render(document, diagnostic, &uri, &elsewhere)]),
                    ),
                    (
                        "isPreferred",
                        Json::Bool(fix.applicability == Applicability::MachineApplicable),
                    ),
                    (
                        "edit",
                        Json::object(vec![(
                            "changes",
                            Json::object(vec![(uri.as_str(), Json::Array(edits))]),
                        )]),
                    ),
                ]))
            })
            .collect();

        Json::Array(actions)
    }

    /// What the file declares, in the order it declares it.
    ///
    /// The outline, the breadcrumbs and the jump-to-symbol list are all this
    /// one answer. Nested rather than flat, because a variant belongs to its
    /// choice and a handler operation belongs to its handler, and a list that
    /// said otherwise would be a worse version of scrolling.
    ///
    /// Read off the parse tree rather than off the resolutions, so a file
    /// being typed into still has an outline. Everything else this server
    /// answers is about a name that resolved; an outline is about what is
    /// written, and half a declaration is still worth drawing.
    ///
    /// Two ranges each, and the difference between them is the point. `range`
    /// is the whole declaration, which is what an editor highlights when you
    /// pick it. `selectionRange` is the name, which is where the cursor lands.
    /// The signature of the call the cursor is inside, and which argument it
    /// is on.
    ///
    /// The whole thesis of the language is that the signature is the contract,
    /// and a call site is the one place a reader is deciding whether they can
    /// keep it. So this answers with the row as well as the types: what a call
    /// performs is part of what it costs, and it is not written at the call.
    ///
    /// Read off the text rather than the tree, for the reason completion is:
    /// `f(` with nothing after it does not parse, and that is exactly the
    /// moment somebody wants this.
    fn signature_help(&self, message: &Json) -> Json {
        let Some((document, offset, checked)) = self.locate(message) else {
            return Json::Null;
        };
        let before = &document.text[..(offset as usize).min(document.text.len())];
        let Some((callee, active)) = call_site(before) else {
            return Json::Null;
        };
        let Some((_, Ty::Fn { params, row, ret })) = checked.types.at(callee) else {
            // Not a call of anything with a signature. A name the checker gave
            // up on lands here too, and saying nothing is better than
            // describing a type that is already wrong.
            return Json::Null;
        };

        let name = narrowest_name(&checked, callee)
            .map(|(_, def)| checked.resolutions.def(def).name.to_string())
            .unwrap_or_else(|| "fn".to_string());

        // Built a piece at a time because the protocol points at a parameter
        // by offset into the label, and counting those afterwards would mean
        // parsing back what was just written.
        let mut label = format!("{name}(");
        let mut ranges = Vec::new();
        for (index, param) in params.iter().enumerate() {
            if index > 0 {
                label.push_str(", ");
            }
            let start = label.chars().count();
            label.push_str(&checked.types.bare(param));
            ranges.push((start, label.chars().count()));
        }
        label.push(')');
        if let FnRow::Declared(entries) = row
            && !entries.is_empty()
        {
            let named: Vec<String> = entries
                .iter()
                .map(|entry| match &entry.operation {
                    Some(operation) => format!("{}.{operation}", entry.effect),
                    None => entry.effect.clone(),
                })
                .collect();
            label.push_str(&format!(" uses {}", named.join(", ")));
        }
        label.push_str(&format!(" -> {}", checked.types.bare(ret)));

        let parameters = ranges
            .into_iter()
            .map(|(start, end)| {
                Json::object(vec![(
                    "label",
                    Json::Array(vec![Json::number(start as i64), Json::number(end as i64)]),
                )])
            })
            .collect();

        Json::object(vec![
            (
                "signatures",
                Json::Array(vec![Json::object(vec![
                    ("label", Json::string(&label)),
                    ("parameters", Json::Array(parameters)),
                ])]),
            ),
            ("activeSignature", Json::number(0)),
            // Past the last parameter on a call with too many arguments. The
            // protocol says an out of range index is ignored, which is the
            // right answer: the extra argument is a mistake the checker
            // reports, and highlighting nothing is how this says it has
            // nothing left to offer.
            ("activeParameter", Json::number(active as i64)),
        ])
    }

    fn document_symbol(&self, message: &Json) -> Json {
        let Some(uri) = text_document_uri(message) else {
            return Json::Null;
        };
        let Some(document) = self.documents.get(&uri) else {
            return Json::Null;
        };

        let mut sources = SourceMap::new();
        let file = sources.add(uri, document.text.clone());
        let lexed = deed_lexer::tokenize(file, &document.text);
        let parsed = deed_parser::parse(file, &lexed.tokens);

        let symbols = parsed
            .module
            .items
            .iter()
            .map(|item| self.symbol_of(document, item))
            .collect();
        Json::Array(symbols)
    }

    fn symbol_of(&self, document: &Document, item: &Item) -> Json {
        match item {
            // The protocol has fewer kinds than this language has
            // declarations. A `type` is a named type with nothing to list
            // inside it, which is the closest thing to an interface the
            // protocol offers, and an `effect` is signatures with no bodies,
            // which is the other thing that word means. Sharing one is more
            // honest than picking a different icon for one of them because it
            // looks different.
            Item::TypeAlias(decl) => self.symbol(
                document,
                &decl.name.name,
                kind::INTERFACE,
                decl.span,
                decl.name.span,
                Vec::new(),
            ),
            Item::Record(decl) => self.symbol(
                document,
                &decl.name.name,
                kind::STRUCT,
                decl.span,
                decl.name.span,
                decl.fields
                    .iter()
                    .map(|field| {
                        self.symbol(
                            document,
                            &field.name.name,
                            kind::FIELD,
                            field.span,
                            field.name.span,
                            Vec::new(),
                        )
                    })
                    .collect(),
            ),
            Item::Choice(decl) => self.symbol(
                document,
                &decl.name.name,
                kind::ENUM,
                decl.span,
                decl.name.span,
                decl.variants
                    .iter()
                    .map(|variant| {
                        self.symbol(
                            document,
                            &variant.name.name,
                            kind::ENUM_MEMBER,
                            variant.span,
                            variant.name.span,
                            Vec::new(),
                        )
                    })
                    .collect(),
            ),
            Item::Effect(decl) => self.symbol(
                document,
                &decl.name.name,
                kind::INTERFACE,
                decl.span,
                decl.name.span,
                decl.operations
                    .iter()
                    .map(|operation| {
                        self.symbol(
                            document,
                            &operation.name.name,
                            kind::METHOD,
                            operation.span,
                            operation.name.span,
                            Vec::new(),
                        )
                    })
                    .collect(),
            ),
            // A handler is the one declaration with an implementation in it,
            // which is what a class is.
            Item::Handler(decl) => {
                let mut children: Vec<Json> = decl
                    .state
                    .iter()
                    .map(|field| {
                        self.symbol(
                            document,
                            &field.name.name,
                            kind::FIELD,
                            field.span,
                            field.name.span,
                            Vec::new(),
                        )
                    })
                    .collect();
                children.extend(decl.operations.iter().map(|operation| {
                    self.symbol(
                        document,
                        &operation.sig.name.name,
                        kind::METHOD,
                        operation.span,
                        operation.sig.name.span,
                        Vec::new(),
                    )
                }));
                self.symbol(
                    document,
                    &decl.name.name,
                    kind::CLASS,
                    decl.span,
                    decl.name.span,
                    children,
                )
            }
            Item::Function(decl) => self.symbol(
                document,
                &decl.sig.name.name,
                kind::FUNCTION,
                decl.span,
                decl.sig.name.span,
                Vec::new(),
            ),
            // Named by a string, so the name is quoted here too. An outline
            // that dropped the quotes would be showing a name nothing in the
            // file is spelled that way.
            Item::Test(decl) => self.symbol(
                document,
                &format!("test {:?}", decl.name),
                kind::FUNCTION,
                decl.span,
                decl.name_span,
                Vec::new(),
            ),
        }
    }

    /// Every declaration in the workspace whose name contains the query.
    ///
    /// The outline answers "what is in this file" and this answers "where is
    /// the thing I can only remember the name of", which is the question
    /// somebody has when they arrive in a workspace they did not write.
    ///
    /// Substring and case insensitive rather than fuzzy. An editor filters and
    /// ranks the answer again on its own terms, so a server that guessed at
    /// ranking would be guessing twice, and a substring is the one rule a
    /// person can predict without being told it.
    ///
    /// An empty query is every declaration. That is what an editor sends when
    /// the box is first opened, and answering it is how the list appears
    /// before anything is typed.
    fn workspace_symbol(&self, message: &Json) -> Json {
        let query = message
            .at(&["params", "query"])
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_lowercase();

        let (sources, entries) = self.check_workspace();
        let mut symbols = Vec::new();

        for entry in &entries {
            // A module that ships inside the compiler is not somewhere this
            // workspace can be navigated to, so it is not in the box that
            // exists to navigate one.
            let Some(uri) = &entry.uri else {
                continue;
            };
            let text = sources.file(entry.checked.file).text();
            let container = entry
                .checked
                .module
                .name
                .as_ref()
                .map(|path| path.to_string_path());

            for (_, data) in entry.checked.resolutions.defs() {
                // A parameter or a `let` cannot be named from outside the
                // function that has it, so it is not something to jump to
                // across a workspace. An empty span is a name the compiler
                // supplied rather than one somebody wrote.
                if !declares(data.kind) || data.span.is_empty() {
                    continue;
                }
                if !data.name.to_lowercase().contains(&query) {
                    continue;
                }

                let mut fields = vec![
                    ("name", Json::string(&data.name)),
                    ("kind", Json::number(kind_of(data.kind))),
                    (
                        "location",
                        Json::object(vec![
                            ("uri", Json::string(uri)),
                            ("range", range_in(text, data.span)),
                        ]),
                    ),
                ];
                if let Some(container) = &container {
                    fields.push(("containerName", Json::string(container)));
                }
                symbols.push((data.name.to_string(), uri.clone(), Json::object(fields)));
            }
        }

        // By name, then by where it is, so that two files declaring the same
        // name come back in an order that does not move between calls.
        symbols.sort_by(|left, right| (&left.0, &left.1).cmp(&(&right.0, &right.1)));
        Json::Array(symbols.into_iter().map(|(_, _, symbol)| symbol).collect())
    }

    fn symbol(
        &self,
        document: &Document,
        name: &str,
        kind: i64,
        span: Span,
        selection: Span,
        children: Vec<Json>,
    ) -> Json {
        Json::object(vec![
            ("name", Json::string(name)),
            ("kind", Json::number(kind)),
            ("range", self.range(document, span)),
            ("selectionRange", self.range(document, selection)),
            ("children", Json::Array(children)),
        ])
    }

    fn notification(&mut self, method: &str, message: &Json) -> Vec<Json> {
        match method {
            "textDocument/didOpen" => {
                let Some(uri) = text_document_uri(message) else {
                    return Vec::new();
                };
                let text = message
                    .at(&["params", "textDocument", "text"])
                    .and_then(Json::as_str)
                    .unwrap_or_default()
                    .to_string();
                self.documents.insert(uri.clone(), Document::new(text));
                self.published()
            }
            "textDocument/didChange" => {
                let Some(uri) = text_document_uri(message) else {
                    return Vec::new();
                };
                // Full sync only. Incremental sync is an optimisation, and an
                // optimisation that can desynchronise the server from the file
                // on disk is not one to take before it is measured.
                let Some(text) = message
                    .at(&["params", "contentChanges"])
                    .and_then(Json::as_array)
                    .and_then(|changes| changes.last())
                    .and_then(|change| change.get("text"))
                    .and_then(Json::as_str)
                else {
                    return Vec::new();
                };
                self.documents
                    .insert(uri.clone(), Document::new(text.to_string()));
                self.published()
            }
            "textDocument/didClose" => {
                let Some(uri) = text_document_uri(message) else {
                    return Vec::new();
                };
                self.documents.remove(&uri);
                // An empty list, not silence. A closed file whose squiggles
                // stayed behind is a file the editor keeps complaining about
                // forever.
                let mut sent = vec![publish(&uri, Vec::new())];
                // What is left may say something different now: the file on
                // disk takes over from the buffer that just went away.
                sent.extend(self.published());
                sent
            }
            _ => Vec::new(),
        }
    }

    /// Diagnostics for every open document.
    ///
    /// All of them, not just the one being typed in. A change in one file
    /// changes what another says: adding an export fixes an import somewhere
    /// else and removing one breaks it, and squiggles left describing a
    /// version of the workspace that no longer exists are worse than none.
    ///
    /// Only the open ones. A closed file's problems belong in the panel of a
    /// tool that was asked about the whole project, and an editor given
    /// diagnostics for a document it never opened has nowhere obvious to put
    /// them and no event that clears them.
    fn published(&self) -> Vec<Json> {
        let (sources, entries) = self.check_workspace();
        self.publish_all(&sources, &entries)
    }

    /// The messages for a set of already checked entries.
    ///
    /// Split out from [`Self::published`] so that a test can hand it an entry
    /// with a problem in it. Nothing that ships inside the compiler fails to
    /// check today, and waiting for the day one does is not a test.
    fn publish_all(&self, sources: &SourceMap, entries: &[Entry]) -> Vec<Json> {
        let elsewhere = reachable(sources, entries);
        entries
            .iter()
            .filter_map(|entry| {
                // A module that ships inside the compiler has no URI, so it
                // never gets past here. That is the answer to what happens
                // when one of them fails to check: nothing is said to the
                // person. It is not their file, they cannot open it and they
                // cannot fix it, and a squiggle they can do nothing about on a
                // document they never opened is worse than none. A shipped
                // module that stops checking is a broken compiler, and
                // `crates/deed-driver/tests/shipped.rs` is what fails then.
                let uri = entry.uri.as_ref()?;
                let document = self.documents.get(uri)?;
                let reported = entry
                    .checked
                    .diagnostics
                    .iter()
                    .map(|diagnostic| self.render(document, diagnostic, uri, &elsewhere))
                    .collect();
                Some(publish(uri, reported))
            })
            .collect()
    }

    /// Checks every open document together with the rest of the workspace.
    ///
    /// One pass rather than one per document. They are all in the same set of
    /// files, so checking them separately would be the same work several times
    /// over and could produce two answers that disagree about the same import.
    ///
    /// An open document's text comes from the editor rather than from disk,
    /// because the buffer is what the person is looking at and the file behind
    /// it may not have been saved.
    ///
    /// The modules that ship inside the compiler go on the end, after every
    /// file the person could open has been offered, so their own
    /// `std/string.deed` still wins. That is the command line tool's rule and
    /// literally its code: `deed check` accepted `use std/list` on three files
    /// in this repository while the editor put `DEED3007` under the same line,
    /// because this used to be the set of files nobody had told about them.
    ///
    /// The [`SourceMap`] comes back with the results because a question about
    /// one file can be answered in another, and turning a span in that other
    /// file into a range needs its text.
    fn check_workspace(&self) -> (SourceMap, Vec<Entry>) {
        // Held before anything is checked, because which modules ship inside
        // the compiler is a question about all of them at once.
        let mut pending: Vec<(String, Option<String>, String)> = Vec::new();
        let mut open: Vec<Option<PathBuf>> = Vec::new();

        for (uri, document) in &self.documents {
            pending.push((uri.clone(), Some(uri.clone()), document.text.clone()));
            open.push(uri::to_path(uri).map(|path| canonical(&path)));
        }

        for path in self.workspace.files() {
            // Compared in canonical form, because the same file arrives from
            // two directions and they differ on Windows. Named in the form the
            // walk produced, because that is the form the editor gave us the
            // folder in, and a canonical path on Windows is a verbatim one
            // that no editor recognises.
            if open
                .iter()
                .any(|other| other.as_ref() == Some(&canonical(&path)))
            {
                continue;
            }
            // A file that cannot be read is not a reason to stop saying
            // anything about the one on the screen.
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            pending.push((
                path.display().to_string(),
                Some(uri::from_path(&path)),
                text,
            ));
        }

        for module in deed_driver::shipped_for(pending.iter().map(|(_, _, text)| text.as_str())) {
            let Some(text) = deed_driver::shipped_source(module) else {
                continue;
            };
            pending.push((format!("<shipped>/{module}.deed"), None, text.to_string()));
        }

        let mut sources = SourceMap::new();
        let mut ids = Vec::new();
        let mut uris = Vec::new();
        for (name, uri, text) in pending {
            ids.push(sources.add(name, text));
            uris.push(uri);
        }

        let entries = uris
            .into_iter()
            .zip(deed_driver::check_all(&sources, &ids))
            .map(|(uri, checked)| Entry { uri, checked })
            .collect();

        (sources, entries)
    }

    /// The same, for one document.
    fn check_one(&self, uri: &str) -> Option<Checked> {
        let (_, checked) = self.check_workspace();
        checked
            .into_iter()
            .find(|entry| entry.uri.as_deref() == Some(uri))
            .map(|entry| entry.checked)
    }

    /// One diagnostic, as the protocol wants it.
    ///
    /// `elsewhere` is every file the editor could be sent to, so that a
    /// secondary label about another one lands there instead of being dropped
    /// or, worse, drawn over whatever sits at those byte offsets in this
    /// document. A module that ships inside the compiler is not in that list
    /// and its labels are left out, for the same reason nothing else leads
    /// into one: there is no file to open.
    fn render(
        &self,
        document: &Document,
        diagnostic: &Diagnostic,
        uri: &str,
        elsewhere: &[(FileId, &str, &str)],
    ) -> Json {
        let mut message = diagnostic.message.clone();
        // The notes are where a diagnostic explains itself, and a hover that
        // dropped them would be a worse version of the terminal output.
        for note in &diagnostic.notes {
            message.push_str("\n\nnote: ");
            message.push_str(note);
        }

        let related: Vec<Json> = diagnostic
            .secondary
            .iter()
            .filter_map(|label| {
                let location = match label.file {
                    Some(other) if other != diagnostic.file => {
                        let (_, other_uri, text) =
                            elsewhere.iter().find(|(id, _, _)| *id == other)?;
                        Json::object(vec![
                            ("uri", Json::string(*other_uri)),
                            ("range", range_in(text, label.span)),
                        ])
                    }
                    _ => Json::object(vec![
                        ("uri", Json::string(uri)),
                        ("range", self.range(document, label.span)),
                    ]),
                };
                Some(Json::object(vec![
                    ("location", location),
                    ("message", Json::string(&label.message)),
                ]))
            })
            .collect();

        Json::object(vec![
            ("range", self.range(document, diagnostic.primary.span)),
            (
                "severity",
                Json::number(match diagnostic.severity {
                    Severity::Error => 1,
                    Severity::Warning => 2,
                }),
            ),
            ("code", Json::string(diagnostic.code)),
            ("source", Json::string("deed")),
            ("message", Json::string(message)),
            ("relatedInformation", Json::Array(related)),
        ])
    }

    fn range(&self, document: &Document, span: Span) -> Json {
        let start = document.lines.position(&document.text, span.start);
        let end = document.lines.position(&document.text, span.end);
        Json::object(vec![("start", position(start)), ("end", position(end))])
    }
}

/// The same, for a file the server has no open document for.
///
/// Builds the line index on the spot, because this happens once per jump into
/// another file rather than once per keystroke.
fn range_in(text: &str, span: Span) -> Json {
    let lines = Lines::of(text);
    Json::object(vec![
        ("start", position(lines.position(text, span.start))),
        ("end", position(lines.position(text, span.end))),
    ])
}

/// Every checked file the editor could be pointed at, with its text.
///
/// A module that ships inside the compiler is left out. It has no URI because
/// there is no file behind it, and that is the same answer go to definition,
/// rename and workspace symbol already give: a location nobody can open is
/// worse than none.
fn reachable<'a>(sources: &'a SourceMap, entries: &'a [Entry]) -> Vec<(FileId, &'a str, &'a str)> {
    entries
        .iter()
        .filter_map(|entry| {
            let uri = entry.uri.as_deref()?;
            let file = entry.checked.file;
            Some((file, uri, sources.file(file).text()))
        })
        .collect()
}

/// Whether a kind of definition is something a module can export.
///
/// A `let` in some function body can share a name with an imported one, and
/// jumping to it would be an answer about a different thing entirely.
fn declares(kind: DefKind) -> bool {
    matches!(
        kind,
        DefKind::Type
            | DefKind::Record
            | DefKind::Choice
            | DefKind::Variant
            | DefKind::Effect
            | DefKind::EffectOp
            | DefKind::Handler
            | DefKind::Function
    )
}

/// Whether some text is a name this language can hold.
///
/// Asked of the lexer rather than answered again here. A second definition of
/// what an identifier is would agree with the first until one of them changed,
/// and the failure would be a rename that produces a file that does not parse.
fn is_identifier(text: &str) -> bool {
    let mut sources = SourceMap::new();
    let file = sources.add("<rename>", text);
    let lexed = deed_lexer::tokenize(file, text);
    if lexed.has_errors() {
        return false;
    }
    // One identifier and the end of the file. A keyword lexes as a keyword
    // rather than as a name, so this refuses `fn` without holding a second
    // copy of the keyword list.
    matches!(
        lexed
            .tokens
            .iter()
            .map(|token| &token.kind)
            .collect::<Vec<_>>()
            .as_slice(),
        [deed_lexer::TokenKind::Ident(_), deed_lexer::TokenKind::Eof]
    )
}

/// LSP completion item kinds, by the numbers the protocol gives them.
const FUNCTION: i64 = 3;
const FIELD: i64 = 5;
const VARIABLE: i64 = 6;
const INTERFACE: i64 = 8;
const ENUM: i64 = 13;
const ENUM_MEMBER: i64 = 20;
const STRUCT: i64 = 22;
const TYPE_PARAMETER: i64 = 25;

fn item(label: &str, kind: i64, detail: &str) -> Json {
    Json::object(vec![
        ("label", Json::string(label)),
        ("kind", Json::number(kind)),
        ("detail", Json::string(detail)),
    ])
}

fn label_of(item: &Json) -> String {
    item.at(&["label"])
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_string()
}

/// What an editor should draw next to a name.
///
/// The protocol's own list, so a record looks like a struct and a choice looks
/// like an enum wherever the editor already has an icon for one.
fn kind_of(kind: DefKind) -> i64 {
    match kind {
        DefKind::Function | DefKind::EffectOp | DefKind::Builtin => FUNCTION,
        DefKind::Record | DefKind::Handler => STRUCT,
        DefKind::Choice => ENUM,
        DefKind::Variant => ENUM_MEMBER,
        DefKind::Effect => INTERFACE,
        DefKind::TypeParam | DefKind::RowParam | DefKind::Type => TYPE_PARAMETER,
        // An import is whatever it imported, and saying so would mean reading
        // the other module. The name is what somebody is typing.
        DefKind::Import => VARIABLE,
        DefKind::Param | DefKind::Local | DefKind::State => VARIABLE,
    }
}

/// The name being called and the argument index, for the innermost call the
/// cursor is inside.
///
/// A scan rather than a walk of the tree, because the text this is asked
/// about is `f(` and half an argument, which does not parse.
///
/// Nesting is a stack rather than a counter so that a comma inside `[1, 2]`
/// or inside another call belongs to whatever opened it. Strings and line
/// comments are skipped, or a `(` in a message would open a call that never
/// closes and every later answer would be wrong.
fn call_site(before: &str) -> Option<(u32, usize)> {
    let bytes = before.as_bytes();
    // (opening byte, offset of it, commas seen at this level)
    let mut open: Vec<(u8, usize, usize)> = Vec::new();
    let mut at = 0;

    while at < bytes.len() {
        match bytes[at] {
            b'"' => {
                at += 1;
                while at < bytes.len() && bytes[at] != b'"' {
                    // A quote that was escaped is inside the string, not the
                    // end of it.
                    at += if bytes[at] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if bytes.get(at + 1) == Some(&b'/') => {
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
                continue;
            }
            opener @ (b'(' | b'[' | b'{') => open.push((opener, at, 0)),
            b')' | b']' | b'}' => {
                open.pop();
            }
            b',' => {
                if let Some(frame) = open.last_mut() {
                    frame.2 += 1;
                }
            }
            _ => {}
        }
        at += 1;
    }

    // The innermost unclosed `(`. Anything nested inside it is still that
    // call's argument, so a cursor inside `f(g(x), [1, |2])` is on `f`'s
    // second argument and on `[`'s second element, and the call is the one
    // with a signature to show.
    let (_, paren, commas) = *open.iter().rev().find(|(byte, _, _)| *byte == b'(')?;

    // What is being called ends just before the bracket, allowing for the
    // space somebody may have left. A `(` with no name in front of it is a
    // grouping, not a call.
    let name_end = before[..paren].trim_end().len();
    let last = before[..name_end].chars().next_back()?;
    if !last.is_alphanumeric() && last != '_' {
        return None;
    }

    Some((name_end as u32 - 1, commas))
}

/// The end of the expression a `.` was typed after.
///
/// `None` when the text does not end in a name preceded by a dot, which is
/// every other position. The offset points one before the dot, which is inside
/// the receiver, because that is what the type table is keyed by.
fn receiver_of(before: &str) -> Option<u32> {
    let trimmed = before.trim_end_matches(|c: char| c.is_alphanumeric() || c == '_');
    let dot = trimmed.strip_suffix('.')?;
    // `1.` is not a receiver, it is a number somebody is still typing, and
    // there are no float literals to make it one.
    let last = dot.chars().next_back()?;
    if !last.is_alphanumeric() && last != '_' && last != ')' && last != ']' {
        return None;
    }
    Some(dot.len() as u32 - 1)
}

/// The module path of a `use` line the cursor is inside the braces of.
///
/// Read off the text rather than the tree, because a `use a/b.{ }` with
/// nothing in it yet is exactly the shape that does not parse.
fn importing_from(before: &str) -> Option<String> {
    let line = before.rsplit('\n').next()?;
    let rest = line.trim_start().strip_prefix("use ")?;
    let (path, names) = rest.split_once(".{")?;
    // A closed brace means the cursor is past the list, so this is not it.
    if names.contains('}') {
        return None;
    }
    let path = path.trim();
    (!path.is_empty()).then(|| path.to_string())
}

/// The span of the declaration the cursor is inside.
///
/// Used to decide which locals to offer. An offset outside every item, such as
/// a blank line between two functions, has none, and nothing local is in scope
/// there anyway.
fn enclosing_item(checked: &Checked, offset: u32) -> Option<Span> {
    checked
        .module
        .items
        .iter()
        .map(|item| item.span())
        .filter(|span| span.contains(offset))
        .min_by_key(|span| span.end - span.start)
}

fn position(position: Position) -> Json {
    Json::object(vec![
        ("line", Json::number(position.line as i64)),
        ("character", Json::number(position.character as i64)),
    ])
}

fn text_document_uri(message: &Json) -> Option<String> {
    message
        .at(&["params", "textDocument", "uri"])
        .and_then(Json::as_str)
        .map(str::to_string)
}

fn position_of(message: &Json) -> Option<Position> {
    Some(Position::new(
        message
            .at(&["params", "position", "line"])
            .and_then(Json::as_i64)? as u32,
        message
            .at(&["params", "position", "character"])
            .and_then(Json::as_i64)? as u32,
    ))
}

/// The start and end of a `range` parameter.
fn range_of(message: &Json) -> Option<(Position, Position)> {
    let corner = |which: &str| {
        Some(Position::new(
            message.at(&["params", "range", which, "line"])?.as_i64()? as u32,
            message
                .at(&["params", "range", which, "character"])?
                .as_i64()? as u32,
        ))
    };
    Some((corner("start")?, corner("end")?))
}

/// Whether a quick fix is among what the editor asked for.
///
/// `context.only` is how an editor says it wants one kind of action and not
/// the others. A kind answers for an entry when it is that entry or something
/// underneath it, and `quickfix` is the whole of what this server produces, so
/// an editor asking for anything more specific is asking for something that is
/// not here. Nothing there at all means everything.
fn wants_quickfixes(message: &Json) -> bool {
    let Some(only) = message
        .at(&["params", "context", "only"])
        .and_then(Json::as_array)
    else {
        return true;
    };
    only.iter()
        .filter_map(Json::as_str)
        .any(|kind| kind == "quickfix")
}

/// Whether a span is close enough to a range to answer about.
///
/// Overlapping counts, and so does touching at either end. A cursor is a range
/// of width zero, and a cursor sitting immediately after the last character of
/// a misspelled name is where it is when someone has just finished typing it.
fn touches(span: Span, start: u32, end: u32) -> bool {
    span.start <= end && start <= span.end
}

fn initialize_result() -> Json {
    Json::object(vec![(
        "capabilities",
        Json::object(vec![
            // 1 is full sync. See the note in `didChange`.
            ("textDocumentSync", Json::number(1)),
            ("hoverProvider", Json::Bool(true)),
            ("inlayHintProvider", Json::Bool(true)),
            ("definitionProvider", Json::Bool(true)),
            ("referencesProvider", Json::Bool(true)),
            // `prepareProvider` is what lets an editor grey the command out
            // on a prelude name instead of asking for a new spelling and then
            // refusing it.
            (
                "renameProvider",
                Json::object(vec![("prepareProvider", Json::Bool(true))]),
            ),
            ("documentFormattingProvider", Json::Bool(true)),
            ("documentSymbolProvider", Json::Bool(true)),
            ("workspaceSymbolProvider", Json::Bool(true)),
            // A `(` opens an argument list and a `,` moves to the next one.
            // Nothing else changes which parameter is being written, and an
            // editor asks again on its own after that.
            (
                "signatureHelpProvider",
                Json::object(vec![(
                    "triggerCharacters",
                    Json::Array(vec![Json::string("("), Json::string(",")]),
                )]),
            ),
            // Quick fixes only. A code action is also how editors offer
            // refactorings and source-wide commands, and this server has
            // neither: what it has is the patch a diagnostic already carries.
            (
                "codeActionProvider",
                Json::object(vec![(
                    "codeActionKinds",
                    Json::Array(vec![Json::string("quickfix")]),
                )]),
            ),
            // A dot is the one character that changes the answer completely,
            // and a brace is what opens a `use` list. Everything else an
            // editor asks about on its own schedule.
            (
                "completionProvider",
                Json::object(vec![(
                    "triggerCharacters",
                    Json::Array(vec![Json::string("."), Json::string("{")]),
                )]),
            ),
        ]),
    )])
}

/// The innermost name covering an offset, and what it refers to.
///
/// Innermost for the same reason a hover wants the innermost expression: in
/// `a.b` the cursor is inside both, and the one under it is the one to answer
/// about.
fn narrowest_name(checked: &Checked, offset: u32) -> Option<(Span, DefId)> {
    checked
        .resolutions
        .names()
        .filter(|(span, _)| span.contains(offset))
        .min_by_key(|(span, _)| (span.end - span.start, span.start))
}

/// The contract of the function declared at `declared`, as it was written.
///
/// Quoted from the file rather than printed back from the tree, because the
/// review surface is somebody's own words and a second printer for them would
/// be a second opinion about what they wrote. Each clause is put on its own
/// line and its internal line breaks are flattened, because a wrapped
/// signature reads as one clause per line and a tooltip is not the place to
/// re-litigate the layout.
///
/// `None` for a contract with nothing in it. Most declarations in this
/// language have one, so writing an empty block under every other name would
/// be noise in the place a reader looks most.
///
/// `None` too for a name declared in another file. What crosses a module
/// boundary here is the type, which the line above already says; reaching the
/// other file's text would mean checking the workspace a second time on a
/// keystroke, and go to definition is one keypress and already goes there.
fn contract_declared_at(checked: &Checked, text: &str, declared: Span) -> Option<String> {
    if declared.is_empty() {
        return None;
    }
    let function = checked.module.items.iter().find_map(|item| match item {
        Item::Function(function) if function.sig.name.span == declared => Some(function),
        _ => None,
    })?;
    let contract = &function.contract;
    if contract.is_empty() {
        return None;
    }

    let mut clauses: Vec<String> = Vec::new();
    let mut clause = |keyword: &str, parts: Vec<String>| {
        if !parts.is_empty() {
            clauses.push(format!("{keyword} {}", parts.join(", ")));
        }
    };
    clause(
        "where",
        contract
            .requires
            .iter()
            .map(|expr| quoted(text, expr.span()))
            .collect(),
    );
    clause(
        "uses",
        contract
            .uses
            .iter()
            .map(|effect| quoted(text, effect.span))
            .collect(),
    );
    clause(
        "ensures",
        contract
            .ensures
            .iter()
            .map(|ensures| quoted(text, ensures.span))
            .collect(),
    );

    Some(format!("```deed\n{}\n```", clauses.join("\n")))
}

/// The source between two offsets, on one line.
fn quoted(text: &str, span: Span) -> String {
    text.get(span.as_range())
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// What kind of thing a name is, as a phrase rather than a word.
///
/// `DefKind::describe` hands back a bare noun because its callers write the
/// sentence around it. A tooltip is the sentence, so it puts the article on.
fn with_article(kind: deed_resolve::DefKind) -> String {
    let word = kind.describe();
    let article = if word.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
    format!("{article} {word}")
}

fn publish(uri: &str, diagnostics: Vec<Json>) -> Json {
    notification(
        "textDocument/publishDiagnostics",
        Json::object(vec![
            ("uri", Json::string(uri)),
            ("diagnostics", Json::Array(diagnostics)),
        ]),
    )
}

fn notification(method: &str, params: Json) -> Json {
    Json::object(vec![
        ("jsonrpc", Json::string("2.0")),
        ("method", Json::string(method)),
        ("params", params),
    ])
}

fn result(id: &Json, value: Json) -> Json {
    Json::object(vec![
        ("jsonrpc", Json::string("2.0")),
        ("id", id.clone()),
        ("result", value),
    ])
}

fn error(id: &Json, code: i64, message: &str) -> Json {
    Json::object(vec![
        ("jsonrpc", Json::string("2.0")),
        ("id", id.clone()),
        (
            "error",
            Json::object(vec![
                ("code", Json::number(code)),
                ("message", Json::string(message)),
            ]),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    const URI: &str = "file:///work/two.deed";
    /// A file that reaches for a module living inside the compiler.
    const IMPORTER: &str = "module two\n\nuse std/list.{map}\n\nfn f(xs: List<Int>) -> List<Int> {\n    map(xs, |n: Int| n + 1)\n}\n";

    fn server_with(uri: &str, text: &str) -> Server {
        let mut server = Server::new();
        server
            .documents
            .insert(uri.to_string(), Document::new(text.to_string()));
        server
    }

    #[test]
    fn a_module_that_ships_is_checked_alongside_the_document() {
        // The half that makes the `use` resolve. It is here, it went through
        // the same pipeline as everything else, and it is context: nobody
        // named it, so it is not a place.
        let server = server_with(URI, IMPORTER);
        let (_, entries) = server.check_workspace();

        let shipped: Vec<&Entry> = entries.iter().filter(|entry| entry.uri.is_none()).collect();
        assert_eq!(
            shipped.len(),
            1,
            "one module was imported, so one should have been taken"
        );
        assert_eq!(
            shipped[0]
                .checked
                .module
                .name
                .as_ref()
                .map(|path| path.to_string_path()),
            Some("std/list".to_string())
        );
    }

    #[test]
    fn a_problem_in_a_module_that_ships_is_not_published_as_the_persons_own() {
        // The decision this records: a shipped module that fails to check says
        // nothing to the person at the keyboard. It is not their file, they
        // cannot open it and they cannot fix it.
        //
        // Nothing that ships fails to check today, so the failure is put here
        // by hand. The alternative is a test that passes because the thing it
        // guards against has not happened yet, which is the same as no test.
        let server = server_with(URI, IMPORTER);
        let (sources, mut entries) = server.check_workspace();

        let shipped = entries
            .iter()
            .position(|entry| entry.uri.is_none())
            .expect("std/list should have been checked alongside");
        let file = entries[shipped].checked.file;
        entries[shipped].checked.diagnostics.push(Diagnostic::error(
            "DEED0000",
            file,
            Span::new(0, 1),
            "a module that ships stopped checking",
        ));

        let sent = server.publish_all(&sources, &entries);
        assert_eq!(sent.len(), 1, "only the open document is published for");
        assert_eq!(
            sent[0].at(&["params", "uri"]).and_then(Json::as_str),
            Some(URI)
        );
        assert_eq!(
            sent[0]
                .at(&["params", "diagnostics"])
                .and_then(Json::as_array)
                .map(<[Json]>::len),
            Some(0),
            "the invented failure belongs to nobody's document"
        );
    }

    /// One call that can be proven and one that cannot, plus a value whose
    /// type has something to say about it.
    const TIERED: &str = "module t\n\n\
         type Positive = Int where value > 0\n\n\
         fn halve(n: Int) -> Int\n\
         \x20 where\n\
         \x20   n >= 0,\n\
         {\n\
         \x20 n\n\
         }\n\n\
         fn settled() -> Int {\n\
         \x20 halve(5)\n\
         }\n\n\
         fn unsettled(n: Int) -> Int {\n\
         \x20 halve(n)\n\
         }\n";

    /// An inlay hint request over the whole of `text`.
    fn hints_over(uri: &str, text: &str) -> Vec<Json> {
        let end = text.lines().count() as i64;
        hints_between(uri, text, (0, 0), (end, 0))
    }

    fn hints_between(uri: &str, text: &str, start: (i64, i64), end: (i64, i64)) -> Vec<Json> {
        let corner = |(line, character): (i64, i64)| {
            Json::object(vec![
                ("line", Json::number(line)),
                ("character", Json::number(character)),
            ])
        };
        let server = server_with(uri, text);
        let message = Json::object(vec![(
            "params",
            Json::object(vec![
                (
                    "textDocument",
                    Json::object(vec![("uri", Json::string(uri))]),
                ),
                (
                    "range",
                    Json::object(vec![("start", corner(start)), ("end", corner(end))]),
                ),
            ]),
        )]);
        match server.inlay_hint(&message) {
            Json::Array(hints) => hints,
            other => panic!("an inlay hint request answers with an array, got {other:?}"),
        }
    }

    /// The label of each hint, in the order they were sent.
    fn labels(hints: &[Json]) -> Vec<&str> {
        hints
            .iter()
            .map(|hint| {
                hint.at(&["label"])
                    .and_then(Json::as_str)
                    .expect("a hint has a label")
            })
            .collect()
    }

    #[test]
    fn every_obligation_in_view_gets_its_tier() {
        // Including the proven one. A reader shown only what went wrong cannot
        // tell a discharged contract from a question nobody asked, which is
        // the rule `hover` and `deed check --obligations` already follow.
        let hints = hints_over("file:///work/t.deed", TIERED);
        assert_eq!(labels(&hints), vec!["proven", "guarded"]);
    }

    #[test]
    fn a_hint_sits_at_the_end_of_what_it_is_about() {
        let uri = "file:///work/t.deed";
        let hints = hints_over(uri, TIERED);
        let lines = Lines::of(TIERED);

        let offset_of = |hint: &Json| {
            let line = hint
                .at(&["position", "line"])
                .and_then(Json::as_i64)
                .expect("a hint has a line") as u32;
            let character = hint
                .at(&["position", "character"])
                .and_then(Json::as_i64)
                .expect("a hint has a character") as u32;
            lines.offset(TIERED, Position::new(line, character))
        };

        // Each hint lands immediately after the call it is about, rather than
        // at the start of it or at the end of the line.
        assert_eq!(
            offset_of(&hints[0]) as usize,
            TIERED
                .find("halve(5)")
                .expect("the settled call is written")
                + "halve(5)".len()
        );
        assert_eq!(
            offset_of(&hints[1]) as usize,
            TIERED
                .find("halve(n)")
                .expect("the unsettled call is written")
                + "halve(n)".len()
        );
    }

    #[test]
    fn a_hint_is_shaped_the_way_an_editor_reads_one() {
        let hints = hints_over("file:///work/t.deed", TIERED);
        // `Type`, which is what the protocol numbers 1. A tier is worked out
        // about the code to its left rather than being the name of a hole.
        assert_eq!(hints[0].at(&["kind"]).and_then(Json::as_i64), Some(1));
        // Without this the label is drawn against the last character of the
        // call, which reads as part of it.
        assert_eq!(hints[0].at(&["paddingLeft"]), Some(&Json::Bool(true)));
    }

    #[test]
    fn a_range_covering_one_call_is_answered_about_that_call() {
        let uri = "file:///work/t.deed";
        // The line `halve(5)` is written on, and nothing else.
        let line = TIERED
            .lines()
            .position(|line| line.contains("halve(5)"))
            .expect("the settled call is written") as i64;
        let hints = hints_between(uri, TIERED, (line, 0), (line, 40));
        assert_eq!(labels(&hints), vec!["proven"]);
    }

    #[test]
    fn a_range_with_no_obligation_in_it_gets_no_hints() {
        // Not an error and not a `null`. An editor handed anything other than
        // an array here logs a protocol failure at every keystroke.
        let uri = "file:///work/t.deed";
        assert!(hints_between(uri, TIERED, (0, 0), (1, 0)).is_empty());
    }

    #[test]
    fn two_obligations_that_landed_alike_in_one_place_are_one_hint() {
        // A call whose precondition is proven and whose result is a refinement
        // leaves two obligations ending at the same offset. Two hints in one
        // column are drawn as one word, so they are joined instead, and two
        // that say the same word are one word.
        let text = "module t\n\n\
             type Positive = Int where value > 0\n\n\
             fn twice(n: Positive) -> Positive\n\
             \x20 where\n\
             \x20   n > 0,\n\
             {\n\
             \x20 n\n\
             }\n\n\
             fn f() -> Positive {\n\
             \x20 twice(2)\n\
             }\n";
        let hints = hints_over("file:///work/t.deed", text);
        for label in labels(&hints) {
            let words: Vec<&str> = label.split(' ').collect();
            let mut seen = words.clone();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(
                seen.len(),
                words.len(),
                "`{label}` says the same tier twice"
            );
        }
    }

    /// Two open documents, so a label about one can be filed against the other.
    fn server_with_two() -> Server {
        let mut server = server_with(
            "file:///work/a.deed",
            "module a\n\nuse b.{g}\n\nfn f() -> Int {\n    g(1)\n}\n",
        );
        server.documents.insert(
            "file:///work/b.deed".to_string(),
            Document::new(
                "module b\n\n// a longer file, on purpose\n\nfn g(n: Int) -> Int {\n    n\n}\n"
                    .to_string(),
            ),
        );
        server
    }

    #[test]
    fn a_label_about_another_file_is_reported_against_that_file() {
        // Put here by hand for the same reason as the test above: nothing that
        // the server publishes carries a cross-file label today, because the
        // two producers that have one are the interpreter's and a run is not
        // something an editor asks for. Waiting for a check to grow one is the
        // same as no test, and the thing being guarded is that the location
        // this hands the editor is the label's own rather than the document
        // the diagnostic happened to be filed against.
        let server = server_with_two();
        let (sources, mut entries) = server.check_workspace();

        let a = entries
            .iter()
            .position(|entry| entry.uri.as_deref() == Some("file:///work/a.deed"))
            .expect("the document was opened");
        let b = entries
            .iter()
            .position(|entry| entry.uri.as_deref() == Some("file:///work/b.deed"))
            .expect("the document was opened");
        let (a_file, b_file) = (entries[a].checked.file, entries[b].checked.file);
        entries[a].checked.diagnostics.push(
            Diagnostic::error("DEED0000", a_file, Span::new(0, 1), "about the call")
                // `fn g` on line 5 of the other file.
                .with_secondary_in(b_file, Span::new(46, 50), "declared here"),
        );

        let sent = server.publish_all(&sources, &entries);
        let published = sent
            .iter()
            .find(|message| {
                message.at(&["params", "uri"]).and_then(Json::as_str) == Some("file:///work/a.deed")
            })
            .expect("the document with the problem is published for");
        let reported = published
            .at(&["params", "diagnostics"])
            .and_then(Json::as_array)
            .expect("a list of diagnostics");
        let mine = reported
            .iter()
            .find(|d| d.at(&["code"]).and_then(Json::as_str) == Some("DEED0000"))
            .unwrap_or_else(|| panic!("the invented diagnostic should be published: {reported:?}"));
        let related = mine
            .at(&["relatedInformation"])
            .and_then(Json::as_array)
            .expect("a list of labels");
        let [label] = related else {
            panic!("one label, got {related:?}");
        };

        assert_eq!(
            label.at(&["location", "uri"]).and_then(Json::as_str),
            Some("file:///work/b.deed"),
            "{label:?}"
        );
        // Line 4 zero based, which is where `fn g` is in `b.deed` and is not
        // where those byte offsets land in `a.deed`.
        assert_eq!(
            label
                .at(&["location", "range", "start", "line"])
                .and_then(Json::as_i64),
            Some(4),
            "{label:?}"
        );
    }

    #[test]
    fn a_label_about_a_module_that_ships_is_left_off() {
        // The same answer go to definition, rename and workspace symbol give.
        // There is no file behind a shipped module, so a location pointing
        // into one is one nobody can open, and the label goes rather than the
        // diagnostic it is attached to.
        let server = server_with(URI, IMPORTER);
        let (sources, mut entries) = server.check_workspace();

        let shipped = entries
            .iter()
            .position(|entry| entry.uri.is_none())
            .expect("std/list should have been checked alongside");
        let shipped_file = entries[shipped].checked.file;
        let open = entries
            .iter()
            .position(|entry| entry.uri.as_deref() == Some(URI))
            .expect("the document was opened");
        let open_file = entries[open].checked.file;
        entries[open].checked.diagnostics.push(
            Diagnostic::error("DEED0000", open_file, Span::new(0, 1), "about the call")
                .with_secondary_in(shipped_file, Span::new(0, 1), "declared here"),
        );

        let sent = server.publish_all(&sources, &entries);
        let reported = sent[0]
            .at(&["params", "diagnostics"])
            .and_then(Json::as_array)
            .expect("a list of diagnostics");
        let mine = reported
            .iter()
            .find(|d| d.at(&["code"]).and_then(Json::as_str) == Some("DEED0000"))
            .unwrap_or_else(|| panic!("the invented diagnostic should be published: {reported:?}"));
        let related = mine
            .at(&["relatedInformation"])
            .and_then(Json::as_array)
            .expect("a list of labels");
        assert!(related.is_empty(), "{related:?}");
    }
}
