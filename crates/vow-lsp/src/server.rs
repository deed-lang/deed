//! The server: what to do about each message, and nothing about how it
//! arrived.
//!
//! [`Server::handle`] takes one message and hands back the messages to send in
//! reply, which is what makes the whole protocol testable without a pipe, a
//! thread or an editor. The loop that owns the streams is in
//! [`crate::serve`] and is deliberately dull.
//!
//! One document at a time is checked on its own, with an empty universe, so a
//! `use` of another module reports that there is nothing behind it. That is
//! honest rather than convenient: the unit of compilation is the set of files
//! handed to the compiler, and the server has been handed one. Checking a
//! whole project is a real piece of work and pretending otherwise would put
//! wrong squiggles under correct code.

use std::collections::BTreeMap;

use vow_diagnostics::{Diagnostic, Severity, SourceMap, Span};
use vow_driver::Checked;
use vow_resolve::DefId;

use crate::json::Json;
use crate::position::{Lines, Position};

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

#[derive(Default)]
pub struct Server {
    /// Keyed by URI, ordered so that anything iterating them is reproducible.
    documents: BTreeMap<String, Document>,
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
                result(id, initialize_result())
            }
            "shutdown" => {
                self.shutting_down = true;
                result(id, Json::Null)
            }
            "textDocument/hover" => result(id, self.hover(message)),
            "textDocument/definition" => result(id, self.definition(message)),
            "textDocument/formatting" => result(id, self.formatting(message)),
            _ => error(
                id,
                METHOD_NOT_FOUND,
                &format!("`{method}` is not something this server does"),
            ),
        }
    }

    /// The document a request is about, and the byte offset it points at.
    fn locate(&self, message: &Json) -> Option<(&Document, u32)> {
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
        Some((document, document.lines.offset(&document.text, position)))
    }

    /// What the cursor is on.
    ///
    /// The type of the narrowest expression covering it, and failing that, what
    /// the name under it refers to. Both, when the cursor is on a name that is
    /// also an expression, because "`total` is a function" and
    /// "`Fn(Int) -> Int`" answer different halves of the same question.
    fn hover(&self, message: &Json) -> Json {
        let Some((document, offset)) = self.locate(message) else {
            return Json::Null;
        };
        let checked = check(document);

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
    /// Only inside this file. A name from another module resolves to the `use`
    /// that brought it in, which is where the reader can see what was imported
    /// and is the honest answer for a server that has been handed one file.
    fn definition(&self, message: &Json) -> Json {
        let Some((document, offset)) = self.locate(message) else {
            return Json::Null;
        };
        let checked = check(document);

        let Some((_, def)) = narrowest_name(&checked, offset) else {
            return Json::Null;
        };
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
                vec![self.diagnostics_for(&uri)]
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
                vec![self.diagnostics_for(&uri)]
            }
            "textDocument/didClose" => {
                let Some(uri) = text_document_uri(message) else {
                    return Vec::new();
                };
                self.documents.remove(&uri);
                // An empty list, not silence. A closed file whose squiggles
                // stayed behind is a file the editor keeps complaining about
                // forever.
                vec![publish(&uri, Vec::new())]
            }
            _ => Vec::new(),
        }
    }

    /// Checks one document and turns what came back into a notification.
    fn diagnostics_for(&self, uri: &str) -> Json {
        let Some(document) = self.documents.get(uri) else {
            return publish(uri, Vec::new());
        };

        let checked = check(document);
        let reported = checked
            .diagnostics
            .iter()
            .map(|diagnostic| self.render(document, diagnostic, uri))
            .collect();

        publish(uri, reported)
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
            ("documentFormattingProvider", Json::Bool(true)),
        ]),
    )])
}

/// Checks one document on its own.
///
/// A whole check per request, and per keystroke. P9 is about the edit loop and
/// nothing here has measured it yet, so this does the simple thing first on
/// files small enough for it not to matter. When it stops being small enough,
/// that is the measurement that says what to cache.
fn check(document: &Document) -> Checked {
    let mut sources = SourceMap::new();
    let file = sources.add("<document>", document.text.clone());
    vow_driver::check(&sources, file)
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
