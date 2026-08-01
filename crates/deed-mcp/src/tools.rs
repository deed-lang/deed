//! The five questions an agent can ask, and the one thing it can ask about a
//! diagnostic code.
//!
//! Each tool takes a whole program's text, because that is the unit this
//! language has: `design/refusals.md` says why there is no REPL, and the same
//! reasoning applies here. There is no expression to evaluate at a prompt, so
//! there is nothing smaller than a module to send.
//!
//! Every answer is the JSON the rest of the compiler already publishes, handed
//! back as text. A tool result in MCP is content, not a typed value, so the
//! shape has to survive being a string either way; using the shape
//! `deed check --format json` writes means an agent that has seen one has seen
//! the other.

use deed_diagnostics::json_string;
use deed_driver::fix::fix;
use deed_lsp::Json;

use crate::{Failure, INVALID_PARAMS};

/// One tool: what it is called, what it is for, and what it takes.
struct Tool {
    name: &'static str,
    description: &'static str,
    /// The single argument this tool reads, and what to say about it.
    argument: (&'static str, &'static str),
}

/// The tools this server offers, in the order an agent would use them.
const TOOLS: &[Tool] = &[
    Tool {
        name: "deed_check",
        description: "Check a Deed program and report what the compiler found. Writes one JSON \
                      object a line. A `diagnostic` line is something wrong with the program. An \
                      `obligation` line is a contract clause the checker looked at: `tier` is \
                      `proven` when it was settled at compile time, `tested` when a test pins it, \
                      and `guarded` when it falls to a runtime check, in which case `reason` says \
                      what stopped it from being proven. Silence means the program is well formed.",
        argument: ("source", "The whole text of one Deed module."),
    },
    Tool {
        name: "deed_test",
        description: "Run every `test` block in a Deed program. Writes one JSON object a line \
                      with the test's name and whether it passed, and the failing diagnostic \
                      when it did not, then a `summary` line counting them, so a program with no \
                      tests in it says so rather than answering with nothing. Refuses without \
                      running when the program does not check: ask `deed_check` for what is \
                      wrong with it.",
        argument: ("source", "The whole text of one Deed module."),
    },
    Tool {
        name: "deed_run",
        description: "Run a Deed program's `main` and report what it printed. Refuses before \
                      running if the program does not check, or if `main`'s row asks for a \
                      capability this server does not hand out: there is no filesystem here, so \
                      a program that reads or writes files is refused rather than failed.",
        argument: ("source", "The whole text of one Deed module."),
    },
    Tool {
        name: "deed_fmt",
        description: "Format a Deed program into the one layout the formatter chooses. Returns \
                      the formatted text, or the parse diagnostics when the file does not parse, \
                      because a file with no tree has no layout to pick.",
        argument: ("source", "The whole text of one Deed module."),
    },
    Tool {
        name: "deed_fix",
        description: "Apply every machine-applicable repair the compiler offers and return the \
                      repaired program. Only the repairs `deed fix` would apply without asking; \
                      a suggestion the compiler is not sure about is left for the reader and \
                      shows up in `deed_check` instead.",
        argument: ("source", "The whole text of one Deed module."),
    },
    Tool {
        name: "deed_explain",
        description: "Explain one diagnostic code, such as DEED4025. Returns the page that code \
                      carries: what it means, why the rule exists, and usually an example.",
        argument: ("code", "A diagnostic code, like `DEED4025`."),
    },
];

/// The `tools/list` result.
pub fn listing() -> Json {
    Json::object(vec![(
        "tools",
        Json::Array(
            TOOLS
                .iter()
                .map(|tool| {
                    let (argument, about) = tool.argument;
                    Json::object(vec![
                        ("name", Json::string(tool.name)),
                        ("description", Json::string(tool.description)),
                        (
                            "inputSchema",
                            Json::object(vec![
                                ("type", Json::string("object")),
                                (
                                    "properties",
                                    Json::object(vec![(
                                        argument,
                                        Json::object(vec![
                                            ("type", Json::string("string")),
                                            ("description", Json::string(about)),
                                        ]),
                                    )]),
                                ),
                                ("required", Json::Array(vec![Json::string(argument)])),
                            ]),
                        ),
                    ])
                })
                .collect(),
        ),
    )])
}

/// Runs one tool, or says why it could not.
///
/// A tool that ran and found the program wrong is a success: the answer to
/// "check this" is the list of what is wrong with it. `isError` is reserved
/// for the call itself failing, which here means an unknown tool or a missing
/// argument.
pub fn call(name: &str, arguments: Option<&Json>) -> Result<Json, Failure> {
    let Some(tool) = TOOLS.iter().find(|tool| tool.name == name) else {
        return Err(Failure {
            code: INVALID_PARAMS,
            message: format!("this server has no `{name}` tool"),
        });
    };

    let (wanted, _) = tool.argument;
    let Some(value) = arguments
        .and_then(|arguments| arguments.get(wanted))
        .and_then(Json::as_str)
    else {
        return Err(Failure {
            code: INVALID_PARAMS,
            message: format!("`{name}` needs a `{wanted}` argument, as a string"),
        });
    };

    Ok(text_result(&answer(name, value)))
}

/// What each tool actually asks the compiler.
fn answer(name: &str, value: &str) -> String {
    match name {
        "deed_check" => deed_wasm::check_source(value),
        "deed_test" => deed_wasm::test_source(value),
        "deed_run" => deed_wasm::run_source(value),
        "deed_fmt" => deed_wasm::fmt_source(value),
        "deed_fix" => fixed(value),
        "deed_explain" => explained(value),
        // `call` looked the name up in the same table, so there is no other
        // arm to reach.
        other => unreachable!("no such tool: {other}"),
    }
}

/// The repaired program, and how much was repaired.
///
/// `gave_up` is reported rather than dropped: it says two repairs are undoing
/// each other, which makes the text that came back the eighth round's guess
/// rather than a settled answer.
fn fixed(source: &str) -> String {
    let outcome = fix(source, deed_wasm::diagnostics_of);
    format!(
        "{{\"kind\":\"fixed\",\"applied\":{},\"gave_up\":{},\"source\":{}}}\n",
        outcome.applied,
        outcome.gave_up,
        json_string(&outcome.source)
    )
}

/// One diagnostic code's page, or a line saying there is no such code.
///
/// The same five fields [`deed_wasm::explain_all`] writes for the whole index,
/// so an agent reading one page and a site rendering all of them are looking
/// at the same record.
fn explained(code: &str) -> String {
    let Some(page) = deed_explain::page(code) else {
        return format!(
            "{{\"kind\":\"explanation\",\"code\":{},\"found\":false}}\n",
            json_string(code)
        );
    };

    let optional = |text: Option<&str>| match text {
        Some(text) => json_string(text),
        None => "null".to_string(),
    };

    format!(
        "{{\"kind\":\"explanation\",\"code\":{},\"name\":{},\"text\":{},\"example\":{},\"example_source\":{}}}\n",
        json_string(page.code),
        json_string(page.name),
        json_string(page.text),
        optional(page.example),
        optional(page.example_source),
    )
}

/// The MCP shape for "here is some text".
fn text_result(text: &str) -> Json {
    Json::object(vec![(
        "content",
        Json::Array(vec![Json::object(vec![
            ("type", Json::string("text")),
            ("text", Json::string(text)),
        ])]),
    )])
}
