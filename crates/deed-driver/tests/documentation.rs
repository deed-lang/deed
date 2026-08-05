//! The counts and lists the documents state, checked against what is there.
//!
//! Five sentences in the design documents were wrong at once when this was
//! written, and every one of them had been right at the commit that wrote it:
//! the prelude grew, `Io` grew from six operations to ten, `examples/` grew
//! from eleven files to eighteen, a construct that used to be fiction started
//! parsing, and a walk the README describes in the present tense stopped
//! existing. Prose does not fail a build, so nothing noticed.
//!
//! What is checkable here is narrow on purpose: a list of names the compiler
//! also holds, or a number the repository can count. An argument is not
//! checkable and is not attempted. The point is that the claims which go stale
//! are almost always the countable ones, because those are the claims a later
//! change quietly falsifies without touching the sentence.
//!
//! The documents are not the only prose that does this. A header comment on a
//! corpus file is the first thing read after the README, and two of them were
//! wrong at once: `lists.deed` said a generic type could not be declared while
//! three files in the same directory declared one, and `transfer.deed` said
//! every design document referred back to it when two of the five did. Those
//! are checked here too.
//!
//! `editors/` is prose about this compiler as well, and most of what holds it
//! is deliberately not here. The keywords a TextMate grammar paints are
//! compared with the lexer's in `crates/deed-parser/tests/grammar.rs`, and the
//! list of what the language server answers is compared with what it
//! advertises in `crates/deed-lsp/tests/session.rs`. Each sits beside the
//! thing it reads, because this crate knows nothing about a grammar or a
//! server and a test here that made it know would point a dependency
//! backwards. What is left over is the sentence in `editors/README.md` about
//! the modules that ship inside the compiler, and those are names this crate
//! does hold, so that one is checked below.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use deed_ast::{BinaryOp, Item};
use deed_diagnostics::SourceMap;
use deed_interp::{Program, PropertyConfig, run_properties, run_tests};
use deed_resolve::{IO_OPERATIONS, PRELUDE};

/// The repository root, from this crate's manifest.
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(relative: &str) -> String {
    let path = root().join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} should be there", path.display()))
}

fn normalize(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(part) => out.push(part),
            Component::RootDir => out.push(component.as_os_str()),
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
        }
    }
    out
}

fn relative_to_root(path: &Path) -> String {
    let root = normalize(root());
    let path = normalize(path.to_path_buf());
    path.strip_prefix(root)
        .unwrap_or(path.as_path())
        .to_string_lossy()
        .replace('\\', "/")
}

/// The text between two markers, so a check names the paragraph it is about.
fn between<'a>(text: &'a str, from: &str, to: &str) -> &'a str {
    let start = text
        .find(from)
        .unwrap_or_else(|| panic!("the sentence starting {from:?} has moved or been rewritten"));
    let rest = &text[start..];
    let end = rest
        .find(to)
        .unwrap_or_else(|| panic!("the sentence starting {from:?} no longer reaches {to:?}"));
    &rest[..end]
}

/// Every `` `name` `` in a stretch of prose, in the order it is written.
fn backticked(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('`') else { break };
        found.push(rest[..close].to_string());
        rest = &rest[close + 1..];
    }
    found
}

/// One English enumeration of backticked names, read from the start of `text`:
/// `` `a` ``, or `` `a` and `b` ``, or `` `a`, `b` and `c` ``.
///
/// [`backticked`] takes every name in a stretch of prose, which only works
/// where the stretch is nothing but the list. The paragraphs that write out a
/// library go on to say something about it, in backticks, in the same
/// sentence, so a list has to end where the English says it ends: at the first
/// thing that is not a comma, the word `and`, or the line the paragraph
/// happened to wrap at.
///
/// A rewording therefore drops the rest of the names rather than reaching past
/// them. That is the safe direction, but it means a missing name and a name
/// this stopped short of look the same from here, so a caller reporting one
/// has to say it could be either and show how far it got.
fn enumerated(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(body) = rest.strip_prefix('`') {
        let Some(close) = body.find('`') else { break };
        found.push(body[..close].to_string());
        rest = &body[close + 1..];

        let mut next = rest.strip_prefix(',').unwrap_or(rest);
        next = next.trim_start_matches([' ', '\n']);
        next = next.strip_prefix("and").unwrap_or(next);
        next = next.trim_start_matches([' ', '\n']);
        if next.len() == rest.len() {
            break;
        }
        rest = next;
    }
    found
}

/// How the documents write a small number, because they write them as words.
///
/// A table rather than a crate, and it stops where the documents stop. A count
/// that outgrows it should be read as a sign that whatever is being counted is
/// no longer the kind of thing to state in a sentence.
fn spelled(n: usize) -> &'static str {
    const WORDS: [&str; 121] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
        "twenty-one",
        "twenty-two",
        "twenty-three",
        "twenty-four",
        "twenty-five",
        "twenty-six",
        "twenty-seven",
        "twenty-eight",
        "twenty-nine",
        "thirty",
        "thirty-one",
        "thirty-two",
        "thirty-three",
        "thirty-four",
        "thirty-five",
        "thirty-six",
        "thirty-seven",
        "thirty-eight",
        "thirty-nine",
        "forty",
        "forty-one",
        "forty-two",
        "forty-three",
        "forty-four",
        "forty-five",
        "forty-six",
        "forty-seven",
        "forty-eight",
        "forty-nine",
        "fifty",
        "fifty-one",
        "fifty-two",
        "fifty-three",
        "fifty-four",
        "fifty-five",
        "fifty-six",
        "fifty-seven",
        "fifty-eight",
        "fifty-nine",
        "sixty",
        "sixty-one",
        "sixty-two",
        "sixty-three",
        "sixty-four",
        "sixty-five",
        "sixty-six",
        "sixty-seven",
        "sixty-eight",
        "sixty-nine",
        "seventy",
        "seventy-one",
        "seventy-two",
        "seventy-three",
        "seventy-four",
        "seventy-five",
        "seventy-six",
        "seventy-seven",
        "seventy-eight",
        "seventy-nine",
        "eighty",
        "eighty-one",
        "eighty-two",
        "eighty-three",
        "eighty-four",
        "eighty-five",
        "eighty-six",
        "eighty-seven",
        "eighty-eight",
        "eighty-nine",
        "ninety",
        "ninety-one",
        "ninety-two",
        "ninety-three",
        "ninety-four",
        "ninety-five",
        "ninety-six",
        "ninety-seven",
        "ninety-eight",
        "ninety-nine",
        "one hundred",
        "one hundred and one",
        "one hundred and two",
        "one hundred and three",
        "one hundred and four",
        "one hundred and five",
        "one hundred and six",
        "one hundred and seven",
        "one hundred and eight",
        "one hundred and nine",
        "one hundred and ten",
        "one hundred and eleven",
        "one hundred and twelve",
        "one hundred and thirteen",
        "one hundred and fourteen",
        "one hundred and fifteen",
        "one hundred and sixteen",
        "one hundred and seventeen",
        "one hundred and eighteen",
        "one hundred and nineteen",
        "one hundred and twenty",
    ];
    WORDS
        .get(n)
        .unwrap_or_else(|| panic!("{n} is past what these documents spell out"))
}

