//! The command line and the editor, asked about the same file.
//!
//! Both of them have to work out which files a file is compiled with before
//! they can compile it, and for a while they worked it out differently. The
//! command line tool injected the modules that ship inside the compiler and
//! the language server did not, so `deed check examples/todo.deed` was silent
//! while an editor put `DEED3007 UNKNOWN_MODULE` under the same `use` line.
//! Neither of them was in a position to notice: each one was right about
//! itself.
//!
//! So this puts them side by side. The observable is the set of diagnostic
//! codes, and it answers the question because the codes distinguish the two
//! outcomes that matter here: `DEED3007` is a module nothing found, `DEED3008`
//! is a module that was found and does not export that name. Which modules got
//! resolved is exactly what tells those apart.
//!
//! Agreement on its own is not enough, because both of them agreeing on the
//! wrong answer is the state this repository was already in. Every case below
//! also pins what the answer is.

use std::path::{Path, PathBuf};
use std::process::Command;

use deed_lsp::{Json, Next, Server, json};

const DEED: &str = env!("CARGO_BIN_EXE_deed");

/// A scratch directory, named so parallel tests do not collide.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("deed-agreement-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, contents).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The URI an editor would send for a path.
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

/// What `deed check` says about one file, by code.
///
/// The real binary, because that is the thing a person runs. Only the file
/// that was named: the tool reports its context too, and the editor is not
/// being asked about anything it has not been shown.
fn from_the_command_line(path: &Path) -> Vec<String> {
    let output = Command::new(DEED)
        .args(["check", "--format", "json"])
        .arg(path)
        .output()
        .expect("the deed binary should run");
    let text = String::from_utf8(output.stdout).expect("the tool writes UTF-8");
    let named = path.to_string_lossy().replace('\\', "/");

    let mut codes = Vec::new();
    for line in text.lines() {
        let Ok(message) = json::parse(line) else {
            panic!("the tool should write one JSON object a line, got {line}");
        };
        if message.at(&["kind"]).and_then(Json::as_str) != Some("diagnostic") {
            continue;
        }
        if message.at(&["diagnostic", "file"]).and_then(Json::as_str) != Some(named.as_str()) {
            continue;
        }
        codes.push(
            message
                .at(&["diagnostic", "code"])
                .and_then(Json::as_str)
                .expect("every diagnostic carries a code")
                .to_string(),
        );
    }
    codes.sort();
    codes
}

/// What the server publishes for the same file, by code.
///
/// In process rather than over a pipe, because the framing is somebody else's
/// test. The folder is the one the file is in, which is what an editor sends
/// and is the nearest thing the server has to being handed one file.
fn from_the_editor(folder: &Path, path: &Path) -> Vec<String> {
    let uri = file_uri(path);
    let text = std::fs::read_to_string(path).expect("the file should be readable");
    let mut server = Server::new();

    let (_, next) = server.handle(&Json::object(vec![
        ("jsonrpc", Json::string("2.0")),
        ("id", Json::number(1)),
        ("method", Json::string("initialize")),
        (
            "params",
            Json::object(vec![(
                "workspaceFolders",
                Json::Array(vec![Json::object(vec![
                    ("uri", Json::string(file_uri(folder))),
                    ("name", Json::string("scratch")),
                ])]),
            )]),
        ),
    ]));
    assert_eq!(next, Next::Continue);

    let (sent, _) = server.handle(&Json::object(vec![
        ("jsonrpc", Json::string("2.0")),
        ("method", Json::string("textDocument/didOpen")),
        (
            "params",
            Json::object(vec![(
                "textDocument",
                Json::object(vec![
                    ("uri", Json::string(&uri)),
                    ("languageId", Json::string("deed")),
                    ("version", Json::number(1)),
                    ("text", Json::string(&text)),
                ]),
            )]),
        ),
    ]));

    let published = sent
        .iter()
        .find(|message| {
            message.at(&["method"]).and_then(Json::as_str)
                == Some("textDocument/publishDiagnostics")
                && message.at(&["params", "uri"]).and_then(Json::as_str) == Some(uri.as_str())
        })
        .unwrap_or_else(|| panic!("nothing was published for {uri}: {sent:?}"));

    let mut codes: Vec<String> = published
        .at(&["params", "diagnostics"])
        .and_then(Json::as_array)
        .expect("a list of diagnostics")
        .iter()
        .map(|diagnostic| {
            diagnostic
                .at(&["code"])
                .and_then(Json::as_str)
                .expect("every diagnostic carries a code")
                .to_string()
        })
        .collect();
    codes.sort();
    codes
}

