//! Integration tests for `deed.manifest` parsing and diagnostic reporting.
//!
//! The manifest answers exactly one question: where do components not in the
//! current source tree live? These tests check that the parser accepts valid
//! manifests, rejects invalid ones with the right diagnostic codes, and that
//! errors on one line do not suppress valid declarations on others.
//!
//! The diagnostic codes tested here:
//! - `DEED7001`: an unrecognized directive
//! - `DEED7002`: a `component` directive with no path
//! - `DEED7003`: a `module` directive with nothing after it
//! - `DEED7004`: a `module` directive with a location and no hash
//! - `DEED7005`: a hash that is not a lowercase SHA-256

use std::path::PathBuf;

use deed_diagnostics::SourceMap;
use deed_driver::codes::{
    BAD_MODULE_HASH, MISSING_COMPONENT_PATH, MISSING_MODULE_HASH, MISSING_MODULE_SOURCE,
    UNKNOWN_DIRECTIVE,
};
use deed_driver::{ComponentRoot, parse_manifest};

fn parse(text: &str) -> deed_driver::Manifest {
    let mut sources = SourceMap::new();
    let file = sources.add("deed.manifest", text.to_string());
    parse_manifest(file, text)
}

// ---- valid inputs ----

#[test]
fn an_empty_manifest_is_accepted() {
    let m = parse("");
    assert!(m.components.is_empty());
    assert!(m.diagnostics.is_empty());
}

#[test]
fn a_manifest_with_only_comments_is_accepted() {
    let m = parse("# no components here\n# that is fine\n");
    assert!(m.components.is_empty());
    assert!(m.diagnostics.is_empty());
}

#[test]
fn a_single_relative_component_is_parsed() {
    let m = parse("component ../other\n");
    assert_eq!(
        m.components,
        vec![ComponentRoot {
            path: PathBuf::from("../other")
        }]
    );
    assert!(m.diagnostics.is_empty());
}

#[test]
fn multiple_component_directives_are_all_parsed() {
    let m = parse("component ../a\ncomponent ../b\ncomponent ../c\n");
    assert_eq!(m.components.len(), 3);
    assert_eq!(m.components[0].path, PathBuf::from("../a"));
    assert_eq!(m.components[1].path, PathBuf::from("../b"));
    assert_eq!(m.components[2].path, PathBuf::from("../c"));
    assert!(m.diagnostics.is_empty());
}

#[test]
fn an_absolute_path_is_accepted() {
    let m = parse("component /usr/local/lib\n");
    assert_eq!(m.components.len(), 1);
    assert_eq!(m.components[0].path, PathBuf::from("/usr/local/lib"));
}

#[test]
fn blank_lines_between_directives_are_ignored() {
    let m = parse("component ../a\n\ncomponent ../b\n");
    assert_eq!(m.components.len(), 2);
    assert!(m.diagnostics.is_empty());
}

#[test]
fn inline_comments_after_hash_are_not_treated_as_directives() {
    let m = parse("# component ../ignored\ncomponent ../real\n");
    assert_eq!(m.components.len(), 1);
    assert_eq!(m.components[0].path, PathBuf::from("../real"));
    assert!(m.diagnostics.is_empty());
}

#[test]
fn leading_whitespace_before_a_directive_is_accepted() {
    let m = parse("  component ../x\n");
    assert_eq!(m.components.len(), 1);
    assert_eq!(m.components[0].path, PathBuf::from("../x"));
    assert!(m.diagnostics.is_empty());
}

#[test]
fn manifest_with_no_trailing_newline_is_accepted() {
    let m = parse("component ../x");
    assert_eq!(m.components.len(), 1);
    assert!(m.diagnostics.is_empty());
}

// ---- DEED7001: unknown directive ----

#[test]
fn an_unknown_directive_is_deed7001() {
    let m = parse("target release\n");
    assert!(m.components.is_empty());
    assert_eq!(m.diagnostics.len(), 1);
    assert_eq!(m.diagnostics[0].code, UNKNOWN_DIRECTIVE);
}

#[test]
fn an_unknown_directive_names_the_unrecognized_word_in_its_message() {
    let m = parse("profile production\n");
    assert_eq!(m.diagnostics.len(), 1);
    let msg = &m.diagnostics[0].message;
    assert!(
        msg.contains("profile"),
        "expected the directive name in the message, got: {msg}"
    );
}

// ---- DEED7002: missing component path ----

#[test]
fn component_with_no_path_is_deed7002() {
    let m = parse("component\n");
    assert!(m.components.is_empty());
    assert_eq!(m.diagnostics.len(), 1);
    assert_eq!(m.diagnostics[0].code, MISSING_COMPONENT_PATH);
}

#[test]
fn component_with_only_whitespace_is_deed7002() {
    let m = parse("component   \n");
    assert!(m.components.is_empty());
    assert_eq!(m.diagnostics.len(), 1);
    assert_eq!(m.diagnostics[0].code, MISSING_COMPONENT_PATH);
}

// ---- errors do not suppress valid declarations ----

#[test]
fn a_bad_line_does_not_hide_the_good_ones() {
    let m = parse("component ../a\nbuild debug\ncomponent ../b\n");
    assert_eq!(m.components.len(), 2, "valid components should be kept");
    assert_eq!(m.diagnostics.len(), 1, "only the bad line should error");
    assert_eq!(m.diagnostics[0].code, UNKNOWN_DIRECTIVE);
}

#[test]
fn two_bad_lines_each_produce_their_own_diagnostic() {
    let m = parse("component\nfeature fast\n");
    assert!(m.components.is_empty());
    assert_eq!(m.diagnostics.len(), 2);
    let codes: Vec<&str> = m.diagnostics.iter().map(|d| d.code).collect();
    assert!(codes.contains(&MISSING_COMPONENT_PATH));
    assert!(codes.contains(&UNKNOWN_DIRECTIVE));
}