fn std_api_index() -> String {
    let modules: Vec<&str> = deed_driver::shipped_modules().collect();
    let listed: String = modules
        .iter()
        .map(|module| {
            let name = module.trim_start_matches("std/");
            format!("- [`{module}`](std/{name}.md)\n")
        })
        .collect();

    format!(
        "# Standard library API\n\n\
         These pages are generated from the shipped module declarations under `std/` and the module tests that name each function. Each page records the declaration the compiler ships, the row variables and declared row it carries, and the example lines those tests exercise.\n\n\
         ## Pages\n\n\
         {listed}\n\
         ## User modules\n\n\
         `deed doc <path>...` writes the same page for any module, to standard output. There are no visibility modifiers here, so every declaration is API and there is nothing to decide about what belongs on a page. These pages are checked in only for the shipped modules, because those are the ones a reader has without having written them.\n"
    )
}

/// Every page the shipped modules would generate, from the library that
/// generates them.
///
/// The rendering lives in `deed_driver::docs` because `deed doc` publishes it
/// for any module. What stays here is the part that is a claim about this
/// repository rather than about the language: a shipped module documents every
/// function it declares, and every one of them is named by one of its own
/// tests.
fn shipped_std_api_pages() -> Vec<(String, String)> {
    let mut sources = SourceMap::new();
    let mut ids = Vec::new();
    for module in deed_driver::shipped_modules() {
        let text = deed_driver::shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("<shipped>/{module}.deed"), text.to_string()));
    }

    deed_driver::check_all(&sources, &ids)
        .into_iter()
        .map(|checked| {
            let text = sources.file(checked.file).text().to_string();
            let page = deed_driver::docs::ModuleDocs::of(&checked, &text);

            assert!(
                page.summary.is_some(),
                "`{}` needs a file header comment so the generated page can introduce it",
                page.module
            );
            assert!(
                !page.functions.is_empty(),
                "`{}` declares no function, so there is nothing to document",
                page.module
            );
            for function in &page.functions {
                assert!(
                    function.description.is_some(),
                    "`{}` in `{}` needs a comment block above it so the generated std API page can say what it does",
                    function.name,
                    page.module
                );
                assert!(
                    !function.examples.is_empty(),
                    "`{}` should be named by one of its tests before it reaches the std API page",
                    function.name
                );
            }

            let short = page.module.trim_start_matches("std/").to_string();
            (
                format!("docs/std/{short}.md"),
                page.to_markdown(&format!("/std/{short}.deed")),
            )
        })
        .collect()
}
fn examples() -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(root().join("examples"))
        .expect("examples/ should be there")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "deed"))
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no .deed files found under examples/");
    found
}

fn public_docs() -> Vec<String> {
    fn walk(at: &Path, found: &mut Vec<String>) {
        for entry in std::fs::read_dir(at)
            .unwrap_or_else(|_| panic!("{} should be there", at.display()))
            .filter_map(Result::ok)
        {
            let path = entry.path();
            let hidden_or_generated = path.file_name().is_some_and(|name| {
                name == ".git"
                    || name == ".github"
                    || name == "target"
                    || name.to_string_lossy().starts_with("mutants.out")
            });
            if hidden_or_generated {
                continue;
            }
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "md") {
                found.push(relative_to_root(&path));
            }
        }
    }

    let mut found = Vec::new();
    walk(&root(), &mut found);
    found.sort();
    assert!(
        !found.is_empty(),
        "no public markdown files found under the repository root"
    );
    found
}

fn indexed_public_docs() -> Vec<String> {
    public_docs()
        .into_iter()
        .filter(|doc| !doc.starts_with("design/decisions/") || doc == "design/decisions/README.md")
        .collect()
}

fn markdown_links(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("](") {
        rest = &rest[open + 2..];
        let Some(close) = rest.find(')') else { break };
        let target = &rest[..close];
        let target = target.split('#').next().unwrap_or("");
        if !target.is_empty()
            && !target.starts_with('#')
            && !target.contains("://")
            && !target.starts_with("mailto:")
        {
            found.push(target.to_string());
        }
        rest = &rest[close + 1..];
    }
    found
}

fn resolved_from(doc: &str, target: &str) -> PathBuf {
    let base = root()
        .join(doc)
        .parent()
        .expect("a document should have a parent")
        .to_path_buf();
    normalize(base.join(target))
}

fn named_paths(text: &str, prefix: &str, extension: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(prefix) {
        rest = &rest[at..];
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || matches!(c, '/' | '_' | '-' | '.')))
            .unwrap_or(rest.len());
        let path = &rest[..end];
        if path.ends_with(extension) && !found.iter().any(|seen| seen == path) {
            found.push(path.to_string());
        }
        rest = &rest[end..];
    }
    found
}

/// The one paragraph that writes out the whole prelude, in order.
///
/// It carries the names, the built-in effect, its operations and the second
/// effect, so one comparison covers every list a reader could be misled by.
#[test]
fn the_prelude_the_syntax_document_lists_is_the_prelude_there_is() {
    let syntax = read("design/02-syntax.md");
    let paragraph = between(&syntax, "**The prelude is ", "Everything else");

    let mut expected: Vec<String> = PRELUDE.iter().map(|name| name.to_string()).collect();
    expected.push("Io".to_string());
    expected.extend(IO_OPERATIONS.iter().map(|name| name.to_string()));
    expected.push("Diverge".to_string());

    assert_eq!(backticked(paragraph), expected);
}

