//! The protocol, driven the way a client drives it: bytes in, bytes out.
//!
//! `crates/deed-lsp/tests/session.rs` does this for the language server and
//! for the same reason. A server is what it says on the wire, and a test that
//! calls the handler with a struct it built itself is a test of the struct.

use std::io::Cursor;

use deed_lsp::{Json, json};
use deed_mcp::{PROTOCOL_VERSION, serve};

/// Drives a session and hands back one parsed answer a line.
fn session(messages: &[&str]) -> Vec<Json> {
    let input = messages.join("\n") + "\n";
    let mut reader = Cursor::new(input.into_bytes());
    let mut written: Vec<u8> = Vec::new();

    serve(&mut reader, &mut written).expect("a cursor does not fail to read or write");

    String::from_utf8(written)
        .expect("the server writes UTF-8")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| json::parse(line).expect("the server writes JSON"))
        .collect()
}

fn hello() -> &'static str {
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#
}

fn call_with_arguments(id: i64, name: &str, arguments: Json) -> String {
    Json::object(vec![
        ("jsonrpc", Json::string("2.0")),
        ("id", Json::number(id)),
        ("method", Json::string("tools/call")),
        (
            "params",
            Json::object(vec![("name", Json::string(name)), ("arguments", arguments)]),
        ),
    ])
    .to_text()
}

/// The tool call a client makes, with one string argument.
fn call(id: i64, name: &str, argument: &str, value: &str) -> String {
    call_with_arguments(
        id,
        name,
        Json::object(vec![(argument, Json::string(value))]),
    )
}

fn review_call(id: i64, before: &[&str], after: &[&str], policy: Option<Json>) -> String {
    let sources =
        |items: &[&str]| Json::Array(items.iter().map(|source| Json::string(*source)).collect());
    let mut arguments = vec![("before", sources(before)), ("after", sources(after))];
    if let Some(policy) = policy {
        arguments.push(("policy", policy));
    }
    call_with_arguments(id, "deed_review", Json::object(arguments))
}

/// The text a tool call came back with.
fn content(answer: &Json) -> &str {
    answer
        .at(&["result", "content"])
        .and_then(Json::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("text"))
        .and_then(Json::as_str)
        .unwrap_or_else(|| panic!("a tool result carries text: {}", answer.to_text()))
}

#[test]
fn the_handshake_answers_with_a_version_and_a_name() {
    let answers = session(&[hello()]);
    assert_eq!(answers.len(), 1, "one request, one answer");

    let result = answers[0].get("result").expect("initialize succeeds");
    assert_eq!(
        result.get("protocolVersion").and_then(Json::as_str),
        Some(PROTOCOL_VERSION)
    );
    assert_eq!(
        result.at(&["serverInfo", "name"]).and_then(Json::as_str),
        Some("deed")
    );
    assert_eq!(
        answers[0].get("jsonrpc").and_then(Json::as_str),
        Some("2.0")
    );
}

/// The handshake carries the one sentence an agent needs before it reads a
/// tool result: that a `guarded` obligation says why it is guarded.
///
/// Without it an agent has the tier and no idea the reason is there, which is
/// the half of `deed check --obligations` a reader has to be told to look at.
#[test]
fn the_handshake_points_at_the_thing_an_agent_would_otherwise_miss() {
    let answers = session(&[hello()]);
    let instructions = answers[0]
        .at(&["result", "instructions"])
        .and_then(Json::as_str)
        .expect("initialize carries instructions");

    for word in ["obligation", "guarded", "deed_check", "deed_explain"] {
        assert!(
            instructions.contains(word),
            "the instructions never say `{word}`: {instructions}"
        );
    }
}

