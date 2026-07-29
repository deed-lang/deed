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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
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
    initialize_at(id, &scratch.uri())
}

/// The same, for a folder that is not a scratch one.
fn initialize_at(id: i64, uri: &str) -> String {
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

/// The capabilities `editors/README.md` does not call by their field name
/// with the `Provider` filed off.
///
/// Most of them it does: `documentSymbolProvider` is "document symbol" and
/// `signatureHelpProvider` is "signature help", so [`english_for`] works those
/// out and stores nothing. Only three are irregular, and they are here.
///
/// The reason for deriving rather than tabulating is that a stored pair can be
/// permuted and nothing notices. Written out in full, this table could say
/// `hoverProvider` is "document symbol" and `documentSymbolProvider` is
/// "hover", and the check below would go green while the document told an
/// editor author the server answers something it does not advertise, which is
/// the one event the check exists to prevent. A derived pair cannot be written
/// down wrongly because it is not written down. The ones that are left are
/// held to their fields by
/// [`each_irregular_name_is_irregular_and_still_names_its_capability`].
const IRREGULAR: [(&str, &str); 3] = [
    ("codeActionProvider", "code actions"),
    ("definitionProvider", "go to definition"),
    ("documentFormattingProvider", "formatting"),
];

/// The rule: drop `Provider`, and write the camel case out as words.
fn spelled_out(field: &str) -> String {
    let stem = field.strip_suffix("Provider").unwrap_or_else(|| {
        panic!(
            "`{field}` is advertised and is neither `textDocumentSync` nor a `...Provider`, so there is no rule for what editors/README.md would call it"
        )
    });
    assert!(
        !stem.is_empty(),
        "a capability called `Provider` and nothing else has no name to write out"
    );

    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    for letter in stem.chars() {
        if letter.is_ascii_uppercase() && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        word.push(letter.to_ascii_lowercase());
    }
    words.push(word);
    words.join(" ")
}

/// What `editors/README.md` calls a capability: the rule, or the exception.
fn english_for(field: &str) -> String {
    match IRREGULAR.iter().find(|(name, _)| *name == field) {
        Some((_, english)) => (*english).to_string(),
        None => spelled_out(field),
    }
}

/// The word a phrase ends in, with a plural `s` taken off, which is the thing
/// the phrase is about: "code actions" and "code action" are both about an
/// action, and "go to definition" is about a definition.
fn head_word(phrase: &str) -> &str {
    phrase
        .rsplit(' ')
        .next()
        .expect("splitting a phrase gives at least one word")
        .trim_end_matches('s')
}

/// `editors/README.md`, which is the document this test is about.
fn editor_guide() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("this crate lives two directories under the repository root");
    let path = root.join("editors").join("README.md");
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} should be there", path.display()))
}

#[test]
fn the_advertised_set_is_exactly_this() {
    // Every other test here asks whether one capability is present, which says
    // nothing about one that appeared. `editors/README.md` is where an editor
    // author reads what the server answers, and a configuration is written
    // once and then trusted, so a provider arriving unannounced is a feature
    // nobody is told about.
    //
    // That was the reason this gave, and then it compared against a list
    // written out in Rust, so the document it named for its whole argument
    // could have said anything at all. It reads the document now.
    let guide = editor_guide();

    // The whole clause, because `and answers ` on its own also starts the
    // sentence about which files the server answers about, and a marker that
    // matches two sentences reads whichever one moved last.
    const ANSWERS: &str = "It publishes diagnostics on open and on change, and answers ";
    let listed: Vec<String> = {
        let start = guide.find(ANSWERS).unwrap_or_else(|| {
            panic!("editors/README.md should still say {ANSWERS:?}, which is where it says what the server does")
        });
        let rest = &guide[start + ANSWERS.len()..];
        let end = rest
            .find('.')
            .expect("the sentence saying what the server answers should end");
        rest[..end]
            .replace('\n', " ")
            .split(',')
            .flat_map(|part| part.split(" and "))
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect()
    };
    assert!(
        !listed.is_empty(),
        "editors/README.md names nothing the server answers, so the sentence has been reworded"
    );

    let sent = session(&[request(1, "initialize")]);
    let Some(Json::Object(fields)) = sent[0].at(&["result", "capabilities"]) else {
        panic!("initialize should answer with capabilities");
    };
    let advertised: Vec<&str> = fields.iter().map(|(name, _)| name.as_str()).collect();
    assert!(
        !advertised.is_empty(),
        "the server advertised nothing, so there is no set for the document to be wrong about"
    );

    // Sync is not something the server answers, so it is not in that sentence.
    // The document gives it a paragraph of its own, and this is the claim in it.
    assert!(
        advertised.contains(&"textDocumentSync"),
        "the server no longer syncs documents and editors/README.md still describes how it does"
    );
    assert!(
        guide.contains("Full sync only."),
        "editors/README.md should still say \"Full sync only.\", which is the only place sync is written down"
    );

    let answered: Vec<String> = advertised
        .iter()
        .filter(|name| **name != "textDocumentSync")
        .map(|name| english_for(name))
        .collect();
    assert!(
        !answered.is_empty(),
        "the server advertises nothing but document sync, so there is nothing for that sentence to be about"
    );

    for (name, english) in advertised
        .iter()
        .filter(|name| **name != "textDocumentSync")
        .zip(&answered)
    {
        assert!(
            listed.iter().any(|item| item == english),
            "`{name}` is advertised and editors/README.md does not say the server answers {english}"
        );
    }

    for item in &listed {
        assert!(
            answered.iter().any(|english| english == item),
            "editors/README.md says the server answers {item:?} and the server advertises nothing that is called that"
        );
    }
}