#[test]
fn the_number_of_prelude_names_is_the_number_it_says() {
    let syntax = read("design/02-syntax.md");
    let claim = format!(
        "**The prelude is {} names and two effects:**",
        spelled(PRELUDE.len())
    );
    assert!(
        syntax.contains(&claim),
        "the prelude has {} names, so the sentence should read {claim:?}",
        PRELUDE.len()
    );
}

/// The string paragraph answers the question this issue was about: what a
/// string contains, what `length` counts, and what `split(s, "")` walks.
#[test]
fn the_syntax_document_states_what_string_length_means() {
    let syntax = read("design/02-syntax.md");
    let paragraph = between(
        &syntax,
        "The runtime holds a `String` as UTF-8 text.",
        "It carries four more for taking a string apart and putting one back together:",
    );

    assert!(paragraph.contains("UTF-8 text"), "{paragraph}");
    assert!(paragraph.contains("Unicode scalar values"), "{paragraph}");
    assert!(
        paragraph.contains("not ten bytes and not ten grapheme clusters"),
        "{paragraph}"
    );
    assert!(
        paragraph.contains("`split(s, \"\")`"),
        "the paragraph should name the empty-separator split directly:\n{paragraph}"
    );
}

/// The capability document lists the operations a second time, which is a
/// second place to go stale and did.
#[test]
fn the_operations_the_capability_document_lists_are_the_ones_there_are() {
    let capabilities = read("design/04-capabilities.md");
    let sentence = between(
        &capabilities,
        "What exists is one built-in\neffect,",
        "and a `System` carrying",
    );

    let mut expected = vec!["Io".to_string()];
    expected.extend(IO_OPERATIONS.iter().map(|name| name.to_string()));
    assert_eq!(backticked(sentence), expected);
}

#[test]
fn the_odd_operation_is_odd_among_however_many_there_are() {
    let capabilities = read("design/04-capabilities.md");
    let claim = format!("the odd operation of the {}", spelled(IO_OPERATIONS.len()));
    assert!(
        capabilities.contains(&claim),
        "there are {} operations, so the sentence should read {claim:?}",
        IO_OPERATIONS.len()
    );
}

/// The security policy repeats the operation list as part of the boundary.
#[test]
fn the_security_policy_lists_the_io_operations_that_exist() {
    let security = read("SECURITY.md");
    let sentence = between(
        &security,
        "The interpreter and backend expose one built-in effect, `Io`, with ",
        ".\n",
    );
    let mut expected = vec!["Io".to_string()];
    expected.extend(IO_OPERATIONS.iter().map(|name| name.to_string()));
    assert_eq!(backticked(sentence), expected);
}

/// What ships inside the compiler, where the document says what ships.
///
/// The table is a list of names the compiler holds, so it is the checkable
/// kind of claim. It is also the kind that goes stale quietly: adding a module
/// is one line in one file and nothing about that line has any reason to look
/// at a paragraph in a design document.
#[test]
fn the_modules_that_ship_are_the_ones_the_syntax_document_names() {
    let syntax = read("design/02-syntax.md");
    let sentence = between(&syntax, "modules ship", ", and `crates/");

    let shipping: Vec<String> = deed_driver::shipped_modules().map(str::to_string).collect();
    assert_eq!(backticked(sentence), shipping);

    let claim = format!("{} modules ship", spelled(shipping.len()));
    let claim = claim[..1].to_uppercase() + &claim[1..];
    assert!(
        syntax.contains(&claim),
        "{} modules ship, so the sentence should read {claim:?}",
        shipping.len()
    );
}

#[test]
fn the_examples_the_readme_names_are_the_examples_there_are() {
    let readme = read("README.md");
    let named: Vec<String> = {
        let mut found: Vec<String> = Vec::new();
        let mut rest = readme.as_str();
        while let Some(at) = rest.find("examples/") {
            rest = &rest[at + "examples/".len()..];
            let end = rest
                .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
                .unwrap_or(rest.len());
            let name = &rest[..end];
            if name.ends_with(".deed") && !found.iter().any(|seen| seen == name) {
                found.push(name.to_string());
            }
        }
        found.sort();
        found
    };

    assert_eq!(
        named,
        examples(),
        "the README should name every example and no others"
    );
}

#[test]
fn the_files_the_timing_paragraph_counted_are_the_files_there_are() {
    let principles = read("design/01-principles.md");
    let claim = format!(
        "checking the {} files in `examples/`",
        spelled(examples().len())
    );
    assert!(
        principles.contains(&claim),
        "there are {} examples, so the sentence should read {claim:?}",
        examples().len()
    );
}

/// The one place the README writes out what a corpus file contains. It is the
/// claim the file makes about itself, so it should say all of it.
#[test]
fn the_library_the_readme_lists_is_the_library_that_is_written() {
    let readme = read("README.md");
    let sentence = between(
        &readme,
        "list library written in Deed:",
        "none of them known",
    );

    let written: Vec<String> = read("std/list.deed")
        .lines()
        .filter_map(|line| line.strip_prefix("fn "))
        .map(|rest| {
            rest.chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect()
        })
        .collect();

    assert!(!written.is_empty(), "std/list.deed declares nothing");
    assert_eq!(backticked(sentence), written);
}

/// Every function each module that ships declares, in declaration order.
///
/// Off the parse tree rather than off the text, because what the prose is
/// about is what a program that imports the module can call, and the parser is
/// what decides that. They are checked together because they are allowed to
/// import each other.
fn functions_that_ship() -> Vec<(String, Vec<String>)> {
    let modules: Vec<&str> = deed_driver::shipped_modules().collect();
    assert!(
        !modules.is_empty(),
        "no module ships inside the compiler, so there is nothing for the syntax document to be wrong about"
    );

    let mut sources = SourceMap::new();
    let mut ids = Vec::new();
    for module in &modules {
        let text = deed_driver::shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("<shipped>/{module}.deed"), text.to_string()));
    }

    let mut found = Vec::new();
    for (module, checked) in modules.iter().zip(deed_driver::check_all(&sources, &ids)) {
        let declared: Vec<String> = checked
            .module
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Function(function) => Some(function.sig.name.name.to_string()),
                _ => None,
            })
            .collect();
        assert!(
            !declared.is_empty(),
            "`{module}` ships and declares no function at all"
        );
        found.push(((*module).to_string(), declared));
    }
    found
}

