//! A whole session, driven through the same entry point an editor uses.
//!
//! The unit tests in the crate cover the pieces. These cover the thing an
//! editor actually does: open a file, get squiggles, fix it, get none, close
//! it, and get told the squiggles are gone.

use std::io::BufReader;
use std::path::{Path, PathBuf};

use deed_lsp::{Json, json, serve};

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
         {{\"textDocument\":{{\"uri\":\"{uri}\",\"languageId\":\"deed\",\"version\":1,\"text\":{text}}}}}}}"
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

/// A request about one place in a document.
fn at(id: i64, method: &str, uri: &str, line: u32, character: u32) -> String {
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"{method}\",\"params\":\
         {{\"textDocument\":{{\"uri\":\"{uri}\"}},\
         \"position\":{{\"line\":{line},\"character\":{character}}}}}}}"
    ))
}

/// A references request, which carries whether the declaration counts.
fn references(id: i64, uri: &str, line: u32, character: u32, declaration: bool) -> String {
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"textDocument/references\",\"params\":\
         {{\"textDocument\":{{\"uri\":\"{uri}\"}},\
         \"position\":{{\"line\":{line},\"character\":{character}}},\
         \"context\":{{\"includeDeclaration\":{declaration}}}}}}}"
    ))
}

fn format_request(id: i64, uri: &str) -> String {
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"textDocument/formatting\",\"params\":\
         {{\"textDocument\":{{\"uri\":\"{uri}\"}},\"options\":{{\"tabSize\":4,\"insertSpaces\":true}}}}}}"
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

/// The last diagnostics published for one document.
///
/// Last, because a change to one file republishes every open one, and what
/// matters is what the editor is left showing.
fn published_for<'a>(sent: &'a [Json], uri: &str) -> &'a [Json] {
    sent.iter()
        .rev()
        .find(|message| {
            message.at(&["params", "uri"]).and_then(Json::as_str) == Some(uri)
                && published(message).is_some()
        })
        .and_then(published)
        .unwrap_or_else(|| panic!("nothing was published for {uri}"))
}

/// A directory with files in it, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("deed-lsp-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, contents: &str) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, contents).unwrap();
        file_uri(&path)
    }

    fn uri(&self) -> String {
        file_uri(&self.0)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The URI an editor would send for a path.
///
/// Written out here rather than shared with the crate, because a test that
/// used the same code as the thing it is testing would agree with it by
/// construction.
fn file_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// `initialize`, with a workspace folder the way an editor sends one.
fn initialize_in(id: i64, scratch: &Scratch) -> String {
    let uri = scratch.uri();
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"initialize\",\"params\":\
         {{\"workspaceFolders\":[{{\"uri\":\"{uri}\",\"name\":\"scratch\"}}]}}}}"
    ))
}

const URI: &str = "file:///work/a.deed";
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

    for capability in [
        "hoverProvider",
        "definitionProvider",
        "documentFormattingProvider",
        "documentSymbolProvider",
    ] {
        assert_eq!(
            sent[0].at(&["result", "capabilities", capability]),
            Some(&Json::Bool(true)),
            "{capability} should be advertised, or an editor never asks"
        );
    }

    assert_eq!(
        sent[0].at(&[
            "result",
            "capabilities",
            "codeActionProvider",
            "codeActionKinds"
        ]),
        Some(&Json::Array(vec![Json::string("quickfix")])),
        "quick fixes are the only kind of action this server has"
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
    //
    // The method named here has to be one this server will not grow. A colour
    // picker needs colour literals and the language has none, so it is safe
    // in a way that picking whatever is unimplemented today is not: this test
    // used to name `signatureHelp` and started failing the day that landed.
    let sent = session(&[
        request(1, "initialize"),
        request(2, "textDocument/colorPresentation"),
    ]);
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
    assert_eq!(first.at(&["source"]).and_then(Json::as_str), Some("deed"));
    assert_eq!(
        first.at(&["code"]).and_then(Json::as_str),
        Some("DEED4001"),
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

// -- hover ------------------------------------------------------------------

/// The markdown a hover came back with.
fn hover_text(message: &Json) -> String {
    message
        .at(&["result", "contents", "value"])
        .and_then(Json::as_str)
        .unwrap_or_else(|| panic!("expected a hover, got {message:?}"))
        .to_string()
}

#[test]
fn hovering_over_an_expression_says_what_it_turned_out_to_be() {
    // `n + n` on line 3. Column 4 is the first `n`, column 8 is the second.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, GOOD),
        at(2, "textDocument/hover", URI, 3, 4),
    ]);

    let text = hover_text(&sent[2]);
    assert!(text.contains("`Int`"), "{text}");
    assert!(text.contains("`n`, a parameter"), "{text}");
}

#[test]
fn hovering_picks_the_innermost_thing_under_the_cursor() {
    // The cursor is inside the call and inside the argument. The argument is
    // the thing under it, so its `String` wins over the call's `Int`.
    let source = "module a\n\nfn f() -> Int {\n    length(\"hello\")\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        at(2, "textDocument/hover", URI, 3, 12),
    ]);

    assert!(hover_text(&sent[2]).contains("`String`"), "{:?}", sent[2]);
}

#[test]
fn hovering_over_a_function_says_its_type_rather_than_its_arity() {
    // This used to read "a function of 1 returning `Int`", which is a riddle
    // in a diagnostic and nothing at all in a tooltip.
    let source = "module a\n\nfn double(n: Int) -> Int {\n    n + n\n}\n\nfn g() -> Int {\n    apply(double)\n}\n\nfn apply(f: Fn(Int) -> Int) -> Int {\n    f(1)\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        at(2, "textDocument/hover", URI, 7, 10),
    ]);

    let text = hover_text(&sent[2]);
    assert!(text.contains("`Fn(Int) -> Int`"), "{text}");
}

#[test]
fn hovering_over_nothing_is_null_rather_than_an_empty_tooltip() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, GOOD),
        // The blank line between the module and the function.
        at(2, "textDocument/hover", URI, 1, 0),
    ]);

    assert!(sent[2].at(&["result"]).is_some_and(Json::is_null));
}

// -- go to definition -------------------------------------------------------