/// Every exception in [`IRREGULAR`] is one, and is about its own capability.
///
/// Three pairs are still written out by hand, and a pair written by hand can
/// be written against the wrong field. Nothing above would catch that: the
/// document and the server would be compared through a dictionary that agrees
/// with itself and with neither of them. So each entry has to earn its place.
/// It has to name something the server really advertises, or it is dead. It
/// has to differ from what the rule already produces, or it should not be
/// here. And it has to end in the word the rule ends in, which is what makes
/// it the same capability said differently rather than a different one:
/// "formatting" is what `documentFormattingProvider` does and "code actions"
/// is not, so writing the second against the first fails here.
#[test]
fn each_irregular_name_is_irregular_and_still_names_its_capability() {
    assert!(
        !IRREGULAR.is_empty(),
        "no name is written out by hand any more, so delete this rather than let it pass on nothing"
    );

    let sent = session(&[request(1, "initialize")]);
    let Some(Json::Object(fields)) = sent[0].at(&["result", "capabilities"]) else {
        panic!("initialize should answer with capabilities");
    };
    let advertised: Vec<&str> = fields.iter().map(|(name, _)| name.as_str()).collect();
    assert!(
        !advertised.is_empty(),
        "the server advertised nothing, so no exception here can be about anything"
    );

    for (field, english) in IRREGULAR {
        assert!(
            advertised.contains(&field),
            "`{field}` is written out here and the server does not advertise it, so this entry is dead and nothing else would say so"
        );
        let rule = spelled_out(field);
        assert_ne!(
            english,
            rule.as_str(),
            "`{field}` is written out here as the exception it is not, because the rule already gives {rule:?}"
        );
        assert_eq!(
            head_word(english),
            head_word(&rule),
            "`{field}` is written out here as {english:?}, and the rule gives {rule:?}, so the two are not about the same capability"
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

// -- the tier, where the reader is ------------------------------------------

/// One file carrying one obligation of each kind, and one contract of each
/// shape.
///
/// `deed check --obligations` on this text reports three: `halve ensures ok`
/// is tested, `Positive` on the call is guarded, and `halve requires` at the
/// same call is proven. The last two sit on the same span, which is why a
/// hover reports every obligation covering a position rather than picking one.
///
/// `announce` is the one that writes all three contract clauses. It performs
/// an effect and nothing calls it, so `use_it` stays pure and stays the one
/// with no contract at all.
const CONTRACTS: &str = "module a\n\n\
     effect Log {\n\
     \x20   fn note(line: String) -> ()\n\
     }\n\n\
     type Positive = Int where value > 0\n\n\
     fn halve(n: Int) -> Int\n\
     \x20 where\n\
     \x20   n > 1,\n\
     \x20 ensures\n\
     \x20   ok => result >= 0,\n\
     {\n\
     \x20   n / 2\n\
     }\n\n\
     fn use_it() -> Positive {\n\
     \x20   halve(10)\n\
     }\n\n\
     fn announce(line: String) -> Int\n\
     \x20 where\n\
     \x20   length(line) > 0,\n\
     \x20 uses\n\
     \x20   Log.note,\n\
     \x20 ensures\n\
     \x20   ok => result > 0,\n\
     {\n\
     \x20   Log.note(line)\n\
     \x20   1\n\
     }\n";

#[test]
fn hovering_over_a_call_says_which_tier_its_precondition_landed_in() {
    // The line the issue is about. `deed check --obligations` said `proven`
    // about this call and an editor said nothing at all, so a reader could not
    // tell a discharged contract from one nobody looked at.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, CONTRACTS),
        at(2, "textDocument/hover", URI, 18, 4),
    ]);

    let text = hover_text(&sent[2]);
    assert!(text.contains("`halve requires`, proven"), "{text}");
}

#[test]
fn a_position_carrying_two_obligations_reports_both_of_them() {
    // The call is `Guarded` against the return type's refinement and `Proven`
    // against the callee's `where`, at the same span. Reporting one of them
    // would be picking, and the terminal picks neither.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, CONTRACTS),
        at(2, "textDocument/hover", URI, 18, 4),
    ]);

    let text = hover_text(&sent[2]);
    assert!(text.contains("`Positive`, guarded"), "{text}");
    assert!(text.contains("`halve requires`, proven"), "{text}");
}

#[test]
fn hovering_over_an_ensures_clause_says_it_is_tested() {
    // Nothing else answers here. An `ensures` clause is not an expression and
    // the resolver keeps no name for it, so before this the tooltip was empty
    // on the one line that states what the function guarantees.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, CONTRACTS),
        at(2, "textDocument/hover", URI, 12, 6),
    ]);

    let text = hover_text(&sent[2]);
    assert!(text.contains("`halve ensures ok`, tested"), "{text}");
}

#[test]
fn hovering_over_a_function_name_shows_its_contract() {
    // The review surface, quoted from the file. The type line above it says
    // what goes in and comes out and says nothing about what is required,
    // performed or guaranteed, which is the half this language is about.
    //
    // All three clauses and in the order they are written, because the tree
    // keeps them in three lists and a renderer that dropped one would still
    // look like a contract to anyone who did not know what was in the file.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, CONTRACTS),
        at(2, "textDocument/hover", URI, 21, 4),
    ]);

    let text = hover_text(&sent[2]);
    assert!(
        text.contains(
            "```deed\nwhere length(line) > 0\nuses Log.note\nensures ok => result > 0\n```"
        ),
        "{text}"
    );
}

#[test]
fn a_contract_with_only_some_clauses_writes_only_those() {
    // `halve` performs nothing, so there is no `uses` line to write. A
    // renderer that wrote every heading whether or not it had anything under
    // it would be saying something about the function that is not in the file.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, CONTRACTS),
        at(2, "textDocument/hover", URI, 8, 4),
    ]);

    let text = hover_text(&sent[2]);
    assert!(
        text.contains("```deed\nwhere n > 1\nensures ok => result >= 0\n```"),
        "{text}"
    );
}

#[test]
fn a_function_with_nothing_in_its_contract_gets_no_empty_block() {
    // `use_it` declares no contract. Most names in a file are like this, so a
    // block saying so under each of them would be noise where a reader looks
    // most.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, CONTRACTS),
        at(2, "textDocument/hover", URI, 17, 4),
    ]);

    let text = hover_text(&sent[2]);
    assert!(text.contains("`use_it`, a function"), "{text}");
    assert!(!text.contains("```"), "{text}");
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
    // The first half of the bug this fixes. Every file with a `use` in it used
    // to get a red line under the import, because the module it names was not
    // among the files being compiled. A server that reports errors on a
    // working program is worse than one that reports nothing.
    //
    // The second half was the same thing for a module that is in the compiler
    // rather than next door, and is further down under its own heading.
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
fn a_contract_does_not_reach_across_a_module_boundary() {
    // What crosses is the type, which the line above already says. The
    // contract is quoted from the file being hovered, and quoting the other
    // one would mean holding its text, which would mean checking the
    // workspace a second time on a keystroke. Go to definition is one
    // keypress and already goes there.
    //
    // The importing file has a contract of its own, so a hover that reached
    // for whichever contract it found first would answer with that one and
    // this would notice.
    let scratch = Scratch::new("contract-across");
    scratch.write(
        "one.deed",
        "module scratch/one\n\nfn double(n: Int) -> Int\n  where\n    n > 0,\n{\n    n + n\n}\n",
    );
    let importer = "module scratch/two\n\nuse scratch/one.{double}\n\n\
         fn f() -> Int {\n    double(2)\n}\n\n\
         fn g(n: Int) -> Int\n  where\n    n < 99,\n{\n    n\n}\n";
    let two = scratch.write("two.deed", importer);

    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&two, importer),
        at(2, "textDocument/hover", &two, 5, 6),
    ]);

    let text = hover_text(sent.last().unwrap());
    assert!(text.contains("`Fn(Int) -> Int`"), "{text}");
    assert!(!text.contains("```"), "{text}");
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

