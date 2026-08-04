//! Runs every Deed compiler pass over a file and collects the result.
//!
//! This exists so that the command line tool, the tests and eventually an
//! editor server all drive the compiler the same way. A pipeline that gets
//! reassembled by hand at each call site drifts, and then two of them disagree
//! about what "checked" means.
//!
//! # Every pass runs, always
//!
//! Even when an earlier one failed.
//!
//! Stopping at the first failing pass would cost a round trip for everything
//! the later passes would have found, and round trips are the multiplier in the
//! cost model in `design/00-motivation.md`. The design already makes running on
//! safe: a parse error becomes an error node, an unresolved name becomes an
//! unknown type, and an unknown type agrees with everything. Later passes add
//! nothing to a mess they did not make, and there are tests in `deed-resolve`
//! and `deed-typeck` asserting exactly that.

pub mod codes;
pub mod docs;
pub mod fix;
mod imports;
mod inputs;
mod library;
pub mod manifest;
pub mod program_gen;
mod report;
mod rows;
mod shipped;
pub mod wit;

pub use inputs::resolve_inputs;
pub use manifest::{ComponentRoot, FetchedModule, Manifest, parse_manifest};
pub use report::json_report;
pub use shipped::{shipped_for, shipped_modules, shipped_source, take_shipped};
pub use wit::wit_world_for;

use std::collections::HashMap;
use std::time::Duration;

use clock::{since, started};

/// Where the check pipeline reads a clock, on a target that has one.
///
/// `Instant::now` panics on `wasm32-unknown-unknown`. There is no clock there
/// to read, and every verb a page calls goes through `check_all`, so the
/// wasm artifact trapped on any input at all rather than on some of it.
///
/// Zero is the honest answer on a target that cannot measure. Nothing decides
/// anything with these numbers; they are printed by `--timings` and read by
/// the benchmarks, and neither runs where this returns zero.
#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
mod clock {
    use std::time::{Duration, Instant};

    pub(crate) type Started = Instant;

    pub(crate) fn started() -> Started {
        Instant::now()
    }

    pub(crate) fn since(start: Started) -> Duration {
        start.elapsed()
    }
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod clock {
    use std::time::Duration;

    pub(crate) type Started = ();

    pub(crate) fn started() -> Started {}

    // cargo-mutants reports replacing this with `Default::default()` as
    // missed, and it is right that no test kills it: `Duration`'s default is
    // zero, so the mutant is this function. A test that appeared to catch it
    // would be asserting nothing. The same mutation of the branch above is a
    // real change and is rejected before it runs, since dropping `start`
    // leaves an unused binding and CI builds with `-D warnings`.
    pub(crate) fn since(_: Started) -> Duration {
        Duration::ZERO
    }
}

/// A clock that always answers zero is not a clock, and the numbers it feeds
/// are only ever printed, so nothing else in the crate would notice.
///
/// This runs on the host, the only target the tests run on and the only one
/// where an answer of zero would be wrong. The branch above is checked by
/// loading the artifact and calling it, in `crates/deed-wasm/smoke.mjs`.
#[cfg(test)]
mod clock_tests {
    use std::hint::black_box;
    use std::time::Duration;

    #[test]
    fn the_clock_measures_something() {
        let start = super::clock::started();
        let mut sum = 0u64;
        for i in 0..500_000u64 {
            sum = sum.wrapping_add(black_box(i));
        }
        black_box(sum);
        assert!(
            super::clock::since(start) > Duration::ZERO,
            "half a million additions took no time at all, so this is not reading a clock",
        );
    }
}

use deed_ast::{Item, Module, Outcome};
use deed_diagnostics::{Diagnostic, FileId, Severity, SourceMap, Span};
use deed_effects::Effects;
use deed_interp::{DeclaredRows, Guard, Guards, OperatorCalls, RowItem};
use deed_lexer::tokenize;
use deed_parser::parse;
use deed_resolve::{Resolutions, Universe};
use deed_typeck::{Reason, Tier, Types, World};

/// One obligation and how it was discharged.
#[derive(Clone, Debug)]
pub struct ObligationReport {
    pub tier: Tier,
    pub span: Span,
    /// What had to hold, such as a refinement name or which outcome an
    /// `ensures` clause was about.
    pub subject: String,
    /// Why this landed in `Guarded` rather than `Proven`, when it did.
    pub reason: Option<Reason>,
}

/// How long each pass took, for one file.
///
/// P9 says check latency is a language feature and it is budgeted. A budget
/// nobody measures is a wish, so this is measured and reported rather than
/// asserted in a document.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    pub lex: Duration,
    pub parse: Duration,
    pub resolve: Duration,
    pub typeck: Duration,
    pub effects: Duration,
}