#[test]
fn a_use_of_a_name_leads_back_to_where_it_was_declared() {
    let source = "module a\n\nfn double(n: Int) -> Int {\n    n + n\n}\n\nfn g() -> Int {\n    double(2)\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        // The `double` on line 7.
        at(2, "textDocument/definition", URI, 7, 6),
    ]);

    let location = sent[2].at(&["result"]).expect("a location");
    assert_eq!(location.at(&["uri"]).and_then(Json::as_str), Some(URI));
    assert_eq!(
        location
            .at(&["range", "start", "line"])
            .and_then(Json::as_i64),
        Some(2),
        "{location:?}"
    );
    assert_eq!(
        location
            .at(&["range", "start", "character"])
            .and_then(Json::as_i64),
        Some(3)
    );
}

#[test]
fn a_builtin_has_nowhere_to_go() {
    // `length` is declared in no file. Jumping to the top of whichever file
    // happened to be open would be worse than saying there is nowhere.
    let source = "module a\n\nfn f() -> Int {\n    length(\"hello\")\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        at(2, "textDocument/definition", URI, 3, 5),
    ]);

    assert!(sent[2].at(&["result"]).is_some_and(Json::is_null));
}

// -- formatting -------------------------------------------------------------

#[test]
fn formatting_replaces_the_whole_document_with_the_canonical_form() {
    let cramped = "module a\nfn   f( n:Int )->Int{n+n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, cramped),
        format_request(2, URI),
    ]);

    let edits = sent[2].at(&["result"]).and_then(Json::as_array).unwrap();
    assert_eq!(edits.len(), 1, "{:?}", sent[2]);
    assert_eq!(
        edits[0].at(&["newText"]).and_then(Json::as_str),
        Some("module a\n\nfn f(n: Int) -> Int {\n    n + n\n}\n")
    );
    assert_eq!(
        edits[0]
            .at(&["range", "start", "line"])
            .and_then(Json::as_i64),
        Some(0)
    );
}

#[test]
fn formatting_a_file_that_is_already_canonical_changes_nothing() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, GOOD),
        format_request(2, URI),
    ]);

    assert_eq!(
        sent[2]
            .at(&["result"])
            .and_then(Json::as_array)
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn formatting_a_file_that_does_not_parse_leaves_it_alone() {
    // Reshaping a broken file is guessing at what was meant, and the guess
    // lands in the working tree the moment the editor saves.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, "module a\n\nfn f( -> Int {\n"),
        format_request(2, URI),
    ]);

    assert_eq!(
        sent[2]
            .at(&["result"])
            .and_then(Json::as_array)
            .unwrap()
            .len(),
        0
    );
}

// -- code actions -----------------------------------------------------------

/// A code action request over one line and a bit, the way an editor asks about
/// the line the cursor is on.
fn code_action(id: i64, uri: &str, line: u32, character: u32) -> String {
    code_action_only(id, uri, line, character, None)
}

fn code_action_only(id: i64, uri: &str, line: u32, character: u32, only: Option<&str>) -> String {
    let only = match only {
        Some(kind) => format!(",\"only\":[\"{kind}\"]"),
        None => String::new(),
    };
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"textDocument/codeAction\",\"params\":\
         {{\"textDocument\":{{\"uri\":\"{uri}\"}},\
         \"range\":{{\"start\":{{\"line\":{line},\"character\":{character}}},\
         \"end\":{{\"line\":{line},\"character\":{character}}}}},\
         \"context\":{{\"diagnostics\":[]{only}}}}}}}"
    ))
}

fn actions(message: &Json) -> &[Json] {
    message
        .at(&["result"])
        .and_then(Json::as_array)
        .unwrap_or_else(|| panic!("a code action request should answer with a list: {message:?}"))
}

/// `lenght` on line 4, where `length` is in the prelude.
const MISSPELLED: &str = "module a\n\nfn f(items: List<Int>) -> Int {\n    lenght(items)\n}\n";

#[test]
fn the_patch_a_diagnostic_carries_is_offered_as_a_quick_fix() {
    // "did you mean" already knew the answer. Until this, the only thing that
    // could reach the patch was `deed fix` from a command line, so the reader
    // sitting in the editor was told the name and made to type it.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, MISSPELLED),
        code_action(2, URI, 3, 6),
    ]);

    let offered = actions(&sent[2]);
    assert_eq!(offered.len(), 1, "{:?}", sent[2]);
    assert_eq!(
        offered[0].at(&["title"]).and_then(Json::as_str),
        Some("there is a `length` in scope")
    );
    assert_eq!(
        offered[0].at(&["kind"]).and_then(Json::as_str),
        Some("quickfix")
    );

    let edits = offered[0]
        .at(&["edit", "changes", URI])
        .and_then(Json::as_array)
        .unwrap();
    assert_eq!(edits.len(), 1);
    assert_eq!(
        edits[0].at(&["newText"]).and_then(Json::as_str),
        Some("length")
    );
    assert_eq!(
        edits[0]
            .at(&["range", "start", "character"])
            .and_then(Json::as_i64),
        Some(4)
    );
    assert_eq!(
        edits[0]
            .at(&["range", "end", "character"])
            .and_then(Json::as_i64),
        Some(10)
    );
}

#[test]
fn a_quick_fix_names_the_diagnostic_it_answers() {
    // Without this the editor cannot tell which squiggle the action belongs
    // to, and it offers every action on the line for every one of them.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, MISSPELLED),
        code_action(2, URI, 3, 6),
    ]);

    let attached = actions(&sent[2])[0]
        .at(&["diagnostics"])
        .and_then(Json::as_array)
        .unwrap();
    assert_eq!(attached.len(), 1);
    assert_eq!(
        attached[0].at(&["code"]).and_then(Json::as_str),
        Some("DEED3001")
    );
}

