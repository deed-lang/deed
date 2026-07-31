//! The agent and the page get the same answer, byte for byte.
//!
//! `crates/deed-cli/tests/agreement.rs` holds the CLI against the language
//! server for the same reason: this repository's recurring mistake is two
//! consumers of one idea drifting apart, and the one nobody is looking at is
//! the one that drifts.
//!
//! Here the risk is specific. `deed-wasm` answers a browser and `deed-mcp`
//! answers an agent, and both were written to be "text in, JSON out". The
//! moment one of them grows a field the other does not, an agent and a person
//! are reading different compilers.

use deed_lsp::{Json, json};
use deed_mcp::tools;

/// What a tool call comes back with, without the protocol around it.
fn answer(name: &str, argument: &str, value: &str) -> String {
    let arguments = Json::object(vec![(argument, Json::string(value))]);
    let result = tools::call(name, Some(&arguments)).expect("the tool exists and got its argument");

    result
        .get("content")
        .and_then(Json::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(Json::as_str)
        .expect("a tool result carries text")
        .to_string()
}

/// Programs chosen so each verb has something to say about them: one that
/// checks clean and runs, one that does not check, one with a test, one with a
/// guarded obligation, and one the formatter has work to do on.
fn subjects() -> Vec<&'static str> {
    vec![
        "module p\n\nfn twice(n: Int) -> Int {\n    n * 2\n}\n",
        "module p\n\nfn f() -> Int {\n    nope\n}\n",
        "module p\n\nfn twice(n: Int) -> Int {\n    n * 2\n}\n\n\
         test \"doubles\" {\n    assert twice(2) == 4\n}\n",
        "module p\n\ntype Positive = Int where value > 0\n\n\
         fn keep(n: Int) -> Positive {\n    n\n}\n",
        "module p\nfn f( ) -> Int {   1 }\n",
        "module p\n\nfn main(sys: System) -> () uses Io.write {\n    \
         Io.write(sys.console, \"hi\")\n}\n",
    ]
}

#[test]
fn checking_gives_an_agent_what_it_gives_a_page() {
    for source in subjects() {
        assert_eq!(
            answer("deed_check", "source", source),
            deed_wasm::check_source(source),
            "check drifted on:\n{source}"
        );
    }
}

#[test]
fn testing_gives_an_agent_what_it_gives_a_page() {
    for source in subjects() {
        assert_eq!(
            answer("deed_test", "source", source),
            deed_wasm::test_source(source),
            "test drifted on:\n{source}"
        );
    }
}

#[test]
fn running_gives_an_agent_what_it_gives_a_page() {
    for source in subjects() {
        assert_eq!(
            answer("deed_run", "source", source),
            deed_wasm::run_source(source),
            "run drifted on:\n{source}"
        );
    }
}

#[test]
fn formatting_gives_an_agent_what_it_gives_a_page() {
    for source in subjects() {
        assert_eq!(
            answer("deed_fmt", "source", source),
            deed_wasm::fmt_source(source),
            "fmt drifted on:\n{source}"
        );
    }
}

/// The corpus is not empty and the verbs are not all answering nothing.
///
/// Without this the four tests above pass on a list of programs that produce
/// no output at all, which is this repository's oldest way of passing for
/// free (#194, #205, #207).
#[test]
fn the_agreement_is_handed_something_to_disagree_about() {
    let sources = subjects();
    assert!(sources.len() >= 5, "too few programs to be a corpus");

    let mut said_something = 0;
    for source in &sources {
        for name in ["deed_check", "deed_test", "deed_run", "deed_fmt"] {
            if !answer(name, "source", source).is_empty() {
                said_something += 1;
            }
        }
    }

    assert!(
        said_something >= sources.len(),
        "only {said_something} of the calls produced any output at all"
    );
}

/// One page per code on both sides.
///
/// `deed_explain` answers one code and `deed_wasm::explain_all` answers every
/// code, so the shape is what agrees rather than the text: an agent reading
/// one record and a site rendering the index should not be looking at
/// different fields.
#[test]
fn one_explanation_has_the_fields_the_whole_index_has() {
    let index = deed_wasm::explain_all();
    let first = index.lines().next().expect("there is at least one page");
    let indexed = json::parse(first).expect("the index is JSON");

    let single = answer("deed_explain", "code", "DEED4025");
    let single = json::parse(single.trim()).expect("one explanation is JSON");

    for field in ["code", "name", "text", "example", "example_source"] {
        assert!(
            indexed.get(field).is_some(),
            "the index lost its `{field}` field"
        );
        assert!(
            single.get(field).is_some(),
            "one explanation has no `{field}` field"
        );
    }
}

/// Every code `deed explain` can answer, `deed_explain` can answer.
#[test]
fn no_code_is_reachable_from_the_page_and_not_from_an_agent() {
    for page in deed_explain::all_pages() {
        let text = answer("deed_explain", "code", page.code);
        assert!(
            !text.contains("\"found\":false"),
            "`{}` is in the index but an agent cannot ask for it",
            page.code
        );
    }
}