/// The three sentences in the syntax document that write out what a module
/// that ships contains.
///
/// One of these lists was already held: the README's copy of `std/list` is
/// checked above. The other two copies of that same list were not, and
/// `std/string` and `std/table` were written down nowhere a change had to
/// pass. A module gains a function in one file, and no line of that file has
/// any reason to look at a design document.
///
/// Order is held rather than membership. A paragraph that lists what a module
/// has is read down the page against the module, so the two orders agreeing is
/// most of what makes it readable, and a set comparison passes on a sentence
/// nobody can follow. That cost one edit: `std/string.deed` declared
/// `pad_right` first and the sentence read `pad_left` first, and the file was
/// reordered rather than the check weakened. The note above `pad_left` says so.
#[test]
fn the_functions_the_syntax_document_lists_are_the_ones_the_modules_declare() {
    let syntax = read("design/02-syntax.md");

    for (module, declared) in functions_that_ship() {
        let marker = format!("`{module}` has ");
        let paragraph = between(&syntax, &marker, "\n\n");
        let named = enumerated(&paragraph[marker.len()..]);
        assert!(
            !named.is_empty(),
            "the sentence starting {marker:?} names no function, so it has been reworded into something this cannot read"
        );

        for name in &declared {
            assert!(
                named.contains(name),
                "`{module}` declares `{name}` and the sentence in design/02-syntax.md does not list it: either the name is missing, or the enumeration is worded in a way this stops reading before reaching it. What it read was {named:?}"
            );
        }
        for name in &named {
            assert!(
                declared.contains(name),
                "design/02-syntax.md says `{module}` has `{name}` and no such function is declared there"
            );
        }
        assert_eq!(
            named, declared,
            "design/02-syntax.md reads {module} out in a different order than the module declares it"
        );
    }
}

/// The one claim `editors/README.md` makes that this crate can answer.
///
/// It tells an editor author that the modules inside the compiler are checked
/// alongside their own files, and names one to show what that looks like. The
/// name is one the compiler also holds, which is the checkable kind of claim,
/// and it is the reason this file reaches into `editors/` at all. What the
/// rest of that document says about the language server is held beside the
/// language server; the note at the top of this file says where.
#[test]
fn the_module_the_editor_guide_names_is_one_that_ships() {
    let shipping: Vec<&str> = deed_driver::shipped_modules().collect();
    assert!(!shipping.is_empty(), "no module ships inside the compiler");

    let guide = read("editors/README.md");
    let sentence = between(&guide, "The modules that ship inside the compiler", "\n\n");
    let named: Vec<String> = backticked(sentence)
        .into_iter()
        .filter_map(|item| item.strip_prefix("use ").map(str::to_string))
        .collect();
    assert!(
        !named.is_empty(),
        "the paragraph telling an editor author about the modules that ship names none of them"
    );

    for module in &named {
        assert!(
            shipping.contains(&module.as_str()),
            "editors/README.md tells an editor author that `use {module}` is not an error and no such module ships"
        );
    }
}

/// The paragraph whose whole job is to say which parts of the document are
/// fiction. It said four and listed three, because generic application was
/// deleted from the list when it started parsing and the number was not.
#[test]
fn the_constructs_that_do_not_parse_are_counted_where_they_are_listed() {
    let syntax = read("design/02-syntax.md");
    let after = syntax
        .split_once("constructs appear in the illustrations and do not parse at\nall")
        .expect("the paragraph naming what does not parse has moved")
        .1;
    let listed = after
        .lines()
        .skip_while(|line| !line.starts_with("- "))
        .take_while(|line| line.starts_with("- "))
        .count();

    let claim = format!("{} constructs appear in the illustrations", spelled(listed));
    let claim = claim[..1].to_uppercase() + &claim[1..];
    assert!(
        syntax.contains(&claim),
        "{listed} are listed, so the sentence should read {claim:?}"
    );
}

/// The design documents are read as a set, so a missing one would make every
/// check above pass by having nothing to disagree with.
#[test]
fn the_documents_these_read_are_all_there() {
    for name in [
        "design/00-motivation.md",
        "design/01-principles.md",
        "design/02-syntax.md",
        "design/03-effects.md",
        "design/04-capabilities.md",
        "design/05-backend.md",
        "README.md",
        "editors/README.md",
    ] {
        let path: &Path = &root().join(name);
        assert!(path.is_file(), "{name} should be there");
    }
}

/// Public documentation should not point at files that are gone.
///
/// The design documents are already held to concrete names and counts, but new
/// public prose can still grow broken links without touching one of the checks
/// above. That is a quieter version of the same failure mode: the sentence was
/// right when it was written, the repository moved, and nothing reading it had
/// reason to look back. Relative links are the objective part, so those are
/// what fail here.
#[test]
fn the_public_documents_link_to_things_that_are_there() {
    for doc in public_docs() {
        for target in markdown_links(&read(&doc)) {
            let path = resolved_from(&doc, &target);
            assert!(
                path.exists(),
                "{doc} links to {target:?}, which resolves to {} and is not there",
                relative_to_root(&path)
            );
        }
    }
}

/// Public documentation that lands should be findable from other public prose.
///
/// The existing ratchets only hold documents once some earlier document already
/// points at them. A new README under a public directory can therefore land and
/// then quietly drift because nothing names it, which is the same as a missing
/// chapter in an index: the file exists, but a reader is not led to it and no
/// future edit has to pass by it. What counts as indexed here is deliberately
/// narrow and mechanical: another public document has a relative markdown link
/// to it, either directly or by linking the directory that carries its README.
#[test]
fn the_public_documents_are_indexed_somewhere() {
    let docs = indexed_public_docs();
    let public: BTreeSet<String> = docs.iter().cloned().collect();
    let mut indexed: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for doc in &docs {
        for target in markdown_links(&read(doc)) {
            let path = resolved_from(doc, &target);
            let targets: Vec<String> = if path.is_dir() {
                let readme = path.join("README.md");
                if readme.is_file() {
                    vec![relative_to_root(&readme)]
                } else {
                    Vec::new()
                }
            } else if path.is_file() {
                vec![relative_to_root(&path)]
            } else {
                Vec::new()
            };

            for indexed_doc in targets {
                if indexed_doc != *doc && public.contains(&indexed_doc) {
                    indexed.entry(indexed_doc).or_default().push(doc.clone());
                }
            }
        }
    }

    for doc in docs {
        if doc == "README.md" {
            continue;
        }
        assert!(
            indexed.contains_key(&doc),
            "{doc} is public documentation and no other public document links to it"
        );
    }
}

