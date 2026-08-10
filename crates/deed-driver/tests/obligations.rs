//! Every guarded obligation says why it is guarded.
//!
//! `Guarded` is the tier a reader is meant to act on: the contract holds, but
//! it holds because it is checked while the program runs rather than because
//! anything settled it first. Acting on that needs the reason, and the reason
//! is only useful if it is always there.
//!
//! It was not always there. Sixteen obligations in `examples/` came back
//! guarded and thirteen of them said nothing, because an `ensures` clause was
//! reported with no reason at all. That is a different fact from the other
//! three, and saying nothing made it look like the same one: the three are the
//! checker having looked and come back without an answer, and the thirteen are
//! the checker never having looked. A reader could not tell "I could not prove
//! this" from "nobody tried", which is the distinction the whole tier exists to
//! draw.
//!
//! One reason for all of them was still one reason too few. Most of these
//! clauses are about an effect, and no pass here will ever settle one of those:
//! the function is checked once, and the `with` block deciding what the effect
//! means is written by whoever calls it. "Nobody tried" reads as a job somebody
//! could finish, so those now say the caller installs the handler instead.

use std::fs;
use std::path::PathBuf;

use deed_diagnostics::SourceMap;
use deed_driver::{Checked, check_all, shipped_modules, shipped_source};
use deed_typeck::Tier;
use deed_typeck::facts::Reason;

/// Every `.deed` file this repository ships, checked together.
///
/// Both `examples/` and `std/`: a rule about what an obligation carries is
/// about the compiler rather than about one directory, and the shipped library
/// is the half a user reads without having written it.
fn everything() -> Vec<Checked> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");

    let mut paths: Vec<PathBuf> = ["examples", "std"]
        .iter()
        .flat_map(|directory| {
            fs::read_dir(root.join(directory))
                .expect("the directory should be there")
                .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        })
        .filter(|path| path.extension().is_some_and(|ext| ext == "deed"))
        .collect();
    paths.sort();
    assert!(paths.len() > 20, "only {} files found", paths.len());

    let mut sources = SourceMap::new();
    let mut ids: Vec<_> = paths
        .iter()
        .map(|path| {
            let text = fs::read_to_string(path).expect("a file should be readable");
            let name = path
                .strip_prefix(&root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            sources.add(name, text)
        })
        .collect();

    let subject = ids.len();
    for module in shipped_modules() {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("<shipped>/{module}.deed"), text.to_string()));
    }

    let mut checks = check_all(&sources, &ids);
    checks.truncate(subject);
    checks
}

/// The ratchet. Nothing lands in `Guarded` without saying why.
#[test]
fn every_guarded_obligation_says_what_stopped_it_being_proven() {
    let checks = everything();

    let mut guarded = 0;
    let mut silent = Vec::new();
    for checked in &checks {
        for obligation in &checked.obligations {
            if obligation.tier != Tier::Guarded {
                continue;
            }
            guarded += 1;
            if obligation.reason.is_none() {
                silent.push(obligation.subject.clone());
            }
        }
    }

    assert!(
        guarded > 0,
        "no guarded obligation anywhere, so this holds nothing"
    );
    assert!(
        silent.is_empty(),
        "{} of {guarded} guarded obligations say nothing about why: {silent:?}",
        silent.len()
    );
}

/// The other half. A reason is only worth requiring if the reasons differ.
///
/// One reason used everywhere would satisfy the test above and tell a reader
/// exactly as much as no reason did.
#[test]
fn the_reasons_are_not_all_the_same_one() {
    let checks = everything();

    let mut reasons: Vec<Reason> = checks
        .iter()
        .flat_map(|checked| checked.obligations.iter())
        .filter(|obligation| obligation.tier == Tier::Guarded)
        .filter_map(|obligation| obligation.reason)
        .collect();
    reasons.sort_by_key(|reason| reason.text());
    reasons.dedup();

    assert!(
        reasons.len() >= 2,
        "every guarded obligation gives the same reason: {:?}",
        reasons
            .iter()
            .map(|reason| reason.text())
            .collect::<Vec<_>>()
    );
}

/// `Tested` is an answer, not an absence, so it does not carry a reason.
///
/// A property test exercising a clause is the thing that happened to it. Giving
/// it "nothing tries to prove this" as well would be two answers to one
/// question, and the wrong one is the one a reader would act on.
#[test]
fn a_tested_obligation_is_not_given_a_reason_as_well() {
    let checks = everything();

    let mut tested = 0;
    for checked in &checks {
        for obligation in &checked.obligations {
            if obligation.tier != Tier::Tested {
                continue;
            }
            tested += 1;
            assert!(
                obligation.reason.is_none(),
                "`{}` is tested and still carries a reason",
                obligation.subject
            );
        }
    }

    assert!(tested > 0, "nothing in the corpus reaches the tested tier");
}