// -- the library that ships inside the compiler ------------------------------

/// The folder of examples in this repository.
///
/// Real files rather than made up ones, because the bug was reported against
/// these three: `deed check` was silent on them and the editor was not.
fn examples() -> PathBuf {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("this crate lives two directories under the repository root");
    crates.join("examples")
}

/// A file's own module, which is a workspace's own answer to a `use`.
const OWN_LIST: &str = "module std/list\n\nfn mine(n: Int) -> Int {\n    n\n}\n";

#[test]
fn an_example_that_imports_the_library_that_ships_is_not_an_error() {
    // The finding. `examples/todo.deed`, `examples/using_list.deed` and
    // `examples/logs.deed` all import a module that lives inside the compiler,
    // and the server used to know nothing about those, so it put
    // `DEED3007 UNKNOWN_MODULE` under a `use` line that `deed check` accepts.
    let examples = examples();
    let folder = file_uri(&examples);

    let named = ["todo.deed", "using_list.deed", "logs.deed"];
    let mut checked = 0;
    for name in named {
        let path = examples.join(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{} should be readable: {error}", path.display()));
        // The file has to be the kind of file this is about, or the test below
        // passes for the wrong reason. `lines()` and `trim_end()` because the
        // text on disk has whichever line ending this checkout was made with.
        assert!(
            text.lines()
                .any(|line| line.trim_end().starts_with("use std/")),
            "{name} should import a module that ships"
        );

        let uri = file_uri(&path);
        let sent = session(&[initialize_at(1, &folder), did_open(&uri, &text)]);

        let unknown: Vec<&Json> = published_for(&sent, &uri)
            .iter()
            .filter(|diagnostic| {
                diagnostic.at(&["code"]).and_then(Json::as_str) == Some("DEED3007")
            })
            .collect();
        assert!(
            unknown.is_empty(),
            "{name} should have nothing unknown in it: {unknown:?}"
        );
        checked += 1;
    }
    assert_eq!(checked, named.len(), "every named example should be opened");
}

#[test]
fn a_workspaces_own_module_wins_over_the_one_that_ships() {
    // The precedence half, and the reason the compiler's table is asked last
    // rather than first. The one that is right there is the one they can read
    // and the one they can change, so it is the one that answers.
    let scratch = Scratch::new("shipped-own");
    scratch.write("std/list.deed", OWN_LIST);
    let text = "module two\n\nuse std/list.{mine}\n\nfn f() -> Int {\n    mine(1)\n}\n";
    let two = scratch.write("two.deed", text);

    let sent = session(&[initialize_in(1, &scratch), did_open(&two, text)]);
    assert_eq!(
        published_for(&sent, &two).len(),
        0,
        "their own `std/list` declares `mine`: {sent:?}"
    );
}

#[test]
fn what_the_shipped_module_exports_is_not_reachable_once_it_has_been_replaced() {
    // The other side of the same rule, and the one that fails if the table is
    // consulted first: `map` is in the module that ships and not in theirs, so
    // importing it has to be an error about a name rather than silently
    // reaching past the file they wrote.
    let scratch = Scratch::new("shipped-replaced");
    scratch.write("std/list.deed", OWN_LIST);
    let text = "module two\n\nuse std/list.{map}\n\nfn f() -> Int {\n    1\n}\n";
    let two = scratch.write("two.deed", text);

    let sent = session(&[initialize_in(1, &scratch), did_open(&two, text)]);
    let diagnostics = published_for(&sent, &two);
    assert_eq!(
        diagnostics
            .first()
            .and_then(|diagnostic| diagnostic.at(&["code"]))
            .and_then(Json::as_str),
        Some("DEED3008"),
        "{diagnostics:?}"
    );
}

#[test]
fn nothing_is_published_for_a_file_the_person_does_not_have() {
    // A module that ships is context, the same way it is for `deed check`. It
    // has no file behind it, so there is no document for the editor to put a
    // squiggle on and no event that would ever clear one.
    let scratch = Scratch::new("shipped-quiet");
    let text = "module two\n\nuse std/list.{map}\n\nfn f(xs: List<Int>) -> List<Int> {\n    map(xs, |n: Int| n + 1)\n}\n";
    let two = scratch.write("two.deed", text);

    let sent = session(&[initialize_in(1, &scratch), did_open(&two, text)]);

    let addressed: Vec<&str> = sent
        .iter()
        .filter(|message| published(message).is_some())
        .filter_map(|message| message.at(&["params", "uri"]).and_then(Json::as_str))
        .collect();
    assert!(
        !addressed.is_empty(),
        "something should have been published"
    );
    for uri in addressed {
        assert_eq!(uri, two, "only the open document is published for");
    }
}

#[test]
fn a_module_that_ships_is_there_even_without_a_workspace_folder() {
    // An editor that names no folder gets the single file behaviour, and this
    // is the one thing that behaviour still has: a module inside the compiler
    // is under no folder, so there is no folder to be missing. `deed check`
    // handed one file answers the same way.
    let text = "module a\n\nuse std/list.{map}\n\nfn f(xs: List<Int>) -> List<Int> {\n    map(xs, |n: Int| n + 1)\n}\n";
    let sent = session(&[request(1, "initialize"), did_open(URI, text)]);

    assert_eq!(published_for(&sent, URI).len(), 0, "{sent:?}");
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

// -- document highlight ------------------------------------------------------

#[test]
fn document_highlight_lights_up_every_mention_in_the_file() {
    // The `n` parameter: declared on line 2, used twice on line 3.
    // Including the declaration is what an editor expects here: a cursor
    // on a use should light up the declaration too, which is what
    // `prepareRename` already treats as one thing.
    let source = "module a\n\nfn f(n: Int) -> Int {\n    n + n\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        // The `n` in the parameter list.
        at(2, "textDocument/documentHighlight", URI, 2, 5),
    ]);

    let highlights = sent[2]
        .at(&["result"])
        .and_then(Json::as_array)
        .unwrap_or_else(|| panic!("{:?}", sent[2]));

    // Declared on line 2, used twice on line 3.
    let lines: Vec<i64> = highlights
        .iter()
        .filter_map(|h| h.at(&["range", "start", "line"]).and_then(Json::as_i64))
        .collect();
    assert_eq!(lines, vec![2, 3, 3], "{:?}", sent[2]);

    // Kind 1 (Text) for all: the resolver does not track reads from writes.
    let kinds: Vec<i64> = highlights
        .iter()
        .filter_map(|h| h.at(&["kind"]).and_then(Json::as_i64))
        .collect();
    assert!(kinds.iter().all(|&k| k == 1), "{:?}", sent[2]);
}

