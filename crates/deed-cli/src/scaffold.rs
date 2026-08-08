//! What `deed new` writes.
//!
//! A scaffold is a claim about what a project in this language looks like, so
//! it is held rather than described: `crates/deed-cli/tests/new.rs` runs the
//! command into a temporary directory and then runs `deed check`, `deed test`,
//! `deed run` and `deed fmt --check` on what came out. A scaffold that stops
//! checking, stops being canonical, or stops running fails that test by name.
//!
//! There is no `deed.manifest` here. The format has two directives and both of
//! them are about code that is not in your tree, so a new project has nothing
//! to put in one, and a file of commented-out examples is dead text that the
//! rest of this compiler refuses to write.

use deed_lexer::Keyword;

/// The library module, the program that imports it, and nothing else.
///
/// Two files rather than one, because the second thing anyone does is split a
/// program in two, and the `module` line plus the `use` line are the whole of
/// how that works here. Getting it wrong is a diagnostic; seeing it once is not.
pub fn files(name: &str) -> Vec<(String, String)> {
    vec![
        (format!("{name}.deed"), library(name)),
        ("main.deed".to_string(), program(name)),
    ]
}

/// Refuses a name that could not be both a directory and a module path segment.
///
/// The module path is the file's identity here, so the name on the command line
/// becomes a directory on disk and a word inside the file at the same time. The
/// language would take more than this, and so would the filesystem, but the
/// intersection is the part nobody has to remember two rules for.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("`deed new` needs a name, e.g. `deed new greeter`".to_string());
    }
    if Keyword::from_ident(name).is_some() {
        return Err(format!(
            "`{name}` is a keyword, so it cannot be a module name: pick another word"
        ));
    }
    let lowercase: String = name.to_lowercase();
    if name != lowercase && lowercase.chars().all(acceptable) {
        return Err(format!(
            "`{name}` has a capital in it, and module paths here are lowercase: try `{lowercase}`"
        ));
    }
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        return Err(format!(
            "`{name}` does not start with a lowercase letter, and a module path segment has to"
        ));
    }
    if let Some(bad) = name.chars().find(|c| !acceptable(*c)) {
        return Err(format!(
            "`{name}` contains `{bad}`, and a name here is lowercase letters, digits and `_`"
        ));
    }
    Ok(())
}

fn acceptable(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'
}

fn library(name: &str) -> String {
    format!(
        "\
// The library half of a new Deed project.
//
// A signature here is a promise the compiler checks. `where` is what a caller
// has to establish before calling, `ensures` is what comes back, and neither is
// a comment: the compiler proves what it can and checks the rest at runtime.

module {name}

fn greeting(name: String) -> String
  where
    length(name) > 0,
  ensures
    ok  => length(result) > length(name),
{{
    join([\"hello, \", name], \"\")
}}

test \"a greeting carries the name it was given\" {{
    assert greeting(\"world\") == \"hello, world\"
}}

// `assert refuses` is how a program tests its own refusals. Nothing else in
// the language catches a broken contract.
test \"there is no greeting for nobody\" {{
    assert refuses greeting(\"\")
}}
"
    )
}

fn program(name: &str) -> String {
    format!(
        "\
// The program half. `deed run main.deed` calls `main`.
//
// There is no ambient authority in Deed. `main` receives one `System`, and
// everything below it can only reach what it was handed, so this signature is
// the whole of what the program can do.

module main

use {name}.{{greeting}}

fn main(sys: System) -> ()
  uses
    Io.write,
{{
    Io.write(sys.console, greeting(\"world\"))
}}
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keyword_is_not_a_module_name() {
        let message = check_name("effect").unwrap_err();
        assert!(message.contains("keyword"), "{message}");
    }

    #[test]
    fn a_capital_gets_the_lowercase_form_rather_than_a_rule() {
        let message = check_name("Greeter").unwrap_err();
        assert!(message.contains("`greeter`"), "{message}");
    }

    #[test]
    fn a_name_lowercasing_would_not_fix_is_not_offered_a_lowercase_form() {
        let message = check_name("Grüße").unwrap_err();
        assert!(!message.contains("try `"), "{message}");
    }

    #[test]
    fn a_digit_cannot_start_one() {
        let message = check_name("2fast").unwrap_err();
        assert!(message.contains("lowercase letter"), "{message}");
    }

    #[test]
    fn a_path_separator_is_named_where_it_appears() {
        let message = check_name("a/b").unwrap_err();
        assert!(message.contains('/'), "{message}");
    }

    #[test]
    fn an_ordinary_name_is_accepted() {
        assert!(check_name("greeter_2").is_ok());
    }

    #[test]
    fn the_program_imports_the_library_by_the_name_it_was_given() {
        let written = files("ledger");
        assert_eq!(written[0].0, "ledger.deed");
        assert!(written[0].1.contains("module ledger\n"), "{}", written[0].1);
        assert!(
            written[1].1.contains("use ledger.{greeting}"),
            "{}",
            written[1].1
        );
    }
}