/// File paths in public prose should still name files that exist.
///
/// Markdown links are not the only place a document can go stale. The example
/// corpus is also cited in plain prose and transcripts, and those references
/// are exactly the kind that survive a rename because nothing reading them has
/// to touch the sentence. This only checks paths a reader could independently
/// verify from the tree: example and shipped-library `.deed` files, and other
/// markdown documents named by path.
#[test]
fn the_public_documents_name_files_that_are_there() {
    for doc in public_docs() {
        let text = read(&doc);
        for path in named_paths(&text, "examples/", ".deed")
            .into_iter()
            .chain(named_paths(&text, "std/", ".deed"))
            .chain(named_paths(&text, "design/", ".md"))
        {
            let full = root().join(&path);
            assert!(
                full.is_file(),
                "{doc} names `{path}` and no such file exists"
            );
        }
    }
}

/// Every crate under `crates/`, by directory name.
fn crates() -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(root().join("crates"))
        .expect("crates/ should be there")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.join("Cargo.toml").is_file())
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .collect();
    found.sort();
    assert!(!found.is_empty(), "no crates found under crates/");
    found
}

/// Whether anything under one crate's `src/` contains any of `needles`.
fn crate_mentions(name: &str, needles: &[&str]) -> bool {
    fn walk(at: &Path, needles: &[&str]) -> bool {
        let Ok(entries) = std::fs::read_dir(at) else {
            return false;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                if walk(&path, needles) {
                    return true;
                }
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && let Ok(text) = std::fs::read_to_string(&path)
                && needles.iter().any(|needle| text.contains(needle))
            {
                return true;
            }
        }
        false
    }

    walk(&root().join("crates").join(name).join("src"), needles)
}

/// The two counts the answer about `Result` and `List` rests on.
///
/// The whole argument for leaving them built in is that the type is named in
/// few crates and the syntax over it is named in most of them, so if that
/// stops being true the answer is wrong and nothing else would say so. This
/// is the one claim in these documents where a refactor moving a match arm is
/// exactly the event that should make somebody read the paragraph again, and
/// it has already happened once: the backend gave a list a second meaning,
/// its layout, and the count went from one crate to three.
#[test]
fn what_holds_result_in_the_language_is_counted_where_it_is_claimed() {
    let all = crates();

    let types: Vec<&String> = all
        .iter()
        .filter(|name| crate_mentions(name, &["Ty::Result", "Ty::List"]))
        .collect();
    let syntax_doc = read("design/02-syntax.md");
    let named = format!("named in {} crates", spelled(types.len()));
    assert!(
        syntax_doc.contains(&named),
        "the type is named in {types:?}, so the paragraph should say {named:?}"
    );

    // `?` and the outcome an `ensures` clause is keyed by. Both are syntax
    // over `Result` rather than the type, which is the point being made.
    let syntax: Vec<&String> = all
        .iter()
        .filter(|name| crate_mentions(name, &["Expr::Try", "Outcome::"]))
        .collect();

    let claim = format!(
        "those are in {} of the {} crates",
        spelled(syntax.len()),
        spelled(all.len())
    );
    assert!(
        syntax_doc.contains(&claim),
        "{} crates name `?` or an outcome ({syntax:?}) out of {}, so the sentence should read {claim:?}",
        syntax.len(),
        all.len()
    );

    // `design/refusals.md` makes the same argument in its own words and was
    // the copy nothing read: it went on saying nineteen after a twentieth
    // crate arrived. Whitespace is collapsed because the sentence wraps.
    let refusals = read("design/refusals.md")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let same = format!(
        "reaches into {} of the {} crates in this workspace",
        spelled(syntax.len()),
        spelled(all.len())
    );
    assert!(
        refusals.contains(&same),
        "design/refusals.md makes the same claim and should read {same:?}"
    );
}

/// The number in the README's own transcript of `deed test examples/`.
///
/// The transcript is the first thing anybody sees and it is the claim that is
/// easiest to leave behind, because every change that adds a test to the
/// corpus falsifies it and none of them has any reason to look. It said 84
/// for the eighteen tests it took to notice.
///
/// It can also fall, which is a different thing to be wrong about. A library
/// moving out of `examples/` takes its tests with it and the corpus honestly
/// runs fewer, so the paragraph under the transcript says where they went and
/// this checks that too: what the libraries run, and that the two together are
/// more than the transcript used to report.
#[test]
fn the_readme_reports_the_number_of_tests_the_corpus_runs() {
    let mut sources = SourceMap::new();
    let mut ids = Vec::new();
    for name in examples() {
        let path = root().join("examples").join(&name);
        let text = std::fs::read_to_string(&path).expect("an example should be readable");
        ids.push(sources.add(format!("examples/{name}"), text));
    }

    // The corpus imports a module that ships inside the compiler, so it goes
    // in last and the count below is taken off the first `subject` of them.
    // That is the same split the command line makes: a library is context,
    // and its tests are not the ones the transcript reports.
    let subject = ids.len();
    for module in deed_driver::shipped_modules() {
        let text = deed_driver::shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("<shipped>/{module}.deed"), text.to_string()));
    }

    // Together, because they import each other, and every file runs only its
    // own tests, which is what `deed test examples/` does.
    let checks = deed_driver::check_all(&sources, &ids);
    let mut program = Program::new();
    for checked in &checks {
        assert!(
            !checked.has_errors(),
            "{} should check cleanly",
            sources.file(checked.file).name()
        );
        program.add(
            checked.file,
            &checked.module,
            &checked.resolutions,
            checked.guards(),
            checked.rows(),
            checked.operators(),
        );
    }

    let mut passed = 0;
    for checked in &checks[..subject] {
        for outcome in run_tests(&program, checked.file) {
            assert!(
                outcome.failure.is_none(),
                "`{}` should pass in `{}`",
                outcome.name,
                sources.file(checked.file).name()
            );
            passed += 1;
        }
        // A property is a line in that transcript too, so it is one of the
        // numbers being claimed.
        for property in run_properties(
            &program,
            checked.file,
            &checked.module,
            &checked.resolutions,
            PropertyConfig::default(),
        ) {
            assert!(
                property.failure.is_none(),
                "`{}` should hold in `{}`",
                property.function,
                sources.file(checked.file).name()
            );
            passed += 1;
        }
    }

    assert!(passed > 0, "the corpus ran no tests at all");
    let claim = format!("{passed} passed, 0 failed");
    assert!(
        read("README.md").contains(&claim),
        "the corpus runs {passed} tests, so the transcript should read {claim:?}"
    );

    // The other half of the same paragraph, and the reason the number above is
    // allowed to fall. Seven of the tests it used to count are in `std/table`
    // now, and a library that ships is context rather than subject, so the
    // corpus stopped running them. With only the corpus number written down,
    // moving seven tests into a library and deleting seven tests read the same
    // from outside, which is the sentence this checks.
    let mut shipped = 0;
    for checked in &checks[subject..] {
        for outcome in run_tests(&program, checked.file) {
            assert!(
                outcome.failure.is_none(),
                "`{}` should pass in `{}`",
                outcome.name,
                sources.file(checked.file).name()
            );
            shipped += 1;
        }
    }

    let carried = format!("carry {} tests of their own", spelled(shipped));
    assert!(
        read("README.md").contains(&carried),
        "the modules that ship run {shipped} tests, so the README should read {carried:?}"
    );

    // And the arithmetic the paragraph is asking to be believed. The number it
    // says the transcript used to report is read back out of it, so what is
    // compared against is what a reader was told rather than a constant here.
    let readme = read("README.md");
    let before: usize = between(&readme, "That number used to be ", ".")
        .trim_start_matches("That number used to be ")
        .parse()
        .expect("the README should say what the transcript used to report");
    assert!(
        passed + shipped > before,
        "{passed} in the corpus and {shipped} in the libraries is not more than the {before} the README says used to run, so tests went missing rather than moving"
    );
}

