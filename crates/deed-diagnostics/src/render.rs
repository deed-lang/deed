//! Renderings of a [`Diagnostic`].
//!
//! Both renderers read the same struct. Neither is derived from the other, and
//! in particular the machine readable form is not produced by parsing the human
//! one. That is the whole point of P7: the data is the source of truth and the
//! text is a view.

use crate::diagnostic::{Diagnostic, Label, SuggestedEdit};
use crate::source::{SourceFile, SourceMap};
use crate::span::Span;

/// Renders a diagnostic for a person, with the offending line and an underline.
pub fn render_human(map: &SourceMap, diagnostic: &Diagnostic) -> String {
    let file = map.file(diagnostic.file);
    let primary = file.location(diagnostic.primary.span.start);

    let widest_line = std::iter::once(primary.line)
        .chain(
            diagnostic
                .secondary
                .iter()
                .map(|label| file.location(label.span.start).line),
        )
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
        push_snippet(&mut out, file, label, '-', gutter);
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
        &json_label(file, &diagnostic.primary),
        false,
    );

    let secondary: Vec<String> = diagnostic
        .secondary
        .iter()
        .map(|label| json_label(file, label))
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

fn json_label(file: &SourceFile, label: &Label) -> String {
    format!(
        "{{\"span\":{},\"message\":{}}}",
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
}
