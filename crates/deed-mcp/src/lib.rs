//! The Model Context Protocol surface: the door an agent walks through.
//!
//! `design/00-motivation.md` says most code being written today is not typed
//! out by a person. Everything else in this repository followed from that and
//! then stopped one step short: the compiler answers a human at a terminal and
//! a human in an editor, and an agent had no way in at all. It could shell out
//! to `deed check` and scrape the text, which is the thing every other
//! language's agent integration does and the reason those integrations break
//! whenever a message is reworded.
//!
//! This crate is the other half of the language's own claim. An agent asks a
//! question, the compiler answers it in the shape it already publishes, and
//! nothing in between guesses.
//!
//! ## What this server may do
//!
//! Nothing. It has no capability, and that is not an omission.
//!
//! A program's text arrives as an argument and the answer leaves as text. This
//! server opens no file, resolves no path, and runs nothing that could reach
//! one: [`deed_wasm::run_source`] refuses a program whose row asks for a
//! directory before it runs it. So the row a caller can see is the whole of
//! what this can do, which is the same sentence `design/04-capabilities.md`
//! writes about a Deed function.
//!
//! That is a real cost and it is written down rather than hidden: an agent
//! working on a module set has to send every file it wants reviewed as source
//! text. `deed_review` accepts those texts as two arrays and resolves imports
//! among each array. There is still no root and no path lookup. The shipped
//! library is the exception, and only because it travels inside the binary
//! already.
//!
//! ## Why the answers come from `deed-wasm`
//!
//! The one-file tools are the same questions the playground asks: text in,
//! JSON out, one file, no filesystem. Those answers live in [`deed_wasm`], and
//! this crate calls them rather than writing a second copy. Review has no page
//! equivalent: it checks two in-memory module sets and delegates the evidence
//! and policy decision to [`deed_driver::review`], the same source as the CLI.
//! Two copies would be two answers, and the one an agent got would be the one
//! nobody was looking at. `crates/deed-mcp/tests/agreement.rs` holds that.
//!
//! ## The transport
//!
//! MCP's stdio transport is JSON-RPC 2.0, one message a line, newline
//! delimited. That is not the framing [`deed_lsp`] uses, so the reader here is
//! its own; the JSON reader and writer are not, for the reason above.

use std::io::{self, BufRead, Write};

use deed_lsp::Json;

pub mod tools;

/// The protocol revision this server implements.
///
/// A client that asks for a different one is answered with this rather than
/// refused: the specification says a server that cannot speak the requested
/// revision replies with one it does speak, and lets the client decide.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// What this server calls itself in `initialize`.
pub const SERVER_NAME: &str = "deed";

/// Reads messages until the input ends, writing one answer a line.
///
/// A notification gets no answer, which is the whole of why [`Server::handle`]
/// returns an `Option`.
pub fn serve(input: &mut impl BufRead, output: &mut impl Write) -> io::Result<()> {
    let mut server = Server::new();

    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(answer) = server.handle(&line) {
            writeln!(output, "{}", answer.to_text())?;
            output.flush()?;
        }
    }

    Ok(())
}

/// The protocol state: whether `initialize` has been answered yet.
#[derive(Default)]
pub struct Server {
    initialized: bool,
}

impl Server {
    pub fn new() -> Self {
        Server::default()
    }

    /// Answers one message, or returns `None` when the message was a
    /// notification and the protocol says to stay quiet.
    pub fn handle(&mut self, line: &str) -> Option<Json> {
        let message = match deed_lsp::json::parse(line) {
            Ok(message) => message,
            // A parse error has no id to answer under, so the id is null. That
            // is the one place JSON-RPC allows it.
            Err(error) => {
                return Some(failure(Json::Null, PARSE_ERROR, &error.message));
            }
        };

        let id = message.get("id").cloned().unwrap_or(Json::Null);
        let Some(method) = message.get("method").and_then(Json::as_str) else {
            return Some(failure(id, INVALID_REQUEST, "this message names no method"));
        };

        // A notification is a message with no `id`. The protocol's own
        // definition, and the reason `notifications/initialized` is silence
        // rather than an empty result.
        let is_notification = message.get("id").is_none();

        let params = message.get("params");
        let answer = match method {
            "initialize" => Ok(self.initialize()),
            "ping" => Ok(Json::object(vec![])),
            "tools/list" => self.ready().map(|()| tools::listing()),
            "tools/call" => self.ready().and_then(|()| self.call(params)),
            _ if is_notification => return None,
            _ => Err(Failure {
                code: METHOD_NOT_FOUND,
                message: format!("this server has no `{method}` method"),
            }),
        };

        if is_notification {
            return None;
        }

        Some(match answer {
            Ok(result) => success(id, result),
            Err(Failure { code, message }) => failure(id, code, &message),
        })
    }