#[test]
fn document_highlight_on_nothing_is_an_empty_array() {
    // An editor handed null here logs a protocol failure on every keystroke.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, GOOD),
        // The blank line between the module declaration and the function.
        at(2, "textDocument/documentHighlight", URI, 1, 0),
    ]);

    assert_eq!(
        sent[2]
            .at(&["result"])
            .and_then(Json::as_array)
            .map(<[Json]>::len),
        Some(0),
        "{:?}",
        sent[2]
    );
}

#[test]
fn document_highlight_stays_in_this_file() {
    // `references` crosses files on purpose; this does not, because an
    // editor asks about the document it is painting.
    let scratch = Scratch::new("highlight-one-file");
    let one = scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", IMPORTER);

    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&one, EXPORTER),
        // The `double` in `fn double` on line 2.
        at(2, "textDocument/documentHighlight", &one, 2, 3),
    ]);

    let highlights = sent[2]
        .at(&["result"])
        .and_then(Json::as_array)
        .unwrap_or_else(|| panic!("{:?}", sent[2]));

    // `double` is declared once in `one.deed`. The call in `two.deed`
    // should not appear here.
    let lines: Vec<i64> = highlights
        .iter()
        .filter_map(|h| h.at(&["range", "start", "line"]).and_then(Json::as_i64))
        .collect();
    assert_eq!(lines, vec![2], "{:?}", sent[2]);

    // `two` is open in the workspace but should contribute nothing here.
    let _ = two;
}

// -- folding range -----------------------------------------------------------

fn folding_range(id: i64, uri: &str) -> String {
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"textDocument/foldingRange\",\"params\":\
         {{\"textDocument\":{{\"uri\":\"{uri}\"}}}}}}"
    ))
}

fn folds(message: &Json) -> &[Json] {
    message
        .at(&["result"])
        .and_then(Json::as_array)
        .unwrap_or_else(|| panic!("a folding range request should answer with a list: {message:?}"))
}

#[test]
fn declarations_with_bodies_produce_fold_ranges() {
    // `DECLARED` has one of everything, and each item with a body folds.
    // Line numbers are zero based.
    //
    //  0: module a
    //  1: (blank)
    //  2: type Positive = Int where value > 0        <- no body, no fold
    //  3: (blank)
    //  4: record Point {
    //  5:     x: Int,
    //  6:     y: Int,
    //  7: }
    //  8: (blank)
    //  9: choice Flag {
    // 10:     On,
    // 11:     Off,
    // 12: }
    // 13: (blank)
    // 14: effect Log {
    // 15:     fn note(message: String) -> ()
    // 16: }
    // 17: (blank)
    // 18: handler Quiet implements Log {
    // 19:     state seen: Int
    // 20: (blank)
    // 21:     fn note(message) -> () {}               <- single-line body, no fold
    // 22: }
    // 23: (blank)
    // 24: fn f(n: Int) -> Int {
    // 25:     n + n
    // 26: }
    // 27: (blank)
    // 28: test "it doubles" {
    // 29:     assert f(2) == 4
    // 30: }
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, DECLARED),
        folding_range(2, URI),
    ]);
    let ranges = folds(&sent[2]);

    let start_end: Vec<(i64, i64)> = ranges
        .iter()
        .filter(|r| r.at(&["kind"]).is_none())
        .filter_map(|r| {
            Some((
                r.at(&["startLine"]).and_then(Json::as_i64)?,
                r.at(&["endLine"]).and_then(Json::as_i64)?,
            ))
        })
        .collect();

    assert!(
        start_end.contains(&(4, 6)),
        "record Point should fold: {start_end:?}"
    );
    assert!(
        start_end.contains(&(9, 11)),
        "choice Flag should fold: {start_end:?}"
    );
    assert!(
        start_end.contains(&(14, 15)),
        "effect Log should fold: {start_end:?}"
    );
    assert!(
        start_end.contains(&(18, 21)),
        "handler Quiet should fold: {start_end:?}"
    );
    assert!(
        start_end.contains(&(24, 25)),
        "fn f should fold: {start_end:?}"
    );
    assert!(
        start_end.contains(&(28, 29)),
        "test should fold: {start_end:?}"
    );
}

#[test]
fn a_type_alias_has_no_body_and_produces_no_fold() {
    // `type Positive` has no braces, so there is nothing to collapse.
    let source = "module a\n\ntype Positive = Int where value > 0\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        folding_range(2, URI),
    ]);
    let ranges = folds(&sent[2]);
    let item_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.at(&["kind"]).is_none())
        .collect();
    assert!(
        item_ranges.is_empty(),
        "type alias should not produce a fold: {item_ranges:?}"
    );
}

#[test]
fn a_single_line_declaration_produces_no_fold() {
    // A body on one line has nothing to collapse.
    let source = "module a\n\nfn f(n: Int) -> Int { n + n }\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        folding_range(2, URI),
    ]);
    let ranges = folds(&sent[2]);
    let item_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.at(&["kind"]).is_none())
        .collect();
    assert!(
        item_ranges.is_empty(),
        "single-line body should produce no fold: {item_ranges:?}"
    );
}

#[test]
fn handler_operations_with_multi_line_bodies_fold_independently() {
    // The operation body is multi-line, so it folds inside the handler.
    let source = "module a\n\neffect Log {\n    fn note(message: String) -> ()\n}\n\nhandler Quiet implements Log {\n    fn note(message: String) -> () {\n        ()\n    }\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        folding_range(2, URI),
    ]);
    let ranges = folds(&sent[2]);

    let item_starts: Vec<i64> = ranges
        .iter()
        .filter(|r| r.at(&["kind"]).is_none())
        .filter_map(|r| r.at(&["startLine"]).and_then(Json::as_i64))
        .collect();

    // handler Quiet is on line 6, operation `fn note` body opens on line 7.
    assert!(
        item_starts.contains(&6),
        "handler should fold: {item_starts:?}"
    );
    assert!(
        item_starts.contains(&7),
        "handler operation body should fold: {item_starts:?}"
    );
}

