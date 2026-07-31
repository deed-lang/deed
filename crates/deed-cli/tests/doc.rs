//! `deed doc`, run on modules that are not the shipped library.
//!
//! The generator itself is held by `documentation.rs`, which compares the
//! shipped pages byte for byte against what is checked in. What is here is the
//! part that was never true before: that any module gets one.
//!
//! `docs/std.md` used to say the same treatment was "possible for user modules
//! in principle". These are what stops that being a sentence about something
//! nobody had done.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const DEED: &str = env!("CARGO_BIN_EXE_deed");

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../examples/{name}"))
}

fn run(args: &[&str]) -> Output {
    Command::new(DEED)
        .args(args)
        .output()
        .expect("the deed binary should run")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

#[test]
fn a_module_that_is_not_the_shipped_library_gets_a_page() {
    let output = run(&["doc", corpus("hello.deed").to_str().expect("a path")]);
    assert_eq!(output.status.code(), Some(0));

    let page = stdout(&output);
    assert!(page.starts_with("# `examples/hello`"), "{page}");
    assert!(page.contains("## Module"), "{page}");
    assert!(page.contains("## `greet`"), "{page}");
}

/// The four things a signature says, on the page.
///
/// A row and a contract are the whole reason a page here is worth more than a
/// list of names: `design/00-motivation.md`'s pitch is that a reader reviews
/// the signature, and these are the parts of it a list would drop.
#[test]
fn a_page_carries_the_row_and_the_contract_rather_than_only_the_name() {
    let output = run(&["doc", corpus("hello.deed").to_str().expect("a path")]);
    let page = stdout(&output);

    assert!(page.contains("### Signature"), "{page}");
    assert!(page.contains("### Row variables"), "{page}");
    assert!(page.contains("### Declared row"), "{page}");
    assert!(page.contains("### Contract"), "{page}");
    assert!(page.contains("`Io.write`"), "the row is missing: {page}");
    assert!(
        page.contains("fn greet(out: Console, name: String) -> ()"),
        "the signature is quoted rather than printed: {page}"
    );
}

/// A pure function says so rather than leaving the section empty.
///
/// "Nothing is written here" and "this function performs nothing" are
/// different facts, and this file's whole subject is a reader being able to
/// tell them apart.
#[test]
fn a_pure_function_says_it_is_pure() {
    let output = run(&["doc", corpus("hello.deed").to_str().expect("a path")]);
    let page = stdout(&output);

    let twice = page
        .split("## `twice`")
        .nth(1)
        .expect("hello.deed declares `twice`");
    assert!(
        twice.contains("`none`"),
        "`twice` introduces no row variable and the page does not say so: {twice}"
    );
}

/// The examples come from the resolver rather than from the text.
#[test]
fn the_examples_are_lines_from_tests_that_name_the_function() {
    let output = run(&["doc", corpus("counter.deed").to_str().expect("a path")]);
    let page = stdout(&output);

    assert!(page.contains("### Examples from"), "{page}");
    assert!(page.contains("```deed"), "{page}");
}

/// A module with errors still gets a page, and the diagnostics go elsewhere.
///
/// Somebody asking what a module offers is not asking whether it compiles, and
/// a page on stdout has to be redirectable into a file on its own.
#[test]
fn a_module_that_does_not_check_still_documents_what_it_declares() {
    let scratch = std::env::temp_dir().join("deed-doc-broken.deed");
    std::fs::write(
        &scratch,
        "// A module that does not check.\nmodule broken\n\n\
         // Doubles a number.\nfn twice(n: Int) -> Int {\n    nope * 2\n}\n",
    )
    .expect("the temporary directory should be writable");

    let output = run(&["doc", scratch.to_str().expect("a path")]);
    let page = stdout(&output);
    let complaints = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(page.contains("## `twice`"), "{page}");
    assert!(page.contains("Doubles a number."), "{page}");
    assert!(
        complaints.contains("DEED3001"),
        "the diagnostic should be on stderr: {complaints}"
    );
    assert!(
        !page.contains("DEED3001"),
        "the diagnostic leaked onto stdout: {page}"
    );

    let _ = std::fs::remove_file(&scratch);
}

/// A function with no comment gets a page rather than a refusal.
///
/// The shipped library is held to a stricter rule by `documentation.rs`,
/// because that is a claim about this repository. Telling somebody running
/// `deed doc` on their own module what to write is not this command's job.
#[test]
fn a_function_with_no_comment_is_documented_anyway() {
    let scratch = std::env::temp_dir().join("deed-doc-bare.deed");
    std::fs::write(
        &scratch,
        "module bare\n\nfn twice(n: Int) -> Int {\n    n + n\n}\n",
    )
    .expect("the temporary directory should be writable");

    let output = run(&["doc", scratch.to_str().expect("a path")]);
    assert_eq!(output.status.code(), Some(0));

    let page = stdout(&output);
    assert!(page.contains("## `twice`"), "{page}");
    assert!(page.contains("No description."), "{page}");
    assert!(page.contains("No test names this function."), "{page}");

    let _ = std::fs::remove_file(&scratch);
}

/// Several modules in one call are several pages, in a settled order.
///
/// Sorted rather than in the order given, which is what the rest of the
/// command line already does so that output can be diffed between runs. That
/// is worth pinning: a page order that followed the argument list would make
/// two invocations of the same set produce different files.
#[test]
fn more_than_one_module_is_more_than_one_page_in_a_settled_order() {
    let forwards = run(&[
        "doc",
        corpus("hello.deed").to_str().expect("a path"),
        corpus("counter.deed").to_str().expect("a path"),
    ]);
    let backwards = run(&[
        "doc",
        corpus("counter.deed").to_str().expect("a path"),
        corpus("hello.deed").to_str().expect("a path"),
    ]);

    let page = stdout(&forwards);
    assert_eq!(
        page,
        stdout(&backwards),
        "the order of the arguments showed"
    );

    let counter = page.find("# `examples/counter`").expect("the counter page");
    let hello = page.find("# `examples/hello`").expect("the hello page");
    assert!(counter < hello, "the pages are not in sorted order");
}

#[test]
fn doc_needs_something_to_document() {
    let output = run(&["doc"]);
    assert_eq!(output.status.code(), Some(2));
}