// -- what the corpus says about itself -------------------------------------

/// Every generic type the corpus declares, as `(file, name)`.
///
/// Read off the parse tree rather than the text, because the claim being
/// checked is about what the language can express and the parser is the thing
/// that decides that.
fn generic_types_declared() -> Vec<(String, String)> {
    let mut sources = SourceMap::new();
    let mut ids = Vec::new();
    for name in examples() {
        let text = read(&format!("examples/{name}"));
        ids.push(sources.add(format!("examples/{name}"), text));
    }

    let mut found = Vec::new();
    for checked in deed_driver::check_all(&sources, &ids) {
        let file = sources.file(checked.file).name().to_string();
        for item in &checked.module.items {
            let name = match item {
                Item::Record(record) if !record.generics.is_empty() => &record.name.name,
                Item::Choice(choice) if !choice.generics.is_empty() => &choice.name.name,
                _ => continue,
            };
            found.push((file.clone(), name.to_string()));
        }
    }
    found
}

/// The paragraph in `lists.deed` explaining why `List` is built in.
///
/// It used to say a generic type could not be declared, which was the stated
/// reason for the builtin and had been true when written. Three files in the
/// same directory declare one now, so the sentence argued the losing side of a
/// question that had already been decided the other way. Whoever adds a fourth
/// should be the one to read it again.
#[test]
fn the_generic_types_the_list_example_points_at_are_the_ones_declared() {
    let declared = generic_types_declared();
    assert!(
        !declared.is_empty(),
        "the corpus declares no generic type, so `lists.deed` should go back to saying they cannot be declared"
    );

    let header = read("examples/lists.deed");
    let paragraph = between(&header, "`List` is built in", "module ");

    for (file, name) in &declared {
        assert!(
            paragraph.contains(&format!("`{name}`")),
            "`{file}` declares the generic type `{name}` and the paragraph in lists.deed does not name it"
        );
        assert!(
            paragraph.contains(file),
            "`{file}` declares a generic type and the paragraph in lists.deed does not name the file"
        );
    }
}

/// How many design documents work through `transfer.deed`.
///
/// The file called itself the running example and said every design document
/// referred back to it. Two of the five did. This is the one claim in the
/// repository made from the corpus about the documents rather than the other
/// way round, which is why nothing above it was ever going to catch it.
#[test]
fn the_documents_that_work_through_the_running_example_are_the_ones_that_do() {
    let mut documents: Vec<String> = std::fs::read_dir(root().join("design"))
        .expect("design/ should be there")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .filter_map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .collect();
    documents.sort();
    assert!(!documents.is_empty(), "no .md files found under design/");

    let working: Vec<String> = documents
        .iter()
        .filter(|name| read(&format!("design/{name}")).contains("transfer"))
        .cloned()
        .collect();

    let header = read("examples/transfer.deed");
    let claim = format!(
        "{} of the {} design documents work through it",
        spelled(working.len()),
        spelled(documents.len())
    );
    assert!(
        header.contains(&claim),
        "{} of {} design documents mention it ({working:?}), so the header should read {claim:?}",
        working.len(),
        documents.len()
    );
    for name in &working {
        assert!(
            header.contains(&format!("design/{name}")),
            "design/{name} works through transfer.deed and the header does not name it"
        );
    }
}

/// The generated API reference for the shipped standard library modules.
///
/// The source of truth is the shipped module itself: the declaration, the row
/// and contract it carries, the comment block explaining what it does, and the
/// tests in that same file that name it. A checked-in page is still useful,
/// because it is what GitHub renders, but if that page drifts from the module
/// the point of having a generated reference is gone.
#[test]
fn the_std_api_pages_match_the_shipped_modules_the_compiler_ships() {
    let expected_index = std_api_index();
    let actual_index = read("docs/std.md");
    assert_eq!(
        actual_index, expected_index,
        "docs/std.md should match the generated shipped-module index"
    );

    let pages = shipped_std_api_pages();
    assert!(
        !pages.is_empty(),
        "nothing ships inside the compiler, so there is no std API page to read"
    );

    for (path, expected) in pages {
        let actual = read(&path);
        assert_eq!(
            actual, expected,
            "{path} should match the generated API reference for its shipped module"
        );
    }
}

#[test]
#[ignore = "writes docs/std.md and docs/std/*.md from the shipped modules"]
fn regenerate_the_std_api_pages() {
    let docs = root().join("docs").join("std");
    std::fs::create_dir_all(&docs).expect("docs/std should be writable");
    std::fs::write(root().join("docs/std.md"), std_api_index())
        .expect("docs/std.md should be writable");

    for (path, page) in shipped_std_api_pages() {
        let absolute = root().join(path);
        std::fs::write(&absolute, page)
            .unwrap_or_else(|_| panic!("{} should be writable", absolute.display()));
    }
}