// ---- span accuracy ----

#[test]
fn span_of_unknown_directive_covers_the_whole_trimmed_line() {
    let m = parse("target release\n");
    let span = m.diagnostics[0].primary.span;
    // "target release" is 14 characters, starting at offset 0.
    assert_eq!(span.start, 0);
    assert_eq!(span.end, 14);
}

#[test]
fn span_of_missing_path_covers_only_the_component_keyword() {
    let m = parse("component\n");
    let span = m.diagnostics[0].primary.span;
    // "component" is 9 characters, starting at offset 0.
    assert_eq!(span.start, 0);
    assert_eq!(span.end, 9);
}

#[test]
fn span_of_error_on_second_line_accounts_for_first_line_length() {
    let m = parse("component ../ok\ntarget release\n");
    assert_eq!(m.diagnostics.len(), 1);
    let span = m.diagnostics[0].primary.span;
    // First line is "component ../ok\n" = 16 bytes.
    // Second line "target release" starts at offset 16.
    assert_eq!(span.start, 16);
    assert_eq!(span.end, 16 + 14); // "target release" is 14 chars.
}

#[test]
fn spans_account_for_crlf_and_surrounding_whitespace() {
    let m = parse("# ok\r\n  target release  \r\n   component   \r\n");
    assert_eq!(m.diagnostics.len(), 2);
    assert_eq!(m.diagnostics[0].primary.span.start, 8);
    assert_eq!(m.diagnostics[0].primary.span.end, 22);
    assert_eq!(m.diagnostics[1].primary.span.start, 29);
    assert_eq!(m.diagnostics[1].primary.span.end, 38);
}

// ---- DEED7003-7005: a module declared by location and hash ----

const A_DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn a_module_is_read_as_a_location_and_a_hash() {
    let m = parse(&format!(
        "module https://example.com/list.deed sha256:{A_DIGEST}\n"
    ));
    assert!(m.diagnostics.is_empty(), "{:?}", m.diagnostics);
    assert_eq!(m.modules.len(), 1);
    assert_eq!(m.modules[0].url, "https://example.com/list.deed");
    assert_eq!(m.modules[0].hash, A_DIGEST);
}

/// The name is not in the directive on purpose. A fetched module is named by
/// its own `module` line, so the location says where the bytes were and the
/// hash says what they are, and two projects fetching the same bytes from two
/// places get one module.
#[test]
fn a_module_directive_does_not_say_what_the_module_is_called() {
    let m = parse(&format!(
        "module https://example.com/anything sha256:{A_DIGEST}\n"
    ));
    assert!(m.diagnostics.is_empty(), "{:?}", m.diagnostics);
    assert_eq!(m.modules.len(), 1);
}

#[test]
fn module_with_nothing_after_it_is_deed7003() {
    let m = parse("module\n");
    assert!(m.modules.is_empty());
    assert_eq!(m.diagnostics.len(), 1);
    assert_eq!(m.diagnostics[0].code, MISSING_MODULE_SOURCE);
}

#[test]
fn module_with_no_hash_is_deed7004() {
    let m = parse("module https://example.com/list.deed\n");
    assert!(m.modules.is_empty());
    assert_eq!(m.diagnostics.len(), 1);
    assert_eq!(m.diagnostics[0].code, MISSING_MODULE_HASH);
}

/// Every way of writing the hash wrong, because they are one mistake seen from
/// different angles and a reader is told which angle they are at.
#[test]
fn a_hash_that_is_not_a_lowercase_sha256_is_deed7005() {
    for spelled in [
        "md5:0123456789abcdef",
        "0123456789abcdef",
        "sha256:abc",
        "sha256:0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef",
        "sha256:0123456789abcdeg0123456789abcdef0123456789abcdef0123456789abcdef",
    ] {
        let m = parse(&format!("module https://example.com/x {spelled}\n"));
        assert!(m.modules.is_empty(), "`{spelled}` was accepted");
        assert_eq!(
            m.diagnostics.len(),
            1,
            "`{spelled}` should be one diagnostic: {:?}",
            m.diagnostics
        );
        assert_eq!(m.diagnostics[0].code, BAD_MODULE_HASH, "`{spelled}`");
    }
}

/// One module a line, so a diff shows which one changed.
#[test]
fn a_module_line_carrying_two_of_them_is_refused() {
    let m = parse(&format!(
        "module https://example.com/a sha256:{A_DIGEST} https://example.com/b\n"
    ));
    assert!(m.modules.is_empty());
    assert_eq!(m.diagnostics.len(), 1);
    assert_eq!(m.diagnostics[0].code, UNKNOWN_DIRECTIVE);
}

/// A word that starts with the directive is not the directive.
#[test]
fn a_directive_that_merely_starts_with_module_is_unknown() {
    let m = parse("modules foo\n");
    assert!(m.modules.is_empty());
    assert_eq!(m.diagnostics.len(), 1);
    assert_eq!(m.diagnostics[0].code, UNKNOWN_DIRECTIVE);
}

/// An error on one line does not hide what the others said, which is the
/// property the whole file already had and now has across two directives.
#[test]
fn a_bad_module_line_does_not_hide_a_good_component_line() {
    let m = parse("module nothing-after-me\ncomponent ../shared\n");
    assert_eq!(m.components.len(), 1);
    assert_eq!(m.diagnostics.len(), 1);
    assert_eq!(m.diagnostics[0].code, MISSING_MODULE_HASH);
}
