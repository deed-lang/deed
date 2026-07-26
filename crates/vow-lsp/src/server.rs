//! The server: what to do about each message, and nothing about how it
//! arrived.
//!
//! [`Server::handle`] takes one message and hands back the messages to send in
//! reply, which is what makes the whole protocol testable without a pipe, a
//! thread or an editor. The loop that owns the streams is in
//! [`crate::serve`] and is deliberately dull.
//!
//! A document is checked together with every other `.vow` file in the folders
//! the editor said it has open, with the text of anything open coming from the
//! editor's buffer rather than from disk. See [`crate::workspace`] for why the
//! set is that one and not another. An editor that names no folder gets the
//! single file behaviour, which is honest for a server that has been handed
//! one file and nothing else.

use std::collections::BTreeMap;
use std::path::PathBuf;

use vow_diagnostics::{Diagnostic, Severity, SourceMap, Span};
use vow_driver::Checked;
use vow_resolve::{DefId, DefKind};

use crate::json::Json;
use crate::position::{Lines, Position};
use crate::uri;
use crate::workspace::{Workspace, canonical};

/// JSON-RPC error codes this server can produce.
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_REQUEST: i64 = -32600;

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
    uri: String,
    checked: Checked,
}

/// Every place one name is written in one file.
///
/// Kept as spans rather than as answers, because references wants locations
/// and rename wants edits and the walk that finds them is the same walk.
struct Found {
    uri: String,
    file: vow_diagnostics::FileId,
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
            "textDocument/definition" => result(id, self.definition(message)),
            "textDocument/references" => result(id, self.references(message)),
            "textDocument/prepareRename" => result(id, self.prepare_rename(message)),
            // Answers with the whole response rather than a value, because
            // refusing a rename has to be an error: an editor told "here is an
            // edit with nothing in it" reports success and changes nothing.
            "textDocument/rename" => self.rename(id, message),
            "textDocument/formatting" => result(id, self.formatting(message)),
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
        let position = Position::new(
            message
                .at(&["params", "position", "line"])
                .and_then(Json::as_i64)? as u32,
            message
                .at(&["params", "position", "character"])
                .and_then(Json::as_i64)? as u32,
        );
        let offset = document.lines.offset(&document.text, position);
        Some((document, offset, self.check_one(&uri)?))
    }

    /// What the cursor is on.
    ///
    /// The type of the narrowest expression covering it, and failing that, what
    /// the name under it refers to. Both, when the cursor is on a name that is
    /// also an expression, because "`total` is a function" and
    /// "`Fn(Int) -> Int`" answer different halves of the same question.
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

        let text = sources.file(entry.checked.file).text();
        Some(Json::object(vec![
            ("uri", Json::string(&entry.uri)),
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
                None => {
                    (entry.uri == text_document_uri(message).unwrap_or_default()).then_some(def)
                }
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
                    uri: entry.uri.clone(),
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
        let Ok(formatted) = vow_fmt::format(file, &document.text) else {
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
        let (_, checked) = self.check_workspace();
        checked
            .iter()
            .filter_map(|entry| {
                let document = self.documents.get(&entry.uri)?;
                let reported = entry
                    .checked
                    .diagnostics
                    .iter()
                    .map(|diagnostic| self.render(document, diagnostic, &entry.uri))
                    .collect();
                Some(publish(&entry.uri, reported))
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
    /// The [`SourceMap`] comes back with the results because a question about
    /// one file can be answered in another, and turning a span in that other
    /// file into a range needs its text.
    fn check_workspace(&self) -> (SourceMap, Vec<Entry>) {
        let mut sources = SourceMap::new();
        let mut ids = Vec::new();
        let mut uris = Vec::new();
        let mut open: Vec<Option<PathBuf>> = Vec::new();

        for (uri, document) in &self.documents {
            ids.push(sources.add(uri.as_str(), document.text.clone()));
            uris.push(uri.clone());
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
            ids.push(sources.add(path.display().to_string(), text));
            uris.push(uri::from_path(&path));
        }

        let entries = uris
            .into_iter()
            .zip(vow_driver::check_all(&sources, &ids))
            .map(|(uri, checked)| Entry { uri, checked })
            .collect();

        (sources, entries)
    }

    /// The same, for one document.
    fn check_one(&self, uri: &str) -> Option<Checked> {
        let (_, checked) = self.check_workspace();
        checked
            .into_iter()
            .find(|entry| entry.uri == uri)
            .map(|entry| entry.checked)
    }

    fn render(&self, document: &Document, diagnostic: &Diagnostic, uri: &str) -> Json {
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
            .map(|label| {
                Json::object(vec![
                    (
                        "location",
                        Json::object(vec![
                            ("uri", Json::string(uri)),
                            ("range", self.range(document, label.span)),
                        ]),
                    ),
                    ("message", Json::string(&label.message)),
                ])
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
            ("source", Json::string("vow")),
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
    let lexed = vow_lexer::tokenize(file, text);
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
        [vow_lexer::TokenKind::Ident(_), vow_lexer::TokenKind::Eof]
    )
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

fn initialize_result() -> Json {
    Json::object(vec![(
        "capabilities",
        Json::object(vec![
            // 1 is full sync. See the note in `didChange`.
            ("textDocumentSync", Json::number(1)),
            ("hoverProvider", Json::Bool(true)),
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

/// What kind of thing a name is, as a phrase rather than a word.
///
/// `DefKind::describe` hands back a bare noun because its callers write the
/// sentence around it. A tooltip is the sentence, so it puts the article on.
fn with_article(kind: vow_resolve::DefKind) -> String {
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
