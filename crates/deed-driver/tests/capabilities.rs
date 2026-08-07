//! Capabilities.
//!
//! Most of these are about what a program cannot do, because that is the whole
//! claim. A capability system that only demonstrates the things that work has
//! demonstrated nothing.
//!
//! The claim is narrowing rather than scarcity. Two operations hand a
//! capability back, and what makes the mechanism hold is that both are rooted
//! inside the one they were given and nothing widens. For a long time the
//! comment on the function implementing all of this said no operation hands
//! one back at all, which is the flat version and the false one, so the set of
//! two is counted here now.

use std::path::{Path, PathBuf};

use deed_diagnostics::{Diagnostic, SourceMap, render_human};
use deed_driver::{Checked, check_text};
use deed_interp::{Program, Run, run_main};
use deed_rt::Reach;
use deed_typeck::{Ty, io_signatures, is_capability};

/// A directory nothing was granted, for programs that do not touch files.
fn nowhere() -> &'static Path {
    Path::new("")
}

/// The one checked module, as something the interpreter can run.
fn program_of(checked: &Checked) -> Program<'_> {
    let mut program = Program::new();
    program.add(
        checked.file,
        &checked.module,
        &checked.resolutions,
        checked.guards(),
        checked.rows(),
        checked.operators(),
    );
    program
}

fn check(src: &str) -> (SourceMap, Checked) {
    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "test.deed", src);
    (sources, checked)
}

fn check_ok(src: &str) -> (SourceMap, Checked) {
    let (sources, checked) = check(src);
    assert!(
        checked.diagnostics.is_empty(),
        "expected a clean check:\n{}",
        rendered(&sources, &checked.diagnostics)
    );
    (sources, checked)
}

fn run(src: &str) -> (SourceMap, Run) {
    run_in(src, nowhere())
}

fn run_in(src: &str, root: &Path) -> (SourceMap, Run) {
    run_with(src, root, &[])
}

fn run_with(src: &str, root: &Path, arguments: &[String]) -> (SourceMap, Run) {
    let (sources, checked) = check_ok(src);
    let run = run_main(&program_of(&checked), checked.file, root, arguments)
        .expect("there should be a main");
    (sources, run)
}

/// Runs a program that was granted the network, and nothing else.
fn run_reaching(src: &str, hosts: &[&str]) -> (SourceMap, Run) {
    let (sources, checked) = check_ok(src);
    let run = deed_interp::run_main_reaching(
        &program_of(&checked),
        checked.file,
        nowhere(),
        &[],
        &Reach::granting(hosts.iter().copied()),
    )
    .expect("there should be a main");
    (sources, run)
}

/// Runs a program that was handed lines on standard input.
fn run_reading(src: &str, input: &[&str]) -> (SourceMap, Run) {
    let (sources, checked) = check_ok(src);
    let lines: Vec<String> = input.iter().map(|line| (*line).to_string()).collect();
    let run = deed_interp::run_main_given(
        &program_of(&checked),
        checked.file,
        nowhere(),
        &deed_interp::Given {
            input: &lines,
            ..deed_interp::Given::default()
        },
        false,
    )
    .expect("there should be a main");
    (sources, run)
}

fn codes_of(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.code).collect()
}

fn rendered(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| render_human(sources, d))
        .collect::<Vec<_>>()
        .join("\n")
}

// -- what a program cannot do ----------------------------------------------

/// Whether a capability appears anywhere in `ty`.
///
/// Through a `Result`, because that is how both of them come back: the
/// operation can fail, and the capability is in the success arm.
fn hands_back_a_capability(ty: &Ty) -> bool {
    if is_capability(ty) {
        return true;
    }
    match ty {
        Ty::Result(ok, err) => hands_back_a_capability(ok) || hands_back_a_capability(err),
        Ty::List(element) => hands_back_a_capability(element),
        _ => false,
    }
}

/// The two halves of the sentence that describes the whole mechanism.
///
/// A capability is reached only by being handed one, which is the first half
/// and is why every operation takes one first. The second half is not that a
/// capability cannot be produced, because three operations produce one; it is
/// that what comes back reaches strictly less than what went in.
///
/// All three have a test below this one. A fourth would need the same argument
/// made for it, and the way that goes wrong is quietly: `Io.make` joined the
/// set in #152 and the prose about the set was not revisited, which is the
/// second time that happened. So the set is counted rather than described.
#[test]
fn the_operations_that_hand_a_capability_back_are_the_three_with_an_escape_test() {
    let signatures = io_signatures();

    for (name, params, _) in &signatures {
        let first = params
            .first()
            .unwrap_or_else(|| panic!("`Io.{name}` takes no arguments at all"));
        assert!(
            is_capability(first),
            "`Io.{name}` does not take a capability as its first argument"
        );
    }

    let mut producing: Vec<&str> = signatures
        .iter()
        .filter(|(_, _, ret)| hands_back_a_capability(ret))
        .map(|(name, _, _)| *name)
        .collect();
    producing.sort();

    assert_eq!(
        producing,
        vec!["make", "open", "reach"],
        "the operations handing a capability back have changed, so the narrowing argument \
         in design/04-capabilities.md needs making for the new one, with a test beside \
         `opening_narrows_and_there_is_no_way_back`"
    );
}

