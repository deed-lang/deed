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

use vow_ast::{Item, Module, Outcome};
use vow_diagnostics::{Diagnostic, FileId, Severity, SourceMap, Span};
use vow_effects::Effects;
use vow_resolve::{Resolutions, Universe};
use vow_typeck::{Tier, Types};

/// One obligation and how it was discharged.
#[derive(Clone, Debug)]
pub struct ObligationReport {
    pub tier: Tier,
    pub span: Span,
    /// What had to hold, such as a refinement name or which outcome an
    /// `ensures` clause was about.
    pub subject: String,
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
}

impl Checked {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    pub fn error_count(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.is_error()).count()
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
/// Two passes over the list: parse everything, then check everything. A module
/// is named by its own `module` line, so building the universe needs the trees
/// and nothing else, and nothing here has to decide what order to visit them
/// in.
///
/// Import cycles are not detected, because there is nothing to detect. What a
/// module exports is a function of its syntax alone, so `a` importing `b`
/// importing `a` resolves the same way whichever one is looked at first. That
/// stops being true the moment an exported type has to be lowered, which is
/// the next issue and where a cycle check will actually have to live.
pub fn check_all(sources: &SourceMap, files: &[FileId]) -> Vec<Checked> {
    let parsed: Vec<Parsed> = files
        .iter()
        .map(|file| {
            let text = sources.file(*file).text();
            let lexed = vow_lexer::tokenize(*file, text);
            let parsed = vow_parser::parse(*file, &lexed.tokens);
            Parsed {
                file: *file,
                module: parsed.module,
                diagnostics: lexed
                    .diagnostics
                    .into_iter()
                    .chain(parsed.diagnostics)
                    .collect(),
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

    parsed
        .into_iter()
        .map(|entry| {
            let mut checked = check_parsed(entry, &universe);
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
}

fn check_parsed(parsed: Parsed, universe: &Universe) -> Checked {
    let Parsed {
        file,
        module,
        diagnostics: mut collected,
    } = parsed;

    let resolved = vow_resolve::resolve(file, &module, universe);
    let checked = vow_typeck::check(file, &module, &resolved.resolutions);
    let analysed = vow_effects::analyse(file, &module, &resolved.resolutions);

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