    fn initialize(&mut self) -> Json {
        self.initialized = true;

        Json::object(vec![
            ("protocolVersion", Json::string(PROTOCOL_VERSION)),
            (
                "capabilities",
                // Only tools. There is no prompt to offer and nothing to list
                // as a resource, because this server holds no files.
                Json::object(vec![(
                    "tools",
                    Json::object(vec![("listChanged", Json::Bool(false))]),
                )]),
            ),
            (
                "serverInfo",
                Json::object(vec![
                    ("name", Json::string(SERVER_NAME)),
                    ("version", Json::string(env!("CARGO_PKG_VERSION"))),
                ]),
            ),
            (
                "instructions",
                Json::string(
                    "Deed is a contract-first language: a signature carries the \
                     whole contract and the compiler checks the body against it. \
                     Work in this order. `deed_check` first, and read the \
                     `obligation` lines as well as the `diagnostic` ones: an \
                     obligation that came back `guarded` says what stopped it \
                     from being proven, which is what would have to change for \
                     it to be. `deed_fix` applies the repairs the compiler is \
                     sure about, so run it before hand-editing anything. Then \
                     `deed_test`, because a program that checks is not a \
                     program that works: it runs the `test` blocks and the \
                     properties the contracts generate, and a property is one \
                     nobody wrote. `deed_run` last, for what `main` prints. \
                     Before finishing a patch, use `deed_review` with every \
                     module before and after it; read the receipt and its \
                     policy verdict rather than treating a successful tool \
                     call as approval. \
                     `deed_explain` turns any DEED#### code into its page, and \
                     `deed_fmt` into the one layout this language has.",
                ),
            ),
        ])
    }

    /// Refuses anything that needs the handshake to have happened.
    ///
    /// The protocol says a client sends `initialize` first. Answering tool
    /// calls before it would mean the order in the specification is decoration,
    /// and a client that got away with skipping it here would break against
    /// every other server.
    fn ready(&self) -> Result<(), Failure> {
        if self.initialized {
            Ok(())
        } else {
            Err(Failure {
                code: INVALID_REQUEST,
                message: "this server has not been initialized yet".to_string(),
            })
        }
    }

    fn call(&mut self, params: Option<&Json>) -> Result<Json, Failure> {
        let Some(params) = params else {
            return Err(Failure {
                code: INVALID_PARAMS,
                message: "a tool call needs a `name` and `arguments`".to_string(),
            });
        };
        let Some(name) = params.get("name").and_then(Json::as_str) else {
            return Err(Failure {
                code: INVALID_PARAMS,
                message: "a tool call needs a `name`".to_string(),
            });
        };
        let arguments = params.get("arguments");

        tools::call(name, arguments)
    }
}

/// A JSON-RPC error, before it is given an id to travel under.
#[derive(Debug)]
pub struct Failure {
    pub code: i64,
    pub message: String,
}

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;

fn success(id: Json, result: Json) -> Json {
    Json::object(vec![
        ("jsonrpc", Json::string("2.0")),
        ("id", id),
        ("result", result),
    ])
}

fn failure(id: Json, code: i64, message: &str) -> Json {
    Json::object(vec![
        ("jsonrpc", Json::string("2.0")),
        ("id", id),
        (
            "error",
            Json::object(vec![
                ("code", Json::number(code)),
                ("message", Json::string(message)),
            ]),
        ),
    ])
}