/// Every tool, not just the one to start with.
///
/// The first version of this handshake named two of the six and described a
/// workflow that ended at `deed_check`. An agent read it and did exactly that:
/// sixty-five checks across six tasks and not one test run, on tasks scored on
/// whether their tests pass. A tool nothing points at is a tool nobody calls,
/// so a seventh arriving unmentioned should fail here rather than be found in
/// a transcript later.
#[test]
fn the_handshake_names_every_tool_the_server_offers() {
    let answers = session(&[hello(), r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#]);
    let instructions = answers[0]
        .at(&["result", "instructions"])
        .and_then(Json::as_str)
        .expect("initialize carries instructions");

    let offered = answers[1]
        .at(&["result", "tools"])
        .and_then(Json::as_array)
        .expect("tools/list answers with tools");
    assert!(
        !offered.is_empty(),
        "nothing was offered, so nothing is held"
    );

    for tool in offered {
        let name = tool
            .get("name")
            .and_then(Json::as_str)
            .expect("a tool name");
        assert!(
            instructions.contains(name),
            "`{name}` is offered and the handshake never mentions it: {instructions}"
        );
    }
}

/// And it says the thing the tiers are there to make sayable.
///
/// Checking settles the contract. Running the tests is a different question
/// and this is the one language where that difference is the point, so an
/// agent that is told to check and nothing else has been told half of it.
#[test]
fn the_handshake_says_checking_is_not_passing() {
    let answers = session(&[hello()]);
    let instructions = answers[0]
        .at(&["result", "instructions"])
        .and_then(Json::as_str)
        .expect("initialize carries instructions");

    assert!(
        instructions.contains("a program that checks is not a program that works"),
        "{instructions}"
    );
}

/// A notification has no id, and the protocol says a server answers nothing.
///
/// A client that got an answer to one would be reading it as the answer to
/// whatever it asked next.
#[test]
fn a_notification_is_answered_with_silence() {
    let answers = session(&[
        hello(),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#,
    ]);

    assert_eq!(answers.len(), 1, "only `initialize` had an id to answer");
}

#[test]
fn a_tool_call_before_the_handshake_is_refused() {
    let answers = session(&[&call(1, "deed_check", "source", "module p\n")]);

    assert_eq!(
        answers[0].at(&["error", "code"]).and_then(Json::as_i64),
        Some(deed_mcp::INVALID_REQUEST)
    );
    let message = answers[0]
        .at(&["error", "message"])
        .and_then(Json::as_str)
        .unwrap_or_default();
    assert!(message.contains("initialized"), "{message}");
}

#[test]
fn a_method_this_server_does_not_have_says_so_by_name() {
    let answers = session(&[
        hello(),
        r#"{"jsonrpc":"2.0","id":2,"method":"resources/list"}"#,
    ]);

    let message = answers[1]
        .at(&["error", "message"])
        .and_then(Json::as_str)
        .unwrap_or_default();
    assert!(message.contains("resources/list"), "{message}");
    assert_eq!(
        answers[1].at(&["error", "code"]).and_then(Json::as_i64),
        Some(deed_mcp::METHOD_NOT_FOUND)
    );
}

/// A line that is not JSON is answered under a null id, which is the one
/// place JSON-RPC allows one, because there was no id to read.
#[test]
fn a_line_that_is_not_json_is_still_answered() {
    let answers = session(&["this is not json"]);

    assert_eq!(answers[0].get("id"), Some(&Json::Null));
    assert_eq!(
        answers[0].at(&["error", "code"]).and_then(Json::as_i64),
        Some(deed_mcp::PARSE_ERROR)
    );
}

#[test]
fn ping_is_answered_with_an_empty_result() {
    let answers = session(&[hello(), r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#]);

    assert_eq!(answers[1].get("result"), Some(&Json::object(vec![])));
}

/// Every tool is listed with a description and a schema naming its arguments.
///
/// A tool with no schema is a tool an agent cannot call without guessing, and
/// guessing is what this whole surface exists to remove.
#[test]
fn every_tool_says_what_it_takes() {
    let answers = session(&[hello(), r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#]);

    let tools = answers[1]
        .at(&["result", "tools"])
        .and_then(Json::as_array)
        .expect("tools/list returns an array");
    assert!(!tools.is_empty(), "the server offers no tools at all");

    for tool in tools {
        let name = tool
            .get("name")
            .and_then(Json::as_str)
            .expect("a tool has a name");
        let description = tool
            .get("description")
            .and_then(Json::as_str)
            .unwrap_or_default();
        assert!(
            description.len() > 40,
            "`{name}` has nothing worth reading as a description"
        );

        let required = tool
            .at(&["inputSchema", "required"])
            .and_then(Json::as_array)
            .unwrap_or_else(|| panic!("`{name}` has no required arguments"));
        if name == "deed_review" {
            assert_eq!(
                required.iter().filter_map(Json::as_str).collect::<Vec<_>>(),
                ["before", "after"]
            );
            for argument in ["before", "after"] {
                assert_eq!(
                    tool.at(&["inputSchema", "properties", argument, "type"])
                        .and_then(Json::as_str),
                    Some("array")
                );
                assert_eq!(
                    tool.at(&["inputSchema", "properties", argument, "items", "type"])
                        .and_then(Json::as_str),
                    Some("string")
                );
            }
            assert_eq!(
                tool.at(&["inputSchema", "properties", "policy", "type"])
                    .and_then(Json::as_str),
                Some("object")
            );
            for rule in ["denyNewAuthority", "denyWeakerPromises", "denyNewGuarded"] {
                assert_eq!(
                    tool.at(&[
                        "inputSchema",
                        "properties",
                        "policy",
                        "properties",
                        rule,
                        "type",
                    ])
                    .and_then(Json::as_str),
                    Some("boolean"),
                    "`deed_review` policy has no boolean `{rule}`"
                );
            }
            assert_eq!(
                tool.at(&[
                    "inputSchema",
                    "properties",
                    "policy",
                    "additionalProperties",
                ]),
                Some(&Json::Bool(false))
            );
        } else {
            assert_eq!(required.len(), 1, "`{name}` should take exactly one thing");
            let argument = required[0].as_str().expect("an argument is named");
            assert_eq!(
                tool.at(&["inputSchema", "properties", argument, "type"])
                    .and_then(Json::as_str),
                Some("string"),
                "`{name}`'s `{argument}` is not described as a string"
            );
        }
    }
}

/// The set is pinned rather than counted: a tool that appears without anybody
/// deciding to add it is exactly what this asks about.
#[test]
fn the_tools_on_offer_are_exactly_these() {
    let answers = session(&[hello(), r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#]);

    let mut names: Vec<&str> = answers[1]
        .at(&["result", "tools"])
        .and_then(Json::as_array)
        .expect("tools/list returns an array")
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Json::as_str))
        .collect();
    names.sort_unstable();

    assert_eq!(
        names,
        [
            "deed_check",
            "deed_explain",
            "deed_fix",
            "deed_fmt",
            "deed_review",
            "deed_run",
            "deed_test",
        ]
    );
}

#[test]
fn the_agent_guide_names_exactly_the_tools_on_offer() {
    let guide = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../how-to/let-an-agent-use-the-compiler.md"),
    )
    .expect("the agent guide should be there");
    let mut documented = guide
        .lines()
        .filter_map(|line| line.strip_prefix("| `deed_"))
        .filter_map(|line| line.split('`').next())
        .map(|name| format!("deed_{name}"))
        .collect::<Vec<_>>();
    documented.sort();

    let answers = session(&[hello(), r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#]);
    let mut offered = answers[1]
        .at(&["result", "tools"])
        .and_then(Json::as_array)
        .expect("tools/list returns tools")
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Json::as_str))
        .map(str::to_string)
        .collect::<Vec<_>>();
    offered.sort();

    assert_eq!(documented, offered);
}

#[test]
fn reviewing_two_module_sets_returns_a_policy_receipt() {
    let audit = "module services/audit\n\neffect Audit { fn note(value: Int) -> () }\n";
    let before = "module app/main\n\nuse services/audit.{Audit}\n\nfn work() -> () { () }\n";
    let after = "module app/main\n\nuse services/audit.{Audit}\n\n\
                 fn work() -> () uses Audit.note, { Audit.note(1) }\n";
    let policy = Json::object(vec![("denyNewAuthority", Json::Bool(true))]);
    let request = review_call(2, &[before, audit], &[after, audit], Some(policy));
    let answers = session(&[hello(), &request]);
    let receipt = json::parse(content(&answers[1]).trim()).expect("the receipt is JSON");

    assert_eq!(
        receipt
            .at(&["authority_added"])
            .and_then(Json::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("authority"))
            .and_then(Json::as_str),
        Some("services/audit/Audit.note")
    );
    assert_eq!(receipt.at(&["policy", "passed"]), Some(&Json::Bool(false)));
    assert!(answers[1].get("result").is_some());
    assert!(answers[1].get("error").is_none());
}

#[test]
fn every_review_policy_field_enforces_its_driver_rule() {
    let proven = "module review/sample\n\n\
                  type Positive = Int where value > 0\n\n\
                  fn preserve(value: Positive) -> Positive { value + 1 }\n";
    let guarded = "module review/sample\n\n\
                   type Positive = Int where value > 0\n\n\
                   fn preserve(value: Int) -> Positive { value + 1 }\n";
    let no_function = "module review/sample\n\ntype Positive = Int where value > 0\n";
    let new_guarded = "module review/sample\n\n\
                       type Positive = Int where value > 0\n\n\
                       fn accept(value: Int) -> Positive { value }\n";

    for (field, before, after, rule) in [
        (
            "denyWeakerPromises",
            proven,
            guarded,
            "deny-weaker-promises",
        ),
        (
            "denyNewGuarded",
            no_function,
            new_guarded,
            "deny-new-guarded",
        ),
    ] {
        let policy = Json::object(vec![(field, Json::Bool(true))]);
        let request = review_call(2, &[before], &[after], Some(policy));
        let answers = session(&[hello(), &request]);
        let receipt = json::parse(content(&answers[1]).trim()).expect("the receipt is JSON");
        assert_eq!(
            receipt
                .at(&["policy", "violations"])
                .and_then(Json::as_array)
                .and_then(|items| items.first())
                .and_then(|violation| violation.get("rule"))
                .and_then(Json::as_str),
            Some(rule),
            "{field} mapped to the wrong policy"
        );
    }
}

#[test]
fn review_resolves_the_shipped_modules_named_in_its_sources() {
    let source = "module app/main\n\nuse std/list.{sum}\n\n\
                  fn total(items: List<Int>) -> Int { sum(items) }\n";
    let request = review_call(2, &[source], &[source], None);
    let answers = session(&[hello(), &request]);
    let receipt = json::parse(content(&answers[1]).trim()).expect("the receipt is JSON");

    assert_eq!(
        receipt.get("kind").and_then(Json::as_str),
        Some("review_receipt")
    );
    assert_eq!(receipt.get("clean"), Some(&Json::Bool(true)));
}

#[test]
fn review_refuses_broken_or_unnamed_sources_without_failing_the_tool_call() {
    let before = "module app/main\n\nfn work() -> Int { 1 }\n";
    let broken = "module app/main\n\nfn work() -> Missing { 1 }\n";
    let unnamed = "fn work() -> Int { 1 }\n";

    for after in [broken, unnamed] {
        let request = review_call(2, &[before], &[after], None);
        let answers = session(&[hello(), &request]);
        let records = content(&answers[1])
            .lines()
            .map(|line| json::parse(line).expect("review refusal lines are JSON"))
            .collect::<Vec<_>>();
        let kinds = records
            .iter()
            .filter_map(|record| record.get("kind").and_then(Json::as_str))
            .collect::<Vec<_>>();

        assert!(kinds.contains(&"review_refused"), "{kinds:?}");
        assert!(!kinds.contains(&"review_receipt"), "{kinds:?}");
        assert!(answers[1].get("result").is_some());
        assert!(answers[1].get("error").is_none());
    }
}

#[test]
fn review_rejects_malformed_policy() {
    let source = "module app/main\n\nfn work() -> Int { 1 }\n";
    for policy in [
        Json::string("strict"),
        Json::object(vec![("unknownRule", Json::Bool(true))]),
        Json::object(vec![("denyNewAuthority", Json::string("yes"))]),
    ] {
        let request = review_call(2, &[source], &[source], Some(policy));
        let answers = session(&[hello(), &request]);
        assert_eq!(
            answers[1].at(&["error", "code"]).and_then(Json::as_i64),
            Some(deed_mcp::INVALID_PARAMS)
        );
        assert!(
            answers[1]
                .at(&["error", "message"])
                .and_then(Json::as_str)
                .is_some_and(|message| message.contains("policy"))
        );
    }
}

#[test]
fn review_rejects_missing_empty_or_non_string_module_sets() {
    let source = Json::Array(vec![Json::string(
        "module app/main\n\nfn work() -> Int { 1 }\n",
    )]);
    let invalid = [
        Json::object(vec![("after", source.clone())]),
        Json::object(vec![
            ("before", Json::string("module app/main\n")),
            ("after", source.clone()),
        ]),
        Json::object(vec![
            ("before", Json::Array(vec![])),
            ("after", source.clone()),
        ]),
        Json::object(vec![
            ("before", Json::Array(vec![Json::number(1)])),
            ("after", source),
        ]),
    ];

    for arguments in invalid {
        let request = call_with_arguments(2, "deed_review", arguments);
        let answers = session(&[hello(), &request]);
        assert_eq!(
            answers[1].at(&["error", "code"]).and_then(Json::as_i64),
            Some(deed_mcp::INVALID_PARAMS),
            "{}",
            answers[1].to_text()
        );
    }
}

#[test]
fn checking_a_good_program_says_nothing_and_checking_a_bad_one_names_the_code() {
    let good = "module p\n\nfn twice(n: Int) -> Int {\n    n * 2\n}\n";
    let answers = session(&[hello(), &call(2, "deed_check", "source", good)]);
    assert_eq!(
        content(&answers[1]),
        "",
        "a well formed program has nothing to report"
    );

    let bad = "module p\n\nfn twice(n: Int) -> Int {\n    nope * 2\n}\n";
    let answers = session(&[hello(), &call(2, "deed_check", "source", bad)]);
    let text = content(&answers[1]);
    assert!(text.contains("\"kind\":\"diagnostic\""), "{text}");
    assert!(text.contains("DEED3001"), "{text}");
}

/// The half of `check` an agent would otherwise never see.
///
/// A tier alone says an obligation was not proven. The reason says what to do
/// about it, and it is the thing `design/01-principles.md` asks a diagnostic
/// to carry.
#[test]
fn a_guarded_obligation_comes_back_with_the_reason_it_was_not_proven() {
    let source = "module p\n\ntype Positive = Int where value > 0\n\n\
                  fn keep(n: Int) -> Positive {\n    n\n}\n";

    let answers = session(&[hello(), &call(2, "deed_check", "source", source)]);
    let text = content(&answers[1]);

    let obligation = text
        .lines()
        .find(|line| line.contains("\"kind\":\"obligation\"") && line.contains("\"guarded\""))
        .unwrap_or_else(|| panic!("nothing came back guarded:\n{text}"));
    assert!(
        obligation.contains("\"reason\":\"nothing narrowed this name\""),
        "the guarded obligation carries no reason: {obligation}"
    );
}

#[test]
fn tests_run_and_report_one_line_each() {
    let source = "module p\n\nfn twice(n: Int) -> Int {\n    n * 2\n}\n\n\
                  test \"doubles\" {\n    assert twice(2) == 4\n}\n\n\
                  test \"and again\" {\n    assert twice(3) == 6\n}\n";

    let answers = session(&[hello(), &call(2, "deed_test", "source", source)]);
    let text = content(&answers[1]);

    assert_eq!(text.lines().count(), 3, "{text}");
    assert!(
        text.contains("\"name\":\"doubles\",\"passed\":true"),
        "{text}"
    );
    assert!(
        text.contains("\"name\":\"and again\",\"passed\":true"),
        "{text}"
    );
    assert!(
        text.contains("{\"kind\":\"summary\",\"passed\":2,\"failed\":0}"),
        "{text}"
    );
}

/// An agent cannot count what it was not told. `deed_check` on this server
/// answers a well formed program with silence, so `deed_test` answering a
/// file with no tests the same way would leave the two readings of an empty
/// answer sitting on top of each other.
#[test]
fn a_program_with_no_tests_in_it_says_so() {
    let source = "module p\n\nfn twice(n: Int) -> Int {\n    n * 2\n}\n";

    let answers = session(&[hello(), &call(2, "deed_test", "source", source)]);
    let text = content(&answers[1]);

    assert_eq!(
        text.trim_end(),
        "{\"kind\":\"summary\",\"passed\":0,\"failed\":0}",
        "{text}"
    );
}

/// The one that was wrong: this program does not check, and the server used
/// to run its test and report that it passed.
#[test]
fn a_program_that_does_not_check_is_refused_rather_than_tested() {
    let source = "module p\n\nfn twice(n: Int) -> Int {\n    nonesuch\n}\n\n\
                  test \"doubles\" {\n    assert 1 == 1\n}\n";

    let answers = session(&[hello(), &call(2, "deed_test", "source", source)]);
    let text = content(&answers[1]);

    assert!(text.contains("\"kind\":\"refused\""), "{text}");
    assert!(
        !text.contains("\"passed\":true"),
        "a test in a file that does not check was reported as passing: {text}"
    );
}

#[test]
fn running_a_program_prints_what_it_printed() {
    let source = "module p\n\nfn main(sys: System) -> () uses Io.write {\n    \
                  Io.write(sys.console, \"hello\")\n}\n";

    let answers = session(&[hello(), &call(2, "deed_run", "source", source)]);
    let text = content(&answers[1]);

    assert!(
        text.contains("\"kind\":\"output\",\"line\":\"hello\""),
        "{text}"
    );
    assert!(text.contains("\"kind\":\"result\",\"ok\":true"), "{text}");
}

/// The capability claim, on the wire.
///
/// This server hands out no directory, so a program whose row reaches one is
/// refused before it runs rather than failing part way through. That is the
/// same order `design/04-capabilities.md` puts the two in.
///
/// The program checks cleanly, which it has to: a file the checker rejects is
/// refused for that reason first, and this test would then be passing on the
/// wrong refusal.
#[test]
fn a_program_that_wants_a_file_is_refused_before_it_runs() {
    let source = "module p\n\nfn main(sys: System) -> () uses Io.write, Io.read {\n    \
                  Io.write(sys.console, \"before\")\n    \
                  match Io.read(sys.files, \"nothing.txt\") {\n        \
                  ok(text) => Io.write(sys.console, text),\n        \
                  err(why) => Io.write(sys.console, why),\n    }\n}\n";

    let answers = session(&[hello(), &call(2, "deed_run", "source", source)]);
    let text = content(&answers[1]);

    assert!(text.contains("\"kind\":\"capability\""), "{text}");
    assert!(
        !text.contains("\"kind\":\"refused\""),
        "it was refused for the wrong reason: {text}"
    );
    assert!(
        !text.contains("\"kind\":\"output\""),
        "it printed something before refusing: {text}"
    );
}

#[test]
fn formatting_returns_the_one_layout_the_formatter_chooses() {
    let answers = session(&[
        hello(),
        &call(
            2,
            "deed_fmt",
            "source",
            "module p\nfn f( ) -> Int {   1 }\n",
        ),
    ]);
    let text = content(&answers[1]);

    assert!(text.contains("\"kind\":\"formatted\""), "{text}");
    assert!(text.contains("fn f() -> Int {"), "{text}");
}

#[test]
fn fixing_applies_the_repair_and_says_how_many_went_in() {
    // `struct` is spelled `record` here, and the compiler offers that as a
    // machine-applicable repair.
    let answers = session(&[
        hello(),
        &call(
            2,
            "deed_fix",
            "source",
            "module p\n\nstruct Point {\n    x: Int,\n}\n",
        ),
    ]);
    let text = content(&answers[1]);

    assert!(text.contains("\"kind\":\"fixed\""), "{text}");
    assert!(text.contains("\"applied\":1"), "{text}");
    assert!(text.contains("\"gave_up\":false"), "{text}");
    assert!(text.contains("record Point"), "{text}");
}

#[test]
fn explaining_a_code_returns_its_page_and_an_unknown_one_says_so() {
    let answers = session(&[hello(), &call(2, "deed_explain", "code", "DEED4025")]);
    let text = content(&answers[1]);
    assert!(text.contains("\"code\":\"DEED4025\""), "{text}");
    assert!(text.contains("\"text\":"), "{text}");

    let answers = session(&[hello(), &call(2, "deed_explain", "code", "DEED9999")]);
    assert!(content(&answers[1]).contains("\"found\":false"));
}

#[test]
fn a_tool_call_with_no_argument_says_which_one_is_missing() {
    let missing = Json::object(vec![
        ("jsonrpc", Json::string("2.0")),
        ("id", Json::number(2)),
        ("method", Json::string("tools/call")),
        (
            "params",
            Json::object(vec![("name", Json::string("deed_check"))]),
        ),
    ])
    .to_text();

    let answers = session(&[hello(), &missing]);
    let message = answers[1]
        .at(&["error", "message"])
        .and_then(Json::as_str)
        .unwrap_or_default();

    assert!(message.contains("source"), "{message}");
    assert!(message.contains("deed_check"), "{message}");
}

#[test]
fn a_tool_this_server_does_not_have_says_so_by_name() {
    let answers = session(&[hello(), &call(2, "deed_deploy", "source", "module p\n")]);
    let message = answers[1]
        .at(&["error", "message"])
        .and_then(Json::as_str)
        .unwrap_or_default();

    assert!(message.contains("deed_deploy"), "{message}");
}

/// A program that is wrong is not a call that failed.
///
/// `isError` is about the transport; the answer to "check this" is the list of
/// what is wrong with it, and a client that treated that as a failed call
/// would retry instead of reading it.
#[test]
fn a_program_with_errors_is_a_successful_call() {
    let answers = session(&[
        hello(),
        &call(
            2,
            "deed_check",
            "source",
            "module p\n\nfn f() -> Int {\n    nope\n}\n",
        ),
    ]);

    assert!(
        answers[1].get("result").is_some(),
        "{}",
        answers[1].to_text()
    );
    assert!(answers[1].get("error").is_none());
    assert!(content(&answers[1]).contains("DEED3001"));
}