#[test]
fn the_changelog_has_the_sections_the_release_policy_uses() {
    let changelog = read("CHANGELOG.md");
    let required = [
        "## Unreleased",
        "### Programs that used to compile and no longer do",
        "### Language",
        "### Diagnostics",
        "### Standard library",
        "### Tools",
        "### Measurements",
    ];

    for heading in required {
        assert!(
            changelog.contains(heading),
            "CHANGELOG.md should contain {heading:?}, because that is the release-note shape contributors are told to keep"
        );
    }
}

#[test]
fn the_release_workflow_publishes_notes_from_the_changelog() {
    let workflow = read(".github/workflows/release.yml");

    assert!(
        workflow.contains("CHANGELOG.md"),
        "the release workflow should read CHANGELOG.md, because release notes come from the changelog rather than commit titles"
    );
    assert!(
        workflow.contains("--notes-file"),
        "the release workflow should pass explicit notes to gh release create"
    );
    assert!(
        !workflow.contains("--generate-notes"),
        "the release workflow should not ask GitHub to generate notes from commit titles"
    );
}

/// The types every operator is tried against.
///
/// Concrete and declarable inside one module, but nowhere near all of those:
/// `()`, a `choice`, a refined type, a function type and `Result` are all
/// missing, and the measurement below does not need them.
///
/// Two things it does need, and both are held by
/// [`the_probed_types_are_types_and_no_two_are_the_same_one`] rather than left
/// as a sentence here.
///
/// No two entries are one type to the checker. The rule below is "more than
/// one of these and not all of them", so a type that stands in for another
/// already on the list gets counted a second time and an operator with one
/// meaning looks like it has two. `Positive = Int where value > 0` is the near
/// miss: adding it alone would make `%`, `*`, `-` and `/` join the answer.
///
/// And every entry is a type at all. One that no operator takes lengthens the
/// list without anything reaching it, so `==` stops taking all of them and
/// falls into the answer with `!=` beside it. `("T", "")` is that mistake
/// wearing the clothes of a type parameter: nothing declares `T`, so it
/// resolves to nothing and every operator refuses it. A real type parameter
/// cannot be an entry here, because an entry is a type name plus declarations
/// written above the probe and a type parameter is declared on the probe
/// itself. It would not move the answer if it could be, either: the checker
/// takes `==` on one and refuses `<`, `+` and the rest, which is what `Bool`
/// on this list already does.
///
/// This is the hand-written half of the measurement, together with
/// `BinaryOp::ALL` and the four comparisons written out in
/// [`the_types_ordering_takes_are_the_ones_the_syntax_document_names`]. What
/// an operator does with these comes from the checker.
const PROBED_TYPES: [(&str, &str); 5] = [
    ("Int", ""),
    ("String", ""),
    ("Bool", ""),
    ("List<Int>", ""),
    ("Point", "record Point { x: Int }\n\n"),
];

/// Whether a value of type `from` is accepted where `to` is wanted.
///
/// Asked of the checker, like everything else here. Two probed types that
/// answer yes in either direction are one type as far as the count is
/// concerned, since every operator taking one of them takes the other.
fn stands_in_for(from: &str, to: &str) -> bool {
    let mut declarations = String::new();
    for (ty, declaration) in PROBED_TYPES {
        if (ty == from || ty == to) && !declarations.contains(declaration) {
            declarations.push_str(declaration);
        }
    }

    let source =
        format!("module probe\n\n{declarations}fn probe(a: {from}) -> {to} {{\n    a\n}}\n");
    let mut sources = SourceMap::new();
    let file = sources.add("probe.deed", source);
    !deed_driver::check(&sources, file).has_errors()
}

/// The preconditions [`operators_with_two_meanings`] counts under.
fn probed_types_are_sound() {
    for (ty, declaration) in PROBED_TYPES {
        assert!(
            BinaryOp::ALL
                .iter()
                .any(|op| operator_takes(op.as_str(), ty, declaration)),
            "no operator takes a `{ty}`, so it is not a type the checker knows and \
             counting it lengthens PROBED_TYPES without anything reaching it"
        );
    }

    for (from, _) in PROBED_TYPES {
        for (to, _) in PROBED_TYPES {
            if from != to {
                assert!(
                    !stands_in_for(from, to),
                    "a `{from}` is accepted where a `{to}` is wanted, so the two are one \
                     type to the checker and PROBED_TYPES counts it twice"
                );
            }
        }
    }
}

/// The two things the probed types have to be.
///
/// The count below calls an operator ambiguous when it takes more than one
/// probed type and not all of them, which reads the right answer off the
/// checker only while every probed type is a type and no two of them are the
/// same one. Nothing about a list of type names says either, and the list is
/// the obvious thing to extend, so this holds both and fails by name instead
/// of letting the counts demand a number nobody can make true.
#[test]
fn the_probed_types_are_types_and_no_two_are_the_same_one() {
    probed_types_are_sound();
}

/// How the sentence about them opens, in both places that write it.
const TWO_MEANINGS: &str = "operators that mean two things: ";

/// Whether a module doing `a <op> b` over two values of `ty` checks cleanly.
///
/// The value is bound rather than left as a statement, because a statement
/// whose value nobody reads is its own warning and the answer wanted here is
/// about errors.
fn operator_takes(op: &str, ty: &str, declaration: &str) -> bool {
    let source = format!(
        "module probe\n\n{declaration}fn probe(a: {ty}, b: {ty}) -> () {{\n    let _answer = a {op} b\n}}\n"
    );
    let mut sources = SourceMap::new();
    let file = sources.add("probe.deed", source);
    !deed_driver::check(&sources, file).has_errors()
}

/// The types `op` accepts, out of the ones probed.
fn types_taken_by(op: &str) -> Vec<String> {
    let taken: Vec<String> = PROBED_TYPES
        .iter()
        .filter(|(ty, declaration)| operator_takes(op, ty, declaration))
        .map(|(ty, _)| (*ty).to_string())
        .collect();
    assert!(
        !taken.is_empty(),
        "`{op}` took none of the probed types, so either it cannot be written at all \
         or the probe around it is wrong"
    );
    taken
}

