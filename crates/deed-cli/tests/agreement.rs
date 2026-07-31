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
fn a_file_that_imports_a_workspace_module_agrees() {
    // The dependency case: one module importing another that lives in the same
    // workspace. The command line tool follows the import chain from the root;
    // the editor loads the whole workspace folder. Both reach the same file
    // and both compile without any diagnostic.
    //
    // This is what `agreement.rs` covers for a program with a dependency. The
    // existing tests all import shipped or missing modules. This one imports a
    // plain workspace module, which is what most programs actually do.
    let scratch = Scratch::new("workspace-dep");
    scratch.write("lib.deed", "module lib\n\nfn value() -> Int { 42 }\n");
    let app = scratch.write(
        "app.deed",
        "module app\n\nuse lib.{value}\n\nfn answer() -> Int { value() }\n",
    );

    assert_eq!(
        both(&scratch.0, &app),
        Vec::<String>::new(),
        "app imports lib, which is in the workspace"
    );
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

// -- the tier ---------------------------------------------------------------
//
// The second thing both of them answer about one file. `design/02-syntax.md`
// promises that the tier an obligation landed in is always visible, and for a
// while that was true of a terminal and false of the place a reader is: the
// whole `deed-lsp` crate mentioned obligations once, in a comment. Now a hover
// names them, which makes this the same pairing as the codes above and the
// same risk. Two readings of one table are two chances to be wrong.

/// A file carrying one obligation of each tier.
///
/// `halve` guarantees something a property test can exercise, so its `ensures`
/// is `Tested`. The call satisfies `halve`'s `where` from the literal, so that
/// is `Proven`. Its result has to land in `Positive` and nothing here says it
/// does, so that is `Guarded`. The last two are the same span, which is worth
/// having: a position with two answers is where picking one would show.
const CONTRACTS: &str = "module one\n\n\
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
     }\n";

/// Every obligation `deed check --obligations` reports about one file.
///
/// Tier, one based line and column, subject and reason (when there is one),
/// in the order it prints them.
fn obligations_from_the_command_line(
    path: &Path,
) -> Vec<(String, u32, u32, String, Option<String>)> {
    let output = Command::new(DEED)
        .args(["check", "--format", "json", "--obligations"])
        .arg(path)
        .output()
        .expect("the deed binary should run");
    let text = String::from_utf8(output.stdout).expect("the tool writes UTF-8");
    let named = path.to_string_lossy().replace('\\', "/");

    let mut reported = Vec::new();
    for line in text.lines() {
        let Ok(message) = json::parse(line) else {
            panic!("the tool should write one JSON object a line, got {line}");
        };
        if message.at(&["kind"]).and_then(Json::as_str) != Some("obligation") {
            continue;
        }
        if message.at(&["file"]).and_then(Json::as_str) != Some(named.as_str()) {
            continue;
        }
        let field = |name: &str| {
            message
                .at(&[name])
                .and_then(Json::as_str)
                .unwrap_or_else(|| panic!("every obligation carries {name}"))
                .to_string()
        };
        let number = |name: &str| {
            message
                .at(&[name])
                .and_then(Json::as_i64)
                .unwrap_or_else(|| panic!("every obligation carries {name}")) as u32
        };
        // Present for every obligation, but only ever a string when there is
        // one to report: absent is not a shape this reads, null is.
        let reason = message
            .at(&["reason"])
            .and_then(Json::as_str)
            .map(str::to_string);
        reported.push((
            field("tier"),
            number("line"),
            number("column"),
            field("subject"),
            reason,
        ));
    }
    reported
}

/// A server that has been told about the folder and handed the file.
fn opened(folder: &Path, path: &Path) -> Server {
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

    let text = std::fs::read_to_string(path).expect("the file should be readable");
    server.handle(&Json::object(vec![
        ("jsonrpc", Json::string("2.0")),
        ("method", Json::string("textDocument/didOpen")),
        (
            "params",
            Json::object(vec![(
                "textDocument",
                Json::object(vec![
                    ("uri", Json::string(file_uri(path))),
                    ("languageId", Json::string("deed")),
                    ("version", Json::number(1)),
                    ("text", Json::string(&text)),
                ]),
            )]),
        ),
    ]));
    server
}

