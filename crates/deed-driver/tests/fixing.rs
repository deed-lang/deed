//! Applying the fixes diagnostics carry.
//!
//! The interesting cases are the ones where applying a fix is the wrong thing:
//! a guess, an overlap, a span that does not name a range of the text. A tool
//! that rewrites source has to be more careful about when it declines than
//! about when it acts.

use deed_diagnostics::{Applicability, Diagnostic, SourceMap, Span, SuggestedEdit};
use deed_driver::fix::{error_count, fix};

fn diagnose(text: &str) -> Vec<Diagnostic> {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", text.to_string());
    deed_driver::check(&sources, file).diagnostics
}

fn fixed(source: &str) -> String {
    fix(source, diagnose).source
}

/// A diagnostic with a fix, for the cases that need one built by hand.
fn with_fix(span: Span, replacement: &str, applicability: Applicability) -> Diagnostic {
    let mut sources = SourceMap::new();
    let file = sources.add("made-up.deed", String::new());
    Diagnostic::error("DEED0000", file, span, "made up").with_fix(
        "made up",
        span,
        replacement,
        applicability,
    )
}

/// A diagnostic whose repair takes two edits, for the same reason.
fn with_edits(edits: Vec<(Span, &str)>, applicability: Applicability) -> Diagnostic {
    let mut sources = SourceMap::new();
    let file = sources.add("made-up.deed", String::new());
    let span = edits[0].0;
    Diagnostic::error("DEED0000", file, span, "made up").with_edits(
        "made up",
        edits
            .into_iter()
            .map(|(span, replacement)| SuggestedEdit {
                span,
                replacement: replacement.to_string(),
            })
            .collect(),
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

/// A function full of bindings written the way the last language wrote them.
///
/// Each line carries its own fix, so one pass turns the file into one that
/// compiles. The type-first form keeps the type it was given, which is the
/// point of writing `let k: Int` rather than `let k`.
#[test]
fn bindings_written_without_let_all_come_out_in_one_pass() {
    let source = "module a\n\nfn f() -> Int {\n    var n = 1\n    const m = 2\n    Int k = 3\n    n + m + k\n}\n";
    let result = fix(source, diagnose);
    assert_eq!(result.applied, 3);
    assert_eq!(
        result.source,
        "module a\n\nfn f() -> Int {\n    let n = 1\n    let m = 2\n    let k: Int = 3\n    n + m + k\n}\n"
    );
    assert!(
        diagnose(&result.source).is_empty(),
        "it should check clean now: {:?}",
        diagnose(&result.source)
    );
}

/// The one repair in the compiler that has to wrap a value, end to end.
#[test]
fn a_cast_comes_out_as_the_call_it_meant() {
    let source = "module a\n\nfn f(n: Int) -> String {\n    let text = n as String\n    text\n}\n";
    let result = fix(source, diagnose);
    assert_eq!(result.applied, 1);
    assert_eq!(
        result.source,
        "module a\n\nfn f(n: Int) -> String {\n    let text = to_string(n)\n    text\n}\n"
    );
    assert!(
        diagnose(&result.source).is_empty(),
        "it should check clean now: {:?}",
        diagnose(&result.source)
    );
}

// -- rows ------------------------------------------------------------------
//
// The row diagnostics say what to type and, for a long time, did not type it.
// A `SuggestedEdit` is a span and a replacement, and writing one means knowing
// about commas, indentation and a clause that may not exist yet, none of which
// the effect checker has any business knowing. The driver does, so it writes
// them there.

/// Two effects and nothing else, for the row cases.
const EFFECTS: &str = "module a\n\n\
     effect Log {\n\
     \x20   fn note(message: String) -> ()\n\
     }\n\n\
     effect Bell {\n\
     \x20   fn ring() -> ()\n\
     }\n\n";

#[test]
fn a_row_that_is_too_narrow_is_written_out() {
    let source = format!(
        "{EFFECTS}fn talks() -> Int {{\n\
         \x20   Log.note(\"hi\")\n\
         \x20   Bell.ring()\n\
         \x20   1\n\
         }}\n"
    );
    let result = fixed(&source);

    // Sorted, because a generated row has no written order to preserve and the
    // same program has to produce the same text.
    assert!(
        result.contains("fn talks() -> Int\n  uses\n    Bell.ring,\n    Log.note,\n{\n"),
        "{result}"
    );
    assert!(diagnose(&result).is_empty(), "{result}");
}

#[test]
fn a_row_that_is_too_wide_loses_what_it_does_not_use() {
    let source = format!(
        "{EFFECTS}fn quiet() -> Int\n\
         \x20 uses\n\
         \x20   Log.note,\n\
         {{\n\
         \x20   1\n\
         }}\n"
    );
    let result = fixed(&source);

    assert!(result.contains("fn quiet() -> Int {\n"), "{result}");
    assert!(!result.contains("uses"), "{result}");
    assert!(diagnose(&result).is_empty(), "{result}");
}

#[test]
fn a_row_that_is_wrong_in_both_directions_is_one_repair() {
    // Too narrow and too wide at once. Two edits over the same span would be
    // dropped as overlapping, so there is one fix and it says the whole row.
    let source = format!(
        "{EFFECTS}fn talks() -> Int\n\
         \x20 uses\n\
         \x20   Bell.ring,\n\
         {{\n\
         \x20   Log.note(\"hi\")\n\
         \x20   1\n\
         }}\n"
    );
    let result = fix(&source, diagnose);

    assert_eq!(result.applied, 1);
    assert!(
        result
            .source
            .contains("fn talks() -> Int\n  uses\n    Log.note,\n{\n"),
        "{}",
        result.source
    );
    assert!(diagnose(&result.source).is_empty(), "{}", result.source);
}

#[test]
fn what_it_writes_is_what_the_formatter_would_have() {
    let source = format!(
        "{EFFECTS}fn talks() -> Int {{\n\
         \x20   Log.note(\"hi\")\n\
         \x20   1\n\
         }}\n"
    );
    let result = fixed(&source);

    let mut sources = SourceMap::new();
    let file = sources.add("fixed.deed", result.clone());
    let formatted = deed_fmt::format(file, &result).expect("it should parse");
    assert_eq!(formatted, result);
}

#[test]
fn a_comment_in_the_clause_stops_it() {
    // Rewriting the region between a signature and its body would eat the
    // comment, and a machine-applicable fix that deletes a comment is a fix
    // nobody should have applied.
    let source = format!(
        "{EFFECTS}fn quiet() -> Int\n\
         \x20 uses\n\
         \x20   // why\n\
         \x20   Log.note,\n\
         {{\n\
         \x20   1\n\
         }}\n"
    );
    let result = fix(&source, diagnose);

    assert!(!result.changed());
    assert!(result.source.contains("// why"), "{}", result.source);
}

#[test]
fn a_contract_with_more_than_a_row_in_it_is_left_alone() {
    // The region holds `where`, `uses` and `ensures`, and nothing in the tree
    // says where one stops and the next starts, so rewriting it would be a
    // guess about the other two.
    let source = format!(
        "{EFFECTS}fn checked(n: Int) -> Int\n\
         \x20 where\n\
         \x20   n > 0,\n\
         {{\n\
         \x20   Log.note(\"hi\")\n\
         \x20   n\n\
         }}\n"
    );
    let result = fix(&source, diagnose);

    assert!(!result.changed());
    assert!(!diagnose(&result.source).is_empty(), "still an error");
}

// -- imports ---------------------------------------------------------------
//
// The same layering as the rows, one pass earlier. `DEED3003` knows which name
// is unused and nothing else: taking a name out of a list is a question about
// a comma, and taking the last one out is a question about a line and the
// blank line beside it.

/// A module with two things in it, for the import cases.
const DEP: &str = "module dep\n\nfn used() -> Int { 1 }\n\nfn spare() -> Int { 2 }\n";

/// Fixing a file that imports another one.
///
/// An unused import needs something real on the other side of it, or the
/// diagnostic is about the module rather than about the name.
fn fixed_with(source: &str, dependency: &str) -> String {
    fix(source, |text| {
        let mut sources = SourceMap::new();
        let dep = sources.add("dep.deed", dependency.to_string());
        let file = sources.add("test.deed", text.to_string());
        deed_driver::check_all(&sources, &[dep, file])
            .pop()
            .expect("two files in, two results out")
            .diagnostics
    })
    .source
}

#[test]
fn an_import_nobody_uses_is_taken_out_of_the_list() {
    let source = "module a\n\nuse dep.{used, spare}\n\nfn f() -> Int {\n    used()\n}\n";
    let result = fixed_with(source, DEP);
    assert!(result.contains("use dep.{used}\n"), "{result}");
}

#[test]
fn the_last_name_takes_the_line_and_a_blank_with_it() {
    // Deleting only the line leaves the blank above it and the blank below it
    // doing the work of one gap.
    let source = "module a\n\nuse dep.{spare}\n\nfn f() -> Int {\n    1\n}\n";
    let result = fixed_with(source, DEP);
    assert_eq!(result, "module a\n\nfn f() -> Int {\n    1\n}\n");
}

#[test]
fn two_lines_that_both_empty_out_leave_one_gap_between_them() {
    // Removed one at a time these would each leave their own blank behind.
    // One fix for the block is what makes that impossible rather than unlikely.
    let source = "module a\n\nuse dep.{spare}\nuse dep.{used}\n\nfn f() -> Int {\n    1\n}\n";
    let result = fixed_with(source, DEP);
    assert_eq!(result, "module a\n\nfn f() -> Int {\n    1\n}\n");
}

#[test]
fn what_survives_keeps_its_own_line() {
    let source = "module a\n\nuse dep.{spare}\nuse dep.{used}\n\nfn f() -> Int {\n    used()\n}\n";
    let result = fixed_with(source, DEP);
    assert_eq!(
        result,
        "module a\n\nuse dep.{used}\n\nfn f() -> Int {\n    used()\n}\n"
    );
}

#[test]
fn what_it_writes_for_an_import_is_what_the_formatter_would_have() {
    let source = "module a\n\nuse dep.{used, spare}\n\nfn f() -> Int {\n    used()\n}\n";
    let result = fixed_with(source, DEP);

    let mut sources = SourceMap::new();
    let file = sources.add("fixed.deed", result.clone());
    let formatted = deed_fmt::format(file, &result).expect("it should parse");
    assert_eq!(formatted, result);
}

#[test]
fn a_comment_among_the_imports_stops_it() {
    // A comment beside an import is usually about that import. It is not the
    // compiler's to move and certainly not the compiler's to delete.
    let source =
        "module a\n\nuse dep.{used}\n// why\nuse dep.{spare}\n\nfn f() -> Int {\n    used()\n}\n";
    let result = fixed_with(source, DEP);
    assert_eq!(result, source);
}

/// A comment above the block is not in the block, so it is not in the way.
#[test]
fn a_comment_above_the_imports_is_not_in_the_way() {
    let source = "module a\n\n// what this file needs\nuse dep.{used, spare}\n\nfn f() -> Int {\n    used()\n}\n";
    let result = fixed_with(source, DEP);
    assert_eq!(
        result,
        "module a\n\n// what this file needs\nuse dep.{used}\n\nfn f() -> Int {\n    used()\n}\n"
    );
}

#[test]
fn an_import_that_is_used_is_left_where_it_is() {
    let source = "module a\n\nuse dep.{used}\n\nfn f() -> Int {\n    used()\n}\n";
    assert_eq!(fixed_with(source, DEP), source);
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

/// A repair that wraps something is two edits and one answer. Both go in, and
/// it counts as one repair, because what a person counting wants is the number
/// of things that were wrong.
#[test]
fn a_repair_of_two_edits_goes_in_as_one() {
    let source = "0123456789";
    let result = fix(source, |text| {
        if text != "0123456789" {
            return Vec::new();
        }
        vec![with_edits(
            vec![(Span::at(2), "("), (Span::new(5, 7), ")")],
            Applicability::MachineApplicable,
        )]
    });
    assert_eq!(result.source, "01(234)789");
    assert_eq!(result.applied, 1);
}

/// Half a repair is not a smaller repair. If one edit has to be dropped the
/// other goes with it, or the file is left holding an opening parenthesis that
/// nothing closes.
#[test]
fn half_a_repair_is_refused_with_the_other_half() {
    let source = "0123456789";
    let result = fix(source, |_| {
        vec![
            with_edits(
                vec![(Span::at(2), "("), (Span::new(5, 7), ")")],
                Applicability::MachineApplicable,
            ),
            // Overlaps the closing half only. The opening half is untouched
            // and would otherwise sail in on its own.
            with_fix(Span::new(6, 9), "X", Applicability::MachineApplicable),
        ]
    });
    assert_eq!(result.source, source);
    assert!(!result.changed());
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