impl Timings {
    pub fn total(&self) -> Duration {
        self.lex + self.parse + self.resolve + self.typeck + self.effects
    }

    /// Each pass with its name, in the order they run.
    pub fn passes(&self) -> [(&'static str, Duration); 5] {
        [
            ("lex", self.lex),
            ("parse", self.parse),
            ("resolve", self.resolve),
            ("typeck", self.typeck),
            ("effects", self.effects),
        ]
    }
}

/// Everything the compiler worked out about one file.
pub struct Checked {
    pub file: FileId,
    pub module: Module,
    pub resolutions: Resolutions,
    pub types: Types,
    pub effects: Effects,
    /// In source order, so output reads top to bottom.
    pub diagnostics: Vec<Diagnostic>,
    pub obligations: Vec<ObligationReport>,
    pub timings: Timings,
}

impl Checked {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    pub fn error_count(&self) -> usize {
        fix::error_count(&self.diagnostics)
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    pub fn obligations_at(&self, tier: Tier) -> usize {
        self.obligations
            .iter()
            .filter(|obligation| obligation.tier == tier)
            .count()
    }

    /// Everything the checker could not settle, for the interpreter to check.
    ///
    /// The runtime rule is the checker's rule, read off the same table rather
    /// than worked out again from the syntax. When the two were separate, the
    /// checker said "so it becomes a runtime check" at places the interpreter
    /// had no check for, and nothing noticed because nothing compared them.
    pub fn guards(&self) -> Guards {
        self.types
            .obligations()
            .iter()
            .filter(|obligation| obligation.tier == Tier::Guarded)
            .map(|obligation| {
                (
                    obligation.span,
                    Guard {
                        refinement: obligation.refinement,
                        inside_ok: obligation.inside_ok,
                    },
                )
            })
            .collect()
    }

    /// What every function in this file declared it performs.
    ///
    /// Handed to the interpreter so that a run can hold the program to its own
    /// signatures. The rows are what this language is for, and the pass that
    /// produces them was the only thing that ever read one, which is how five
    /// separate ways of getting an effect past them stayed open at once.
    ///
    /// Handler operations are in here too, keyed by where the name was written,
    /// because they have no definition of their own and they are where an
    /// effect is implemented.
    pub fn rows(&self) -> DeclaredRows {
        self.effects
            .declarations()
            .map(|(span, row)| {
                let items = row
                    .iter()
                    .map(|item| RowItem {
                        effect: item.effect,
                        operation: item.operation.clone(),
                    })
                    .collect();
                (span, items)
            })
            .collect()
    }