/// A machine-applicable fix is the one `deed fix` applies without being asked,
/// so it is the one an editor should reach for first. A guess is not.
#[test]
fn a_certain_fix_is_preferred_and_a_guess_is_not() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, MISSPELLED),
        code_action(2, URI, 3, 6),
    ]);
    assert_eq!(
        actions(&sent[2])[0].at(&["isPreferred"]),
        Some(&Json::Bool(true))
    );

    // An import of a name the module does not declare. Two things are wrong
    // with the same word and the editor gets both: the spelling might have
    // been meant, which is a guess, and the import is unused either way, which
    // is not.
    let scratch = Scratch::new("guess");
    scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", "module scratch/two\n");
    let importer = "module scratch/two\n\nuse scratch/one.{doubel}\n\nfn f() -> Int {\n    0\n}\n";
    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&two, importer),
        code_action(2, &two, 2, 18),
    ]);

    let offered = actions(sent.last().unwrap());
    let guess = offered
        .iter()
        .find(|action| {
            action.at(&["title"]).and_then(Json::as_str)
                == Some("`scratch/one` declares a `double`")
        })
        .unwrap_or_else(|| panic!("the spelling suggestion should be offered: {offered:?}"));
    assert_eq!(
        guess.at(&["isPreferred"]),
        Some(&Json::Bool(false)),
        "a guess should be offered and not preferred"
    );
    assert_eq!(
        guess
            .at(&["edit", "changes", two.as_str()])
            .and_then(Json::as_array)
            .map(|edits| edits[0].at(&["newText"]).and_then(Json::as_str)),
        Some(Some("double"))
    );

    let certain = offered
        .iter()
        .find(|action| action.at(&["title"]).and_then(Json::as_str) == Some("remove `doubel`"))
        .unwrap_or_else(|| panic!("the unused import should be offered: {offered:?}"));
    assert_eq!(certain.at(&["isPreferred"]), Some(&Json::Bool(true)));
}

/// The two warnings about something going nowhere both carry a guess, and a
/// guess used to have no consumer at all. This is the one it has.
#[test]
fn a_warning_about_something_going_nowhere_offers_a_way_to_mean_it() {
    let source = "module a\n\nfn twice(n: Int) -> Int {\n    n + n\n}\n\nfn f(n: Int) -> Int {\n    let spare = 1\n    twice(n)\n    n\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        // `let spare = 1` on line 7, `twice(n)` on line 8.
        code_action(2, URI, 7, 9),
        code_action(3, URI, 8, 6),
    ]);

    let unused = actions(&sent[2]);
    assert_eq!(unused.len(), 1, "{:?}", sent[2]);
    assert_eq!(
        unused[0].at(&["title"]).and_then(Json::as_str),
        Some("call it `_spare`")
    );
    assert_eq!(unused[0].at(&["isPreferred"]), Some(&Json::Bool(false)));

    let discarded = actions(&sent[3]);
    assert_eq!(discarded.len(), 1, "{:?}", sent[3]);
    assert_eq!(
        discarded[0].at(&["title"]).and_then(Json::as_str),
        Some("say the value is being dropped")
    );
    let edits = discarded[0]
        .at(&["edit", "changes", URI])
        .and_then(Json::as_array)
        .unwrap();
    assert_eq!(
        edits[0].at(&["newText"]).and_then(Json::as_str),
        Some("let _ = ")
    );
    // An insertion, so the range has no width.
    assert_eq!(
        edits[0].at(&["range", "start"]),
        edits[0].at(&["range", "end"])
    );
}

#[test]
fn a_diagnostic_with_no_patch_offers_nothing() {
    // `BAD` returns an `Int` from a function declared to return a `String`.
    // There is no obvious repair for that and the type checker does not
    // pretend there is.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, BAD),
        code_action(2, URI, 3, 4),
    ]);
    assert!(actions(&sent[2]).is_empty(), "{:?}", sent[2]);
}

#[test]
fn a_cursor_on_another_line_gets_nothing() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, MISSPELLED),
        code_action(2, URI, 0, 0),
    ]);
    assert!(actions(&sent[2]).is_empty(), "{:?}", sent[2]);
}

/// Where a cursor sits when someone has just finished typing the word.
#[test]
fn a_cursor_at_the_end_of_the_name_still_counts() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, MISSPELLED),
        code_action(2, URI, 3, 10),
    ]);
    assert_eq!(actions(&sent[2]).len(), 1, "{:?}", sent[2]);
}

#[test]
fn an_editor_asking_only_for_something_else_gets_nothing() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, MISSPELLED),
        code_action_only(2, URI, 3, 6, Some("refactor")),
    ]);
    assert!(actions(&sent[2]).is_empty(), "{:?}", sent[2]);
}

#[test]
fn an_editor_asking_only_for_quick_fixes_gets_them() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, MISSPELLED),
        code_action_only(2, URI, 3, 6, Some("quickfix")),
    ]);
    assert_eq!(actions(&sent[2]).len(), 1, "{:?}", sent[2]);
}

// -- the outline ------------------------------------------------------------

fn document_symbol(id: i64, uri: &str) -> String {
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"textDocument/documentSymbol\",\"params\":\
         {{\"textDocument\":{{\"uri\":\"{uri}\"}}}}}}"
    ))
}

/// One of everything, so the outline has to say what each of them is.
const DECLARED: &str = "module a\n\n\
     type Positive = Int where value > 0\n\n\
     record Point {\n    x: Int,\n    y: Int,\n}\n\n\
     choice Flag {\n    On,\n    Off,\n}\n\n\
     effect Log {\n    fn note(message: String) -> ()\n}\n\n\
     handler Quiet implements Log {\n    state seen: Int\n\n    fn note(message) -> () {}\n}\n\n\
     fn f(n: Int) -> Int {\n    n + n\n}\n\n\
     test \"it doubles\" {\n    assert f(2) == 4\n}\n";

fn outline(message: &Json) -> &[Json] {
    message
        .at(&["result"])
        .and_then(Json::as_array)
        .unwrap_or_else(|| panic!("an outline request should answer with a list: {message:?}"))
}

fn named<'a>(symbols: &'a [Json], name: &str) -> &'a Json {
    symbols
        .iter()
        .find(|symbol| symbol.at(&["name"]).and_then(Json::as_str) == Some(name))
        .unwrap_or_else(|| panic!("no symbol called {name}: {symbols:?}"))
}

