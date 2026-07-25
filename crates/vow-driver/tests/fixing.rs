//! Applying the fixes diagnostics carry.
//!
//! The interesting cases are the ones where applying a fix is the wrong thing:
//! a guess, an overlap, a span that does not name a range of the text. A tool
//! that rewrites source has to be more careful about when it declines than
//! about when it acts.

use vow_diagnostics::{Applicability, Diagnostic, SourceMap, Span};
use vow_driver::fix::{error_count, fix};

fn diagnose(text: &str) -> Vec<Diagnostic> {
    let mut sources = SourceMap::new();
    let file = sources.add("test.vow", text.to_string());
    vow_driver::check(&sources, file).diagnostics
}

fn fixed(source: &str) -> String {
    fix(source, diagnose).source
}

/// A diagnostic with a fix, for the cases that need one built by hand.
fn with_fix(span: Span, replacement: &str, applicability: Applicability) -> Diagnostic {
    let mut sources = SourceMap::new();
    let file = sources.add("made-up.vow", String::new());
    Diagnostic::error("VOW0000", file, span, "made up").with_fix(
        "made up",
        span,
        replacement,
        applicability,
    )
}

// -- what it does ----------------------------------------------------------

#[test]
fn a_certain_fix_is_applied() {
    let source = "module a\n\nfn balance() -> Int { 0 }\n\nfn f() -> Int { balanse() }\n";
    let result = fixed(source);
    assert!(result.contains("balance()"), "{result}");
    assert!(!result.contains("balanse"), "{result}");
    assert!(diagnose(&result).is_empty(), "it should check clean now");
}

#[test]
fn several_fixes_in_one_file_all_go_in() {
    let source = "module a\n\nfn balance() -> Int { 0 }\n\nfn f() -> Int { balanse() }\n\nfn g() -> Int { balanse() }\n";
    let result = fix(source, diagnose);
    assert_eq!(result.applied, 2);
    assert!(!result.source.contains("balanse"), "{}", result.source);
}

#[test]
fn fixing_is_idempotent() {
    let sources = [
        "module a\n\nfn balance() -> Int { 0 }\n\nfn f() -> Int { balanse() }\n",
        "module a\n\nfn f() -> Int { 0 }\n",
        "module a\n\nrecord R { n: Int }\n\nfn f(r: R) -> Int { r.n }\n",
    ];
    for source in sources {
        let once = fixed(source);
        let twice = fixed(&once);
        assert_eq!(once, twice, "a second run changed it:\n{once}");
    }
}

#[test]
fn a_fix_never_leaves_more_errors_than_it_found() {
    // The version of the check that catches a fix that was wrong rather than
    // merely unhelpful.
    let sources = [
        "module a\n\nfn balance() -> Int { 0 }\n\nfn f() -> Int { balanse() }\n",
        "module a\n\nfn f( -> Int { 0 }\n",
        "module a\n\nfn f() -> Nope { missing() }\n",
        "module a\n\nfn f() -> Int {\n    let s = \"never closed\n    0\n}\n",
        "module a\n\nchoice C {\n    One,\n    Two,\n}\n\nfn f(c: C) -> Int {\n    match c {\n        One => 1,\n    }\n}\n",
    ];
    for source in sources {
        let before = error_count(&diagnose(source));
        let after = error_count(&diagnose(&fixed(source)));
        assert!(
            after <= before,
            "fixing went from {before} errors to {after}:\n{}",
            fixed(source)
        );
    }
}

#[test]
fn a_file_with_nothing_to_fix_is_left_alone() {
    let source = "module a\n\nfn f() -> Int { 0 }\n";
    let result = fix(source, diagnose);
    assert!(!result.changed());
    assert_eq!(result.source, source);
}

// -- what it declines to do ------------------------------------------------

#[test]
fn a_guess_is_never_applied() {
    // `"a\qb"` might have meant a literal backslash and might have been a typo
    // for `\n`. The fix says so by being maybe-incorrect, and a tool that
    // rewrote the file anyway would be picking one for the author.
    let source = "module a\n\nfn f() -> Int {\n    let s = \"a\\qb\"\n    0\n}\n";
    let result = fix(source, diagnose);
    assert!(!result.changed(), "a guess was applied:\n{}", result.source);
}

#[test]
fn overlapping_fixes_are_both_dropped() {
    // Applying one would leave the other's span pointing at text that moved,
    // and there is no order that makes both right.
    let source = "0123456789";
    let result = fix(source, |_| {
        vec![
            with_fix(Span::new(0, 5), "AAA", Applicability::MachineApplicable),
            with_fix(Span::new(3, 8), "BBB", Applicability::MachineApplicable),
        ]
    });
    assert_eq!(result.source, source);
    assert!(!result.changed());
}

#[test]
fn fixes_that_only_touch_at_a_point_both_go_in() {
    // An insertion at the end of one edit and an insertion at the start of the
    // next are independent, so touching is not overlapping.
    let source = "0123456789";
    let result = fix(source, |text| {
        if text != "0123456789" {
            return Vec::new();
        }
        vec![
            with_fix(Span::new(0, 3), "A", Applicability::MachineApplicable),
            with_fix(Span::new(3, 6), "B", Applicability::MachineApplicable),
        ]
    });
    assert_eq!(result.source, "AB6789");
}

#[test]
fn a_span_outside_the_text_is_skipped_rather_than_applied() {
    let source = "short";
    let result = fix(source, |text| {
        if text != "short" {
            return Vec::new();
        }
        vec![with_fix(
            Span::new(100, 200),
            "boom",
            Applicability::MachineApplicable,
        )]
    });
    assert_eq!(result.source, source);
}

#[test]
fn a_span_inside_a_character_is_skipped() {
    // Rewriting from a byte offset that is not a character boundary would
    // corrupt the file, and a panic here would take the whole run with it.
    let source = "üüü";
    let result = fix(source, |text| {
        if text != "üüü" {
            return Vec::new();
        }
        vec![with_fix(
            Span::new(1, 3),
            "x",
            Applicability::MachineApplicable,
        )]
    });
    assert_eq!(result.source, source);
}

#[test]
fn two_fixes_that_undo_each_other_terminate() {
    // Nothing in the compiler does this today. The bound is here so that when
    // something eventually does, it stops rather than hangs.
    let result = fix("a", |text| {
        let replacement = if text == "a" { "b" } else { "a" };
        vec![with_fix(
            Span::new(0, 1),
            replacement,
            Applicability::MachineApplicable,
        )]
    });
    assert!(result.gave_up, "it should say it did not settle");
}
