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
            _ => {
                let _ = message;
                error(
                    id,
                    METHOD_NOT_FOUND,
                    &format!("`{method}` is not something this server does"),
                )
            }
        }
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

        let mut sources = SourceMap::new();
        let file = sources.add(uri, document.text.clone());
        let checked = vow_driver::check(&sources, file);

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
        ]),
    )])
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
