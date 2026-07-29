//! Renderings of a [`Diagnostic`].
//!
//! Both renderers read the same struct. Neither is derived from the other, and
//! in particular the machine readable form is not produced by parsing the human
//! one. That is the whole point of P7: the data is the source of truth and the
//! text is a view.

use crate::diagnostic::{Diagnostic, Label, SuggestedEdit};
use crate::source::{FileId, SourceFile, SourceMap};
use crate::span::Span;

/// Renders a diagnostic for a person, with the offending line and an underline.
pub fn render_human(map: &SourceMap, diagnostic: &Diagnostic) -> String {
    let file = map.file(diagnostic.file);
    let primary = file.location(diagnostic.primary.span.start);

    // The gutter is wide enough for every line number that will be printed,
    // whichever file each of them came out of. One width for the whole
    // diagnostic, because two of them would make the carets stop lining up
    // down the page.
    let widest_line = std::iter::once(primary.line)
        .chain(diagnostic.secondary.iter().map(|label| {
            map.file(label.file_or(diagnostic.file))
                .location(label.span.start)
                .line
        }))
        .max()
        .unwrap_or(1);
    let gutter = widest_line.to_string().len();

    let mut out = String::new();
    out.push_str(&format!(
        "{}[{}]: {}\n",
        diagnostic.severity.as_str(),
        diagnostic.code,
        diagnostic.message
    ));
    out.push_str(&format!(
        "{:gutter$}--> {}:{}:{}\n",
        "",
        file.name(),
        primary.line,
        primary.column,
        gutter = gutter + 1
    ));

    push_snippet(&mut out, file, &diagnostic.primary, '^', gutter);
    for label in &diagnostic.secondary {
        let label_file = map.file(label.file_or(diagnostic.file));
        // A label about another file says so. Without this the caret moves
        // files and the reader is told nothing, which is worse than the label
        // being missing: they would read it as another line of the file above.
        if label.file.is_some_and(|other| other != diagnostic.file) {
            let at = label_file.location(label.span.start);
            out.push_str(&format!(
                "{:gutter$} |\n{:gutter$}--> {}:{}:{}\n",
                "",
                "",
                label_file.name(),
                at.line,
                at.column,
                gutter = gutter + 1
            ));
        }
        push_snippet(&mut out, label_file, label, '-', gutter);
    }

    if !diagnostic.notes.is_empty() || diagnostic.fix.is_some() {
        out.push_str(&format!("{:gutter$} |\n", "", gutter = gutter + 1));
    }
    for note in &diagnostic.notes {
        out.push_str(&format!(
            "{:gutter$} = note: {note}\n",
            "",
            gutter = gutter + 1
        ));
    }
    if let Some(fix) = &diagnostic.fix {
        out.push_str(&format!("help: {}\n", fix.message));
        match fix.edits.as_slice() {
            // One replacement is the text that goes in, and reads as itself.
            [edit] => {
                if !edit.replacement.is_empty() {
                    out.push_str(&format!(
                        "{:gutter$} | {}\n",
                        "",
                        edit.replacement,
                        gutter = gutter + 1
                    ));
                }
            }
            // Several only mean anything together and in the place they go. A
            // `to_string(` and a `)` on two lines of their own say nothing
            // about the line they were going to make, so show the line.
            edits => {
                if let Some(line) = rewritten_line(file, edits) {
                    out.push_str(&format!("{:gutter$} | {}\n", "", line, gutter = gutter + 1));
                }
            }
        }
    }

    out
}