#[test]
fn consecutive_line_comments_fold_as_a_comment_range() {
    // Two `//` lines in a row become one comment fold.
    let source = "// first\n// second\nmodule a\n\nfn f() -> Int {\n    1\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        folding_range(2, URI),
    ]);
    let ranges = folds(&sent[2]);

    let comment_ranges: Vec<(i64, i64)> = ranges
        .iter()
        .filter(|r| r.at(&["kind"]).and_then(Json::as_str) == Some("comment"))
        .filter_map(|r| {
            Some((
                r.at(&["startLine"]).and_then(Json::as_i64)?,
                r.at(&["endLine"]).and_then(Json::as_i64)?,
            ))
        })
        .collect();

    assert_eq!(
        comment_ranges,
        vec![(0, 1)],
        "two consecutive comment lines should fold as one range: {comment_ranges:?}"
    );
}

#[test]
fn a_blank_line_ends_a_comment_run() {
    // A gap between two single-line comments produces no range for either.
    let source = "// first\n\n// second\nmodule a\n\nfn f() -> Int {\n    1\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        folding_range(2, URI),
    ]);
    let ranges = folds(&sent[2]);

    let comment_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.at(&["kind"]).and_then(Json::as_str) == Some("comment"))
        .collect();

    assert!(
        comment_ranges.is_empty(),
        "isolated comment lines should not fold: {comment_ranges:?}"
    );
}

#[test]
fn a_single_comment_line_produces_no_fold() {
    // A run of one has nothing to collapse.
    let source = "// just one\nmodule a\n\nfn f() -> Int {\n    1\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        folding_range(2, URI),
    ]);
    let ranges = folds(&sent[2]);

    let comment_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.at(&["kind"]).and_then(Json::as_str) == Some("comment"))
        .collect();

    assert!(
        comment_ranges.is_empty(),
        "a single comment line should produce no fold: {comment_ranges:?}"
    );
}

/// The comment ranges in a file, as `(startLine, endLine)`.
fn comment_folds_in(source: &str) -> Vec<(i64, i64)> {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        folding_range(2, URI),
    ]);
    folds(&sent[2])
        .iter()
        .filter(|range| range.at(&["kind"]).and_then(Json::as_str) == Some("comment"))
        .filter_map(|range| {
            Some((
                range.at(&["startLine"]).and_then(Json::as_i64)?,
                range.at(&["endLine"]).and_then(Json::as_i64)?,
            ))
        })
        .collect()
}

/// A run that ends because the next comment is a block one still folds.
///
/// A `/* */` is a comment too, so a run of `//` lines that meets one has
/// ended rather than been interrupted, and what it collected up to there is
/// still two lines a reader may want out of the way.
#[test]
fn a_block_comment_ends_a_run_without_taking_it_with_it() {
    let source = "// first\n// second\n/* and a block */\nmodule a\n\nfn f() -> Int {\n    1\n}\n";
    assert_eq!(comment_folds_in(source), vec![(0, 1)]);
}

/// And a run of one that ends the same way is still a run of one.
#[test]
fn a_single_comment_line_before_a_block_comment_produces_no_fold() {
    let source = "// just one\n/* and a block */\nmodule a\n\nfn f() -> Int {\n    1\n}\n";
    assert_eq!(comment_folds_in(source), Vec::new());
}

/// A run that ends because the next comment line is not the next line.
///
/// The blank line closes the first run in the middle of the walk rather than
/// at the end of it, which is a different place in the code and was a
/// different answer for as long as nothing asked.
#[test]
fn a_run_that_ends_at_a_gap_folds_before_the_next_one_starts() {
    let source = "// first\n// second\n\n// apart\nmodule a\n\nfn f() -> Int {\n    1\n}\n";
    assert_eq!(comment_folds_in(source), vec![(0, 1)]);
}

#[test]
fn a_file_that_does_not_check_still_folds() {
    // Folding reads the parse tree, not the checker, so a bad type is no obstacle.
    let source = "module a\n\nfn f() -> Nope {\n    missing()\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        folding_range(2, URI),
    ]);
    let ranges = folds(&sent[2]);
    let item_ranges: Vec<_> = ranges
        .iter()
        .filter(|r| r.at(&["kind"]).is_none())
        .collect();
    assert_eq!(
        item_ranges.len(),
        1,
        "the function body should fold even when the file has errors: {ranges:?}"
    );
}

#[test]
fn a_folding_range_request_for_a_document_nobody_opened_is_null() {
    let sent = session(&[request(1, "initialize"), folding_range(2, URI)]);
    assert_eq!(sent[1].at(&["result"]), Some(&Json::Null));
}

// -- selection range ---------------------------------------------------------

fn selection_range(id: i64, uri: &str, positions: &[(u32, u32)]) -> String {
    let positions_json: Vec<String> = positions
        .iter()
        .map(|(line, character)| format!("{{\"line\":{line},\"character\":{character}}}"))
        .collect();
    let positions_str = positions_json.join(",");
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"textDocument/selectionRange\",\"params\":\
         {{\"textDocument\":{{\"uri\":\"{uri}\"}},\"positions\":[{positions_str}]}}}}"
    ))
}

fn selection_ranges(message: &Json) -> &[Json] {
    message
        .at(&["result"])
        .and_then(Json::as_array)
        .unwrap_or_else(|| {
            panic!("a selection range request should answer with a list: {message:?}")
        })
}

/// Collects the start lines of every range in the chain, innermost first.
fn chain_start_lines(entry: &Json) -> Vec<i64> {
    let mut lines = Vec::new();
    let mut current = entry;
    loop {
        let line = current
            .at(&["range", "start", "line"])
            .and_then(Json::as_i64);
        match line {
            Some(l) => lines.push(l),
            None => break,
        }
        match current.at(&["parent"]) {
            Some(parent) => current = parent,
            None => break,
        }
    }
    lines
}

/// Collects every (startLine, startChar, endLine, endChar) in the chain,
/// innermost first.  Used to verify that no two consecutive steps are equal.
fn chain_ranges(entry: &Json) -> Vec<(i64, i64, i64, i64)> {
    let mut out = Vec::new();
    let mut current = entry;
    loop {
        let r = (
            current
                .at(&["range", "start", "line"])
                .and_then(Json::as_i64),
            current
                .at(&["range", "start", "character"])
                .and_then(Json::as_i64),
            current.at(&["range", "end", "line"]).and_then(Json::as_i64),
            current
                .at(&["range", "end", "character"])
                .and_then(Json::as_i64),
        );
        match r {
            (Some(sl), Some(sc), Some(el), Some(ec)) => out.push((sl, sc, el, ec)),
            _ => break,
        }
        match current.at(&["parent"]) {
            Some(parent) => current = parent,
            None => break,
        }
    }
    out
}

