//! The questions an agent can ask the compiler, including what changed
//! between two module sets.
//!
//! Most tools take one whole program's text, because that is the unit this
//! language has: `design/refusals.md` says why there is no REPL, and the same
//! reasoning applies here. Review takes two arrays of those units so imports
//! can resolve within each in-memory module set.
//!
//! Every answer is the JSON the rest of the compiler already publishes, handed
//! back as text. A tool result in MCP is content, not a typed value, so the
//! shape has to survive being a string either way; using the shape
//! `deed check --format json` writes means an agent that has seen one has seen
//! the other.

use deed_diagnostics::json_string;
use deed_driver::fix::fix;
use deed_driver::review::{ReviewPolicy, review_sources};
use deed_lsp::Json;

use crate::{Failure, INVALID_PARAMS};

/// One tool: what it is called, what it is for, and what it takes.
struct Tool {
    name: &'static str,
    description: &'static str,
    arguments: &'static [Argument],
}

#[derive(Clone, Copy)]
struct Argument {
    name: &'static str,
    description: &'static str,
    kind: ArgumentKind,
    required: bool,
}

#[derive(Clone, Copy)]
enum ArgumentKind {
    String,
    Sources,
    Policy,
}

const SOURCE: &[Argument] = &[Argument {
    name: "source",
    description: "The whole text of one Deed module.",
    kind: ArgumentKind::String,
    required: true,
}];

const CODE: &[Argument] = &[Argument {
    name: "code",
    description: "A diagnostic code, like `DEED4025`.",
    kind: ArgumentKind::String,
    required: true,
}];

const REVIEW: &[Argument] = &[
    Argument {
        name: "before",
        description: "Every Deed module before the patch, as source text.",
        kind: ArgumentKind::Sources,
        required: true,
    },
    Argument {
        name: "after",
        description: "Every Deed module after the patch, as source text.",
        kind: ArgumentKind::Sources,
        required: true,
    },
    Argument {
        name: "policy",
        description: "Optional gates for new authority, weaker promises and new Guarded obligations.",
        kind: ArgumentKind::Policy,
        required: false,
    },
];

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
        arguments: SOURCE,
    },
    Tool {
        name: "deed_test",
        description: "Run a Deed program's tests: every `test` block in it, and every property \
                      its contracts generate. Writes one JSON object a line with the name and \
                      whether it passed, and the failing diagnostic when it did not, then a \
                      `summary` line counting them, so a program with no tests in it says so \
                      rather than answering with nothing. A `property` line is one nobody wrote: \
                      the checker generated inputs from a `where` clause and held the function \
                      to its `ensures`, and the seed is on the line so the same run can be asked \
                      for again. Refuses without running when the program does not check: ask \
                      `deed_check` for what is wrong with it.",
        arguments: SOURCE,
    },
    Tool {
        name: "deed_run",
        description: "Run a Deed program's `main` and report what it printed. Refuses before \
                      running if the program does not check, or if `main`'s row asks for a \
                      capability this server does not hand out: there is no filesystem here, so \
                      a program that reads or writes files is refused rather than failed.",
        arguments: SOURCE,
    },
    Tool {
        name: "deed_fmt",
        description: "Format a Deed program into the one layout the formatter chooses. Returns \
                      the formatted text, or the parse diagnostics when the file does not parse, \
                      because a file with no tree has no layout to pick.",
        arguments: SOURCE,
    },
    Tool {
        name: "deed_fix",
        description: "Apply every machine-applicable repair the compiler offers and return the \
                      repaired program. Only the repairs `deed fix` would apply without asking; \
                      a suggestion the compiler is not sure about is left for the reader and \
                      shows up in `deed_check` instead.",
        arguments: SOURCE,
    },
    Tool {
        name: "deed_explain",
        description: "Explain one diagnostic code, such as DEED4025. Returns the page that code \
                      carries: what it means, why the rule exists, and usually an example.",
        arguments: CODE,
    },
    Tool {
        name: "deed_review",
        description: "Compare the checked module set before a patch with the set after it. Returns one JSON review receipt naming authority additions, obligation tier regressions and newly introduced Guarded obligations. An optional policy object accepts `denyNewAuthority`, `denyWeakerPromises` and `denyNewGuarded`; its verdict is evidence, not a transport error. Both sides stay in memory: this tool opens no file and holds no capability.",
        arguments: REVIEW,
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
                    let properties = tool
                        .arguments
                        .iter()
                        .map(|argument| (argument.name, argument_schema(argument)))
                        .collect();
                    let required = tool
                        .arguments
                        .iter()
                        .filter(|argument| argument.required)
                        .map(|argument| Json::string(argument.name))
                        .collect();
                    Json::object(vec![
                        ("name", Json::string(tool.name)),
                        ("description", Json::string(tool.description)),
                        (
                            "inputSchema",
                            Json::object(vec![
                                ("type", Json::string("object")),
                                ("properties", Json::object(properties)),
                                ("required", Json::Array(required)),
                            ]),
                        ),
                    ])
                })
                .collect(),
        ),
    )])
}