/// The line the edits fall on, with all of them applied.
///
/// `None` when they do not all land on one line, which is a repair with no
/// single line to show rather than a repair to be shown badly.
fn rewritten_line(file: &SourceFile, edits: &[SuggestedEdit]) -> Option<String> {
    let text = file.text();
    let first = edits.first()?.span.start as usize;
    let last = edits.last()?.span.end as usize;
    if first > last || last > text.len() {
        return None;
    }

    let start = text[..first].rfind('\n').map_or(0, |at| at + 1);
    let end = text[last..].find('\n').map_or(text.len(), |at| last + at);
    if text[start..end].contains('\n') {
        return None;
    }

    let mut line = text[start..end].to_string();
    for edit in edits.iter().rev() {
        let from = (edit.span.start as usize).checked_sub(start)?;
        let to = (edit.span.end as usize).checked_sub(start)?;
        if from > to
            || to > line.len()
            || !line.is_char_boundary(from)
            || !line.is_char_boundary(to)
        {
            return None;
        }
        line.replace_range(from..to, &edit.replacement);
    }
    Some(line.trim_start().to_string())
}

fn push_snippet(out: &mut String, file: &SourceFile, label: &Label, caret: char, gutter: usize) {
    let location = file.location(label.span.start);
    let line_text = file.line_text(location.line);
    let width = underline_width(file, label.span);

    out.push_str(&format!("{:gutter$} |\n", "", gutter = gutter + 1));
    out.push_str(&format!(
        "{:>gutter$} | {}\n",
        location.line,
        line_text,
        gutter = gutter + 1
    ));
    out.push_str(&format!(
        "{:gutter$} | {}{} {}\n",
        "",
        " ".repeat(location.column.saturating_sub(1) as usize),
        caret.to_string().repeat(width),
        label.message,
        gutter = gutter + 1
    ));
}

/// Number of carets to draw, clamped to the first line the span touches.
fn underline_width(file: &SourceFile, span: Span) -> usize {
    let text = file.slice(span);
    let first_line = text.split(['\n', '\r']).next().unwrap_or("");
    first_line.chars().count().max(1)
}

/// Renders a diagnostic as a single line of JSON, for tools.
///
/// Written by hand so the compiler has no dependencies while it is this small.
/// If the shape grows past what is comfortable here, reach for a real
/// serializer rather than making this cleverer.
pub fn render_json(map: &SourceMap, diagnostic: &Diagnostic) -> String {
    let file = map.file(diagnostic.file);
    let mut out = String::from("{");

    push_field(&mut out, "code", &json_string(diagnostic.code), true);
    push_field(
        &mut out,
        "severity",
        &json_string(diagnostic.severity.as_str()),
        false,
    );
    push_field(
        &mut out,
        "message",
        &json_string(&diagnostic.message),
        false,
    );
    push_field(&mut out, "file", &json_string(file.name()), false);
    push_field(
        &mut out,
        "primary",
        &json_label(map, diagnostic.file, &diagnostic.primary),
        false,
    );

    let secondary: Vec<String> = diagnostic
        .secondary
        .iter()
        .map(|label| json_label(map, diagnostic.file, label))
        .collect();
    push_field(&mut out, "secondary", &json_array(&secondary), false);

    let notes: Vec<String> = diagnostic.notes.iter().map(|n| json_string(n)).collect();
    push_field(&mut out, "notes", &json_array(&notes), false);

    let fix = match &diagnostic.fix {
        None => "null".to_string(),
        Some(fix) => {
            let edits: Vec<String> = fix
                .edits
                .iter()
                .map(|edit| {
                    format!(
                        "{{\"span\":{},\"replacement\":{}}}",
                        json_span(file, edit.span),
                        json_string(&edit.replacement)
                    )
                })
                .collect();
            format!(
                "{{\"message\":{},\"applicability\":{},\"edits\":{}}}",
                json_string(&fix.message),
                json_string(fix.applicability.as_str()),
                json_array(&edits)
            )
        }
    };
    push_field(&mut out, "fix", &fix, false);

    out.push('}');
    out
}

fn push_field(out: &mut String, name: &str, value: &str, first: bool) {
    if !first {
        out.push(',');
    }
    out.push_str(&format!("\"{name}\":{value}"));
}