#[test]
fn an_expression_inside_a_function_expands_to_the_function() {
    // `GOOD` is:
    //  0: module a
    //  1: (blank)
    //  2: fn f(n: Int) -> Int {
    //  3:     n + n
    //  4: }
    //
    // The cursor on `n` on line 3 should produce a chain with at least the
    // expression, the block, and the function, innermost first.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, GOOD),
        selection_range(2, URI, &[(3, 4)]),
    ]);
    let ranges = selection_ranges(&sent[2]);

    assert_eq!(
        ranges.len(),
        1,
        "one position should give one entry: {sent:?}"
    );

    let lines = chain_start_lines(&ranges[0]);
    // The chain must reach the function declaration on line 2.
    assert!(
        lines.contains(&2),
        "the function declaration (line 2) should be in the chain: {lines:?}"
    );
    // The innermost range must start at or after line 3, where the expression is.
    assert!(
        lines.first().copied() >= Some(3),
        "the innermost range should cover the expression on line 3: {lines:?}"
    );
    // The chain must have at least two steps: expression and function.
    assert!(
        lines.len() >= 2,
        "there should be at least expression and function as separate steps: {lines:?}"
    );
}

#[test]
fn each_press_widens_strictly() {
    // Two equal ranges are one step: the command appears to do nothing if an
    // expand-selection press produces a range identical to the previous one.
    // The chain must therefore contain no two consecutive entries whose ranges
    // are the same (same start and end line/character).
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, GOOD),
        // Cursor on the first `n` in `n + n` on line 3.
        selection_range(2, URI, &[(3, 4)]),
    ]);
    let result = selection_ranges(&sent[2]);
    let ranges = chain_ranges(&result[0]);

    let has_consecutive_duplicate = ranges.windows(2).any(|w| w[0] == w[1]);
    assert!(
        !has_consecutive_duplicate,
        "no two consecutive steps should produce the same range: {ranges:?}"
    );
}

#[test]
fn an_empty_positions_array_answers_with_an_empty_array() {
    // An empty positions array is a valid request and should not error.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, GOOD),
        selection_range(2, URI, &[]),
    ]);
    assert_eq!(
        sent[2]
            .at(&["result"])
            .and_then(Json::as_array)
            .map(<[Json]>::len),
        Some(0),
        "empty positions should answer with an empty array: {:?}",
        sent[2]
    );
}

#[test]
fn a_position_outside_any_item_is_null_in_the_result() {
    // Line 1 is the blank line between the module declaration and the function.
    // Nothing is declared there, so the entry for that position is null.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, GOOD),
        selection_range(2, URI, &[(1, 0)]),
    ]);
    let ranges = selection_ranges(&sent[2]);
    assert_eq!(ranges.len(), 1, "one position gives one entry");
    assert_eq!(
        ranges[0],
        Json::Null,
        "a position in no item should be null: {:?}",
        ranges[0]
    );
}

#[test]
fn multiple_positions_produce_one_entry_each() {
    // Two positions in the same request come back in the same order.
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, GOOD),
        // Line 3, char 4 is inside the function; line 1 is outside any item.
        selection_range(2, URI, &[(3, 4), (1, 0)]),
    ]);
    let ranges = selection_ranges(&sent[2]);
    assert_eq!(ranges.len(), 2, "two positions give two entries: {sent:?}");
    // First entry (inside function) should not be null.
    assert_ne!(
        ranges[0],
        Json::Null,
        "position inside function should have a range: {:?}",
        ranges[0]
    );
    // Second entry (blank line) should be null.
    assert_eq!(
        ranges[1],
        Json::Null,
        "position outside any item should be null: {:?}",
        ranges[1]
    );
}

#[test]
fn a_selection_range_request_for_a_document_nobody_opened_is_null() {
    let sent = session(&[request(1, "initialize"), selection_range(2, URI, &[(0, 0)])]);
    assert_eq!(sent[1].at(&["result"]), Some(&Json::Null));
}

#[test]
fn a_file_that_does_not_check_still_has_selection_ranges() {
    // Selection range reads the parse tree, not the checker.
    let source = "module a\n\nfn f() -> Nope {\n    missing()\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        // Line 3, character 4: inside `missing()`.
        selection_range(2, URI, &[(3, 4)]),
    ]);
    let ranges = selection_ranges(&sent[2]);
    assert_eq!(ranges.len(), 1);
    assert_ne!(
        ranges[0],
        Json::Null,
        "a file with errors should still produce a selection range: {:?}",
        ranges[0]
    );
}

#[test]
fn a_position_in_a_handler_operation_expands_through_the_handler() {
    // The handler declares `fn note`, whose body spans multiple lines.
    // A position inside the body should expand through the operation and then
    // through the handler. Without `collect_handler_spans` the chain never
    // names the operation — only the top-level handler item.
    let source = "module a\n\neffect Log {\n    fn note(message: String) -> ()\n}\n\n\
                  handler Quiet implements Log {\n    fn note(message) -> () {\n        ()\n    }\n}\n";
    //  6: handler Quiet implements Log {
    //  7:     fn note(message) -> () {
    //  8:         ()
    //  9:     }
    // 10: }
    assert_eq!(
        selection_chain(source, 8, 8),
        vec![
            (8, 8, 8, 10), // `()`
            (7, 27, 9, 5), // operation body block
            (7, 4, 9, 5),  // whole operation
            (6, 0, 10, 1), // handler
        ]
    );
}

#[test]
fn a_position_in_a_test_block_expands_through_the_test() {
    // Without `collect_test_spans` a cursor inside a `test` body still answers
    // (the item span is pushed either way), but the block never appears as its
    // own step — expand-selection jumps straight to the whole test.
    let source = "module a\n\ntest \"one\" {\n    1 + 1\n}\n";
    assert_eq!(
        selection_chain(source, 3, 4),
        vec![
            (3, 4, 3, 5),  // first `1`
            (3, 4, 3, 9),  // `1 + 1`
            (2, 11, 4, 1), // test body block
            (2, 0, 4, 1),  // test item
        ]
    );
}

/// One selection-range answer, as the exact chain the server should produce.
///
/// Pinning the whole chain (not just "function is somewhere in it") is what
/// holds the walk: delete a match arm or a contains-guard and a middle step
/// disappears or the wrong statement is chosen.
fn selection_chain(source: &str, line: u32, character: u32) -> Vec<(i64, i64, i64, i64)> {
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        selection_range(2, URI, &[(line, character)]),
    ]);
    let ranges = selection_ranges(&sent[2]);
    assert_eq!(ranges.len(), 1, "one position gives one entry: {sent:?}");
    assert_ne!(
        ranges[0],
        Json::Null,
        "expected a chain, got null: {sent:?}"
    );
    chain_ranges(&ranges[0])
}