#[test]
fn a_function_with_no_console_has_no_way_to_name_one() {
    // Not forbidden by a rule. There is simply nothing to pass.
    let (sources, checked) = check(
        "module a\n\nfn sneaky() -> ()\n  uses Io.write,\n{\n  Io.write(console, \"hello\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_resolve::codes::UNKNOWN_NAME),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_capability_cannot_be_built_out_of_nothing() {
    let (_, checked) = check(
        "module a\n\nfn sneaky() -> ()\n  uses Io.write,\n{\n  Io.write(Console, \"hello\")\n}\n",
    );
    assert!(checked.has_errors());
}

#[test]
fn holding_the_wrong_capability_is_a_type_error() {
    let (sources, checked) = check(
        "module a\n\nfn sneaky(time: Clock) -> ()\n  uses Io.write,\n{\n  Io.write(time, \"hello\")\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::TYPE_MISMATCH),
        "{text}"
    );
    assert!(text.contains("expected `Console`, found `Clock`"), "{text}");
}

#[test]
fn writing_without_declaring_it_is_an_effect_error() {
    // Holding the capability is not enough. The row has to admit to it too.
    let (sources, checked) =
        check("module a\n\nfn quiet(out: Console) -> () {\n  Io.write(out, \"hello\")\n}\n");
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn system_carries_only_what_it_carries() {
    let (sources, checked) = check(
        "module a\n\nfn main(sys: System) -> Int\n  uses Io.write,\n{\n  Io.write(sys.network, \"hello\")\n  0\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::NO_SUCH_FIELD),
        "{text}"
    );
    assert!(text.contains("console"), "{text}");
    assert!(text.contains("files"), "{text}");
}

#[test]
fn declaring_a_capability_type_of_your_own_is_noticed() {
    // Otherwise a program could define its own `Console` and conjure one, and
    // none of the rest of this would mean anything.
    let (sources, checked) = check(
        "module a\n\nrecord Console { pretend: Int }\n\nfn f(c: Console) -> Int { c.pretend }\n",
    );
    assert!(
        !checked.diagnostics.is_empty(),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- installing a handler is a decision ------------------------------------
//
// A `with` block answers for the effect the handler implements. It does not
// answer for what the handler does to implement it. Those effects were charged
// to nobody, so a function holding a `Console` could install a handler that
// writes to it and still declare an empty row, which is the one claim an empty
// row is not allowed to make.
//
// Found by `rows_at_runtime.rs` on the day that test was written, from a
// program that checked clean and then wrote to the screen.

/// A `Log` effect and a handler that implements it by writing to a console.
const LOUD: &str = "effect Log {\n\
     \x20 fn note(message: String) -> ()\n\
     }\n\n\
     handler Loud implements Log {\n\
     \x20 state out: Console\n\n\
     \x20 fn note(message) -> ()\n\
     \x20   uses Io.write,\n\
     \x20 { Io.write(out, message) }\n\
     }\n\n\
     fn talks(n: Int) -> Int uses Log.note {\n\
     \x20 Log.note(\"hi\")\n\
     \x20 n\n\
     }\n\n";

#[test]
fn installing_a_handler_charges_what_the_handler_performs() {
    let (sources, checked) = check(&format!(
        "module a\n\n{LOUD}\
         fn looks_pure(n: Int, screen: Console) -> Int {{\n\
         \x20 with Loud {{ out: screen }} {{ talks(n) }}\n\
         }}\n"
    ));
    let text = rendered(&sources, &checked.diagnostics);
    assert!(checked.has_errors(), "this should not have been accepted");
    assert!(text.contains("DEED5001"), "{text}");
    assert!(text.contains("`Io.write`"), "{text}");
    assert!(text.contains("looks_pure"), "{text}");
}

#[test]
fn declaring_it_is_all_it_takes() {
    // The effect is still discharged. `Log.note` is what the handler is for,
    // so nothing has to declare that; `Io.write` is what it costs.
    check_ok(&format!(
        "module a\n\n{LOUD}\
         fn says_so(n: Int, screen: Console) -> Int\n\
         \x20 uses Io.write,\n\
         {{\n\
         \x20 with Loud {{ out: screen }} {{ talks(n) }}\n\
         }}\n"
    ));
}

#[test]
fn a_handler_that_performs_the_effect_it_implements_is_answered_by_itself() {
    // The handler's own row goes in with the body's row rather than straight
    // onto the function, so the `with` discharges it the same way. Otherwise a
    // handler that called its own effect would be uninstallable.
    check_ok(
        "module a\n\n\
         effect Log {\n\
         \x20 fn note(message: String) -> ()\n\
         \x20 fn warn(message: String) -> ()\n\
         }\n\n\
         handler Chatty implements Log {\n\
         \x20 fn note(message) -> () uses Log.warn, { Log.warn(message) }\n\
         \x20 fn warn(message) -> () { }\n\
         }\n\n\
         fn talks(n: Int) -> Int uses Log.note {\n\
         \x20 Log.note(\"hi\")\n\
         \x20 n\n\
         }\n\n\
         fn quiet(n: Int) -> Int {\n\
         \x20 with Chatty { talks(n) }\n\
         }\n",
    );
}

#[test]
fn a_handler_costs_the_same_from_another_module() {
    // The effect system used to stop at the module boundary, which is where
    // most calls are. An imported handler carries what it performs in its
    // export, the same way an imported function carries its row.
    let mut sources = SourceMap::new();
    let ids = [
        sources.add("loud.deed", format!("module loud\n\n{LOUD}").as_str()),
        sources.add(
            "app.deed",
            "module app\n\n\
             use loud.{Log, Loud, talks}\n\n\
             fn looks_pure(n: Int, screen: Console) -> Int {\n\
             \x20 with Loud { out: screen } { talks(n) }\n\
             }\n",
        ),
    ];
    let checked = deed_driver::check_all(&sources, &ids);
    let said: Vec<Diagnostic> = checked
        .iter()
        .flat_map(|one| one.diagnostics.iter().cloned())
        .collect();
    let text = rendered(&sources, &said);
    assert!(said.iter().any(Diagnostic::is_error), "{text}");
    assert!(text.contains("DEED5001"), "{text}");
    assert!(text.contains("`Io.write`"), "{text}");
}

// -- what it can do --------------------------------------------------------

/// A capability in an exported signature keeps being a capability on the other
/// side of the boundary.
///
/// The surface is lowered by a second pass, and a type it does not recognise
/// becomes `Unknown`, which agrees with everything. So a capability the
/// exported signature failed to carry would not be a type error at the call
/// site, it would be a parameter nothing checks, and `takes(5)` would compile.
/// That is the shape this repository has been caught by before, which is why
/// the test is written from the wrong argument rather than the right one.
#[test]
fn a_capability_crossing_a_module_boundary_is_still_a_capability() {
    for (capability, wrong) in [
        ("Console", "5"),
        ("Clock", "\"x\""),
        ("Dir", "5"),
        ("Net", "5"),
        ("System", "5"),
    ] {
        let mut sources = SourceMap::new();
        let ids = [
            sources.add(
                "lib.deed",
                format!("module lib\n\nfn takes(it: {capability}) -> Int {{ 1 }}\n").as_str(),
            ),
            sources.add(
                "app.deed",
                format!(
                    "module app\n\nuse lib.{{takes}}\n\nfn call() -> Int {{ takes({wrong}) }}\n"
                )
                .as_str(),
            ),
        ];
        let checked = deed_driver::check_all(&sources, &ids);
        let said: Vec<Diagnostic> = checked
            .iter()
            .flat_map(|one| one.diagnostics.iter().cloned())
            .collect();
        assert!(
            said.iter().any(Diagnostic::is_error),
            "`takes({wrong})` should not be accepted where a `{capability}` was wanted:\n{}",
            rendered(&sources, &said)
        );
    }
}

#[test]
fn a_function_handed_a_console_can_write_to_it() {
    check_ok(
        "module a\n\nfn greet(out: Console) -> ()\n  uses Io.write,\n{\n  Io.write(out, \"hello\")\n}\n",
    );
}

#[test]
fn main_receives_the_one_system_there_is() {
    let (_, run) = run(
        "module a\n\nfn main(sys: System) -> Int\n  uses Io.write,\n{\n  Io.write(sys.console, \"hello\")\n  0\n}\n",
    );
    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["hello".to_string()]);
}

#[test]
fn delegation_only_ever_narrows() {
    let (_, run) = run(
        "module a\n\nfn greet(out: Console) -> ()\n  uses Io.write,\n{\n  Io.write(out, \"narrowed\")\n}\n\nfn main(sys: System) -> Int\n  uses Io.write,\n{\n  greet(sys.console)\n  0\n}\n",
    );
    assert_eq!(run.output, vec!["narrowed".to_string()]);
}

#[test]
fn the_clock_is_deterministic() {
    // Wall clock time would make every run different, and P8 says the default
    // is deterministic.
    let source = "module a\n\nfn main(sys: System) -> Int\n  uses Io.now,\n{\n  Io.now(sys.clock) + Io.now(sys.clock)\n}\n";
    let (_, first) = run(source);
    let (_, second) = run(source);
    let one = first.result.expect("should run").to_string();
    let two = second.result.expect("should run").to_string();
    assert_eq!(one, two);
}

/// The default is the right one and it is not the only one anybody needs. A
/// program that wants the actual time says so, and what it says is a row entry
/// rather than a second kind of clock, which is the same answer `save` and
/// `read` get about the same `Dir`.
#[test]
fn the_real_clock_is_a_different_entry_in_the_row() {
    let (sources, checked) = check(
        "module a\n\nfn ticking(clock: Clock) -> Int\n  uses Io.now,\n{\n  Io.epoch(clock)\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("performs `Io.epoch` without declaring it"),
        "{text}"
    );

    // And the other way round, since a split that only holds in one direction
    // is not a split.
    let (sources, checked) = check(
        "module a\n\nfn stamped(clock: Clock) -> Int\n  uses Io.epoch,\n{\n  Io.now(clock)\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("performs `Io.now` without declaring it"),
        "{text}"
    );
}

#[test]
fn the_real_clock_reads_the_machine() {
    // Any machine whose clock is set after 2020. Asserting on the number
    // itself would be asserting on when the test was run, which is the whole
    // reason this is a separate entry from `now`.
    let (_, run) = run(
        "module a\n\nfn main(sys: System) -> Int\n  uses Io.epoch,\n{\n  Io.epoch(sys.clock)\n}\n",
    );
    let stamped = run.result.expect("should run").to_string();
    let millis: i64 = stamped.parse().expect("milliseconds");
    assert!(millis > 1_577_836_800_000, "{millis}");
}

#[test]
fn contracts_still_apply_to_main() {
    let (sources, run) = run(
        "module a\n\nfn main(sys: System) -> Int\n  ensures\n    ok => result > 100,\n{\n  1\n}\n",
    );
    let failure = run
        .result
        .expect_err("the postcondition should have caught this");
    assert_eq!(failure.code, deed_interp::codes::POSTCONDITION_FAILED);
    let _ = render_human(&sources, &failure);
}

// -- directories -----------------------------------------------------------

/// A `main` whose whole job is to report what one operation did.
///
/// The row is spelled out per case rather than shared, because the effects
/// pass rejects declaring an operation the body never performs, which is the
/// rule doing its job.
fn report(uses: &[&str], body: &str) -> String {
    let row: String = uses
        .iter()
        .map(|name| format!("    Io.{name},\n"))
        .collect();
    format!("module a\n\nfn main(sys: System) -> Int\n  uses\n{row}{{\n{body}\n  0\n}}\n")
}

#[test]
fn a_dir_reads_a_file_inside_it() {
    let scratch = Scratch::new("read");
    scratch.write("note.txt", "the contents");

    let (_, run) = run_in(
        &report(
            &["write", "read"],
            "  match Io.read(sys.files, \"note.txt\") {\n    ok(text) => Io.write(sys.console, text),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );
    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["the contents".to_string()]);
}

#[test]
fn every_way_out_of_a_dir_is_refused() {
    let scratch = Scratch::new("escape");
    scratch.write("inside.txt", "fine");

    // Each of these is a real thing someone would try, and none of them are
    // rejected by a list of known bad strings. Written the way a Deed program
    // would write them, so a backslash is escaped.
    for attempt in [
        "..",
        ".",
        "../Cargo.toml",
        "/etc/passwd",
        "C:\\\\Windows\\\\System32",
        "a/b",
        "a\\\\b",
        "",
    ] {
        let (_, run) = run_in(
            &report(
                &["write", "read"],
                &format!(
                    "  match Io.read(sys.files, \"{attempt}\") {{\n    ok(text) => Io.write(sys.console, \"READ IT\"),\n    err(why) => Io.write(sys.console, why),\n  }}"
                ),
            ),
            scratch.path(),
        );
        assert!(run.result.is_ok());
        assert_ne!(
            run.output,
            vec!["READ IT".to_string()],
            "`{attempt}` got out of the directory"
        );
    }
}

#[test]
fn opening_narrows_and_there_is_no_way_back() {
    let scratch = Scratch::new("narrow");
    scratch.write("outer.txt", "the wider directory");
    std::fs::create_dir_all(scratch.path().join("inner")).unwrap();
    scratch.write("inner/inner.txt", "the narrower one");

    // The `Dir` handed to the inner branch reaches `inner` and nothing above
    // it, and there is no operation that would widen it again.
    let (_, run) = run_in(
        &report(
            &["write", "read", "open"],
            "  match Io.open(sys.files, \"inner\") {\n    ok(narrower) => match Io.read(narrower, \"inner.txt\") {\n      ok(text) => Io.write(sys.console, text),\n      err(why) => Io.write(sys.console, why),\n    },\n    err(why) => Io.write(sys.console, why),\n  }\n  match Io.open(sys.files, \"inner\") {\n    ok(narrower) => match Io.read(narrower, \"outer.txt\") {\n      ok(text) => Io.write(sys.console, \"REACHED THE PARENT\"),\n      err(why) => Io.write(sys.console, why),\n    },\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output[0], "the narrower one");
    assert!(
        !run.output[1].contains("REACHED THE PARENT"),
        "a narrowed `Dir` read its parent: {:?}",
        run.output
    );
}

// -- the network -----------------------------------------------------------

/// A server that answers a fixed number of requests and then stops.
///
/// Bound to `127.0.0.1` on a port the operating system picks, so these tests
/// reach a machine that is already running them and nothing else. A test that
/// went out to a real host would be a test of that host, of the network
/// between, and of whoever is paying for it, and it would fail on an aeroplane.
struct Answering {
    port: u16,
    seen: std::sync::mpsc::Receiver<String>,
}

impl Answering {
    /// Serves `answers` in order, one per request, then closes.
    fn with(answers: &[&str]) -> Answering {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port should be free");
        let port = listener.local_addr().expect("a bound address").port();
        let answers: Vec<String> = answers.iter().map(|answer| (*answer).to_string()).collect();
        let (sender, seen) = std::sync::mpsc::channel();

        // Not joined when this is dropped. A server that was handed more
        // answers than it was asked questions sits in `accept`, and a `Drop`
        // that waits for it would hang the suite rather than fail it. The
        // thread serves a bounded number of requests and the process ends when
        // the test binary does.
        std::thread::spawn(move || {
            for answer in answers {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                // Read only the request head. Reading to the end would wait
                // for a close the client is not going to make until it has
                // been answered.
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while let Ok(1) = std::io::Read::read(&mut stream, &mut byte) {
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&request).to_string();
                // A body, when the request said it has one, so a test can
                // check that `send` actually sent what it was given.
                let length = head
                    .split("\r\n")
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                let mut body = vec![0_u8; length];
                if length > 0 {
                    let _ = std::io::Read::read_exact(&mut stream, &mut body);
                }
                let _ = sender.send(format!("{head}{}", String::from_utf8_lossy(&body)));

                let _ = std::io::Write::write_all(&mut stream, answer.as_bytes());
                let _ = std::io::Write::flush(&mut stream);
            }
        });

        Answering { port, seen }
    }

    fn ok(body: &str) -> Answering {
        Answering::with(&[&format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )])
    }

    fn host(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// What the server was actually sent, so a test can check the request
    /// rather than only the answer.
    fn request(&self) -> String {
        self.seen
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the server should have seen a request")
    }
}

impl Drop for Answering {
    fn drop(&mut self) {}
}

#[test]
fn a_net_fetches_from_a_host_it_was_granted() {
    let server = Answering::ok("the body");
    let (_, run) = run_reaching(
        &report(
            &["write", "fetch"],
            &format!(
                "  match Io.fetch(sys.net, \"{}\") {{\n    ok(text) => Io.write(sys.console, text),\n    err(why) => Io.write(sys.console, why),\n  }}",
                server.url("/thing")
            ),
        ),
        &[&server.host()],
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["the body".to_string()]);
    let request = server.request();
    assert!(request.starts_with("GET /thing HTTP/1.1"), "{request}");
}

#[test]
fn sending_carries_the_body_it_was_given() {
    let server = Answering::ok("thanks");
    let (_, run) = run_reaching(
        &report(
            &["write", "send"],
            &format!(
                "  match Io.send(sys.net, \"{}\", \"the payload\") {{\n    ok(text) => Io.write(sys.console, text),\n    err(why) => Io.write(sys.console, why),\n  }}",
                server.url("/inbox")
            ),
        ),
        &[&server.host()],
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["thanks".to_string()]);
    let request = server.request();
    assert!(request.starts_with("POST /inbox HTTP/1.1"), "{request}");
    assert!(request.ends_with("the payload"), "{request}");
}

/// The default, and the one that matters. A run that named no host reaches
/// nothing, so a program cannot phone home just by being run.
#[test]
fn a_program_granted_no_hosts_reaches_nothing() {
    let (_, run) = run_reaching(
        &report(
            &["write", "fetch"],
            "  match Io.fetch(sys.net, \"http://example.com/x\") {\n    ok(text) => Io.write(sys.console, \"REACHED IT\"),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        &[],
    );

    assert!(run.result.is_ok());
    assert!(
        run.output[0].contains("is not one of the hosts this `Net` reaches"),
        "{:?}",
        run.output
    );
}

#[test]
fn a_net_cannot_reach_a_host_it_was_not_granted() {
    let (_, run) = run_reaching(
        &report(
            &["write", "fetch"],
            "  match Io.fetch(sys.net, \"http://elsewhere.example/x\") {\n    ok(text) => Io.write(sys.console, \"REACHED IT\"),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        &["granted.example"],
    );

    assert!(run.result.is_ok());
    assert!(
        !run.output[0].contains("REACHED IT"),
        "a `Net` reached a host nobody granted: {:?}",
        run.output
    );
    assert!(
        run.output[0].contains("elsewhere.example"),
        "{:?}",
        run.output
    );
}

/// The `opening_narrows_and_there_is_no_way_back` argument, made for the third
/// operation that hands a capability back. What comes out of `Io.reach`
/// reaches one host, and the one it did not name is gone for good.
#[test]
fn reaching_narrows_and_there_is_no_way_back() {
    let server = Answering::with(&[
        "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\ninside",
    ]);
    let host = server.host();
    let (_, run) = run_reaching(
        &report(
            &["write", "fetch", "reach"],
            &format!(
                "  match Io.reach(sys.net, \"{host}\") {{\n    \
                   ok(narrower) => match Io.fetch(narrower, \"{}\") {{\n      \
                     ok(text) => Io.write(sys.console, text),\n      \
                     err(why) => Io.write(sys.console, why),\n    \
                   }},\n    \
                   err(why) => Io.write(sys.console, why),\n  \
                 }}\n  \
                 match Io.reach(sys.net, \"{host}\") {{\n    \
                   ok(narrower) => match Io.fetch(narrower, \"http://other.example/x\") {{\n      \
                     ok(text) => Io.write(sys.console, \"REACHED THE OTHER HOST\"),\n      \
                     err(why) => Io.write(sys.console, why),\n    \
                   }},\n    \
                   err(why) => Io.write(sys.console, why),\n  \
                 }}",
                server.url("/x")
            ),
        ),
        &[&host, "other.example"],
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output[0], "inside");
    assert!(
        !run.output[1].contains("REACHED THE OTHER HOST"),
        "a narrowed `Net` reached a host it had given up: {:?}",
        run.output
    );
}

/// Narrowing cannot add a host, which is what separates it from a way of
/// asking for one.
#[test]
fn reaching_for_a_host_that_was_not_granted_is_refused() {
    let (_, run) = run_reaching(
        &report(
            &["write", "reach"],
            "  match Io.reach(sys.net, \"elsewhere.example\") {\n    ok(narrower) => Io.write(sys.console, \"GOT ONE\"),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        &["granted.example"],
    );

    assert!(run.result.is_ok());
    assert!(
        !run.output[0].contains("GOT ONE"),
        "narrowing minted a host: {:?}",
        run.output
    );
}

/// Refused by name with the reason, rather than by failing to connect. A
/// reader who gets "connection reset" from an `https` URL goes looking at
/// their network.
#[test]
fn https_says_why_rather_than_failing_to_connect() {
    let (_, run) = run_reaching(
        &report(
            &["write", "fetch"],
            "  match Io.fetch(sys.net, \"https://granted.example/x\") {\n    ok(text) => Io.write(sys.console, text),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        &["granted.example"],
    );

    assert!(run.result.is_ok());
    assert!(run.output[0].contains("TLS"), "{:?}", run.output);
    assert!(
        run.output[0].contains("no dependencies"),
        "{:?}",
        run.output
    );
}

/// A status the caller did not ask for is an answer about the request, not a
/// failure of the runtime, and the body of a refusal is usually the reason.
#[test]
fn a_status_that_is_not_success_comes_back_as_an_error_carrying_the_body() {
    let server = Answering::with(&[
        "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nno such x",
    ]);
    let (_, run) = run_reaching(
        &report(
            &["write", "fetch"],
            &format!(
                "  match Io.fetch(sys.net, \"{}\") {{\n    ok(text) => Io.write(sys.console, text),\n    err(why) => Io.write(sys.console, why),\n  }}",
                server.url("/x")
            ),
        ),
        &[&server.host()],
    );

    assert!(run.result.is_ok());
    assert!(run.output[0].contains("404"), "{:?}", run.output);
    assert!(run.output[0].contains("no such x"), "{:?}", run.output);
}

/// A test block reaches no host for the reason it reaches no directory: a
/// test whose answer depends on a machine nobody in this repository controls
/// is a test of that machine. The rule is the language's rather than the
/// runner's, and this is where it is: nothing inside a test block can name a
/// capability, so there is no `Net` to hand to `Io.fetch`.
#[test]
fn a_test_block_has_no_way_to_name_a_net() {
    let (_, refused) = check(
        "module a\n\n\
         fn ask(net: Net) -> String\n  uses Io.fetch,\n{\n  \
           match Io.fetch(net, \"http://x.example/\") {\n    ok(text) => text,\n    err(why) => why,\n  }\n\
         }\n\n\
         test \"reaching out\" {\n  assert ask(Net) == \"\"\n}\n",
    );
    assert!(
        codes_of(&refused.diagnostics).contains(&deed_typeck::codes::NOT_A_VALUE),
        "{:?}",
        codes_of(&refused.diagnostics)
    );
}

#[test]
fn a_program_given_no_directory_has_none() {
    // `sys.console` still works. Only the part nothing granted is missing, and
    // the runtime does not quietly substitute the working directory.
    let (sources, run) = run(&report(
        &["write", "read"],
        "  match Io.read(sys.files, \"anything\") {\n    ok(text) => Io.write(sys.console, text),\n    err(why) => Io.write(sys.console, why),\n  }",
    ));
    let failure = run.result.expect_err("there is no directory to reach");
    assert_eq!(failure.code, deed_interp::codes::NOT_RUNNABLE);

    // The one message in the interpreter a reader meets on a file that checked
    // and a program that is right, which is why it is read here rather than
    // only counted. It used to be phrased as a gap in the interpreter, so
    // whoever pointed `deed run --dir` at a directory that is not there went
    // looking in the compiler for their own typo.
    let text = render_human(&sources, &failure);
    assert!(
        text.contains("this program was not given a directory"),
        "{text}"
    );
    // The whole note, not the tail of it. The mistake this usually reports is
    // a typo on the command line, so the sentence has to say where the root
    // comes from before it says what sets it.
    assert!(
        text.contains("`sys.files` hands out the directory the run was rooted at"),
        "{text}"
    );
    assert!(text.contains("`deed run` roots it at `--dir`"), "{text}");
}

#[test]
fn a_function_holding_only_a_dir_cannot_write() {
    let (sources, checked) = check(
        "module a\n\nfn peek(files: Dir) -> ()\n  uses Io.write,\n{\n  Io.write(files, \"hello\")\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::TYPE_MISMATCH),
        "{text}"
    );
    assert!(text.contains("expected `Console`, found `Dir`"), "{text}");
}

// -- writing ---------------------------------------------------------------

#[test]
fn a_dir_writes_a_file_inside_it() {
    let scratch = Scratch::new("save");

    let (_, run) = run_in(
        &report(
            &["write", "save"],
            "  match Io.save(sys.files, \"note.txt\", \"the contents\") {\n    ok(nothing) => Io.write(sys.console, \"saved\"),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["saved".to_string()]);
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("note.txt")).unwrap(),
        "the contents"
    );
}

#[test]
fn every_way_out_of_a_dir_is_refused_for_writing_too() {
    // The same list as the reading case, because writing goes through the same
    // check. A second implementation that agreed today would stop agreeing the
    // first time one of them was edited.
    let scratch = Scratch::new("save-escape");
    let outside = scratch.path().parent().unwrap().to_path_buf();

    for attempt in [
        "..",
        ".",
        "../escaped.txt",
        "/etc/passwd",
        "C:\\\\Windows\\\\System32",
        "a/b",
        "a\\\\b",
        "",
    ] {
        let (_, run) = run_in(
            &report(
                &["write", "save"],
                &format!(
                    "  match Io.save(sys.files, \"{attempt}\", \"escaped\") {{\n    ok(nothing) => Io.write(sys.console, \"WROTE IT\"),\n    err(why) => Io.write(sys.console, why),\n  }}"
                ),
            ),
            scratch.path(),
        );
        assert!(run.result.is_ok());
        assert_ne!(
            run.output,
            vec!["WROTE IT".to_string()],
            "`{attempt}` got out of the directory"
        );
    }

    assert!(
        !outside.join("escaped.txt").exists(),
        "something was written beside the directory"
    );
}

#[test]
fn saving_without_declaring_it_is_an_effect_error() {
    // Holding a `Dir` is not permission to write to it. The row has to say so,
    // which is what keeps `Io.read` and `Io.save` different authorities over
    // the same capability.
    let (sources, checked) = check(
        "module a\n\nfn sneak(files: Dir) -> Result<(), String>\n  uses Io.read,\n{\n  Io.save(files, \"note.txt\", \"hello\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_function_with_no_dir_has_nothing_to_save_into() {
    let (sources, checked) = check(
        "module a\n\nfn sneak() -> Result<(), String>\n  uses Io.save,\n{\n  Io.save(Dir, \"note.txt\", \"hello\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::NOT_A_VALUE),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- deleting --------------------------------------------------------------
//
// Reading, listing and writing all leave what was there. Deleting does not,
// and the difference is not one of degree: a program that writes the wrong
// bytes can be put back from what it overwrote, and one that deletes the wrong
// file cannot be put back from anything.
//
// The claim being tested is that this needed no new mechanism. It is a fourth
// entry in the row over the same `Dir`, the same way `Io.save` and `Io.list`
// were, and holding the directory still says nothing about which of the four a
// function may do.

#[test]
fn a_dir_removes_a_file_inside_it() {
    let scratch = Scratch::new("remove");
    scratch.write("note.txt", "the contents");

    let (_, run) = run_in(
        &report(
            &["write", "remove"],
            "  match Io.remove(sys.files, \"note.txt\") {\n    ok(nothing) => Io.write(sys.console, \"removed\"),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["removed".to_string()]);
    assert!(!scratch.path().join("note.txt").exists());
}

#[test]
fn removing_something_that_is_not_there_is_an_error_rather_than_a_success() {
    // "It was already gone" and "I removed it" are different answers, and a
    // program that cannot tell them apart has a bug waiting.
    let scratch = Scratch::new("remove-missing");

    let (_, run) = run_in(
        &report(
            &["write", "remove"],
            "  match Io.remove(sys.files, \"nothing.txt\") {\n    ok(nothing) => Io.write(sys.console, \"REMOVED IT\"),\n    err(why) => Io.write(sys.console, \"refused\"),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["refused".to_string()]);
}

#[test]
fn a_directory_is_not_a_file_to_remove() {
    // Files only, like `list`. Removing a directory is a different operation
    // with a different blast radius and nothing here wants it.
    let scratch = Scratch::new("remove-dir");
    std::fs::create_dir(scratch.path().join("inside")).unwrap();

    let (_, run) = run_in(
        &report(
            &["write", "remove"],
            "  match Io.remove(sys.files, \"inside\") {\n    ok(nothing) => Io.write(sys.console, \"REMOVED IT\"),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_ne!(run.output, vec!["REMOVED IT".to_string()]);
    assert!(scratch.path().join("inside").exists());
}

#[test]
fn every_way_out_of_a_dir_is_refused_for_removing_too() {
    // The same list as reading and writing, because removing goes through the
    // same check rather than through a second one that agrees today.
    let scratch = Scratch::new("remove-escape");
    let outside = scratch.path().parent().unwrap().to_path_buf();
    std::fs::write(outside.join("target.txt"), "still here").unwrap();

    for attempt in [
        "..",
        ".",
        "../target.txt",
        "/etc/passwd",
        "C:\\\\Windows\\\\System32",
        "a/b",
        "a\\\\b",
        "",
    ] {
        let (_, run) = run_in(
            &report(
                &["write", "remove"],
                &format!(
                    "  match Io.remove(sys.files, \"{attempt}\") {{\n    ok(nothing) => Io.write(sys.console, \"REMOVED IT\"),\n    err(why) => Io.write(sys.console, why),\n  }}"
                ),
            ),
            scratch.path(),
        );
        assert!(run.result.is_ok());
        assert_ne!(
            run.output,
            vec!["REMOVED IT".to_string()],
            "`{attempt}` got out of the directory"
        );
    }

    assert!(
        outside.join("target.txt").exists(),
        "something outside the directory was deleted"
    );
    std::fs::remove_file(outside.join("target.txt")).ok();
}

#[test]
fn removing_without_declaring_it_is_an_effect_error() {
    // The whole argument for not inventing a second kind of `Dir`. This
    // function holds one and may write to it, and it still cannot delete
    // anything, because which of the two it is doing lives in the row.
    let (sources, checked) = check(
        "module a\n\nfn sneak(files: Dir) -> Result<(), String>\n  uses Io.save,\n{\n  Io.remove(files, \"note.txt\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn declaring_removal_does_not_grant_anything_else() {
    // The other direction, and the one that would make the row decorative if
    // it failed. Saying `uses Io.remove` is not a way to read the file first.
    let (sources, checked) = check(
        "module a\n\nfn sneak(files: Dir) -> Result<String, String>\n  uses Io.remove,\n{\n  Io.read(files, \"note.txt\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- making a place --------------------------------------------------------
//
// The one that looks like it breaks the rule and does not. `Io.make` hands
// back a `Dir`, and a `Dir` is authority, so it reads like authority being
// made. It is not: the directory it names is inside the one it was given, so
// what comes back reaches strictly less than what went in. Which paths happen
// to exist was never what a `Dir` granted, which is why `Io.save` writing a
// file that was not there is not authority creation either.

#[test]
fn a_dir_makes_a_directory_inside_it() {
    let scratch = Scratch::new("make");

    let (_, run) = run_in(
        &report(
            &["write", "make"],
            "  match Io.make(sys.files, \"shelf\") {\n    ok(inside) => Io.write(sys.console, \"made\"),\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["made".to_string()]);
    assert!(scratch.path().join("shelf").is_dir());
}

#[test]
fn what_comes_back_reaches_less_than_what_went_in() {
    // The claim, tested rather than asserted in a comment. The new `Dir` is a
    // `Dir` like any other, so climbing out of it is refused by the same rule
    // that refuses climbing out of the one it came from, however many times
    // this is done.
    let scratch = Scratch::new("make-narrower");
    let outside = scratch.path().parent().unwrap().to_path_buf();

    let (_, run) = run_in(
        &report(
            &["write", "make", "save"],
            "  match Io.make(sys.files, \"shelf\") {\n    ok(inside) => match Io.save(inside, \"..\", \"escaped\") {\n      ok(nothing) => Io.write(sys.console, \"ESCAPED\"),\n      err(why) => Io.write(sys.console, why),\n    },\n    err(why) => Io.write(sys.console, why),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_ne!(run.output, vec!["ESCAPED".to_string()]);
    assert!(!outside.join("escaped").exists());
}

#[test]
fn a_name_that_is_already_there_is_an_error_rather_than_a_success() {
    // "I made it" and "it was already there" are different answers, which is
    // the same reasoning that makes a missing file an error for `Io.remove`.
    let scratch = Scratch::new("make-twice");
    scratch.write("shelf", "a file, not a directory");

    let (_, run) = run_in(
        &report(
            &["write", "make"],
            "  match Io.make(sys.files, \"shelf\") {\n    ok(inside) => Io.write(sys.console, \"MADE IT\"),\n    err(why) => Io.write(sys.console, \"refused\"),\n  }",
        ),
        scratch.path(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["refused".to_string()]);
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("shelf")).unwrap(),
        "a file, not a directory"
    );
}

#[test]
fn every_way_out_of_a_dir_is_refused_for_making_too() {
    let scratch = Scratch::new("make-escape");
    let outside = scratch.path().parent().unwrap().to_path_buf();

    for attempt in [
        "..",
        ".",
        "../escaped",
        "/etc/passwd",
        "C:\\\\Windows\\\\System32",
        "a/b",
        "a\\\\b",
        "",
    ] {
        let (_, run) = run_in(
            &report(
                &["write", "make"],
                &format!(
                    "  match Io.make(sys.files, \"{attempt}\") {{\n    ok(inside) => Io.write(sys.console, \"MADE IT\"),\n    err(why) => Io.write(sys.console, why),\n  }}"
                ),
            ),
            scratch.path(),
        );
        assert!(run.result.is_ok());
        assert_ne!(
            run.output,
            vec!["MADE IT".to_string()],
            "`{attempt}` got out of the directory"
        );
    }

    assert!(
        !outside.join("escaped").exists(),
        "something was made beside the directory"
    );
}

#[test]
fn making_without_declaring_it_is_an_effect_error() {
    let (sources, checked) = check(
        "module a\n\nfn sneak(files: Dir) -> Result<Dir, String>\n  uses Io.save,\n{\n  Io.make(files, \"shelf\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn declaring_making_does_not_grant_anything_else() {
    let (sources, checked) = check(
        "module a\n\nfn sneak(files: Dir) -> Result<(), String>\n  uses Io.make,\n{\n  Io.save(files, \"note.txt\", \"hello\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

// -- arguments -------------------------------------------------------------

#[test]
fn a_program_reads_what_it_was_invoked_with() {
    let (_, run) = run_with(
        &report(
            &["write", "args"],
            "  Io.write(sys.console, join(Io.args(sys), \" and \"))",
        ),
        nowhere(),
        &["one".to_string(), "two".to_string()],
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["one and two".to_string()]);
}

#[test]
fn a_program_invoked_with_nothing_gets_an_empty_list() {
    // Not a `Result` and not an error. Being given no arguments is an ordinary
    // way to invoke a program, so there is nothing to fail about.
    let (_, run) = run_in(
        &report(
            &["write", "args"],
            "  Io.write(sys.console, to_string(length(Io.args(sys))))",
        ),
        nowhere(),
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["0".to_string()]);
}

#[test]
fn reading_the_arguments_without_declaring_it_is_an_effect_error() {
    // Arguments are not authority, but they are input from outside, and every
    // other way of getting input from outside says so in the signature.
    let (sources, checked) =
        check("module a\n\nfn peek(sys: System) -> List<String> {\n  Io.args(sys)\n}\n");
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn reading_the_arguments_takes_the_root_capability() {
    // A narrower capability is not enough, which keeps `Io.args` where it
    // belongs: near `main`, with everything below it handed the values rather
    // than the means to read them again.
    let (sources, checked) = check(
        "module a\n\nfn peek(files: Dir) -> List<String>\n  uses Io.args,\n{\n  Io.args(files)\n}\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_typeck::codes::TYPE_MISMATCH),
        "{text}"
    );
    assert!(text.contains("expected `System`, found `Dir`"), "{text}");
}

// -- reading what somebody typed ---------------------------------------------
//
// The `read`/`save` split applied to the console. The capability says which
// terminal; the row says which direction, and holding a `Console` is not
// permission to read from it.

#[test]
fn a_program_reads_the_lines_it_was_handed() {
    let (_, run) = run_reading(
        &report(
            &["write", "line"],
            "  match Io.line(sys.console) {\n    \
             ok(first) => Io.write(sys.console, first),\n    \
             err(why) => Io.write(sys.console, why),\n  }",
        ),
        &["ada", "lin"],
    );

    assert!(run.result.is_ok());
    assert_eq!(run.output, vec!["ada".to_string()]);
}

#[test]
fn each_call_hands_back_the_next_line() {
    let (_, run) = run_reading(
        &report(
            &["write", "line"],
            "  let first = match Io.line(sys.console) {\n    ok(text) => text,\n    \
             err(_) => \"-\",\n  }\n  \
             let second = match Io.line(sys.console) {\n    ok(text) => text,\n    \
             err(_) => \"-\",\n  }\n  \
             Io.write(sys.console, join([first, second], \" then \"))",
        ),
        &["ada", "lin"],
    );

    assert_eq!(run.output, vec!["ada then lin".to_string()]);
}

/// Running out is `err`, and an empty line is not.
///
/// The two have to be told apart or a program that loops until the input ends
/// either never stops or stops early, and both of those are silent. An empty
/// line is something somebody typed.
#[test]
fn running_out_of_input_is_not_the_same_as_an_empty_line() {
    let empty = run_reading(
        &report(
            &["write", "line"],
            "  match Io.line(sys.console) {\n    \
             ok(text) => Io.write(sys.console, join([\"read [\", text, \"]\"], \"\")),\n    \
             err(_) => Io.write(sys.console, \"nothing left\"),\n  }",
        ),
        &[""],
    );
    assert_eq!(empty.1.output, vec!["read []".to_string()]);

    let exhausted = run_reading(
        &report(
            &["write", "line"],
            "  match Io.line(sys.console) {\n    \
             ok(text) => Io.write(sys.console, join([\"read [\", text, \"]\"], \"\")),\n    \
             err(_) => Io.write(sys.console, \"nothing left\"),\n  }",
        ),
        &[],
    );
    assert_eq!(exhausted.1.output, vec!["nothing left".to_string()]);
}

/// The half that makes the row worth reading.
///
/// A function handed a `Console` to write to still cannot find out what
/// somebody typed, which is the same sentence `list` earns about a `Dir`.
#[test]
fn writing_to_a_console_is_not_permission_to_read_from_it() {
    let (sources, checked) = check(
        "module a\n\nfn ask(console: Console) -> Result<String, String>\n  uses Io.write,\n{\n  \
         Io.line(console)\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

/// And the other way round, so neither entry stands for both.
#[test]
fn reading_from_a_console_is_not_permission_to_write_to_it() {
    let (sources, checked) = check(
        "module a\n\nfn shout(console: Console) -> ()\n  uses Io.line,\n{\n  \
         Io.write(console, \"hello\")\n}\n",
    );
    assert!(
        codes_of(&checked.diagnostics).contains(&deed_effects::codes::UNDECLARED_EFFECT),
        "{}",
        rendered(&sources, &checked.diagnostics)
    );
}

#[test]
fn a_run_that_was_handed_nothing_reads_nothing() {
    // The default, and the one every caller but `deed run` takes: a test
    // whose answer depended on what somebody typed would be a test of the
    // typing.
    let (_, run) = run(&report(
        &["write", "line"],
        "  match Io.line(sys.console) {\n    \
         ok(_) => Io.write(sys.console, \"something\"),\n    \
         err(_) => Io.write(sys.console, \"nothing left\"),\n  }",
    ));

    assert_eq!(run.output, vec!["nothing left".to_string()]);
}

// -- the examples ----------------------------------------------------------

#[test]
fn the_hello_example_runs() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/hello.deed");
    let source = std::fs::read_to_string(path).expect("examples/hello.deed should exist");

    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "examples/hello.deed", source);
    assert!(
        checked.diagnostics.is_empty(),
        "the example should check cleanly:\n{}",
        rendered(&sources, &checked.diagnostics)
    );

    let run = run_main(&program_of(&checked), checked.file, nowhere(), &[])
        .expect("the example has a main");
    assert!(run.result.is_ok());
    assert!(run.output.iter().any(|line| line.contains("world")));
}

#[test]
fn the_config_example_reads_itself_and_nothing_else() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples");
    let source = std::fs::read_to_string(format!("{dir}/config.deed"))
        .expect("examples/config.deed should exist");

    let mut sources = SourceMap::new();
    let checked = check_text(&mut sources, "examples/config.deed", source);
    assert!(
        checked.diagnostics.is_empty(),
        "the example should check cleanly:\n{}",
        rendered(&sources, &checked.diagnostics)
    );

    let run = run_main(&program_of(&checked), checked.file, Path::new(dir), &[])
        .expect("the example has a main");
    assert!(run.result.is_ok());

    // The whole line, in order. This used to read the first line and then say
    // that nothing in `output[1..]` was `"found it"`, which is empty whenever
    // the program prints one line, so the half the test is named after cost
    // nothing to satisfy and the five refusals were never looked at.
    //
    // They are the subject. Each one says which rule turned it down, and a
    // change to `examples/config.deed` that stops refusing something has to
    // land here rather than only in the command-line test, because this is
    // where the claim about a `Dir` is made.
    assert_eq!(
        run.output,
        vec![
            "found it",
            "`..` would leave the directory, and there is no way out of a `Dir`",
            "`../Cargo.toml` is not a single name, and a `Dir` only takes one at a time",
            "`/etc/passwd` is not a single name, and a `Dir` only takes one at a time",
            "`nowhere` is not there",
            "used the fallback",
        ]
    );
}

#[test]
fn a_file_with_no_main_is_not_runnable() {
    let (_, checked) = check_ok("module a\n\nfn f() -> Int { 0 }\n");
    assert!(run_main(&program_of(&checked), checked.file, nowhere(), &[]).is_none());
}

// -- listing, which is not reading -------------------------------------------

#[test]
fn listing_needs_its_own_entry_in_the_row() {
    // The claim `design/04-capabilities.md` makes, tested at the first place
    // it can be. Holding a `Dir` and declaring `Io.read` means you may read
    // the file somebody told you about. Finding out what is there is strictly
    // more, and the row is what separates them.
    let (sources, checked) = check(
        "module a\n\n\
         fn sneak(files: Dir) -> Int\n\
         \x20 uses Io.read,\n\
         {\n\
         \x20 match Io.list(files) {\n\
         \x20   ok(found) => length(found),\n\
         \x20   err(why) => 0,\n\
         \x20 }\n\
         }\n",
    );
    let text = rendered(&sources, &checked.diagnostics);
    assert!(
        text.contains("performs `Io.list` without declaring it"),
        "{text}"
    );
}

#[test]
fn a_program_that_declares_it_can_see_what_is_there() {
    let scratch = Scratch::new("list");
    scratch.write("one.txt", "1");
    scratch.write("two.txt", "2");

    let (_, run) = run_in(
        "module a\n\n\
         fn main(sys: System) -> Int\n\
         \x20 uses Io.list, Io.write,\n\
         {\n\
         \x20 match Io.list(sys.files) {\n\
         \x20   ok(names) => Io.write(sys.console, join(names, \",\")),\n\
         \x20   err(why) => Io.write(sys.console, why),\n\
         \x20 }\n\
         \x20 0\n\
         }\n",
        scratch.path(),
    );

    // Sorted, because a caller that depends on the order the filesystem felt
    // like today is a caller with a bug that appears on somebody else's
    // machine.
    assert_eq!(run.output, vec!["one.txt,two.txt".to_string()]);
}

#[test]
fn listing_sees_only_what_is_directly_in_the_directory() {
    // Files only. A list holding two kinds of thing with no way to tell them
    // apart is the sort of thing that turns into a bug in the caller, and
    // `Io.open` already needs a name from somewhere.
    let scratch = Scratch::new("list-nested");
    scratch.write("top.txt", "1");
    std::fs::create_dir_all(scratch.path().join("inner")).unwrap();
    std::fs::write(scratch.path().join("inner").join("deep.txt"), "2").unwrap();

    let (_, run) = run_in(
        "module a\n\n\
         fn main(sys: System) -> Int\n\
         \x20 uses Io.list, Io.write,\n\
         {\n\
         \x20 match Io.list(sys.files) {\n\
         \x20   ok(names) => Io.write(sys.console, join(names, \",\")),\n\
         \x20   err(why) => Io.write(sys.console, why),\n\
         \x20 }\n\
         \x20 0\n\
         }\n",
        scratch.path(),
    );

    assert_eq!(run.output, vec!["top.txt".to_string()]);
}

#[test]
fn listing_a_narrowed_directory_sees_only_that_one() {
    // `Io.open` narrows, and listing the result sees inside the narrower one
    // and nothing above it. Authority only ever shrinks on the way down, and
    // enumerating does not go back up.
    let scratch = Scratch::new("list-narrowed");
    scratch.write("outside.txt", "1");
    std::fs::create_dir_all(scratch.path().join("inner")).unwrap();
    std::fs::write(scratch.path().join("inner").join("deep.txt"), "2").unwrap();

    let (_, run) = run_in(
        "module a\n\n\
         fn main(sys: System) -> Int\n\
         \x20 uses Io.open, Io.list, Io.write,\n\
         {\n\
         \x20 match Io.open(sys.files, \"inner\") {\n\
         \x20   ok(inner) => show(sys.console, inner),\n\
         \x20   err(why) => Io.write(sys.console, why),\n\
         \x20 }\n\
         \x20 0\n\
         }\n\n\
         fn show(out: Console, files: Dir) -> ()\n\
         \x20 uses Io.list, Io.write,\n\
         {\n\
         \x20 match Io.list(files) {\n\
         \x20   ok(names) => Io.write(out, join(names, \",\")),\n\
         \x20   err(why) => Io.write(out, why),\n\
         \x20 }\n\
         }\n",
        scratch.path(),
    );

    assert_eq!(run.output, vec!["deep.txt".to_string()]);
}

/// A scratch directory, named so parallel tests do not collide.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("deed-cap-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.0.join(name), contents).unwrap();
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}