/// Both answers about one file, which have to be the same one.
fn both(folder: &Path, path: &Path) -> Vec<String> {
    let terminal = from_the_command_line(path);
    let editor = from_the_editor(folder, path);
    assert_eq!(
        terminal,
        editor,
        "`deed check` and the server disagree about {}",
        path.display()
    );
    terminal
}

/// A file that reaches for the list library living inside the compiler.
const USES_SHIPPED: &str = "module two\n\nuse std/list.{map}\n\n\
     fn f(xs: List<Int>) -> List<Int> {\n    map(xs, |n: Int| n + 1)\n}\n";

#[test]
fn a_module_that_ships_inside_the_compiler_is_found_by_both_of_them() {
    let scratch = Scratch::new("shipped");
    let two = scratch.write("two.deed", USES_SHIPPED);

    assert_eq!(
        both(&scratch.0, &two),
        Vec::<String>::new(),
        "the module is in the binary, so nothing should be missing"
    );
}

#[test]
fn a_name_that_module_does_not_export_is_the_same_answer_on_both_sides() {
    // The case that pins the answer rather than only the agreement.
    // `DEED3008` says the module was found and this name is not in it, and
    // there is no way to reach it without having resolved the module. If the
    // shipped table stopped being consulted, both of them would say `DEED3007`
    // instead and still agree.
    let scratch = Scratch::new("shipped-name");
    let two = scratch.write(
        "two.deed",
        "module two\n\nuse std/list.{nonesuch}\n\nfn f() -> Int {\n    1\n}\n",
    );

    // The name that does not exist cannot be used either, so the unused
    // import comes with it. Both of them say both things.
    assert_eq!(both(&scratch.0, &two), ["DEED3003", "DEED3008"]);
}

#[test]
fn a_module_that_is_nowhere_is_the_same_answer_on_both_sides() {
    // The other end of it. Neither of them may invent a module, and a `use`
    // naming nothing has one report on both sides rather than two on one.
    let scratch = Scratch::new("nowhere");
    let two = scratch.write(
        "two.deed",
        "module two\n\nuse nope/there.{thing}\n\nfn f() -> Int {\n    1\n}\n",
    );

    assert_eq!(both(&scratch.0, &two), ["DEED3003", "DEED3007"]);
}

#[test]
fn a_workspaces_own_module_wins_on_both_sides() {
    // The precedence half. Both of them offer everything a person can read
    // first, so the `std/list.deed` sitting in their own folder is the one
    // that answers, and the one in the binary is not reached past it.
    let scratch = Scratch::new("own");
    scratch.write(
        "std/list.deed",
        "module std/list\n\nfn mine(n: Int) -> Int {\n    n\n}\n",
    );
    let two = scratch.write(
        "two.deed",
        "module two\n\nuse std/list.{mine}\n\nfn f() -> Int {\n    mine(1)\n}\n",
    );

    assert_eq!(
        both(&scratch.0, &two),
        Vec::<String>::new(),
        "their own module declares `mine`"
    );
}

#[test]
fn what_the_shipped_module_exports_is_out_of_reach_on_both_sides() {
    // The same rule, seen from the side that fails if the table were asked
    // first: `map` is in the module inside the compiler and not in theirs.
    let scratch = Scratch::new("own-shadowed");
    scratch.write(
        "std/list.deed",
        "module std/list\n\nfn mine(n: Int) -> Int {\n    n\n}\n",
    );
    let two = scratch.write(
        "two.deed",
        "module two\n\nuse std/list.{map}\n\nfn f() -> Int {\n    1\n}\n",
    );

    assert_eq!(both(&scratch.0, &two), ["DEED3003", "DEED3008"]);
}

#[test]
fn the_examples_this_was_reported_against_agree() {
    // Real files, and the ones the finding named. A test over made up input
    // says nothing about the corpus this repository ships.
    //
    // The two of them get there differently here, which is the point rather
    // than a flaw. `deed check` is handed the file, works the root back out of
    // its own `module` line and finds `std/list.deed` sitting in this
    // repository; the editor is pointed at `examples/`, where there is no such
    // file, and reaches the one inside the binary. Same module, same answer.
    let examples = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("this crate lives two directories under the repository root")
        .join("examples");

    let named = ["todo.deed", "using_list.deed", "logs.deed"];
    let mut checked = 0;
    for name in named {
        let path = examples.join(name);
        assert_eq!(
            both(&examples, &path),
            Vec::<String>::new(),
            "{name} checks from the command line, so it checks in an editor"
        );
        checked += 1;
    }
    assert_eq!(checked, named.len(), "every named example should be asked");
}
