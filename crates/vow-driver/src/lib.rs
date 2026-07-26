//! Runs every Vow compiler pass over a file and collects the result.
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
//! nothing to a mess they did not make, and there are tests in `vow-resolve`
//! and `vow-typeck` asserting exactly that.

pub mod fix;

use std::time::{Duration, Instant};

use vow_ast::{Item, Module, Outcome};
use vow_diagnostics::{Diagnostic, FileId, Severity, SourceMap, Span};
use vow_effects::Effects;
use vow_interp::{Guard, Guards};
use vow_lexer::tokenize;
use vow_parser::parse;
use vow_resolve::{Resolutions, Universe};
use vow_typeck::{Tier, Types, World};

/// One obligation and how it was discharged.
#[derive(Clone, Debug)]
pub struct ObligationReport {
    pub tier: Tier,
    pub span: Span,
    /// What had to hold, such as a refinement name or which outcome an
    /// `ensures` clause was about.
    pub subject: String,
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

            let start = Instant::now();
            let lexed = vow_lexer::tokenize(*file, text);
            let lex = start.elapsed();

            let start = Instant::now();
            let parsed = vow_parser::parse(*file, &lexed.tokens);
            let parse = start.elapsed();

            Parsed {
                file: *file,
                module: parsed.module,
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
    let mut duplicates = Vec::new();
    for entry in &parsed {
        if universe.add(&entry.module).is_some() {
            // Two files claiming one module is not something to pick a winner
            // for. Whichever one lost would be silently unreachable.
            if let Some(name) = &entry.module.name {
                duplicates.push((entry.file, name.span, name.to_string_path()));
            }
        }
    }

    // Names first, then types. Resolving is what tells the surface pass which
    // of a module's own names are local and which came from somewhere else, so
    // every module has to be resolved before any surface can be lowered.
    let resolutions: Vec<_> = parsed
        .iter()
        .map(|entry| {
            let start = Instant::now();
            let resolved = vow_resolve::resolve(entry.file, &entry.module, &universe);
            (resolved, start.elapsed())
        })
        .collect();

    let mut world = World::new();
    for (entry, (resolved, _)) in parsed.iter().zip(&resolutions) {
        if let Some(name) = &entry.module.name {
            world.insert(
                name.to_string_path(),
                vow_typeck::surface(&entry.module, &resolved.resolutions),
            );
        }
    }

    parsed
        .into_iter()
        .zip(resolutions)
        .map(|(entry, (resolved, resolve_time))| {
            let mut checked = check_parsed(entry, resolved, resolve_time, &world);
            for (file, span, path) in &duplicates {
                if *file == checked.file {
                    checked.diagnostics.push(
                        Diagnostic::error(
                            vow_resolve::codes::DUPLICATE_DEFINITION,
                            *file,
                            *span,
                            format!("another file already declares `module {path}`"),
                        )
                        .with_primary_label("declared twice")
                        .with_note(
                            "a module is named by its `module` line, so two files with the \
                             same one cannot both be imported",
                        ),
                    );
                    checked
                        .diagnostics
                        .sort_by_key(|diagnostic| diagnostic.primary.span.start);
                }
            }
            checked
        })
        .collect()
}

struct Parsed {
    file: FileId,
    module: Module,
    diagnostics: Vec<Diagnostic>,
    timings: Timings,
}

fn check_parsed(
    parsed: Parsed,
    resolved: vow_resolve::Resolved,
    resolve_time: Duration,
    world: &World,
) -> Checked {
    let Parsed {
        file,
        module,
        diagnostics: mut collected,
        mut timings,
    } = parsed;
    timings.resolve = resolve_time;

    let start = Instant::now();
    let checked = vow_typeck::check(file, &module, &resolved.resolutions, world);
    timings.typeck = start.elapsed();

    let start = Instant::now();
    let analysed = vow_effects::analyse(
        file,
        &module,
        &resolved.resolutions,
        checked.types.row_required(),
        &checked.types.function_rows(),
    );
    timings.effects = start.elapsed();

    let mut diagnostics = Vec::new();
    diagnostics.append(&mut collected);
    diagnostics.extend(resolved.diagnostics);
    diagnostics.extend(checked.diagnostics);
    diagnostics.extend(analysed.diagnostics);

    // Source order, not pass order. A reader works down the file; which pass
    // noticed a problem is an implementation detail they should not have to
    // reassemble in their head.
    diagnostics.sort_by_key(|diagnostic| diagnostic.primary.span.start);

    let parsed_module = module;
    let mut obligations: Vec<ObligationReport> = checked
        .types
        .obligations()
        .iter()
        .map(|obligation| ObligationReport {
            tier: obligation.tier,
            span: obligation.span,
            subject: checked.types.name_of(obligation.refinement).to_string(),
        })
        .collect();

    // Contract obligations. Every `ensures` clause is checked at runtime on
    // every call, so the floor is `Guarded`. A pure function whose parameters
    // can be generated gets exercised by a property test as well, which is the
    // `Tested` tier and the only place it comes from.
    for item in &parsed_module.items {
        let Item::Function(function) = item else {
            continue;
        };
        let tested = vow_interp::is_testable(function, &parsed_module, &resolved.resolutions);
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