#[test]
fn the_outline_says_what_each_declaration_is() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, DECLARED),
        document_symbol(2, URI),
    ]);
    let symbols = outline(&sent[2]);

    // In the order the file declares them, because an outline that sorted
    // would be a worse version of a list of names.
    let names: Vec<&str> = symbols
        .iter()
        .filter_map(|symbol| symbol.at(&["name"]).and_then(Json::as_str))
        .collect();
    assert_eq!(
        names,
        vec![
            "Positive",
            "Point",
            "Flag",
            "Log",
            "Quiet",
            "f",
            "test \"it doubles\""
        ]
    );

    for (name, kind) in [
        ("Positive", 11),
        ("Point", 23),
        ("Flag", 10),
        ("Log", 11),
        ("Quiet", 5),
        ("f", 12),
    ] {
        assert_eq!(
            named(symbols, name).at(&["kind"]).and_then(Json::as_i64),
            Some(kind),
            "{name}"
        );
    }
}

#[test]
fn what_belongs_to_a_declaration_is_nested_under_it() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, DECLARED),
        document_symbol(2, URI),
    ]);
    let symbols = outline(&sent[2]);

    let children = |name: &str| -> Vec<String> {
        named(symbols, name)
            .at(&["children"])
            .and_then(Json::as_array)
            .unwrap()
            .iter()
            .filter_map(|child| child.at(&["name"]).and_then(Json::as_str))
            .map(str::to_string)
            .collect()
    };

    assert_eq!(children("Point"), vec!["x", "y"]);
    assert_eq!(children("Flag"), vec!["On", "Off"]);
    assert_eq!(children("Log"), vec!["note"]);
    // State first, then what it is state for.
    assert_eq!(children("Quiet"), vec!["seen", "note"]);
    assert!(children("f").is_empty());
}

/// The whole declaration is what an editor highlights when you pick it. The
/// name is where the cursor lands. A server that answered the same range twice
/// would put the cursor on the `record` keyword.
#[test]
fn the_range_is_the_declaration_and_the_selection_is_the_name() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, DECLARED),
        document_symbol(2, URI),
    ]);
    let point = named(outline(&sent[2]), "Point");

    assert_eq!(
        point.at(&["selectionRange", "start", "character"]),
        Some(&Json::Number(7.0)),
        "the name starts after `record `"
    );
    assert_eq!(
        point.at(&["range", "start", "character"]),
        Some(&Json::Number(0.0))
    );
    assert!(
        point.at(&["range", "end", "line"]).and_then(Json::as_i64)
            > point
                .at(&["selectionRange", "end", "line"])
                .and_then(Json::as_i64),
        "the declaration runs past its own name"
    );
}

/// Everything else this server answers is about a name that resolved. An
/// outline is about what is written, and half a declaration is still worth
/// drawing while somebody is typing the rest of it.
#[test]
fn a_file_that_does_not_check_still_has_an_outline() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, "module a\n\nfn f() -> Nope {\n    missing()\n}\n"),
        document_symbol(2, URI),
    ]);
    assert_eq!(outline(&sent[2]).len(), 1, "{:?}", sent[2]);
}

#[test]
fn an_outline_for_a_document_nobody_opened_is_null() {
    let sent = session(&[request(1, "initialize"), document_symbol(2, URI)]);
    assert_eq!(sent[1].at(&["result"]), Some(&Json::Null));
}

// -- the workspace ----------------------------------------------------------

/// Two modules that see each other, which is the case the server used to get
/// wrong.
const EXPORTER: &str = "module scratch/one\n\nfn double(n: Int) -> Int {\n    n + n\n}\n";
const IMPORTER: &str =
    "module scratch/two\n\nuse scratch/one.{double}\n\nfn f() -> Int {\n    double(2)\n}\n";

#[test]
fn a_file_that_imports_another_one_is_fine_when_the_workspace_has_it() {
    // The bug this fixes. Every file with a `use` in it used to get a red line
    // under the import, because the module it names was not among the files
    // being compiled. A server that reports errors on a working program is
    // worse than one that reports nothing.
    let scratch = Scratch::new("imports");
    scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", IMPORTER);

    let sent = session(&[initialize_in(1, &scratch), did_open(&two, IMPORTER)]);

    assert_eq!(published_for(&sent, &two).len(), 0, "{sent:?}");
}

#[test]
fn without_a_workspace_folder_an_import_still_has_nothing_behind_it() {
    // An editor that names no folder gets the single file behaviour, which is
    // honest for a server that has been handed one file. Guessing at a
    // directory would be inventing the set of files being compiled.
    let sent = session(&[request(1, "initialize"), did_open(URI, IMPORTER)]);

    let diagnostics = published_for(&sent, URI);
    assert_eq!(
        diagnostics[0].at(&["code"]).and_then(Json::as_str),
        Some("DEED3007"),
        "{diagnostics:?}"
    );
}

#[test]
fn the_open_document_is_not_checked_twice() {
    // It is in the workspace and it is open, and adding both copies would be
    // two files claiming one `module` line, which is an error about a program
    // that is fine.
    let scratch = Scratch::new("twice");
    let one = scratch.write("one.deed", EXPORTER);

    let sent = session(&[initialize_in(1, &scratch), did_open(&one, EXPORTER)]);

    assert_eq!(published_for(&sent, &one).len(), 0, "{sent:?}");
}

#[test]
fn an_unsaved_change_in_one_file_is_seen_by_the_other() {
    // The reason the editor's buffer has to win over the file on disk. Both
    // files are open, one of them stops exporting what the other imports, and
    // nothing has been saved.
    let scratch = Scratch::new("unsaved");
    let one = scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", IMPORTER);

    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&one, EXPORTER),
        did_open(&two, IMPORTER),
        did_change(
            &one,
            "module scratch/one\n\nfn halve(n: Int) -> Int {\n    n\n}\n",
        ),
    ]);

    let diagnostics = published_for(&sent, &two);
    assert_eq!(
        diagnostics[0].at(&["code"]).and_then(Json::as_str),
        Some("DEED3008"),
        "the importer should have noticed: {diagnostics:?}"
    );

    // And what is on disk is still fine, which is the point: nothing was
    // saved, so the file behind the buffer has not changed.
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("one.deed")).unwrap(),
        EXPORTER
    );
}

#[test]
fn a_change_republishes_every_open_document_and_not_just_the_one_typed_in() {
    // Otherwise the other file keeps showing squiggles about a version of the
    // workspace that no longer exists.
    let scratch = Scratch::new("republish");
    let one = scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", IMPORTER);

    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&one, EXPORTER),
        did_open(&two, IMPORTER),
        did_change(
            &one,
            "module scratch/one\n\nfn halve(n: Int) -> Int {\n    n\n}\n",
        ),
        did_change(&one, EXPORTER),
    ]);

    assert_eq!(
        published_for(&sent, &two).len(),
        0,
        "putting the export back should have cleared the importer"
    );
}