#[test]
fn a_binary_expression_is_a_step_between_operand_and_block() {
    // GOOD line 3 is `    n + n`. Cursor on the first `n`.
    // Without the Binary arm the middle step collapses to the bare operand.
    assert_eq!(
        selection_chain(GOOD, 3, 4),
        vec![
            (3, 4, 3, 5),  // first `n`
            (3, 4, 3, 9),  // `n + n`
            (2, 20, 4, 1), // block
            (2, 0, 4, 1),  // fn
        ]
    );
    // Right-hand side: delete the `!` on the lhs walk and the rhs is never
    // visited when the lhs does not contain the offset.
    assert_eq!(
        selection_chain(GOOD, 3, 8),
        vec![
            (3, 8, 3, 9),  // second `n`
            (3, 4, 3, 9),  // `n + n`
            (2, 20, 4, 1), // block
            (2, 0, 4, 1),  // fn
        ]
    );
}

#[test]
fn a_unary_operand_expands_through_the_unary() {
    let source = "module a\n\nfn f(n: Int) -> Int {\n    -n\n}\n";
    assert_eq!(
        selection_chain(source, 3, 5),
        vec![
            (3, 5, 3, 6),  // `n`
            (3, 4, 3, 6),  // `-n`
            (2, 20, 4, 1), // block
            (2, 0, 4, 1),  // fn
        ]
    );
}

#[test]
fn a_call_argument_expands_through_the_call() {
    let source = "module a\n\nfn f(n: Int) -> Int {\n    f(n)\n}\n";
    assert_eq!(
        selection_chain(source, 3, 6),
        vec![
            (3, 6, 3, 7),  // arg `n`
            (3, 4, 3, 8),  // `f(n)`
            (2, 20, 4, 1), // block
            (2, 0, 4, 1),  // fn
        ]
    );
}

#[test]
fn a_field_receiver_expands_through_the_field() {
    let source = "module a\n\nrecord P { x: Int }\n\nfn f(p: P) -> Int {\n    p.x\n}\n";
    assert_eq!(
        selection_chain(source, 5, 4),
        vec![
            (5, 4, 5, 5),  // `p`
            (5, 4, 5, 7),  // `p.x`
            (4, 18, 6, 1), // block
            (4, 0, 6, 1),  // fn
        ]
    );
}

#[test]
fn a_list_element_expands_through_the_list() {
    let source = "module a\n\nfn f() -> List<Int> {\n    [1, 2]\n}\n";
    // Second element: without the List arm the chain never names the brackets.
    assert_eq!(
        selection_chain(source, 3, 8),
        vec![
            (3, 8, 3, 9),  // `2`
            (3, 4, 3, 10), // `[1, 2]`
            (2, 20, 4, 1), // block
            (2, 0, 4, 1),  // fn
        ]
    );
}

#[test]
fn a_block_expression_is_its_own_step() {
    let source = "module a\n\nfn f() -> Int {\n    { 1 }\n}\n";
    assert_eq!(
        selection_chain(source, 3, 6),
        vec![
            (3, 6, 3, 7),  // `1`
            (3, 4, 3, 9),  // `{ 1 }`
            (2, 14, 4, 1), // outer block
            (2, 0, 4, 1),  // fn
        ]
    );
}

#[test]
fn a_closure_body_expands_through_the_closure() {
    let source = "module a\n\nfn f() -> Fn() -> Int {\n    || 1\n}\n";
    assert_eq!(
        selection_chain(source, 3, 7),
        vec![
            (3, 7, 3, 8),  // `1`
            (3, 4, 3, 8),  // `|| 1`
            (2, 22, 4, 1), // block
            (2, 0, 4, 1),  // fn
        ]
    );
}

#[test]
fn a_for_body_expands_through_the_for() {
    let source =
        "module a\n\nfn f(xs: List<Int>) -> Int {\n    for x in xs with s = 0 { s + x }\n}\n";
    // Inside `s + x`. Without the For arm the walk stops at the outer block.
    assert_eq!(
        selection_chain(source, 3, 30),
        vec![
            (3, 29, 3, 34), // `s + x`
            (3, 27, 3, 36), // `{ s + x }`
            (3, 4, 3, 36),  // whole `for ...`
            (2, 27, 4, 1),  // fn body
            (2, 0, 4, 1),   // fn
        ]
    );
}

#[test]
fn a_with_body_expands_through_the_with() {
    let source = "module a\n\neffect L { fn n() -> () }\n\n\
                  handler H implements L {\n    fn n() -> () { () }\n}\n\n\
                  fn f() -> () {\n    with H {\n        ()\n    }\n}\n";
    assert_eq!(
        selection_chain(source, 10, 8),
        vec![
            (10, 8, 10, 10), // `()`
            (9, 11, 11, 5),  // with-body block
            (9, 4, 11, 5),   // `with H { ... }`
            (8, 13, 12, 1),  // fn body
            (8, 0, 12, 1),   // fn
        ]
    );
}

#[test]
fn an_if_then_branch_does_not_pick_the_else() {
    // The `&&` between condition/then/else is load-bearing: flip it to `||`
    // and a cursor in the then-branch still walks the else arm first.
    let source = "module a\n\nfn f(b: Bool) -> Int {\n    if b {\n        1\n    } else {\n        2\n    }\n}\n";
    assert_eq!(
        selection_chain(source, 4, 8),
        vec![
            (4, 8, 4, 9),  // `1`
            (3, 9, 5, 5),  // then block
            (3, 4, 7, 5),  // whole `if`
            (2, 21, 8, 1), // fn body
            (2, 0, 8, 1),  // fn
        ]
    );
    assert_eq!(
        selection_chain(source, 6, 8),
        vec![
            (6, 8, 6, 9),  // `2`
            (5, 11, 7, 5), // else block
            (3, 4, 7, 5),  // whole `if`
            (2, 21, 8, 1), // fn body
            (2, 0, 8, 1),  // fn
        ]
    );
}