/// The operators that mean more than one thing, asked of the checker.
///
/// An operator that takes several of the probed types but not all of them is
/// choosing between meanings. One that takes all of them has a single meaning
/// applied everywhere, which is why `==` is not in the answer: structural
/// equality is total by design, and counting it would turn "two meanings" into
/// "more than one type", which is a different and much duller claim. The rule
/// only reads that way while every probed type is a type and no two of them
/// are the same one, so it says so first rather than reporting a number that
/// came from counting one type twice or from counting a name nothing declares.
///
/// Sorted, and compared as a set. `BinaryOp::ALL` is in the parser's
/// precedence order, which is not an order any sentence would use, so holding
/// it would be holding the wrong thing.
fn operators_with_two_meanings() -> Vec<String> {
    probed_types_are_sound();

    let mut found: Vec<String> = BinaryOp::ALL
        .iter()
        .map(|op| op.as_str())
        .filter(|op| {
            let taken = types_taken_by(op).len();
            taken > 1 && taken < PROBED_TYPES.len()
        })
        .map(str::to_string)
        .collect();
    found.sort();
    assert!(
        !found.is_empty(),
        "no operator took more than one type, so the probe is measuring nothing"
    );
    found
}

/// The count the corpus states, against the count the checker produces.
///
/// `examples/strings.deed` said `+` was the one operator with two meanings.
/// The four comparisons had the same property from the commit that wrote the
/// sentence, because the rule narrowing them to `Int` and `String` landed in
/// it, so the number had never been right. Nothing held it. That is the exact
/// shape this file exists for, with the twist that the claim is not about a
/// list the compiler keeps but about how the compiler behaves, so the check
/// has to run it rather than read it.
///
/// What this does not hold: the operator has to be in `BinaryOp::ALL` and the
/// type it means something new about has to be in [`PROBED_TYPES`]. A new
/// operator left out of both would go unmeasured and the sentence would go
/// stale in silence, which is the failure this cannot fix by itself.
#[test]
fn the_operators_that_mean_two_things_are_the_ones_the_corpus_names() {
    let operators = operators_with_two_meanings();
    let strings = read("examples/strings.deed");

    let claim = format!("There are {} {TWO_MEANINGS}", spelled(operators.len()));
    assert!(
        strings.contains(&claim),
        "the checker takes more than one type for {operators:?}, \
         so examples/strings.deed should say {claim:?}"
    );

    let sentence = between(&strings, TWO_MEANINGS, "\n");
    let mut listed = enumerated(&sentence[TWO_MEANINGS.len()..]);
    listed.sort();
    assert_eq!(
        listed, operators,
        "examples/strings.deed lists {listed:?} and the checker gives {operators:?}"
    );
}

/// The same claim, in the document that also carries the operator table.
///
/// Two places state it, which is two places to go stale, and the document was
/// wrong in the same words as the corpus for the same reason.
#[test]
fn the_syntax_document_counts_them_the_same_way() {
    let operators = operators_with_two_meanings();
    let syntax = read("design/02-syntax.md");

    let claim = format!("There are {} {TWO_MEANINGS}", spelled(operators.len()));
    assert!(
        syntax.contains(&claim),
        "the checker takes more than one type for {operators:?}, \
         so design/02-syntax.md should say {claim:?}"
    );

    let sentence = between(&syntax, TWO_MEANINGS, "\n");
    let mut listed = enumerated(&sentence[TWO_MEANINGS.len()..]);
    listed.sort();
    assert_eq!(
        listed, operators,
        "design/02-syntax.md lists {listed:?} and the checker gives {operators:?}"
    );
}

/// What ordering takes, where the document says what it takes.
///
/// This is the decision the sentence above turned up. `String` was never
/// added to the ordering rule; it was what survived the rule being narrowed
/// from "both sides agree" down to two types, and it stays because nothing
/// else in the language orders text without tying two pieces that differ:
/// `length` ties `ab` with `ba`, `to_int` ties `007` with `7` and refuses
/// everything that does not spell a number, and `Io.list` sorts file names but
/// wants a `Dir` and a written file per comparison and ties whatever the
/// filesystem does not tell apart. A set stated in a document and argued for
/// in it is worth holding to what the checker does with it, since the argument
/// is the reason anyone would trust the set.
#[test]
fn the_types_ordering_takes_are_the_ones_the_syntax_document_names() {
    let syntax = read("design/02-syntax.md");
    let sentence = between(&syntax, "**Ordering is ", ".**");

    for op in ["<", "<=", ">", ">="] {
        assert_eq!(
            backticked(sentence),
            types_taken_by(op),
            "design/02-syntax.md names the types ordering takes, and `{op}` takes others"
        );
    }
}

/// The numbers P9's edit-loop section derives from its own table agree with it.
///
/// The table is a measurement on one machine, so nothing can keep it fresh.
/// What can be kept is the arithmetic on top of it: the per-file constant, the
/// file count where a recheck crosses the 100ms budget, and the halved count
/// for a request that checks the workspace twice. Those three moved out of step
/// once already, when the table was re-run and the sentences under it were not,
/// which left the document claiming more margin than the machine had.
#[test]
fn the_edit_loop_trigger_follows_from_the_table_above_it() {
    let principles = read("design/01-principles.md");

    let per_file: f64 = between(
        &principles,
        "the constant is about ",
        " microseconds per file",
    )
    .trim_start_matches("the constant is about ")
    .trim()
    .parse()
    .expect("the constant is written as a number of microseconds");

    let trigger: f64 = between(&principles, "crosses P9's budget at about ", " files")
        .trim_start_matches("crosses P9's budget at about ")
        .trim()
        .replace(',', "")
        .parse()
        .expect("the trigger is written as a number of files");

    let halved: f64 = between(&principles, "the same budget is gone at about ", " files")
        .trim_start_matches("the same budget is gone at about ")
        .trim()
        .replace(',', "")
        .parse()
        .expect("the halved trigger is written as a number of files");

    // 100ms in microseconds, which is P9's budget.
    let expected = 100_000.0 / per_file;
    assert!(
        (trigger - expected).abs() / expected < 0.1,
        "at {per_file}us a file the budget is gone at about {expected:.0} files, \
         and the document says {trigger}"
    );
    assert!(
        (halved - trigger / 2.0).abs() / (trigger / 2.0) < 0.1,
        "two passes should halve {trigger} to about {:.0}, and the document says {halved}",
        trigger / 2.0
    );

    // The table has to be the one those came from, rather than a third number.
    let stated = format!("{per_file}us");
    assert!(
        principles.contains(&format!("512      41.6ms    41.9ms    {stated}"))
            || principles.contains(&stated),
        "the per-file constant in the prose does not appear in the table"
    );
}
