//! What a hover costs, relative to another question about the same position.
//!
//! Not a wall clock budget, for the reason `crates/deed-driver/tests/scaling.rs`
//! gives at the top of itself: a test that fails when the machine is busy is a
//! test people learn to rerun. What is worth holding is a ratio, and here the
//! ratio is about one number.
//!
//! Every request that needs types checks the whole workspace, once, and throws
//! away everything but the file the cursor is in. There is no cache. PR #123
//! measured 38ms at 512 files against a 100ms budget, so the room for a second
//! pass on a keystroke is none.
//!
//! That is the whole reason a hover reports the tier an obligation landed in
//! rather than the language server doing one of the three things on the
//! candidate list. Semantic tokens, inlay hints for types and folding ranges
//! all want work that is not already done. The tier is already worked out:
//! `Checked::obligations` is built by the check the hover is running anyway.
//! An answer that costs nothing is an argument, and an argument that nothing
//! checks is a sentence in a pull request.
//!
//! So: go to definition and hover are asked about the same call, in the same
//! workspace, and the first one is the baseline because it does the same one
//! pass and has not changed. A second pass would show up as a two.

use std::time::{Duration, Instant};

use deed_lsp::{Json, Next, Server};

/// A workspace big enough that checking it is all either request does.
///
/// Small enough to stay quick, large enough that the per request bookkeeping
/// either of them does is lost in the noise. At four functions a file the
/// difference between one pass and two is a rounding error, which is how a
/// second pass gets in.
const FILES: usize = 24;

fn source(index: usize) -> String {
    let mut text = format!("module bench/m{index}\n\ntype Positive = Int where value > 0\n");
    for n in 0..8 {
        text.push_str(&format!(
            "\nfn step{n}(n: Int) -> Int\n\
             \x20 where\n\
             \x20   n > 1,\n\
             \x20 ensures\n\
             \x20   ok => result >= 0,\n\
             {{\n\
             \x20   n / 2\n\
             }}\n\n\
             fn call{n}() -> Positive {{\n\
             \x20   step{n}(10)\n\
             }}\n"
        ));
    }
    text
}

/// The document every request below is about.
const SUBJECT: &str = "file:///bench/m0.deed";

fn opened() -> Server {
    let mut server = Server::new();
    let (_, next) = server.handle(&Json::object(vec![
        ("jsonrpc", Json::string("2.0")),
        ("id", Json::number(1)),
        ("method", Json::string("initialize")),
        ("params", Json::object(Vec::new())),
    ]));
    assert_eq!(next, Next::Continue);

    for index in 0..FILES {
        server.handle(&Json::object(vec![
            ("jsonrpc", Json::string("2.0")),
            ("method", Json::string("textDocument/didOpen")),
            (
                "params",
                Json::object(vec![(
                    "textDocument",
                    Json::object(vec![
                        ("uri", Json::string(format!("file:///bench/m{index}.deed"))),
                        ("languageId", Json::string("deed")),
                        ("version", Json::number(1)),
                        ("text", Json::string(source(index))),
                    ]),
                )]),
            ),
        ]));
    }
    server
}

/// One request at the `step0(10)` call, which is line 15 of the first file.
fn ask(server: &mut Server, method: &str) -> Json {
    let (sent, _) = server.handle(&Json::object(vec![
        ("jsonrpc", Json::string("2.0")),
        ("id", Json::number(2)),
        ("method", Json::string(method)),
        (
            "params",
            Json::object(vec![
                (
                    "textDocument",
                    Json::object(vec![("uri", Json::string(SUBJECT))]),
                ),
                (
                    "position",
                    Json::object(vec![
                        ("line", Json::number(14)),
                        ("character", Json::number(4)),
                    ]),
                ),
            ]),
        ),
    ]));
    sent.into_iter().next().expect("a reply")
}

fn median(server: &mut Server, method: &str) -> Duration {
    let mut samples: Vec<Duration> = (0..5)
        .map(|_| {
            let start = Instant::now();
            let reply = ask(server, method);
            let elapsed = start.elapsed();
            assert!(
                !reply.at(&["result"]).is_some_and(Json::is_null),
                "{method} answered nothing, so it measured the wrong thing"
            );
            elapsed
        })
        .collect();
    samples.sort();
    samples[samples.len() / 2]
}

#[test]
fn a_hover_checks_the_workspace_once() {
    let mut server = opened();

    // Warm up, so neither measurement pays for whatever the allocator was
    // doing the first time round.
    ask(&mut server, "textDocument/definition");
    ask(&mut server, "textDocument/hover");

    let definition = median(&mut server, "textDocument/definition");
    let hover = median(&mut server, "textDocument/hover");

    let ratio = hover.as_secs_f64() / definition.as_secs_f64().max(f64::EPSILON);
    assert!(
        ratio < 1.6,
        "a hover took {ratio:.2}x what go to definition took ({:.2}ms vs {:.2}ms) \
         at the same position, and the two of them do one workspace check each, \
         so this is the shape of a hover having grown a second one",
        hover.as_secs_f64() * 1000.0,
        definition.as_secs_f64() * 1000.0
    );
}

#[test]
fn the_position_being_measured_is_one_that_carries_a_tier() {
    // Otherwise the measurement above is about a tooltip that says the same
    // thing it said before, which would cost the same either way and prove
    // nothing about the answer that was added.
    let mut server = opened();
    let text = ask(&mut server, "textDocument/hover")
        .at(&["result", "contents", "value"])
        .and_then(Json::as_str)
        .expect("a hover")
        .to_string();

    assert!(text.contains("`Positive`, guarded"), "{text}");
    assert!(text.contains("`step0 requires`, proven"), "{text}");
}
