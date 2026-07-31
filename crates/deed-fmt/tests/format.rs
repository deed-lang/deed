//! The formatter.
//!
//! A formatter is a pile of layout decisions, and testing each one by hand
//! tests the decisions I thought of, which is not the set that matters. So the
//! decisions are data here: `DECISIONS` holds one row per decision, a row is a
//! name and one or more spellings and what the output has to look like, and
//! every `#[test]` below walks a table. Adding a decision is adding a row.
//!
//! One test is not a row and cannot be. `a_file_that_does_not_parse_is_refused`
//! hands `format` something that does not parse, and a row is a spelling that
//! formats, so there is no output for it to want.

use deed_diagnostics::SourceMap;
use deed_fmt::format;

fn fmt(source: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", source);
    match format(file, source) {
        Ok(formatted) => formatted,
        Err(diagnostics) => panic!(
            "should have parsed: {}",
            diagnostics
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

// -- the decisions ---------------------------------------------------------

/// One layout decision, named.
///
/// The name is what a failure says. A table of anonymous input and expected
/// pairs would be worse than the pile of functions it replaces, because a
/// failing row has to say which decision broke rather than which index did.
struct Decision {
    /// The decision, in the failure message.
    name: &'static str,
    /// Spellings of one program that the decision has something to say about.
    ///
    /// More than one means the decision holds however the program was written,
    /// and that all of them land in the same place.
    spellings: &'static [&'static str],
    /// What formatting has to produce. Every want is checked against every
    /// spelling.
    wants: &'static [Want],
    /// Whether formatting is allowed to come back with different tokens.
    tokens: Tokens,
}

/// Something the formatted output has to look like.
enum Want {
    /// The output contains this, verbatim.
    Says(&'static str),
    /// The output does not contain this.
    Avoids(&'static str),
    /// The first of these turns up before the last of the second.
    Before(&'static str, &'static str),
}

/// Whether a decision moves text around or writes different tokens.
///
/// Formatting a program normally leaves it with the tokens it had. The
/// exceptions are the two things that are a spelling rather than a program: a
/// parenthesis precedence makes redundant, and the comma before a closing
/// bracket, which is there when the list is broken over lines and gone when it
/// is not. A row that turns on one of those says so, and the token property
/// then asserts the tokens really do change rather than skipping the row.
#[derive(Clone, Copy)]
enum Tokens {
    Kept,
    Rewritten,
}

const DECISIONS: &[Decision] = &[
    // -- parentheses -------------------------------------------------------
    //
    // The tree carries no parentheses, so these have to be reconstructed. The
    // ones that are kept change the answer if they are dropped, and the ones
    // that are dropped are noise if they are kept.
    Decision {
        name: "a sum inside a product keeps its parentheses",
        spellings: &["module a\n\nfn f() -> Int { (a + b) * c }\n"],
        wants: &[Want::Says("(a + b) * c")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "a product inside a sum needs none",
        spellings: &["module a\n\nfn f() -> Int { a + b * c }\n"],
        wants: &[Want::Says("a + b * c")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "a difference on the right of a difference keeps its parentheses",
        spellings: &["module a\n\nfn f() -> Int { a - (b - c) }\n"],
        wants: &[Want::Says("a - (b - c)")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "a difference on the left of a difference needs none",
        spellings: &["module a\n\nfn f() -> Int { (a - b) - c }\n"],
        wants: &[Want::Says("a - b - c")],
        tokens: Tokens::Rewritten,
    },
    Decision {
        name: "an `||` inside an `&&` keeps its parentheses",
        spellings: &["module a\n\nfn f() -> Int { (a || b) && c }\n"],
        wants: &[Want::Says("(a || b) && c")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "an `&&` inside an `||` needs none",
        spellings: &["module a\n\nfn f() -> Int { a || b && c }\n"],
        wants: &[Want::Says("a || b && c")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "an `&&` under a `!` keeps its parentheses",
        spellings: &["module a\n\nfn f() -> Int { !(a && b) }\n"],
        wants: &[Want::Says("!(a && b)")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "a sum whose field is read keeps its parentheses",
        spellings: &["module a\n\nfn f() -> Int { (a + b).field }\n"],
        wants: &[Want::Says("(a + b).field")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "parentheses that change nothing are dropped",
        spellings: &["module a\n\nfn f() -> Int { ((a + b)) }\n"],
        wants: &[Want::Says("    a + b\n")],
        tokens: Tokens::Rewritten,
    },
    // -- the shape of things -----------------------------------------------
    Decision {
        name: "the layout that was there does not survive",
        spellings: &[
            "module a\nfn   f( n:Int )->Int{n+n}\n",
            "module a\n\n\n\nfn f(\n  n: Int\n)\n->\nInt\n{\n\n\n    n   +   n\n\n}\n",
        ],
        wants: &[Want::Says("fn f(n: Int) -> Int {\n    n + n\n}\n")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "a module edition stays on the module line",
        spellings: &["module a edition 2025\nfn f() -> Int { 0 }\n"],
        wants: &[Want::Says(
            "module a edition 2025\n\nfn f() -> Int {\n    0\n}\n",
        )],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "a deprecation declaration is spaced canonically",
        spellings: &[
            "module a\n\ndeprecated  legacy  ->replacement\nfn legacy() -> Int { replacement() }\n",
        ],
        wants: &[Want::Says("deprecated legacy -> replacement\n")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "`ok` is padded so the arrows line up",
        spellings: &[
            "module a\n\nfn f(n: Int) -> Int\n  ensures\n    ok => result > 0,\n{\n  n\n}\n",
        ],
        wants: &[Want::Says("    ok  => result > 0,")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "a contract puts the brace back on the margin",
        spellings: &["module a\n\nfn f() -> Int uses E.op, { 1 }\n"],
        wants: &[Want::Says("    E.op,\n{\n")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "an empty body stays on one line",
        spellings: &["module a\n\nfn f() -> () {\n}\n"],
        wants: &[Want::Says("fn f() -> () {}")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "a long call breaks one argument per line",
        spellings: &[concat!(
            "module a\n\nfn f() -> Int { g(",
            "aaaaaaaaaaaaaaaaaaaa, aaaaaaaaaaaaaaaaaaaa, aaaaaaaaaaaaaaaaaaaa, ",
            "aaaaaaaaaaaaaaaaaaaa, aaaaaaaaaaaaaaaaaaaa) }\n"
        )],
        wants: &[
            Want::Says("g(\n"),
            Want::Says("        aaaaaaaaaaaaaaaaaaaa,\n"),
        ],
        // Breaking the call puts a comma after the last argument.
        tokens: Tokens::Rewritten,
    },
    Decision {
        name: "a short list stays on one line",
        spellings: &["module a\n\nfn f() -> List<Int> {\n[1,\n2,\n3,]\n}\n"],
        wants: &[Want::Says("    [1, 2, 3]\n")],
        // Joining the list takes the comma before the `]` away again.
        tokens: Tokens::Rewritten,
    },
    Decision {
        name: "a long list breaks one element per line",
        spellings: &[concat!(
            "module a\n\nfn f() -> Int { [",
            "aaaaaaaaaaaaaaaaaaaa, aaaaaaaaaaaaaaaaaaaa, aaaaaaaaaaaaaaaaaaaa, ",
            "aaaaaaaaaaaaaaaaaaaa] }\n"
        )],
        wants: &[
            Want::Says("[\n"),
            Want::Says("        aaaaaaaaaaaaaaaaaaaa,\n"),
        ],
        // Breaking the list puts a comma after the last element.
        tokens: Tokens::Rewritten,
    },
    Decision {
        name: "a `for` keeps its head on one line",
        spellings: &["module a\n\nfn f() -> Int {\nfor n in ns with sum=0 {sum+n}\n}\n"],
        wants: &[Want::Says("    for n in ns with sum = 0 {\n")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "a `for` that says where it is keeps that in the head",
        spellings: &["module a\n\nfn f() -> Int {\nfor n at i in ns with sum=0 {sum+i}\n}\n"],
        wants: &[Want::Says("    for n at i in ns with sum = 0 {\n")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "a `for` that stops early keeps that in the head too",
        spellings: &[
            "module a\n\nfn f() -> Bool {\nfor n in ns with hit=false while !hit {n>2}\n}\n",
        ],
        wants: &[Want::Says(
            "    for n in ns with hit = false while !hit {\n",
        )],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "a trailing comment stays on its line",
        spellings: &["module a\n\nfn f() -> Int {\n    let x = 1 // why\n    x\n}\n"],
        wants: &[Want::Says("let x = 1 // why")],
        tokens: Tokens::Kept,
    },
    Decision {
        name: "at most one blank line survives",
        spellings: &["module a\n\nfn f() -> Int {\n    let x = 1\n\n\n\n\n    x\n}\n"],
        wants: &[Want::Says("let x = 1\n\n"), Want::Avoids("\n\n\n")],
        tokens: Tokens::Kept,
    },
    Decision {
        // This is the one that was wrong first. Blocks inside an expression
        // are rendered into their own buffer, and a comment written straight
        // to the output surfaced after the enclosing statement instead, which
        // is why the second want is about where it is and not only that it is.
        name: "a comment inside a nested block stays there",
        spellings: &[concat!(
            "module a\n\ntest \"t\" {\n    with H { s: 0 } {\n",
            "        // inside\n        assert true\n    }\n}\n"
        )],
        wants: &[
            Want::Says("        // inside\n"),
            Want::Before("// inside", "}"),
        ],
        tokens: Tokens::Kept,
    },
];

// -- the properties --------------------------------------------------------

#[test]
fn every_layout_decision_holds() {
    // Every row is checked before anything fails, because one change to the
    // printer can break several decisions and the useful thing to be told is
    // all of them rather than whichever comes first in the table.
    let mut broke: Vec<String> = Vec::new();
    let mut checked = 0;

    for decision in DECISIONS {
        assert!(
            !decision.spellings.is_empty(),
            "`{}` is a decision about nothing",
            decision.name
        );
        assert!(
            !decision.wants.is_empty(),
            "`{}` wants nothing, so it holds whatever the formatter does",
            decision.name
        );

        for spelling in decision.spellings {
            let formatted = fmt(spelling);
            for want in decision.wants {
                let name = decision.name;
                match want {
                    Want::Says(text) => {
                        if !formatted.contains(text) {
                            broke.push(format!("`{name}` wanted `{text}` and got:\n{formatted}"));
                        }
                    }
                    Want::Avoids(text) => {
                        if formatted.contains(text) {
                            broke.push(format!(
                                "`{name}` did not want `{text}` and got:\n{formatted}"
                            ));
                        }
                    }
                    Want::Before(first, last) => {
                        match (formatted.find(first), formatted.rfind(last)) {
                            (Some(at), Some(end)) if at < end => {}
                            (None, _) => {
                                broke.push(format!("`{name}` lost `{first}` from:\n{formatted}"))
                            }
                            (_, None) => {
                                broke.push(format!("`{name}` lost `{last}` from:\n{formatted}"))
                            }
                            _ => broke.push(format!(
                                "`{name}` wanted `{first}` before `{last}` and got:\n{formatted}"
                            )),
                        }
                    }
                }
                checked += 1;
            }
        }
    }

    // The loop above is the only thing that reads `wants`, so an empty table
    // or a table of empty rows would pass it without formatting anything.
    assert_eq!(
        checked,
        DECISIONS
            .iter()
            .map(|d| d.spellings.len() * d.wants.len())
            .sum::<usize>(),
        "a row was skipped"
    );
    assert!(checked > 0, "no decision was checked");
    assert!(broke.is_empty(), "\n{}", broke.join("\n"));
}

#[test]
fn every_decision_is_a_decision_of_its_own() {
    // Two rows under one name is a name that cannot say which one broke, and
    // saying which one broke is the whole reason the rows are named.
    let mut names: Vec<&str> = DECISIONS.iter().map(|d| d.name).collect();
    let before = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(before, names.len(), "two decisions share a name");
    assert!(before > 0, "there are no decisions");
}

#[test]
fn every_spelling_of_a_decision_lands_in_the_same_place() {
    let mut compared = 0;
    for decision in DECISIONS {
        let mut spellings = decision.spellings.iter();
        let Some(first) = spellings.next() else {
            continue;
        };
        let landed = fmt(first);
        for other in spellings {
            assert_eq!(fmt(other), landed, "`{}` came out two ways", decision.name);
            compared += 1;
        }
    }

    // Most rows have one spelling, so this loop compares nothing most of the
    // time and would compare nothing at all if the row that carries two ever
    // lost one.
    assert!(compared > 0, "no two spellings were compared");
}

#[test]
fn formatting_is_idempotent() {
    for (name, source) in inputs() {
        let once = fmt(source);
        let twice = fmt(&once);
        assert_eq!(once, twice, "`{name}` changed on a second pass:\n{once}");
    }
}

#[test]
fn formatting_does_not_change_what_the_program_means() {
    // The tree after formatting has to be the tree before it. A formatter that
    // reshapes a program is worse than no formatter, and the place this goes
    // wrong is parentheses, which the tree does not record. The comparison is
    // over tokens, so the rows that change one on purpose are asserted from
    // the other side rather than skipped.
    for (index, source) in SOURCES.iter().enumerate() {
        let formatted = fmt(source);
        assert_eq!(
            shape(source),
            shape(&formatted),
            "source {index} changed the tree:\n{formatted}"
        );
    }

    let mut rewritten = 0;
    for decision in DECISIONS {
        for spelling in decision.spellings {
            let formatted = fmt(spelling);
            match decision.tokens {
                Tokens::Kept => assert_eq!(
                    shape(spelling),
                    shape(&formatted),
                    "`{}` changed the tree:\n{formatted}",
                    decision.name
                ),
                Tokens::Rewritten => {
                    assert_ne!(
                        shape(spelling),
                        shape(&formatted),
                        "`{}` says a token changes and none did:\n{formatted}",
                        decision.name
                    );
                    rewritten += 1;
                }
            }
        }
    }
    assert!(
        rewritten > 0,
        "no decision claims to rewrite a token, so the exception is unused"
    );
}

#[test]
fn every_comment_survives() {
    let mut seen = 0;
    for (name, source) in inputs() {
        let formatted = fmt(source);
        for comment in comments(source) {
            assert!(
                formatted.contains(&comment),
                "`{name}` lost `{comment}` from:\n{formatted}"
            );
            seen += 1;
        }
    }

    // Only a few of these have comments in them, so the loop above is empty
    // most of the time. If `comments` ever stops finding them, every iteration
    // is empty and nothing here notices.
    assert!(seen > 0, "no comment was found to survive anything");
}

#[test]
fn output_ends_with_exactly_one_newline() {
    for (name, source) in inputs() {
        let formatted = fmt(source);
        assert!(
            formatted.ends_with('\n'),
            "`{name}` has no trailing newline"
        );
        assert!(
            !formatted.ends_with("\n\n"),
            "`{name}` has more than one trailing newline"
        );
    }
}

#[test]
fn no_line_has_trailing_whitespace() {
    for (name, source) in inputs() {
        for line in fmt(source).lines() {
            assert_eq!(
                line,
                line.trim_end(),
                "`{name}` has trailing whitespace on `{line}`"
            );
        }
    }
}

#[test]
fn the_properties_are_handed_something_to_do() {
    // `format` is the identity function on input that is already canonical,
    // and a property run over canonical input is a property run over `==`.
    // That is the hole #207 found in `repository.rs`, where the corpus is
    // checked in canonically and formatting it proved nothing; the answer
    // there was to mangle it first. `SOURCES` is more than half in that state
    // on its own, so the spellings under the decisions are where the badly
    // laid out input lives, and this counts it.
    //
    // Pointing the properties at the corpus instead would not help. The
    // corpus is canonical, so every claim below about `format`'s output over
    // it is a claim about the checked-in files, and
    // `any_layout_of_a_real_program_formats_back_to_the_canonical_one` in
    // `repository.rs` already makes all of them at once and byte for byte,
    // from a layout that was thrown away first. What the properties here are
    // for is the spellings the corpus does not contain, because the corpus
    // cannot contain a badly laid out file.
    let inputs = inputs();
    let moved = inputs.iter().filter(|(_, s)| fmt(s) != *s).count();
    assert!(
        moved * 2 > inputs.len(),
        "only {moved} of {} inputs are laid out badly enough for formatting \
         to have anything to do",
        inputs.len()
    );
}

#[test]
fn generated_layouts_feed_the_formatter_properties() {
    let generated = generated_inputs();
    assert!(
        !generated.is_empty(),
        "no generated input was produced for the properties"
    );

    let mut idempotent = 0;
    let mut comments_kept = 0;
    let mut braces_checked = 0;
    let mut rewritten = 0;
    for sample in &generated {
        let once = fmt(&sample.generated);
        let twice = fmt(&once);
        assert_eq!(
            once, twice,
            "`{}` changed on a second pass after layout generation:\n{once}",
            sample.name
        );
        idempotent += 1;

        for comment in comments(sample.source) {
            assert!(
                once.contains(&comment),
                "`{}` lost `{comment}` from:\n{once}",
                sample.name
            );
            comments_kept += 1;
        }

        match sample.tokens {
            Tokens::Kept => assert_eq!(
                shape(sample.source),
                shape(&once),
                "`{}` changed the tree after layout generation:\n{once}",
                sample.name
            ),
            Tokens::Rewritten => {
                assert_ne!(
                    shape(sample.source),
                    shape(&once),
                    "`{}` says a token changes and none did:\n{once}",
                    sample.name
                );
                rewritten += 1;
            }
        }

        braces_checked += assert_closing_braces_align(&sample.name, &once);
    }
    assert!(idempotent > 0, "idempotence was not checked");
    assert!(
        comments_kept > 0,
        "no comment was checked on generated input"
    );
    assert!(
        braces_checked > 0,
        "no closing brace was checked on generated input"
    );
    assert!(
        rewritten > 0,
        "no known rewrite was exercised on generated input"
    );
}

// -- broken input ----------------------------------------------------------

#[test]
fn a_file_that_does_not_parse_is_refused() {
    let source = "module a\n\nfn f( -> Int {\n";
    let mut sources = SourceMap::new();
    let file = sources.add("bad.deed", source);
    assert!(
        format(file, source).is_err(),
        "reshaping a broken file is guessing at what was meant"
    );
}

// -- helpers ---------------------------------------------------------------

/// Every spelling this file has, with something to call it in a failure.
fn inputs() -> Vec<(String, &'static str)> {
    let mut all: Vec<(String, &'static str)> = SOURCES
        .iter()
        .enumerate()
        .map(|(index, source)| (format!("source {index}"), *source))
        .collect();

    for decision in DECISIONS {
        for spelling in decision.spellings {
            all.push((decision.name.to_string(), *spelling));
        }
    }
    all
}

/// Every source this file has, with deterministic badly laid out variants.
fn generated_inputs() -> Vec<GeneratedInput> {
    let mut all = Vec::new();

    for (index, source) in SOURCES.iter().enumerate() {
        for (variant, generated) in generated_layouts(source).into_iter().enumerate() {
            assert_ne!(
                generated, *source,
                "`source {index}` variant {variant} was not laid out differently"
            );
            all.push(GeneratedInput {
                name: format!("source {index} (generated {variant})"),
                source,
                generated,
                tokens: Tokens::Kept,
            });
        }
    }

    for decision in DECISIONS {
        for (index, spelling) in decision.spellings.iter().enumerate() {
            for (variant, generated) in generated_layouts(spelling).into_iter().enumerate() {
                assert_ne!(
                    generated, *spelling,
                    "`{}` spelling {index} variant {variant} was not laid out differently",
                    decision.name
                );
                all.push(GeneratedInput {
                    name: format!("{} (spelling {index}, generated {variant})", decision.name),
                    source: spelling,
                    generated,
                    tokens: decision.tokens,
                });
            }
        }
    }
    all
}

struct GeneratedInput {
    name: String,
    source: &'static str,
    generated: String,
    tokens: Tokens,
}

/// Deterministic variations of one source with deliberately bad layout.
fn generated_layouts(source: &str) -> Vec<String> {
    let mut made = Vec::new();

    for variant in 0..3 {
        let mut out = String::new();
        for (line, text) in source.lines().enumerate() {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                out.push('\n');
                if (line + variant) % 3 == 0 {
                    out.push('\n');
                }
                continue;
            }

            for _ in 0..(((line + 1) * (variant + 2)) % 6) * 2 {
                out.push(' ');
            }
            out.push_str(trimmed);
            for _ in 0..(variant + 1) * 2 {
                out.push(' ');
            }
            out.push('\n');
        }

        if out != source {
            made.push(out);
        }
    }

    made
}

/// A rough shape of the source, for comparing two spellings of one program.
///
/// Everything but the tokens themselves is dropped, which is enough to catch a
/// formatter that added, removed or reordered anything.
fn shape(source: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add("shape.deed", source);
    let lexed = deed_lexer::tokenize(file, source);
    lexed
        .tokens
        .iter()
        .map(|token| format!("{:?}", token.kind))
        .collect::<Vec<_>>()
        .join(" ")
}

fn comments(source: &str) -> Vec<String> {
    let mut sources = SourceMap::new();
    let file = sources.add("comments.deed", source);
    let lexed = deed_lexer::tokenize(file, source);
    lexed
        .trivia
        .iter()
        .map(|t| {
            source[t.span.start as usize..t.span.end as usize]
                .trim()
                .to_string()
        })
        .filter(|text| !text.contains('\n'))
        .collect()
}

/// A block's closing brace sits under the line that opened it.
fn assert_closing_braces_align(name: &str, source: &str) -> usize {
    let mut opened: Vec<(usize, usize)> = Vec::new();
    let mut checked = 0;

    for (number, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let mut leading = trimmed.starts_with('}');

        for brace in braces(line) {
            if brace == '{' {
                opened.push((number + 1, indent));
                leading = false;
                continue;
            }
            let (at, was) = opened
                .pop()
                .unwrap_or_else(|| panic!("`{name}`:{} closes a block nothing opened", number + 1));
            if leading {
                assert_eq!(
                    indent,
                    was,
                    "`{name}`:{} closes the block opened on line {at}, so it belongs at {was} \
                     spaces and is at {indent}",
                    number + 1
                );
                checked += 1;
            }
            leading = false;
        }
    }

    assert!(
        opened.is_empty(),
        "`{name}` leaves a block open at line {}",
        opened[0].0
    );

    checked
}

/// The braces on a line, with the ones inside text and comments left out.
fn braces(line: &str) -> Vec<char> {
    let mut found = Vec::new();
    let mut chars = line.chars().peekable();
    let mut in_string = false;

    while let Some(c) = chars.next() {
        match c {
            '\\' if in_string => {
                chars.next();
            }
            '"' => in_string = !in_string,
            '/' if !in_string && chars.peek() == Some(&'/') => break,
            '{' | '}' if !in_string => found.push(c),
            _ => {}
        }
    }
    found
}

/// Whole programs, for the properties that are about the output in general.
///
/// These say nothing about any one decision. They are here so that the
/// properties see every construct the language has rather than only the ones
/// some decision happened to need.
const SOURCES: &[&str] = &[
    "module a\n",
    "module a\n\nfn f() -> Int { 0 }\n",
    "// leading\nmodule a\n\n// about f\nfn f() -> Int {\n    // inside\n    let x = 1 // trailing\n\n    x\n}\n",
    "module a\n\nuse b/c.{D, e}\n\nrecord R {\n    a: Int,\n    b: String,\n}\n",
    "module a\n\nchoice C {\n    One,\n    Two { x: Int, y: Int },\n}\n",
    "module a\n\ntype P = Int where value > 0\n",
    "module a\n\neffect E {\n    fn op(n: Int) -> Int\n}\n\nhandler H implements E {\n    state s: Int\n\n    fn op(n) -> Int { s + n }\n}\n",
    "module a\n\nfn f(n: Int) -> Result<Int, String>\n  where\n    n > 0,\n  ensures\n    ok => result > 0,\n    err => true,\n{\n    if n > 10 {\n        return err(\"too big\")\n    }\n    ok(n)\n}\n",
    "module a\n\nfn f(v: C) -> Int {\n    match v {\n        One => 1,\n        Two { x, y } => x + y,\n    }\n}\n",
    "module a\n\nfn f() -> Int { g(1)? + h(2)? }\n",
    "module a\n\nfn f() -> List<Int> { [1, 2, 3] }\n",
    "module a\n\nfn f(ns: List<Int>) -> Int {\n    for n in ns with sum = 0 {\n        sum + n\n    }\n}\n",
    "module a\n\nfn f(ns: List<Int>) -> Int {\n    for n at i in ns with sum = 0 {\n        sum + n * i\n    }\n}\n",
    "module a\n\nfn f(ns: List<Int>) -> Bool {\n    for n in ns with hit = false while !hit {\n        n > 2\n    }\n}\n",
    "module a\n\nfn f(ns: List<Int>) -> () {\n    for n in ns {\n        g(n)\n    }\n}\n",
    "module a\n\nfn f() -> List<List<Int>> { [[], [1]] }\n",
    "module a\n\nfn f() -> Int { (1 + 2) * (3 - 4) / (5 % 6) }\n",
    "module a\n\nfn f(g: Fn(Int, String) -> Int) -> Int { g(1, \"x\") }\n",
    "module a\n\nfn first<T>(items: List<T>) -> Result<T, String> { at(items, 0) }\n",
    "module a\n\nfn apply<A, B>(f: Fn(A) -> B, value: A) -> B { f(value) }\n",
    "module a\n\neffect E {\n    fn op() -> ()\n}\n\nfn f(g: Fn(Int) uses E.op -> Int) -> Int\n  uses\n    E.op,\n{\n    g(1)\n}\n",
    "module a\n\ntest \"a test\" {\n    assert 1 == 1\n}\n",
    "module a\n\ntest \"a test\" {\n    assert refuses order_of(0)\n}\n",
    "module a\n\ntest \"t\" {\n    with H {} {\n        f()\n    }\n}\n",
    // A call that has to break with something inside it that breaks as well.
    // The arguments go a level further in than the line that could not hold
    // them, and everything they contain goes with them.
    "module a\n\nfn f(xs: List<Int>) -> List<Int> {\n    map(\n        xs,\n        |n: Int| {\n            let doubled = n + n\n            doubled + 1\n        },\n    )\n}\n",
    // A signature that wraps for a reason other than a contract. The brace
    // goes to the left margin either way, because what it wraps onto is
    // indented and a brace on the end of an indented line reads as part of it.
    "module a\n\nfn placed(sorted: List<Entry<String, Int>>, one: Entry<String, Int>)\n    -> List<Entry<String, Int>>\n{\n    sorted\n}\n",
];
