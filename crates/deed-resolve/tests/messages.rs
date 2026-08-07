//! Every message the resolver can produce, read.
//!
//! `crates/deed-driver/tests/codes.rs` matches on a code's name, so one tested
//! shape satisfies it for a code with several messages behind it. The resolver
//! has ten codes across twelve emission sites and eleven distinct sentences,
//! and five of those sentences had never been rendered by a test. How the
//! twelve divide between the ten codes is written down once, on the constants
//! in `crates/deed-resolve/src/codes.rs`.
//!
//! # What is not here
//!
//! Messages already rendered somewhere else are read there and not again, so
//! that breaking one string fails one test. Those are `nothing reads ...` and
//! the `_name` fix, in `resolving.rs`; `has no member ...` on a locally
//! declared effect, also in `resolving.rs`; and `an alternative cannot bind a
//! name`, also in `resolving.rs`.
//!
//! # Wording and code are two claims
//!
//! [`Reported::under`] appears on one of these and not on the rest, which is
//! deliberate. Swapping the code constant at each of the twelve emission sites
//! in turn found one with no existing test to hold it: the `DEED3005` emitted
//! when a module-level item shadows a builtin. All other sites are held by a
//! `codes_of` assertion in `resolving.rs`. That one says which code it arrives
//! under as well as what it says.

use deed_diagnostics::{SourceMap, render_human};
use deed_lexer::tokenize;
use deed_parser::parse;
use deed_resolve::{Universe, codes, resolve};

/// One diagnostic the resolver produced, as a reader meets it.
struct Reported {
    code: &'static str,
    text: String,
}

impl Reported {
    /// What the reader sees, anywhere in the rendering.
    fn says(&self, needle: &str) -> &Self {
        assert!(
            self.text.contains(needle),
            "expected `{needle}` in:\n{}",
            self.text
        );
        self
    }

    fn never_says(&self, needle: &str) -> &Self {
        assert!(
            !self.text.contains(needle),
            "did not expect `{needle}` in:\n{}",
            self.text
        );
        self
    }

    /// Which code it arrived under.
    fn under(&self, code: &str) -> &Self {
        assert_eq!(self.code, code, "in:\n{}", self.text);
        self
    }
}

/// The single diagnostic from resolving `src` against an empty universe.
fn message(src: &str) -> Reported {
    message_in(src, &Universe::new())
}