/// Every obligation a hover names at a zero based line and character.
///
/// Read back out of the markdown the way a person reads it, rather than from
/// somewhere only a test can see. A tooltip nobody can parse is a tooltip that
/// says nothing, and the shape is the one the type and the name lines already
/// use.
///
/// Every line of that shape is taken, and one whose second half is neither an
/// article nor a tier brings this down rather than being skipped. Skipping was
/// the first version of this and it let a hover invent an obligation and stay
/// green: the only lines being compared were the ones already spelled the way
/// the comparison expected, so the half of the claim about what an editor must
/// not say was answering about nothing.
fn tiers_from_the_editor(
    server: &mut Server,
    uri: &str,
    line: u32,
    character: u32,
) -> Vec<(String, String, Option<String>)> {
    let (sent, _) = server.handle(&Json::object(vec![
        ("jsonrpc", Json::string("2.0")),
        ("id", Json::number(2)),
        ("method", Json::string("textDocument/hover")),
        (
            "params",
            Json::object(vec![
                (
                    "textDocument",
                    Json::object(vec![("uri", Json::string(uri))]),
                ),
                (
                    "position",
                    Json::object(vec![
                        ("line", Json::number(line as i64)),
                        ("character", Json::number(character as i64)),
                    ]),
                ),
            ]),
        ),
    ]));

    let text = sent
        .last()
        .and_then(|message| message.at(&["result", "contents", "value"]))
        .and_then(Json::as_str)
        .unwrap_or_else(|| panic!("nothing was hovered at {line}:{character}"))
        .to_string();

    let mut named = Vec::new();
    for written in text.lines() {
        let Some((subject, rest)) = written.rsplit_once("`, ") else {
            continue;
        };
        // The line saying what a name refers to, which reads "`n`, a
        // parameter". Not an obligation, and the only other thing a hover
        // writes this way.
        if rest.starts_with("a ") || rest.starts_with("an ") {
            continue;
        }
        // A guarded obligation with a reason reads "guarded (nothing
        // narrowed this name)"; the tier is the part before the reason.
        let (tier, reason) = match rest.split_once(" (") {
            Some((tier, remainder)) => (
                tier,
                Some(
                    remainder
                        .strip_suffix(')')
                        .unwrap_or_else(|| {
                            panic!("a reason should close its parenthesis, got {written:?}")
                        })
                        .to_string(),
                ),
            ),
            None => (rest, None),
        };
        assert!(
            ["proven", "tested", "guarded"].contains(&tier),
            "a hover wrote {written:?}, and an obligation goes by proven, \
             tested or guarded and by nothing else"
        );
        named.push((
            tier.to_string(),
            subject.trim_start_matches('`').to_string(),
            reason,
        ));
    }
    named
}

#[test]
fn the_tier_of_every_obligation_is_the_same_in_both_places() {
    let scratch = Scratch::new("obligations");
    let one = scratch.write("one.deed", CONTRACTS);

    // The absolute value first, so that both of them being silent is not a
    // pass. This is the whole table for this file.
    let terminal = obligations_from_the_command_line(&one);
    assert_eq!(
        terminal,
        vec![
            (
                "tested".to_string(),
                9,
                5,
                "halve ensures ok".to_string(),
                None
            ),
            (
                "guarded".to_string(),
                15,
                5,
                "Positive".to_string(),
                Some("nothing narrowed this name".to_string())
            ),
            (
                "proven".to_string(),
                15,
                5,
                "halve requires".to_string(),
                None
            ),
        ],
        "`deed check --obligations` should report one obligation of each tier"
    );

    let uri = file_uri(&one);
    let mut server = opened(&scratch.0, &one);

    let mut editor: Vec<(String, String, Option<String>)> = Vec::new();
    let mut asked: Vec<(u32, u32)> = Vec::new();
    for (_, line, column, _, _) in &terminal {
        if asked.contains(&(*line, *column)) {
            continue;
        }
        asked.push((*line, *column));
        // One based and counting characters on both sides, so the editor's
        // zero based position is one less of each.
        editor.extend(tiers_from_the_editor(
            &mut server,
            &uri,
            line - 1,
            column - 1,
        ));
    }

    // Both directions. Everything the terminal reported is in a tooltip, and
    // nothing is in a tooltip that the terminal did not report: an editor that
    // invents a tier, or a reason, is the failure this pairing exists to
    // catch, and it would pass a check that only looked for what it was told
    // to find.
    let mut expected: Vec<(String, String, Option<String>)> = terminal
        .iter()
        .map(|(tier, _, _, subject, reason)| (tier.clone(), subject.clone(), reason.clone()))
        .collect();
    expected.sort();
    editor.sort();
    assert_eq!(
        editor,
        expected,
        "`deed check --obligations` and the server disagree about {}",
        one.display()
    );
}
