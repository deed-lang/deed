//! A whole session, driven through the same entry point an editor uses.
//!
//! The unit tests in the crate cover the pieces. These cover the thing an
//! editor actually does: open a file, get squiggles, fix it, get none, close
//! it, and get told the squiggles are gone.

use std::io::BufReader;

use vow_lsp::{Json, json, serve};

/// One message, framed the way the protocol wants it.
fn framed(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{body}", body.len())
}

fn request(id: i64, method: &str) -> String {
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"{method}\"}}"
    ))
}

fn did_open(uri: &str, text: &str) -> String {
    let text = Json::string(text).to_text();
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":\
         {{\"textDocument\":{{\"uri\":\"{uri}\",\"languageId\":\"vow\",\"version\":1,\"text\":{text}}}}}}}"
    ))
}

fn did_change(uri: &str, text: &str) -> String {
    let text = Json::string(text).to_text();
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didChange\",\"params\":\
         {{\"textDocument\":{{\"uri\":\"{uri}\",\"version\":2}},\"contentChanges\":[{{\"text\":{text}}}]}}}}"
    ))
}

fn did_close(uri: &str) -> String {
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didClose\",\"params\":\
         {{\"textDocument\":{{\"uri\":\"{uri}\"}}}}}}"
    ))
}

/// Runs a session and hands back every message the server sent.
fn session(messages: &[String]) -> Vec<Json> {
    let input = messages.concat();
    let mut reader = BufReader::new(input.as_bytes());
    let mut written: Vec<u8> = Vec::new();

    serve(&mut reader, &mut written).expect("the server should not fail on well formed input");

    let text = String::from_utf8(written).expect("the server writes UTF-8");
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(blank) = rest.find("\r\n\r\n") {
        let header = &rest[..blank];
        let length: usize = header
            .split_once(':')
            .expect("a header with a colon")
            .1
            .trim()
            .parse()
            .expect("a numeric length");
        let body_start = blank + 4;
        let body = &rest[body_start..body_start + length];
        out.push(json::parse(body).expect("the server writes JSON"));
        rest = &rest[body_start + length..];
    }
    assert!(rest.is_empty(), "trailing bytes after the last message");
    out
}

fn published(message: &Json) -> Option<&[Json]> {
    if message.at(&["method"]).and_then(Json::as_str)? != "textDocument/publishDiagnostics" {
        return None;
    }
    message.at(&["params", "diagnostics"])?.as_array()
}

const URI: &str = "file:///work/a.vow";
const GOOD: &str = "module a\n\nfn f(n: Int) -> Int {\n    n + n\n}\n";
const BAD: &str = "module a\n\nfn f(n: Int) -> String {\n    n\n}\n";

#[test]
fn initialize_says_what_the_server_can_do() {
    let sent = session(&[request(1, "initialize")]);

    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].at(&["id"]).and_then(Json::as_i64), Some(1));
    assert_eq!(
        sent[0]
            .at(&["result", "capabilities", "textDocumentSync"])
            .and_then(Json::as_i64),
        // Full sync. Incremental is an optimisation that can desynchronise the
        // server from the file, and nothing here has measured that it is worth
        // the risk.
        Some(1)
    );
}

#[test]
fn a_question_before_initialize_is_refused() {
    // Otherwise the server answers about state the editor has not set up, and
    // the protocol has a code for exactly this.
    let sent = session(&[request(1, "shutdown")]);
    assert_eq!(
        sent[0].at(&["error", "code"]).and_then(Json::as_i64),
        Some(-32600)
    );
}

#[test]
fn a_method_nobody_implemented_is_an_error_rather_than_silence() {
    // A request with no reply is a request the editor waits on forever.
    let sent = session(&[request(1, "initialize"), request(2, "textDocument/rename")]);
    assert_eq!(
        sent[1].at(&["error", "code"]).and_then(Json::as_i64),
        Some(-32601)
    );
}