#[test]
fn hover_reaches_across_a_module_boundary() {
    // The type of an imported function is only knowable if the other file was
    // checked alongside this one.
    let scratch = Scratch::new("hover-across");
    scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", IMPORTER);

    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&two, IMPORTER),
        // The `double` on the last line of `two.deed`.
        at(2, "textDocument/hover", &two, 5, 6),
    ]);

    let text = hover_text(sent.last().unwrap());
    assert!(text.contains("`Fn(Int) -> Int`"), "{text}");
}

#[test]
fn definition_reaches_across_a_module_boundary() {
    // An imported name leads to the file that declares it, not to the `use`
    // that brought it in. Nothing carries a `DefId` across to do that: what
    // crosses is the module path and the name, which is what crosses
    // everywhere else in this compiler.
    let scratch = Scratch::new("definition-across");
    let one = scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", IMPORTER);

    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&two, IMPORTER),
        at(2, "textDocument/definition", &two, 5, 6),
    ]);

    let location = sent.last().unwrap().at(&["result"]).expect("a location");
    assert_eq!(
        location.at(&["uri"]).and_then(Json::as_str),
        Some(one.as_str()),
        "{location:?}"
    );
    // `fn double` is on line 2 of `one.deed`, and the name starts at column 3.
    assert_eq!(
        location
            .at(&["range", "start", "line"])
            .and_then(Json::as_i64),
        Some(2),
        "{location:?}"
    );
    assert_eq!(
        location
            .at(&["range", "start", "character"])
            .and_then(Json::as_i64),
        Some(3)
    );
}

#[test]
fn an_import_with_nothing_behind_it_leads_to_the_use_line() {
    // The module is not in the workspace, which the editor is already showing
    // as an error. Answering nothing would be worse than answering with the
    // line that says what was imported and what the other file is called.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, IMPORTER),
        at(2, "textDocument/definition", URI, 5, 6),
    ]);

    let location = sent.last().unwrap().at(&["result"]).expect("a location");
    assert_eq!(location.at(&["uri"]).and_then(Json::as_str), Some(URI));
    assert_eq!(
        location
            .at(&["range", "start", "line"])
            .and_then(Json::as_i64),
        Some(2),
        "{location:?}"
    );
}

// -- find references --------------------------------------------------------

/// The locations a references request came back with.
fn locations(message: &Json) -> &[Json] {
    message
        .at(&["result"])
        .and_then(Json::as_array)
        .unwrap_or_else(|| panic!("expected locations, got {message:?}"))
}

fn lines_of(locations: &[Json], uri: &str) -> Vec<i64> {
    locations
        .iter()
        .filter(|location| location.at(&["uri"]).and_then(Json::as_str) == Some(uri))
        .filter_map(|location| {
            location
                .at(&["range", "start", "line"])
                .and_then(Json::as_i64)
        })
        .collect()
}

#[test]
fn references_to_a_local_stay_in_the_file() {
    // A parameter cannot be named anywhere else, so the answer is exact: every
    // span in this file that resolves to that one definition.
    let source =
        "module a\n\nfn f(n: Int) -> Int {\n    n + n\n}\n\nfn g(n: Int) -> Int {\n    n\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        // The `n` in the first parameter list.
        references(2, URI, 2, 5, true),
    ]);

    // Declared on line 2, used twice on line 3. The `n` in `g` is a different
    // definition and is not an answer to this question.
    assert_eq!(
        lines_of(locations(sent.last().unwrap()), URI),
        vec![2, 3, 3]
    );
}

#[test]
fn the_declaration_is_left_out_when_the_editor_says_so() {
    let source = "module a\n\nfn f(n: Int) -> Int {\n    n + n\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        references(2, URI, 2, 5, false),
    ]);

    assert_eq!(lines_of(locations(sent.last().unwrap()), URI), vec![3, 3]);
}

#[test]
fn references_to_an_exported_name_cross_the_boundary() {
    // The other module's uses count, and so does the `use` line that brought
    // the name in, which is where a reader looking at the list would want to
    // start.
    let scratch = Scratch::new("references-across");
    let one = scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", IMPORTER);

    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&one, EXPORTER),
        // The `double` in `fn double`.
        references(2, &one, 2, 3, true),
    ]);

    let found = locations(sent.last().unwrap());
    assert_eq!(lines_of(found, &one), vec![2], "{found:?}");
    // The `use` on line 2 of the importer and the call on line 5.
    assert_eq!(lines_of(found, &two), vec![2, 5], "{found:?}");
}

#[test]
fn asking_from_the_importing_side_gives_the_same_answer() {
    let scratch = Scratch::new("references-back");
    let one = scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", IMPORTER);

    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&two, IMPORTER),
        // The `double` in the call.
        references(2, &two, 5, 6, true),
    ]);

    let found = locations(sent.last().unwrap());
    assert_eq!(lines_of(found, &one), vec![2], "{found:?}");
    assert_eq!(lines_of(found, &two), vec![2, 5], "{found:?}");
}

#[test]
fn a_builtin_has_no_useful_answer() {
    // Every mention of `length` in a workspace is not an answer to a question
    // anybody was asking.
    let source = "module a\n\nfn f(s: String) -> Int {\n    length(s)\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        references(2, URI, 3, 5, true),
    ]);

    assert!(locations(sent.last().unwrap()).is_empty());
}

// -- rename ------------------------------------------------------------------

fn rename(id: i64, uri: &str, line: u32, character: u32, to: &str) -> String {
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"textDocument/rename\",\"params\":\
         {{\"textDocument\":{{\"uri\":\"{uri}\"}},\
         \"position\":{{\"line\":{line},\"character\":{character}}},\
         \"newName\":\"{to}\"}}}}"
    ))
}