fn argument_schema(argument: &Argument) -> Json {
    let mut fields = vec![("description", Json::string(argument.description))];
    match argument.kind {
        ArgumentKind::String => fields.push(("type", Json::string("string"))),
        ArgumentKind::Sources => {
            fields.push(("type", Json::string("array")));
            fields.push((
                "items",
                Json::object(vec![("type", Json::string("string"))]),
            ));
            fields.push(("minItems", Json::number(1)));
        }
        ArgumentKind::Policy => {
            fields.push(("type", Json::string("object")));
            fields.push((
                "properties",
                Json::object(vec![
                    (
                        "denyNewAuthority",
                        Json::object(vec![("type", Json::string("boolean"))]),
                    ),
                    (
                        "denyWeakerPromises",
                        Json::object(vec![("type", Json::string("boolean"))]),
                    ),
                    (
                        "denyNewGuarded",
                        Json::object(vec![("type", Json::string("boolean"))]),
                    ),
                ]),
            ));
            fields.push(("additionalProperties", Json::Bool(false)));
        }
    }
    Json::object(fields)
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

    if name == "deed_review" {
        return review(arguments);
    }

    let wanted = tool.arguments[0].name;
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

fn review(arguments: Option<&Json>) -> Result<Json, Failure> {
    let Some(arguments) = arguments else {
        return Err(invalid(
            "`deed_review` needs `before` and `after` arguments",
        ));
    };
    let before_sources = source_set(arguments, "before")?;
    let after_sources = source_set(arguments, "after")?;
    let (policy, policy_was_given) = policy_of(arguments)?;
    let policy = policy_was_given.then_some(policy);
    Ok(text_result(&review_sources(
        &before_sources,
        &after_sources,
        policy,
    )))
}

fn source_set<'a>(arguments: &'a Json, name: &str) -> Result<Vec<&'a str>, Failure> {
    let Some(value) = arguments.get(name) else {
        return Err(invalid(&format!(
            "`deed_review` needs a `{name}` argument, as a non-empty array of strings"
        )));
    };
    let Some(items) = value.as_array() else {
        return Err(invalid(&format!(
            "`deed_review` needs `{name}` as a non-empty array of strings"
        )));
    };
    if items.is_empty() {
        return Err(invalid(&format!(
            "`deed_review` needs at least one `{name}` module"
        )));
    }
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_str().ok_or_else(|| {
                invalid(&format!(
                    "`deed_review` `{name}` module {} is not a string",
                    index + 1
                ))
            })
        })
        .collect()
}

fn policy_of(arguments: &Json) -> Result<(ReviewPolicy, bool), Failure> {
    let Some(raw) = arguments.get("policy") else {
        return Ok((ReviewPolicy::default(), false));
    };
    let Json::Object(fields) = raw else {
        return Err(invalid("`deed_review` needs `policy` as an object"));
    };

    let mut policy = ReviewPolicy::default();
    for (name, value) in fields {
        let Json::Bool(enabled) = value else {
            return Err(invalid(&format!(
                "`deed_review` policy `{name}` must be a boolean"
            )));
        };
        match name.as_str() {
            "denyNewAuthority" => policy.deny_new_authority = *enabled,
            "denyWeakerPromises" => policy.deny_weaker_promises = *enabled,
            "denyNewGuarded" => policy.deny_new_guarded = *enabled,
            _ => {
                return Err(invalid(&format!(
                    "`deed_review` policy has no `{name}` rule"
                )));
            }
        }
    }
    Ok((policy, true))
}

fn invalid(message: &str) -> Failure {
    Failure {
        code: INVALID_PARAMS,
        message: message.to_string(),
    }
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