    /// Which function each bound operator in this file goes to.
    ///
    /// Handed to the interpreter for the same reason `guards` is: only the
    /// checker can answer it. A record value carries its fields and not the
    /// name of its type, so a `+` between two of them is a question nothing
    /// downstream could work out on its own.
    pub fn operators(&self) -> OperatorCalls {
        self.types.operators().clone()
    }
}

/// The module a file declares and the modules it imports.
///
/// `None` when the file has no `module` line, which is a file nothing can
/// import and which therefore says nothing about where anything else lives.
///
/// This exists so that whatever is about to compile a set of files can work
/// out which files those are, which has to happen before the compilation. Done
/// with the real parser rather than by matching the text, because a second,
/// worse parser would disagree with this one about comments and strings on
/// exactly the day it mattered.
pub fn imports_of(text: &str) -> Option<(String, Vec<String>)> {
    let mut sources = SourceMap::new();
    let file = sources.add("<imports>", text.to_string());
    let lexed = tokenize(file, text);
    let parsed = parse(file, &lexed.tokens);

    let module = parsed.module.name.as_ref()?.to_string_path();
    let uses = parsed
        .module
        .uses
        .iter()
        .map(|entry| entry.path.to_string_path())
        .collect();
    Some((module, uses))
}

/// Runs the whole pipeline over a file already in `sources`.
///
/// The file is compiled on its own, so a `use` in it has nowhere to point.
/// Use [`check_all`] for anything with more than one module in it.
pub fn check(sources: &SourceMap, file: FileId) -> Checked {
    check_all(sources, &[file])
        .pop()
        .expect("one file in, one out")
}

/// What every entry point says when a program has no `main`.
///
/// Three copies of this sentence had drifted into two different sentences: the
/// CLI explained itself and the wasm artifact did not, so a page said only
/// "no `main` found" to a reader looking at one of the twenty-two library
/// files in the corpus, for whom that is the expected answer rather than a
/// mistake.
pub const NOTHING_TO_RUN: &str = "no `main` found, so there is nothing to run";

/// What every entry point says when it is asked to run a file the checker
/// rejected.
///
/// The command line has refused this since there has been a command line, and
/// the reason is in `deed-cli`: running code that does not check answers a
/// question nobody asked, and the failure it produces is about the wrong
/// thing. The sentence lives here so the artifact a page and an agent talk to
/// gives the same answer rather than running anyway.
pub const DOES_NOT_CHECK: &str =
    "this program does not check, and running it would report the wrong mistake";

/// Runs the whole pipeline over a set of files that see each other.
///
/// Three passes over the list: parse everything, work out what each module
/// offers, then check everything. A module is named by its own `module` line,
/// so building the universe needs the trees and nothing else, and nothing here
/// has to decide what order to visit them in.
///
/// Import cycles are still not detected, and still do not need to be. Lowering
/// an exported type looks like it should need the other module resolved first,
/// and does not: a type from elsewhere lowers to its module path and its name,
/// which are both in the syntax. `a` and `b` importing each other terminates
/// because neither lowering recurses into the other.
pub fn check_all(sources: &SourceMap, files: &[FileId]) -> Vec<Checked> {
    let parsed: Vec<Parsed> = files
        .iter()
        .map(|file| {
            let text = sources.file(*file).text();

            let start = started();
            let lexed = deed_lexer::tokenize(*file, text);
            let lex = since(start);

            let start = started();
            let parsed = deed_parser::parse(*file, &lexed.tokens);
            let parse = since(start);

            Parsed {
                file: *file,
                module: parsed.module,
                trivia: lexed.trivia,
                diagnostics: lexed
                    .diagnostics
                    .into_iter()
                    .chain(parsed.diagnostics)
                    .collect(),
                timings: Timings {
                    lex,
                    parse,
                    ..Timings::default()
                },
            }
        })
        .collect();

    let mut universe = Universe::new();
    // Track the first source file to claim each module path, so that when a
    // second claims the same path both are reported rather than only the one
    // that arrived later.  The precedence rule in one sentence: a user file
    // always beats a shipped module with the same path; between two user files
    // that claim the same path, neither wins and the compiler reports both.
    let mut first_claims: HashMap<String, (FileId, Span)> = HashMap::new();
    // (first_file, first_span, second_file, second_span, path)
    let mut collisions: Vec<(FileId, Span, FileId, Span, String)> = Vec::new();
    for entry in &parsed {
        universe.add(&entry.module);
        if let Some(name) = &entry.module.name {
            let path = name.to_string_path();
            if let Some(&(first_file, first_span)) = first_claims.get(&path) {
                collisions.push((first_file, first_span, entry.file, name.span, path));
            } else {
                first_claims.insert(path, (entry.file, name.span));
            }
        }
    }

    // Names first, then types. Resolving is what tells the surface pass which
    // of a module's own names are local and which came from somewhere else, so
    // every module has to be resolved before any surface can be lowered.
    let resolutions: Vec<_> = parsed
        .iter()
        .map(|entry| {
            let start = started();
            let resolved = deed_resolve::resolve(entry.file, &entry.module, &universe);
            (resolved, since(start))
        })
        .collect();

    // Every module together rather than one at a time. Lowering a module does
    // not ask about another, so the order does not matter here, but a
    // signature can name a transparent alias a third module declared and what
    // that names needs the whole set.
    let world = World::of(
        parsed
            .iter()
            .zip(&resolutions)
            .filter_map(|(entry, (resolved, _))| {
                let name = entry.module.name.as_ref()?;
                Some((
                    name.to_string_path(),
                    deed_typeck::surface(entry.file, &entry.module, &resolved.resolutions),
                ))
            }),
    );

    parsed
        .into_iter()
        .zip(resolutions)
        .map(|(entry, (resolved, resolve_time))| {
            let text = sources.file(entry.file).text();
            let mut checked = check_parsed(entry, resolved, resolve_time, &world, text);
            for (file_a, span_a, file_b, span_b, path) in &collisions {
                let (this_span, other_file, other_span) = if *file_a == checked.file {
                    (*span_a, *file_b, *span_b)
                } else if *file_b == checked.file {
                    (*span_b, *file_a, *span_a)
                } else {
                    continue;
                };
                checked.diagnostics.push(
                    Diagnostic::error(
                        deed_resolve::codes::AMBIGUOUS_MODULE,
                        checked.file,
                        this_span,
                        format!("two files both declare `module {path}`"),
                    )
                    .with_primary_label("declared here")
                    .with_secondary_in(other_file, other_span, "also declared here")
                    .with_note(
                        "a module is identified by its path, so two files with the same \
                         module path cannot both be imported; give one of them a different path",
                    ),
                );
                checked
                    .diagnostics
                    .sort_by_key(|diagnostic| diagnostic.primary.span.start);
            }
            checked
        })
        .collect()
}

struct Parsed {
    file: FileId,
    module: Module,
    /// The comments, kept so that a fix can decline to rewrite a region that
    /// has one in it rather than quietly eating it.
    trivia: Vec<deed_lexer::Trivia>,
    diagnostics: Vec<Diagnostic>,
    timings: Timings,
}

fn check_parsed(
    parsed: Parsed,
    resolved: deed_resolve::Resolved,
    resolve_time: Duration,
    world: &World,
    source: &str,
) -> Checked {
    let Parsed {
        file,
        module,
        trivia,
        diagnostics: mut collected,
        mut timings,
    } = parsed;
    timings.resolve = resolve_time;

    let start = started();
    let checked = deed_typeck::check(file, &module, &resolved.resolutions, world);
    timings.typeck = since(start);

    let start = started();
    let analysed = deed_effects::analyse(
        file,
        &module,
        &resolved.resolutions,
        checked.types.row_required(),
        &checked.types.function_rows(),
    );
    timings.effects = since(start);

    let mut diagnostics = Vec::new();
    diagnostics.append(&mut collected);
    diagnostics.extend(resolved.diagnostics);
    diagnostics.extend(checked.diagnostics);
    diagnostics.extend(analysed.diagnostics);

    // Source order, not pass order. A reader works down the file; which pass
    // noticed a problem is an implementation detail they should not have to
    // reassemble in their head.
    diagnostics.sort_by_key(|diagnostic| diagnostic.primary.span.start);

    // The row diagnostics say what to type. This is where the text, the tree
    // and the one canonical layout are all in scope, so it is where they get
    // to type it.
    rows::attach_fixes(
        &mut diagnostics,
        &module,
        &resolved.resolutions,
        &analysed.effects,
        source,
        &trivia,
    );
    imports::attach_fixes(&mut diagnostics, &module, source, &trivia);

    // The resolver is right that the name is not in scope and cannot know the
    // compiler is carrying it. This is the layer that has the shipped table.
    library::attach(&mut diagnostics, &module, source, &trivia);

    let parsed_module = module;
    let mut obligations: Vec<ObligationReport> = checked
        .types
        .obligations()
        .iter()
        .map(|obligation| ObligationReport {
            tier: obligation.tier,
            span: obligation.span,
            subject: checked.types.name_of(obligation.refinement).to_string(),
            reason: obligation.reason,
        })
        .collect();

    // What a caller answered for. A `where` clause is checked inside the
    // callee on every call whatever happens here, so the floor is `Guarded`
    // and `Proven` means the call site settled it as well.
    for precondition in checked.types.preconditions() {
        obligations.push(ObligationReport {
            tier: precondition.tier,
            span: precondition.span,
            subject: format!("{} requires", precondition.callee),
            reason: precondition.reason,
        });
    }

    // Contract obligations. Every `ensures` clause is checked at runtime on
    // every call, so the floor is `Guarded`. A pure function whose parameters
    // can be generated gets exercised by a property test as well, which is the
    // `Tested` tier and the only place it comes from.
    //
    // A guarded one carries `NothingTriesToProveThis` rather than no reason at
    // all. The distinction matters more here than anywhere else: the other
    // guarded obligations are the checker having looked and failed, and these
    // are the checker never having looked, and until this said so the two were
    // the same word with nothing to tell them apart. Thirteen of the sixteen
    // guarded obligations in `examples/` are this case.
    for item in &parsed_module.items {
        let Item::Function(function) = item else {
            continue;
        };
        let tested = deed_interp::is_testable(function, &parsed_module, &resolved.resolutions);
        for obligation in &function.contract.ensures {
            obligations.push(ObligationReport {
                tier: if tested { Tier::Tested } else { Tier::Guarded },
                span: obligation.span,
                subject: format!(
                    "{} ensures {}",
                    function.sig.name.name,
                    match obligation.outcome {
                        Outcome::Ok => "ok",
                        Outcome::Err => "err",
                    }
                ),
                // `Tested` says a property test exercises it, which is an
                // answer rather than the absence of one.
                reason: match tested {
                    true => None,
                    false => Some(deed_typeck::facts::Reason::NothingTriesToProveThis),
                },
            });
        }
    }

    obligations.sort_by_key(|obligation| obligation.span.start);

    Checked {
        file,
        module: parsed_module,
        resolutions: resolved.resolutions,
        types: checked.types,
        effects: analysed.effects,
        diagnostics,
        obligations,
        timings,
    }
}

/// Convenience for callers holding text rather than a populated map.
pub fn check_text(
    sources: &mut SourceMap,
    name: impl Into<String>,
    text: impl Into<String>,
) -> Checked {
    let file = sources.add(name, text);
    check(sources, file)
}