/// The lines a rename wants to edit in one file, and what it wants to put
/// there.
fn edits(message: &Json, uri: &str) -> Vec<(i64, String)> {
    let changes = message
        .at(&["result", "changes"])
        .unwrap_or_else(|| panic!("expected a workspace edit, got {message:?}"));
    let Some(file) = changes.at(&[uri]) else {
        return Vec::new();
    };
    file.as_array()
        .expect("a list of edits")
        .iter()
        .map(|edit| {
            (
                edit.at(&["range", "start", "line"])
                    .and_then(Json::as_i64)
                    .expect("a line"),
                edit.at(&["newText"])
                    .and_then(Json::as_str)
                    .expect("the new text")
                    .to_string(),
            )
        })
        .collect()
}

#[test]
fn renaming_a_local_edits_every_place_it_is_written() {
    let source =
        "module a\n\nfn f(n: Int) -> Int {\n    n + n\n}\n\nfn g(n: Int) -> Int {\n    n\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        rename(2, URI, 2, 5, "count"),
    ]);

    // The declaration and both uses, and not the `n` in `g`, which is a
    // different definition that happens to be spelled the same.
    assert_eq!(
        edits(sent.last().unwrap(), URI),
        vec![
            (2, "count".to_string()),
            (3, "count".to_string()),
            (3, "count".to_string()),
        ]
    );
}

#[test]
fn renaming_an_exported_name_rewrites_the_use_line_too() {
    // The one that matters. A rename that edited the declaration and left the
    // `use` that brought it in would fix one file and break another, which is
    // worse than not having rename at all.
    let scratch = Scratch::new("rename-across");
    let one = scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", IMPORTER);

    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&one, EXPORTER),
        // The `double` in `fn double`.
        rename(2, &one, 2, 3, "twice"),
    ]);

    let message = sent.last().unwrap();
    assert_eq!(edits(message, &one), vec![(2, "twice".to_string())]);
    // The `use` on line 2 of the importer and the call on line 5.
    assert_eq!(
        edits(message, &two),
        vec![(2, "twice".to_string()), (5, "twice".to_string())]
    );
}

#[test]
fn the_declaration_is_always_edited_even_asking_from_the_other_side() {
    let scratch = Scratch::new("rename-back");
    let one = scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", IMPORTER);

    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&two, IMPORTER),
        // The `double` in the call.
        rename(2, &two, 5, 6, "twice"),
    ]);

    let message = sent.last().unwrap();
    assert_eq!(edits(message, &one), vec![(2, "twice".to_string())]);
    assert_eq!(
        edits(message, &two),
        vec![(2, "twice".to_string()), (5, "twice".to_string())]
    );
}

#[test]
fn a_prelude_name_is_not_this_workspaces_to_rename() {
    let source = "module a\n\nfn f(s: String) -> Int {\n    length(s)\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        rename(2, URI, 3, 5, "size"),
    ]);

    let message = sent.last().unwrap();
    // An error rather than an edit with nothing in it. An editor told "here is
    // your edit" reports success and changes nothing, which looks like a bug
    // in the editor.
    assert!(message.at(&["error"]).is_some(), "{message:?}");
    assert!(message.at(&["result"]).is_none(), "{message:?}");
}

#[test]
fn a_new_name_the_language_cannot_hold_is_refused() {
    let source = "module a\n\nfn f(n: Int) -> Int {\n    n + n\n}\n";
    for bad in ["fn", "2", "a b", "", "n!"] {
        let sent = session(&[
            request(1, "initialize"),
            did_open(URI, source),
            rename(2, URI, 2, 5, bad),
        ]);
        let message = sent.last().unwrap();
        assert!(
            message.at(&["error"]).is_some(),
            "`{bad}` should have been refused: {message:?}"
        );
    }
}

#[test]
fn prepare_rename_says_where_the_name_is() {
    let source = "module a\n\nfn f(n: Int) -> Int {\n    n + n\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        at(2, "textDocument/prepareRename", URI, 2, 5),
    ]);

    let range = sent.last().unwrap().at(&["result"]).expect("a range");
    assert_eq!(
        range.at(&["start", "line"]).and_then(Json::as_i64),
        Some(2),
        "{range:?}"
    );
    assert_eq!(
        range.at(&["start", "character"]).and_then(Json::as_i64),
        Some(5),
        "{range:?}"
    );
}

#[test]
fn prepare_rename_refuses_what_rename_would_refuse() {
    // So the command is greyed out rather than asking for a new spelling and
    // then turning it down.
    let source = "module a\n\nfn f(s: String) -> Int {\n    length(s)\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        at(2, "textDocument/prepareRename", URI, 3, 5),
    ]);

    assert!(
        sent.last()
            .unwrap()
            .at(&["result"])
            .is_some_and(Json::is_null),
        "{:?}",
        sent.last().unwrap()
    );
}

// -- completion ---------------------------------------------------------------

fn completion(id: i64, uri: &str, line: u32, character: u32) -> String {
    at(id, "textDocument/completion", uri, line, character)
}

/// The labels a completion request came back with, in the order it gave them.
fn labels(message: &Json) -> Vec<String> {
    message
        .at(&["result"])
        .and_then(Json::as_array)
        .unwrap_or_else(|| panic!("expected completions, got {message:?}"))
        .iter()
        .filter_map(|item| item.at(&["label"]).and_then(Json::as_str))
        .map(str::to_string)
        .collect()
}

#[test]
fn a_bare_name_offers_what_the_file_declared() {
    let source =
        "module a\n\nfn double(n: Int) -> Int {\n    n + n\n}\n\nfn go() -> Int {\n    d\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        // Just after the `d` on the last line but one.
        completion(2, URI, 7, 5),
    ]);

    let found = labels(sent.last().unwrap());
    assert!(found.contains(&"double".to_string()), "{found:?}");
    assert!(found.contains(&"go".to_string()), "{found:?}");
    // The prelude is in scope everywhere and nothing else put it there.
    assert!(found.contains(&"length".to_string()), "{found:?}");
}

#[test]
fn a_local_is_offered_inside_its_own_declaration_and_not_outside_it() {
    let source =
        "module a\n\nfn one(here: Int) -> Int {\n    here\n}\n\nfn two() -> Int {\n    0\n}\n";

    let inside = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        completion(2, URI, 3, 8),
    ]);
    assert!(
        labels(inside.last().unwrap()).contains(&"here".to_string()),
        "{:?}",
        labels(inside.last().unwrap())
    );

    let outside = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        completion(2, URI, 7, 5),
    ]);
    assert!(
        !labels(outside.last().unwrap()).contains(&"here".to_string()),
        "a parameter of another function is not in scope here"
    );
}