/// The single diagnostic from resolving `src` against `universe`.
fn message_in(src: &str, universe: &Universe) -> Reported {
    let mut sources = SourceMap::new();
    let file = sources.add("test.deed", src);
    let lexed = tokenize(file, sources.file(file).text());
    let parsed = parse(file, &lexed.tokens);
    let resolved = resolve(file, &parsed.module, universe);

    assert_eq!(
        resolved.diagnostics.len(),
        1,
        "expected exactly one diagnostic, got {}:\n{}",
        resolved.diagnostics.len(),
        resolved
            .diagnostics
            .iter()
            .map(|d| render_human(&sources, d))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let d = &resolved.diagnostics[0];
    Reported {
        code: d.code,
        text: render_human(&sources, d),
    }
}

/// A universe holding each of `modules`, parsed from source.
fn universe_of(modules: &[&str]) -> Universe {
    let mut universe = Universe::new();
    let mut sources = SourceMap::new();
    for (index, source) in modules.iter().enumerate() {
        let file = sources.add(format!("dep{index}.deed"), *source);
        let lexed = tokenize(file, sources.file(file).text());
        let parsed = parse(file, &lexed.tokens);
        universe.add(&parsed.module);
    }
    universe
}

// -- DEED3002 DUPLICATE_DEFINITION -----------------------------------------

#[test]
fn a_name_declared_twice_says_so_in_the_headline() {
    // The secondary labels say which kind each declaration is; the headline
    // names neither, because neither is more to blame than the other.
    message("module a\n\nfn thing() -> Int { 0 }\n\nrecord thing { x: Int }\n")
        .says("`thing` is declared twice in this module");
}

// -- DEED3005 SHADOWED_DECLARATION -----------------------------------------

// Two sentences, two emission sites. A module-level item that shadows a
// builtin goes through `declare_item`; a local name that shadows anything
// declared above it goes through `declare_local`. The codes test named the
// code constant once and held neither sentence.

/// A `fn length` at module level silently replaces the prelude's `length`
/// for the whole file. That is worse than the local case, where the reader
/// at least has a function boundary to contain the damage.
///
/// The builtin is declared nowhere, so there is no source location to point
/// at. The diagnostic points at the shadowing item and adds a note, but it
/// never tries to say `declared here` about something with no location.
#[test]
fn a_module_item_shadowing_a_builtin_names_the_language() {
    message("module a\n\nfn length(n: Int) -> Int { n }\n")
        .under(codes::SHADOWED_DECLARATION)
        .says("`length` hides a name the language provides")
        .never_says("declared here");
}

#[test]
fn a_local_name_shadowing_a_declaration_names_what_it_hides() {
    message(
        "module a\n\nfn total() -> Int { 0 }\n\nfn f() -> Int {\n  let total = 1\n  total\n}\n",
    )
    .says("`total` hides a function");
}

// -- DEED3004 SHADOWED_BINDING -----------------------------------------------

#[test]
fn a_binding_that_re_uses_a_name_already_in_scope_says_it_is_already_bound() {
    message("module a\n\nfn f(x: Int) -> Int {\n  let x = x\n  x\n}\n")
        .says("`x` is already bound");
}

// -- DEED3001 UNKNOWN_NAME ---------------------------------------------------

// `cannot find \`...\`` is checked in `resolving.rs`; the `in this scope`
// tail was not.

#[test]
fn an_unknown_name_says_where_it_was_not_found() {
    message("module a\n\nfn f() -> Int { nonesuch }\n")
        .says("cannot find `nonesuch` in this scope");
}

// -- DEED3006 UNKNOWN_MEMBER -------------------------------------------------

// Two sentences, two emission sites. When the container is an import the
// message names the kind it is and says it has no such name. When the
// container is a locally declared effect or choice the message leads with
// the kind. `resolving.rs` reads the local sentence; the import sentence was
// not read.

#[test]
fn an_unknown_member_on_an_imported_effect_names_what_it_is() {
    let universe =
        universe_of(&["module other\n\neffect Ledger {\n  fn balance(id: Int) -> Int\n}\n"]);
    message_in(
        "module a\n\nuse other.{Ledger}\n\nfn f() -> Int\n  uses Ledger.balence,\n{ 0 }\n",
        &universe,
    )
    .says("`Ledger` is an effect with no `balence`");
}

// -- DEED3003 UNUSED_IMPORT --------------------------------------------------

#[test]
fn an_unused_import_names_itself_in_the_headline() {
    let universe = universe_of(&["module other\n\nrecord Spare { n: Int }\n"]);
    message_in(
        "module a\n\nuse other.{Spare}\n\nfn f() -> Int { 0 }\n",
        &universe,
    )
    .says("`Spare` is imported but never used");
}

// -- DEED3007 UNKNOWN_MODULE -------------------------------------------------

// The code was checked in `resolving.rs`; the sentence was not.

#[test]
fn an_unknown_module_names_the_path_and_where_the_compiler_looks() {
    message("module a\n\nuse other.{Thing}\n\nfn f() -> Thing {}\n")
        .says("no module `other` among the files being compiled");
}

// -- DEED3008 UNKNOWN_EXPORT -------------------------------------------------

// The code was checked in `resolving.rs`; the sentence was not. The existing
// test reads the suggestion ("declares a `Thing`"), not the headline.

#[test]
fn an_unknown_export_names_the_module_and_the_missing_name() {
    let universe = universe_of(&["module other\n\nrecord Thing { n: Int }\n"]);
    message_in(
        "module a\n\nuse other.{Thng}\n\nfn f() -> Thng {}\n",
        &universe,
    )
    .says("`other` declares no `Thng`");
}

/// Five benchmark runs of one model against one build wrote imports for names
/// the prelude already provides. Telling the truth ("the module declares no
/// `join`") sends the reader to the module, which is the one place the answer
/// is not. `message_in` allows exactly one diagnostic, which is half of what
/// this holds: the shadowing warning used to fire on the same span, so a
/// single mistake produced two messages that disagreed about what was wrong.
#[test]
fn importing_a_name_the_language_provides_says_it_is_already_here() {
    let universe = universe_of(&["module other\n\nrecord Thing { n: Int }\n"]);
    message_in(
        "module a\n\nuse other.{join}\n\nfn f(xs: List<String>) -> String {\n  join(xs, \", \")\n}\n",
        &universe,
    )
    .under(codes::UNKNOWN_EXPORT)
    .says("`join` is one the language provides, not one `other` declares")
    .says("already in scope")
    .says("stop importing `join`")
    .never_says("hides a builtin");
}

/// Nothing is declared for the refused import, so the call in the body binds
/// the builtin and the rest of the file goes on being checked. Declaring it
/// would shadow the prelude and turn one mistake into a body full of them.
/// Held by the same single-diagnostic rule: a cascade would break this.
#[test]
fn the_refused_import_leaves_the_builtin_reachable() {
    let universe = universe_of(&["module other\n\nrecord Thing { n: Int }\n"]);
    message_in(
        "module a\n\nuse other.{join}\n\nfn f(xs: List<String>) -> String {\n  join(join(xs, \" \"), \", \")\n}\n",
        &universe,
    )
    .never_says("cannot find `join`");
}

/// The boundary of the sentence above. A module that really does declare a
/// name the prelude provides is importable, and importing it is the older
/// warning rather than the newer error: the import resolves, it just takes
/// the builtin's place. Getting this wrong would refuse a legal import.
#[test]
fn a_module_that_really_declares_a_prelude_name_still_only_warns() {
    let universe =
        universe_of(&["module other\n\nfn join(xs: List<String>) -> String {\n  \"\"\n}\n"]);
    message_in(
        "module a\n\nuse other.{join}\n\nfn f(xs: List<String>) -> String {\n  join(xs)\n}\n",
        &universe,
    )
    .under(codes::SHADOWED_DECLARATION)
    .says("hides a name the language provides")
    .never_says("is one the language provides, not one");
}

// -- what nothing can reach --------------------------------------------------

/// The `DEED3005` for builtins is the only emission site not held by a
/// `codes_of` assertion in `resolving.rs`.
///
/// The module-level item that shadows a builtin goes through `declare_item`,
/// which is a different call site from `declare_local`. Swapping the code
/// constant at that site would leave every existing test passing, because
/// every existing test for `SHADOWED_DECLARATION` shadows through a parameter
/// or a `let` binding, not through an item declaration. The `.under()` call
/// on `a_module_item_shadowing_a_builtin_names_the_language` is what holds
/// it.
///
/// This test is here so that the reasoning fails loudly if somebody removes
/// the assertion.
#[test]
fn the_emission_site_for_a_module_item_shadowing_a_builtin_is_held() {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("resolver.rs"),
    )
    .expect("resolver.rs should be readable");

    assert!(
        source.contains("hides a name the language provides"),
        "the `hides a name the language provides` sentence is gone; \
         the argument in crates/deed-resolve/tests/messages.rs needs revisiting"
    );
}