#[test]
fn a_match_arm_body_expands_through_the_arm() {
    let source = "module a\n\nfn f(r: Result<Int, Int>) -> Int {\n    match r {\n        ok(n) => n\n        err(e) => e\n    }\n}\n";
    // Cursor on the `n` after `=>`. Without the Match arm there is no
    // arm-sized step between the body and the whole match.
    let chain = selection_chain(source, 4, 17);
    // Innermost is the arm body `n`.
    assert_eq!(chain[0], (4, 17, 4, 18), "arm body: {chain:?}");
    // Somewhere above that sits the whole match (starts on line 3).
    assert!(
        chain.iter().any(|r| r.0 == 3 && r.2 >= 5),
        "match expression should be in the chain: {chain:?}"
    );
    // And the arm itself is a step between body and match: same start line as
    // the body, end at or before the match's end, longer than the body alone.
    assert!(
        chain.iter().any(|r| r.0 == 4 && r != &chain[0] && r.2 == 4),
        "arm should be its own step: {chain:?}"
    );
}

#[test]
fn a_struct_field_value_expands_through_the_field() {
    let source = "module a\n\nrecord P { x: Int }\n\nfn f() -> P {\n    P { x: 1 }\n}\n";
    assert_eq!(
        selection_chain(source, 5, 11),
        vec![
            (5, 11, 5, 12), // `1`
            (5, 8, 5, 12),  // `x: 1`
            (5, 4, 5, 14),  // `P { x: 1 }`
            (4, 12, 6, 1),  // block
            (4, 0, 6, 1),   // fn
        ]
    );
}

#[test]
fn a_later_statement_does_not_claim_an_earlier_one() {
    // The contains-guard on statements is what stops a cursor on `a + 1` from
    // expanding as if it were inside `let a = n`. Delete the `!` and the walk
    // accepts the first statement and never reaches the second.
    let source = "module a\n\nfn f(n: Int) -> Int {\n    let a = n\n    a + 1\n}\n";
    assert_eq!(
        selection_chain(source, 4, 4),
        vec![
            (4, 4, 4, 5),  // `a`
            (4, 4, 4, 9),  // `a + 1`
            (2, 20, 5, 1), // block
            (2, 0, 5, 1),  // fn
        ]
    );
}

#[test]
fn a_let_initializer_expands_through_the_let() {
    let source = "module a\n\nfn f(n: Int) -> Int {\n    let a = n + 1\n    a\n}\n";
    assert_eq!(
        selection_chain(source, 3, 12),
        vec![
            (3, 12, 3, 13), // `n`
            (3, 12, 3, 17), // `n + 1`
            (3, 4, 3, 17),  // whole `let`
            (2, 20, 5, 1),  // block
            (2, 0, 5, 1),   // fn
        ]
    );
}

#[test]
fn an_old_expression_in_ensures_expands_through_old() {
    // `old` only appears in contracts. Without the Old arm the chain jumps
    // from the inner name straight to the ensures clause.
    let source = "module a\n\nfn f(n: Int) -> Int\nensures ok => result == old(n) {\n    n\n}\n";
    // `old(n)` sits on the ensures line. Cursor on the `n` inside it.
    let chain = selection_chain(source, 3, 28);
    assert!(
        chain.len() >= 3,
        "name, old, ensures/fn at least: {chain:?}"
    );
    // Innermost is `n` inside old(...).
    assert_eq!(chain[0], (3, 28, 3, 29), "inner name: {chain:?}");
    // Next step is wider (old wraps it).
    assert!(
        chain[1].1 <= chain[0].1 && chain[1].3 >= chain[0].3 && chain[1] != chain[0],
        "old should wrap the name: {chain:?}"
    );
}

// -- document link -----------------------------------------------------------

fn document_link(id: i64, uri: &str) -> String {
    framed(&format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"textDocument/documentLink\",\"params\":\
         {{\"textDocument\":{{\"uri\":\"{uri}\"}}}}}}"
    ))
}

fn links(message: &Json) -> &[Json] {
    message
        .at(&["result"])
        .and_then(Json::as_array)
        .unwrap_or_else(|| panic!("a document link request should answer with a list: {message:?}"))
}

#[test]
fn a_use_path_links_to_the_module_it_names() {
    // Go to definition answers about an imported name. This answers about the
    // path itself: click `scratch/one` and the file opens.
    let scratch = Scratch::new("document-link");
    let one = scratch.write("one.deed", EXPORTER);
    let two = scratch.write("two.deed", IMPORTER);

    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&one, EXPORTER),
        did_open(&two, IMPORTER),
        document_link(2, &two),
    ]);
    let answer = sent
        .iter()
        .find(|message| message.at(&["id"]).and_then(Json::as_i64) == Some(2))
        .unwrap_or_else(|| panic!("no answer for documentLink: {sent:?}"));
    let found = links(answer);
    assert_eq!(found.len(), 1, "one use path, one link: {answer:?}");
    assert_eq!(
        found[0].at(&["target"]).and_then(Json::as_str),
        Some(one.as_str()),
        "the path should open the module it names: {found:?}"
    );
    // The range is the path, not the whole `use` line: line 2 is
    // `use scratch/one.{double}`.
    assert_eq!(
        found[0]
            .at(&["range", "start", "line"])
            .and_then(Json::as_i64),
        Some(2),
        "{found:?}"
    );
}

#[test]
fn a_use_of_a_shipped_module_is_not_a_link() {
    // There is no file behind `std/list`. Inventing a target would hand the
    // editor a place it cannot open, which is worse than nothing.
    let source = "module a\n\nuse std/list.{map}\n\nfn f(xs: List<Int>) -> List<Int> {\n    map(xs, |n: Int| n)\n}\n";
    let sent = session(&[
        request(1, "initialize"),
        did_open(URI, source),
        document_link(2, URI),
    ]);
    assert_eq!(
        links(&sent[2]).len(),
        0,
        "a shipped module has no file to open: {sent:?}"
    );
}

#[test]
fn a_document_link_request_for_a_document_nobody_opened_is_null() {
    let sent = session(&[request(1, "initialize"), document_link(2, URI)]);
    assert_eq!(sent[1].at(&["result"]), Some(&Json::Null));
}

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
fn a_use_line_offers_what_the_module_that_ships_exports() {
    // The names in a module that lives inside the compiler are the ones least
    // likely to be remembered exactly, because there is no file to go and look
    // at. This used to come back empty: the list is read off the check, and
    // the check had never been told the module existed.
    let scratch = Scratch::new("completion-shipped");
    let typing = "module two\n\nuse std/list.{\n";
    let two = scratch.write("two.deed", typing);

    let sent = session(&[
        initialize_in(1, &scratch),
        did_open(&two, typing),
        // Inside the braces, with nothing written yet.
        completion(2, &two, 2, 14),
    ]);

    let found = labels(sent.last().unwrap());
    assert!(!found.is_empty(), "the module exports something");
    for name in ["map", "filter", "fold"] {
        assert!(found.contains(&name.to_string()), "{name} in {found:?}");
    }
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