#[test]
fn a_dot_offers_the_fields_of_what_is_to_the_left_of_it() {
    let source = "module a\n\nrecord Task {\n    done: Bool,\n    title: String,\n}\n\nfn f(task: Task) -> Bool {\n    task.\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        // Just after the dot.
        completion(2, URI, 8, 9),
    ]);

    let found = labels(sent.last().unwrap());
    assert_eq!(found, vec!["done".to_string(), "title".to_string()]);
}

#[test]
fn a_dot_offers_nothing_for_something_with_no_fields() {
    let source = "module a\n\nfn f(n: Int) -> Int {\n    n.\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        completion(2, URI, 3, 6),
    ]);
    assert!(labels(sent.last().unwrap()).is_empty());
}

#[test]
fn a_use_line_offers_what_the_other_module_exports() {
    // The one place a name has to match another file exactly, and so the one
    // most likely to be wrong when it is typed by hand.
    let scratch = Scratch::new("completion-use");
    scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", IMPORTER);

    let typing = "module scratch/two\n\nuse scratch/one.{\n";
    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&two, typing),
        // Inside the braces, with nothing written yet.
        completion(2, &two, 2, 17),
    ]);

    let found = labels(sent.last().unwrap());
    assert!(found.contains(&"double".to_string()), "{found:?}");
}

#[test]
fn a_use_line_that_is_already_closed_is_not_one() {
    let scratch = Scratch::new("completion-use-closed");
    scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", IMPORTER);

    // Past the closing brace, so this is ordinary code and the answer should
    // be the ordinary one rather than the other module's names.
    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&two, IMPORTER),
        completion(2, &two, 5, 6),
    ]);

    let found = labels(sent.last().unwrap());
    assert!(found.contains(&"length".to_string()), "{found:?}");
}

#[test]
fn a_number_followed_by_a_dot_is_not_a_receiver() {
    // There are no float literals, which is what makes `40.try` unambiguous,
    // and it is also what stops this from being a receiver with fields.
    let source = "module a\n\nfn f() -> Int {\n    1.\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        completion(2, URI, 3, 6),
    ]);
    assert!(labels(sent.last().unwrap()).is_empty());
}

#[test]
fn every_name_appears_once() {
    let source = "module a\n\nfn f(n: Int) -> Int {\n    n + n + n\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        completion(2, URI, 3, 13),
    ]);

    let found = labels(sent.last().unwrap());
    let mut sorted = found.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(found, sorted, "the list should be sorted and unique");
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

// -- signature help -----------------------------------------------------------

fn signature_help(id: i64, uri: &str, line: u32, character: u32) -> String {
    at(id, "textDocument/signatureHelp", uri, line, character)
}

/// The one signature offered, and which parameter it says is being written.
fn signature(message: &Json) -> (String, i64) {
    let signatures = message
        .at(&["result", "signatures"])
        .and_then(Json::as_array)
        .unwrap_or_else(|| panic!("expected a signature, got {message:?}"));
    let label = signatures
        .first()
        .and_then(|first| first.at(&["label"]))
        .and_then(Json::as_str)
        .expect("a signature has a label")
        .to_string();
    let active = message
        .at(&["result", "activeParameter"])
        .and_then(Json::as_i64)
        .expect("an active parameter");
    (label, active)
}

const CALLS: &str = "module a\n\n\
     fn add(left: Int, right: Int) -> Int {\n\
     \x20   left + right\n\
     }\n\n\
     fn go() -> Int {\n\
     \x20   add(1, 2)\n\
     }\n";

#[test]
fn a_call_says_what_it_takes_and_what_it_gives_back() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, CALLS),
        // Just inside the `(` of `add(1, 2)`.
        signature_help(2, URI, 7, 8),
    ]);

    let (label, active) = signature(sent.last().unwrap());
    assert_eq!(label, "add(Int, Int) -> Int");
    assert_eq!(active, 0);
}

#[test]
fn a_comma_moves_to_the_next_parameter() {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, CALLS),
        // After the comma in `add(1, 2)`.
        signature_help(2, URI, 7, 11),
    ]);

    assert_eq!(signature(sent.last().unwrap()).1, 1);
}

#[test]
fn the_row_is_part_of_the_signature() {
    // What a call performs is what it costs, and the call site is the one
    // place it is not written down. This is the reason to have the feature at
    // all rather than let an editor show types alone.
    let source = "module a\n\n\
         fn shout(console: Console, message: String) -> () uses Io.write {\n\
         \x20   Io.write(console, message)\n\
         }\n\n\
         fn go(console: Console) -> () uses Io.write {\n\
         \x20   shout(console, \"hi\")\n\
         }\n";

    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        signature_help(2, URI, 7, 10),
    ]);

    let (label, _) = signature(sent.last().unwrap());
    assert!(label.contains("uses Io.write"), "{label}");
}

#[test]
fn a_call_inside_an_argument_is_the_one_answered_about() {
    // `add(add(1, |2), 3)`. The cursor is on the second argument of the inner
    // call, and a counter rather than a stack would have said the third of
    // the outer one.
    let source = "module a\n\n\
         fn add(left: Int, right: Int) -> Int {\n\
         \x20   left + right\n\
         }\n\n\
         fn go() -> Int {\n\
         \x20   add(add(1, 2), 3)\n\
         }\n";

    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        // Just before the `2`.
        signature_help(2, URI, 7, 15),
    ]);

    assert_eq!(signature(sent.last().unwrap()).1, 1);
}

#[test]
fn a_comma_in_a_list_belongs_to_the_list() {
    let source = "module a\n\n\
         fn total(items: List<Int>, extra: Int) -> Int {\n\
         \x20   extra\n\
         }\n\n\
         fn go() -> Int {\n\
         \x20   total([1, 2, 3], 4)\n\
         }\n";

    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        // Inside the list, after its second comma.
        signature_help(2, URI, 7, 18),
    ]);

    // Still the first argument of `total`, because the commas counted so far
    // were the list's.
    assert_eq!(signature(sent.last().unwrap()).1, 0);
}

