//! A whole session, driven through the same entry point an editor uses.
//!
//! The unit tests in the crate cover the pieces. These cover the thing an
//! editor actually does: open a file, get squiggles, fix it, get none, close
//! it, and get told the squiggles are gone.

use std::io::BufReader;
use std::path::{Path, PathBuf};

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
        let dir = std::env::temp_dir().join(format!("vow-lsp-{tag}-{nanos}"));
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

    for capability in [
        "hoverProvider",
        "definitionProvider",
        "documentFormattingProvider",
    ] {
        assert_eq!(
            sent[0].at(&["result", "capabilities", capability]),
            Some(&Json::Bool(true)),
            "{capability} should be advertised, or an editor never asks"
        );
    }
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
    let sent = session(&[
        request(1, "initialize"),
        request(2, "textDocument/codeAction"),
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
    scratch.write("one.vow", EXPORTER);
    let two = scratch.write("two.vow", IMPORTER);

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
        Some("VOW3007"),
        "{diagnostics:?}"
    );
}

#[test]
fn the_open_document_is_not_checked_twice() {
    // It is in the workspace and it is open, and adding both copies would be
    // two files claiming one `module` line, which is an error about a program
    // that is fine.
    let scratch = Scratch::new("twice");
    let one = scratch.write("one.vow", EXPORTER);

    let sent = session(&[initialize_in(1, &scratch), did_open(&one, EXPORTER)]);

    assert_eq!(published_for(&sent, &one).len(), 0, "{sent:?}");
}

#[test]
fn an_unsaved_change_in_one_file_is_seen_by_the_other() {
    // The reason the editor's buffer has to win over the file on disk. Both
    // files are open, one of them stops exporting what the other imports, and
    // nothing has been saved.
    let scratch = Scratch::new("unsaved");
    let one = scratch.write("one.vow", EXPORTER);
    let two = scratch.write("two.vow", IMPORTER);

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
        Some("VOW3008"),
        "the importer should have noticed: {diagnostics:?}"
    );

    // And what is on disk is still fine, which is the point: nothing was
    // saved, so the file behind the buffer has not changed.
    assert_eq!(
        std::fs::read_to_string(scratch.0.join("one.vow")).unwrap(),
        EXPORTER
    );
}

#[test]
fn a_change_republishes_every_open_document_and_not_just_the_one_typed_in() {
    // Otherwise the other file keeps showing squiggles about a version of the
    // workspace that no longer exists.
    let scratch = Scratch::new("republish");
    let one = scratch.write("one.vow", EXPORTER);
    let two = scratch.write("two.vow", IMPORTER);

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
    scratch.write("one.vow", EXPORTER);
    let two = scratch.write("two.vow", IMPORTER);

    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&two, IMPORTER),
        // The `double` on the last line of `two.vow`.
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
    let one = scratch.write("one.vow", EXPORTER);
    let two = scratch.write("two.vow", IMPORTER);

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
    // `fn double` is on line 2 of `one.vow`, and the name starts at column 3.
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
    let one = scratch.write("one.vow", EXPORTER);
    let two = scratch.write("two.vow", IMPORTER);

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
    let one = scratch.write("one.vow", EXPORTER);
    let two = scratch.write("two.vow", IMPORTER);

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
    let one = scratch.write("one.vow", EXPORTER);
    let two = scratch.write("two.vow", IMPORTER);

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
    let one = scratch.write("one.vow", EXPORTER);
    let two = scratch.write("two.vow", IMPORTER);

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
    scratch.write("one.vow", EXPORTER);
    let two = scratch.write("two.vow", IMPORTER);

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
    scratch.write("one.vow", EXPORTER);
    let two = scratch.write("two.vow", IMPORTER);

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