/// Every reason the checker can give is a sentence somebody wrote.
///
/// A reason whose text is empty, or which reads like a solver log rather than
/// like advice, is a reason a reader cannot act on, which is the thing this
/// whole field exists to avoid.
#[test]
fn every_reason_reads_as_advice() {
    for reason in [
        Reason::NothingNarrowedThisName,
        Reason::NothingEstablishedThisLength,
        Reason::NothingNamesThisValue,
        Reason::CrossedAModuleBoundary,
        Reason::NotAShapeTheCheckerReasonsAbout,
        Reason::NothingTriesToProveThis,
        Reason::TheCallerInstallsTheHandler,
    ] {
        let text = reason.text();
        assert!(text.len() > 20, "{reason:?} says almost nothing: {text:?}");
        assert!(
            text.chars().next().is_some_and(char::is_lowercase),
            "{reason:?} starts a sentence the caller has to fit into: {text:?}"
        );
        assert!(
            !text.ends_with('.'),
            "{reason:?} ends a sentence the caller has to put somewhere: {text:?}"
        );
    }
}

/// The measurement that made this worth doing, kept so it stays true.
///
/// An `ensures` clause is checked on every call and nothing settles one ahead
/// of time, so it is the floor of what lands in `Guarded`. Both reasons that
/// belong to one say so in different words, and no other kind of obligation is
/// allowed to borrow either: a `where` clause or a refinement that came back
/// with one of these would mean the reason had stopped being about contracts.
#[test]
fn only_an_ensures_clause_gives_a_reason_about_a_contract() {
    let checks = everything();

    let mut unattempted = 0;
    let mut the_callers = 0;
    let mut total = 0;
    for checked in &checks {
        for obligation in &checked.obligations {
            if obligation.tier != Tier::Guarded {
                continue;
            }
            total += 1;
            let contract = match obligation.reason {
                Some(Reason::NothingTriesToProveThis) => {
                    unattempted += 1;
                    true
                }
                Some(Reason::TheCallerInstallsTheHandler) => {
                    the_callers += 1;
                    true
                }
                _ => false,
            };
            assert!(
                !contract || obligation.subject.contains(" ensures "),
                "`{}` gives a reason only an `ensures` clause should give",
                obligation.subject
            );
        }
    }

    assert!(
        unattempted > 0 && the_callers > 0 && total > unattempted + the_callers,
        "{unattempted} unattempted and {the_callers} the caller's, of {total} guarded, \
         which is not the shape this file was written about"
    );
}

/// The split, asked of a program small enough to read.
///
/// Both directions, because a rule that only ever says yes would satisfy the
/// counts above just as well as one that reads the clause. The two functions
/// differ in one thing: whether the postcondition mentions the effect.
#[test]
fn a_clause_about_an_effect_says_the_caller_answers_it_and_one_that_is_not_does_not() {
    let source = "module scratch/handlers

effect Counter {
    fn value() -> Int
}

fn watched(by: Int) -> Int
  uses
    Counter.value,
  ensures
    ok  => Counter.value() >= by,
{
    Counter.value() + by
}

fn plain(by: Int) -> Int
  uses
    Counter.value,
  ensures
    ok  => result >= by,
{
    Counter.value() + by
}
";

    let mut sources = SourceMap::new();
    let checked = deed_driver::check_text(&mut sources, "scratch/handlers.deed", source);
    assert!(!checked.has_errors(), "the fixture should check cleanly");

    let reason_for = |name: &str| {
        checked
            .obligations
            .iter()
            .find(|obligation| obligation.subject == format!("{name} ensures ok"))
            .unwrap_or_else(|| panic!("`{name}` should carry one obligation"))
            .reason
    };

    assert_eq!(
        reason_for("watched"),
        Some(Reason::TheCallerInstallsTheHandler),
        "a clause performing an operation is one only the caller's handler can answer"
    );
    assert_eq!(
        reason_for("plain"),
        Some(Reason::NothingTriesToProveThis),
        "a clause about the returned value is one nothing here tried"
    );
}

/// `unchanged(E)` reaches the same answer by naming the effect alone.
///
/// It is the shape that is hardest to read off the text, since it performs no
/// operation and mentions no dot. Held separately so that removing the arm
/// that recognises it fails with a name rather than with a count.
#[test]
fn an_unchanged_clause_is_the_callers_to_answer_as_well() {
    let source = "module scratch/still

effect Counter {
    fn value() -> Int
}

fn peek() -> Int
  uses
    Counter.value,
  ensures
    ok  => unchanged(Counter),
{
    Counter.value()
}
";

    let mut sources = SourceMap::new();
    let checked = deed_driver::check_text(&mut sources, "scratch/still.deed", source);
    assert!(!checked.has_errors(), "the fixture should check cleanly");

    let obligation = checked
        .obligations
        .iter()
        .find(|obligation| obligation.subject == "peek ensures ok")
        .expect("`peek` should carry one obligation");
    assert_eq!(
        obligation.reason,
        Some(Reason::TheCallerInstallsTheHandler),
        "`unchanged` names an effect, and the handler behind it belongs to the caller"
    );
}