/// One label, with the file its span is an offset into.
///
/// The name is on every label rather than only on the ones that differ from
/// the diagnostic's. A reader of this output would otherwise have to know the
/// rule to work out what a missing key meant, and P7 says this form is the API
/// rather than a view of the other one.
fn json_label(map: &SourceMap, diagnostic: FileId, label: &Label) -> String {
    let file = map.file(label.file_or(diagnostic));
    format!(
        "{{\"file\":{},\"span\":{},\"message\":{}}}",
        json_string(file.name()),
        json_span(file, label.span),
        json_string(&label.message)
    )
}

fn json_span(file: &SourceFile, span: Span) -> String {
    let start = file.location(span.start);
    let end = file.location(span.end);
    format!(
        "{{\"start\":{},\"end\":{},\"startLine\":{},\"startColumn\":{},\"endLine\":{},\"endColumn\":{}}}",
        span.start, span.end, start.line, start.column, end.line, end.column
    )
}

fn json_array(items: &[String]) -> String {
    format!("[{}]", items.join(","))
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::{render_human, render_json};
    use crate::diagnostic::{Applicability, Diagnostic, SuggestedEdit};
    use crate::source::SourceMap;
    use crate::span::Span;

    #[test]
    fn human_output_points_at_the_right_column() {
        let source = "module a\nlet x = 1\n";
        let mut map = SourceMap::new();
        let file = map.add("t.deed", source);
        let span = Span::new(
            source.find('x').unwrap() as u32,
            source.find('x').unwrap() as u32 + 1,
        );
        let d =
            Diagnostic::error("DEED9001", file, span, "example problem").with_primary_label("here");

        let text = render_human(&map, &d);
        assert!(text.starts_with("error[DEED9001]: example problem\n"));
        assert!(text.contains("--> t.deed:2:5"));
        assert!(text.contains("2 | let x = 1"));
        assert!(text.contains("    ^ here"));
    }

    #[test]
    fn underline_stops_at_the_end_of_the_line() {
        let source = "\"abc\ndef\n";
        let mut map = SourceMap::new();
        let file = map.add("t.deed", source);
        let d = Diagnostic::error("DEED9002", file, Span::new(0, 8), "spans two lines");

        let text = render_human(&map, &d);
        assert!(text.contains("^^^^ "), "unexpected rendering:\n{text}");
    }

    #[test]
    fn json_escapes_and_carries_locations() {
        let source = "let s = \"x\n";
        let mut map = SourceMap::new();
        let file = map.add("t.deed", source);
        let d = Diagnostic::error(
            "DEED9003",
            file,
            Span::new(8, 10),
            "unterminated \"string\"",
        )
        .with_note("line one\nline two")
        .with_fix(
            "close it",
            Span::new(10, 10),
            "\"",
            Applicability::MachineApplicable,
        );

        let json = render_json(&map, &d);
        assert!(json.contains("\"code\":\"DEED9003\""));
        assert!(json.contains("unterminated \\\"string\\\""));
        assert!(json.contains("line one\\nline two"));
        assert!(json.contains("\"applicability\":\"machine-applicable\""));
        assert!(json.contains("\"startLine\":1,\"startColumn\":9"));
    }

    #[test]
    fn json_fix_is_null_when_absent() {
        let mut map = SourceMap::new();
        let file = map.add("t.deed", "abc");
        let d = Diagnostic::error("DEED9004", file, Span::new(0, 1), "no fix");
        assert!(render_json(&map, &d).contains("\"fix\":null"));
    }

    /// One replacement is the text that goes in and reads as itself. Two of
    /// them are `to_string(` and `)`, which on lines of their own say nothing
    /// about the line they were going to make.
    #[test]
    fn a_fix_that_wraps_something_is_shown_as_the_line_it_makes() {
        let source = "fn f() -> String {\n    n as String\n}\n";
        let mut map = SourceMap::new();
        let file = map.add("t.deed", source);
        let d = Diagnostic::error("DEED9005", file, Span::new(25, 34), "no cast").with_edits(
            "call `to_string`",
            vec![
                SuggestedEdit {
                    span: Span::at(23),
                    replacement: "to_string(".to_string(),
                },
                SuggestedEdit {
                    span: Span::new(24, 34),
                    replacement: ")".to_string(),
                },
            ],
            Applicability::MachineApplicable,
        );

        let text = render_human(&map, &d);
        assert!(text.contains("help: call `to_string`"), "{text}");
        assert!(text.contains("| to_string(n)\n"), "{text}");
        assert!(!text.contains("| )\n"), "{text}");
    }

    /// The line above `line`, so a test can say "there is a gap here" rather
    /// than count spaces.
    fn line_before<'a>(text: &'a str, line: &str) -> &'a str {
        let lines: Vec<&str> = text.lines().collect();
        let at = lines
            .iter()
            .position(|written| written.trim_start().starts_with(line))
            .unwrap_or_else(|| panic!("{line:?} should be in:\n{text}"));
        assert!(at > 0, "{line:?} is the first line, so nothing is above it");
        lines[at - 1]
    }

    /// The blank gutter line between the code and what is said about it.
    ///
    /// `cargo mutants` found this one: with the separator gone a note runs
    /// straight on from the caret line and nothing noticed. Both halves of
    /// the condition get a test, because a note with no fix and a fix with no
    /// note are both ordinary and either one alone leaves the other unheld.
    #[test]
    fn a_note_is_separated_from_the_code_it_is_about() {
        let mut map = SourceMap::new();
        let file = map.add("t.deed", "module a\nlet x = 1\n");
        let d = Diagnostic::error("DEED9006", file, Span::new(13, 14), "example problem")
            .with_primary_label("here")
            .with_note("something worth saying");

        let text = render_human(&map, &d);
        assert_eq!(
            line_before(&text, "= note:").trim(),
            "|",
            "the note should not run straight on from the caret:\n{text}"
        );
    }

    #[test]
    fn a_fix_with_no_note_is_separated_the_same_way() {
        let mut map = SourceMap::new();
        let file = map.add("t.deed", "module a\nlet x = 1\n");
        let d = Diagnostic::error("DEED9007", file, Span::new(13, 14), "example problem")
            .with_primary_label("here")
            .with_fix(
                "call it something else",
                Span::new(13, 14),
                "y",
                Applicability::MachineApplicable,
            );

        let text = render_human(&map, &d);
        assert!(!text.contains("= note:"), "{text}");
        assert_eq!(
            line_before(&text, "help:").trim(),
            "|",
            "the help should not run straight on from the caret:\n{text}"
        );
    }

    #[test]
    fn a_fix_that_only_deletes_offers_no_line_to_read() {
        // The replacement is what a reader is shown, and an empty one is a
        // gutter with nothing after it. Saying "here is what it would look
        // like" and then showing a blank line is worse than saying nothing.
        let mut map = SourceMap::new();
        let file = map.add("t.deed", "module a\nlet x = 1\n");
        let d = Diagnostic::error("DEED9008", file, Span::new(13, 14), "example problem")
            .with_primary_label("here")
            .with_fix(
                "take it out",
                Span::new(13, 14),
                "",
                Applicability::MachineApplicable,
            );

        let text = render_human(&map, &d);
        assert!(text.contains("help: take it out"), "{text}");
        assert_eq!(
            text.lines().last(),
            Some("help: take it out"),
            "nothing should be offered after the help line:\n{text}"
        );
    }

    /// Two files, so that a wrong answer cannot come out looking right.
    fn two_files() -> (SourceMap, crate::source::FileId, crate::source::FileId) {
        let mut map = SourceMap::new();
        let caller = map.add("caller.deed", "module a\n\nfn f() -> Int {\n    g(1)\n}\n");
        let callee = map.add(
            "callee.deed",
            "module b\n\n// a longer file, on purpose\n\nfn g(n: Int) -> Int\n  where\n    n > 1,\n{\n    n\n}\n",
        );
        (map, caller, callee)
    }

    /// Where a piece of text is, so a test says what it means rather than a
    /// pair of byte offsets nobody can check by reading.
    fn spanning(map: &SourceMap, file: crate::source::FileId, text: &str) -> Span {
        let at = map
            .file(file)
            .text()
            .find(text)
            .unwrap_or_else(|| panic!("{text:?} should be in {}", map.file(file).name()))
            as u32;
        Span::new(at, at + text.len() as u32)
    }

    #[test]
    fn a_label_about_another_file_says_which_file_it_is_about() {
        // Before a label could carry a file, this label was either dropped or
        // drawn over whatever sat at those byte offsets in the file above it.
        // The header is the part that matters: without it the caret changes
        // files and a reader is told nothing, so they read it as another line
        // of the first one.
        let (map, caller, callee) = two_files();
        let d = Diagnostic::error(
            "DEED9006",
            caller,
            spanning(&map, caller, "g(1)"),
            "the call is wrong",
        )
        .with_primary_label("here")
        .with_secondary_in(
            callee,
            spanning(&map, callee, "n > 1"),
            "the clause it does not satisfy",
        );

        let text = render_human(&map, &d);
        assert!(text.contains("--> caller.deed:4:5"), "{text}");
        assert!(text.contains("--> callee.deed:7:5"), "{text}");
        assert!(
            text.contains("----- the clause it does not satisfy"),
            "{text}"
        );
        // Drawn from the other file's text, not from the first one's bytes.
        assert!(text.contains("7 |     n > 1,"), "{text}");
    }

    #[test]
    fn a_label_about_the_diagnostics_own_file_says_nothing_extra() {
        // The other 23 of the 31 places this compiler builds a secondary
        // label. Adding a file to `Label` may not add a line to any of them.
        let (map, caller, _) = two_files();
        let d = Diagnostic::error(
            "DEED9007",
            caller,
            spanning(&map, caller, "g(1)"),
            "the call is wrong",
        )
        .with_primary_label("here")
        .with_secondary(spanning(&map, caller, "fn f()"), "the function it is in");

        let text = render_human(&map, &d);
        assert_eq!(text.matches("-->").count(), 1, "{text}");
    }

    #[test]
    fn passing_the_diagnostics_own_file_reads_the_same_as_not_passing_one() {
        // `with_secondary_in` is for a producer that has a file in hand and
        // cannot always tell whether it is the same one. Making it say
        // something different when the two agree would push that decision back
        // onto every caller, which is the arrangement this replaced.
        let (map, caller, _) = two_files();
        let at = spanning(&map, caller, "g(1)");
        let there = spanning(&map, caller, "fn f()");
        let plain =
            Diagnostic::error("DEED9008", caller, at, "problem").with_secondary(there, "there");
        let spelled = Diagnostic::error("DEED9008", caller, at, "problem")
            .with_secondary_in(caller, there, "there");

        assert_eq!(render_human(&map, &plain), render_human(&map, &spelled));
        assert_eq!(render_json(&map, &plain), render_json(&map, &spelled));
    }

    #[test]
    fn json_says_which_file_every_label_is_about() {
        // P7: this form is the API rather than a view of the human one. A
        // reader of it cannot apply the rule that a missing key means the
        // diagnostic's own file unless they already know the rule, so the name
        // is on every label including the primary.
        let (map, caller, callee) = two_files();
        let d = Diagnostic::error(
            "DEED9009",
            caller,
            spanning(&map, caller, "g(1)"),
            "the call is wrong",
        )
        .with_secondary_in(
            callee,
            spanning(&map, callee, "n > 1"),
            "the clause it does not satisfy",
        );

        let json = render_json(&map, &d);
        assert!(
            json.contains("\"primary\":{\"file\":\"caller.deed\""),
            "{json}"
        );
        assert!(
            json.contains("\"secondary\":[{\"file\":\"callee.deed\""),
            "{json}"
        );
    }
}