#[test]
fn opening_a_file_publishes_what_the_compiler_found() {
    let sent = session(&[request(1, "initialize"), did_open(URI, BAD)]);

    let diagnostics = published(&sent[1]).expect("the second message publishes diagnostics");
    assert_eq!(diagnostics.len(), 1, "{:?}", sent[1]);

    let first = &diagnostics[0];
    assert_eq!(first.at(&["source"]).and_then(Json::as_str), Some("vow"));
    assert_eq!(
        first.at(&["code"]).and_then(Json::as_str),
        Some("VOW4001"),
        "{first:?}"
    );
    assert_eq!(first.at(&["severity"]).and_then(Json::as_i64), Some(1));

    // Line 3, zero based, which is the one with `"two"` on it.
    assert_eq!(
        first.at(&["range", "start", "line"]).and_then(Json::as_i64),
        Some(3)
    );
}

#[test]
fn a_clean_file_publishes_an_empty_list_rather_than_nothing() {
    // Silence would leave whatever was there last time on the screen.
    let sent = session(&[request(1, "initialize"), did_open(URI, GOOD)]);
    assert_eq!(published(&sent[1]).unwrap().len(), 0);
}

#[test]
fn fixing_the_file_takes_the_squiggles_away() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, BAD),
        did_change(URI, GOOD),
    ]);

    assert_eq!(published(&sent[1]).unwrap().len(), 1);
    assert_eq!(published(&sent[2]).unwrap().len(), 0);
}

#[test]
fn closing_a_file_clears_it() {
    let sent = session(&[request(1, "initialize"), did_open(URI, BAD), did_close(URI)]);

    assert_eq!(published(&sent[2]).unwrap().len(), 0);
    assert_eq!(
        sent[2].at(&["params", "uri"]).and_then(Json::as_str),
        Some(URI)
    );
}

#[test]
fn a_range_is_measured_in_utf16_units_and_not_in_bytes() {
    // The error is on the `n`, after two characters that are two bytes each.
    // Counting bytes would put the squiggle two columns to the right, which is
    // the ordinary case in a repository whose comments are half in Turkish.
    let source = "module a\n\nfn f(n: Int) -> String {\n    \"üç\" + n\n}\n";
    let line = "    \"üç\" + n";
    assert_ne!(line.len(), line.chars().count());

    let sent = session(&[request(1, "initialize"), did_open(URI, source)]);
    let first = &published(&sent[1]).unwrap()[0];

    assert_eq!(
        first.at(&["range", "start", "line"]).and_then(Json::as_i64),
        Some(3)
    );
    assert_eq!(
        first
            .at(&["range", "start", "character"])
            .and_then(Json::as_i64),
        Some(line.chars().count() as i64 - 1),
        "{first:?}"
    );
}

#[test]
fn a_note_travels_with_the_message() {
    // The notes are where a diagnostic explains itself. An editor that only
    // showed the first line would be a worse version of the terminal.
    let source = "module a\n\nfn f(items: List<Int, String>) -> Int {\n    0\n}\n";
    let sent = session(&[request(1, "initialize"), did_open(URI, source)]);

    let message = published(&sent[1]).unwrap()[0]
        .at(&["message"])
        .and_then(Json::as_str)
        .unwrap()
        .to_string();
    assert!(
        message.contains("note: it is written `List<Element>`"),
        "{message}"
    );
}

#[test]
fn shutdown_then_exit_ends_it() {
    let sent = session(&[
        request(1, "initialize"),
        request(2, "shutdown"),
        framed("{\"jsonrpc\":\"2.0\",\"method\":\"exit\"}"),
        // Anything after `exit` is never read, so this reply must not appear.
        request(3, "initialize"),
    ]);

    assert_eq!(sent.len(), 2);
    assert!(sent[1].at(&["result"]).is_some_and(Json::is_null));
}

#[test]
fn the_stream_ending_is_not_an_error() {
    // Editors do not always get to say goodbye.
    let sent = session(&[request(1, "initialize")]);
    assert_eq!(sent.len(), 1);
}