#[test]
fn a_bracket_that_opens_nothing_is_not_a_call() {
    let source = "module a\n\nfn go() -> Int {\n    (1 + 2)\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        signature_help(2, URI, 3, 6),
    ]);

    assert!(
        sent.last()
            .unwrap()
            .at(&["result"])
            .is_some_and(Json::is_null),
        "grouping is not a call and has no signature to show"
    );
}

#[test]
fn a_parenthesis_inside_a_string_does_not_open_a_call() {
    // Without skipping strings this `(` never closes, and every later
    // position in the file would be answered about a call that is not there.
    let source = "module a\n\n\
         fn go(console: Console) -> () uses Io.write {\n\
         \x20   Io.write(console, \"a ( in a message\")\n\
         \x20   let n = 1\n\
         }\n";

    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        // On the line after the string, well outside any call.
        signature_help(2, URI, 4, 13),
    ]);

    assert!(
        sent.last()
            .unwrap()
            .at(&["result"])
            .is_some_and(Json::is_null),
        "the `(` inside the string should not have opened a call"
    );
}

#[test]
fn the_server_says_it_answers_signature_help() {
    let sent = session(&[request(1, "initialize")]);
    let triggers = sent[0]
        .at(&[
            "result",
            "capabilities",
            "signatureHelpProvider",
            "triggerCharacters",
        ])
        .and_then(Json::as_array)
        .expect("signature help should be advertised");
    let triggers: Vec<&str> = triggers.iter().filter_map(Json::as_str).collect();
    assert_eq!(triggers, vec!["(", ","]);
}

// -- workspace symbol ---------------------------------------------------------

fn workspace_symbol(id: i64, query: &str) -> String {
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"workspace/symbol\",\
         \"params\":{{\"query\":\"{query}\"}}}}"
    ))
}

fn symbol_names(message: &Json) -> Vec<String> {
    message
        .at(&["result"])
        .and_then(Json::as_array)
        .unwrap_or_else(|| panic!("expected symbols, got {message:?}"))
        .iter()
        .filter_map(|symbol| symbol.at(&["name"]).and_then(Json::as_str))
        .map(str::to_string)
        .collect()
}

const ONE: &str = "module scratch/one\n\n\
     record Ledger {\n\
     \x20   total: Int,\n\
     }\n\n\
     fn deposit(into: Int, amount: Int) -> Int {\n\
     \x20   into + amount\n\
     }\n";

const TWO: &str = "module scratch/two\n\n\
     fn withdraw(from: Int, amount: Int) -> Int {\n\
     \x20   from - amount\n\
     }\n";

#[test]
fn an_empty_query_is_every_declaration_in_the_workspace() {
    // What an editor sends the moment the box opens, and answering it is how
    // the list appears before anything is typed.
    let scratch = Scratch::new("workspace-symbol-all");
    scratch.write("one.deed", ONE);
    scratch.write("two.deed", TWO);

    let sent = session(&[initialize_in(1, &scratch), workspace_symbol(2, "")]);
    let found = symbol_names(sent.last().unwrap());

    assert!(found.contains(&"deposit".to_string()), "{found:?}");
    assert!(found.contains(&"withdraw".to_string()), "{found:?}");
    assert!(found.contains(&"Ledger".to_string()), "{found:?}");
}

#[test]
fn a_query_is_a_substring_and_ignores_case() {
    let scratch = Scratch::new("workspace-symbol-query");
    scratch.write("one.deed", ONE);
    scratch.write("two.deed", TWO);

    let sent = session(&[initialize_in(1, &scratch), workspace_symbol(2, "DRAW")]);
    let found = symbol_names(sent.last().unwrap());

    assert_eq!(found, vec!["withdraw".to_string()]);
}

#[test]
fn a_parameter_is_not_something_to_jump_to_across_a_workspace() {
    // `amount` is written in both files and cannot be named from outside the
    // function that has it, so it is not an answer to "where is the thing I
    // half remember the name of".
    let scratch = Scratch::new("workspace-symbol-locals");
    scratch.write("one.deed", ONE);
    scratch.write("two.deed", TWO);

    let sent = session(&[initialize_in(1, &scratch), workspace_symbol(2, "amount")]);
    assert!(
        symbol_names(sent.last().unwrap()).is_empty(),
        "a parameter is not a workspace symbol"
    );
}

#[test]
fn a_symbol_says_which_module_it_is_in_and_where() {
    let scratch = Scratch::new("workspace-symbol-where");
    let one = scratch.write("one.deed", ONE);
    scratch.write("two.deed", TWO);

    let sent = session(&[initialize_in(1, &scratch), workspace_symbol(2, "deposit")]);
    let result = sent.last().unwrap().at(&["result"]).unwrap();
    let first = result
        .as_array()
        .and_then(|all| all.first())
        .expect("one symbol");

    assert_eq!(
        first.at(&["containerName"]).and_then(Json::as_str),
        Some("scratch/one")
    );
    assert_eq!(
        first.at(&["location", "uri"]).and_then(Json::as_str),
        Some(one.as_str())
    );
    // Counting from zero: the module line, a blank, the three lines of the
    // record, a blank, and then `deposit`.
    assert_eq!(
        first
            .at(&["location", "range", "start", "line"])
            .and_then(Json::as_i64),
        Some(6)
    );
}

#[test]
fn an_unsaved_change_is_searched_rather_than_what_is_on_disk() {
    // The same rule every other answer follows: the file on the screen is the
    // file, and it may not have been saved.
    let scratch = Scratch::new("workspace-symbol-unsaved");
    let two = scratch.write("two.deed", TWO);

    let renamed = "module scratch/two\n\nfn refund(to: Int) -> Int {\n    to\n}\n";
    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&two, renamed),
        workspace_symbol(2, "re"),
    ]);

    let found = symbol_names(sent.last().unwrap());
    assert!(found.contains(&"refund".to_string()), "{found:?}");
    assert!(
        !found.contains(&"withdraw".to_string()),
        "the saved name is gone from the buffer: {found:?}"
    );
}

#[test]
fn the_server_says_it_answers_workspace_symbol() {
    let sent = session(&[request(1, "initialize")]);
    assert_eq!(
        sent[0].at(&["result", "capabilities", "workspaceSymbolProvider"]),
        Some(&Json::Bool(true))
    );
}
