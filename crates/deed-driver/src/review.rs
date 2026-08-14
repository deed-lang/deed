//! What changed in the parts of a Deed module a reviewer has to trust.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use deed_ast::Item;
use deed_diagnostics::{SourceMap, json_string};
use deed_resolve::{ExportKind, Exports, RowEntry};
use deed_typeck::Tier;

use crate::{Checked, ObligationReport, check_all, json_report, shipped_for, shipped_source};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AuthorityChange {
    pub module: String,
    pub declaration: String,
    pub authority: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TierRegression {
    pub module: String,
    pub declaration: String,
    pub subject: String,
    pub occurrence: usize,
    pub before: Tier,
    pub after: Tier,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GuardedAddition {
    pub module: String,
    pub declaration: String,
    pub subject: String,
    pub occurrence: usize,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct ReviewPolicy {
    pub deny_new_authority: bool,
    pub deny_weaker_promises: bool,
    pub deny_new_guarded: bool,
}

impl ReviewPolicy {
    pub fn is_empty(self) -> bool {
        !self.deny_new_authority && !self.deny_weaker_promises && !self.deny_new_guarded
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PolicyRule {
    NewAuthority,
    WeakerPromises,
    NewGuarded,
}

impl PolicyRule {
    pub fn name(self) -> &'static str {
        match self {
            Self::NewAuthority => "deny-new-authority",
            Self::WeakerPromises => "deny-weaker-promises",
            Self::NewGuarded => "deny-new-guarded",
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PolicyViolation {
    pub rule: PolicyRule,
    pub findings: usize,
}

#[derive(Default, PartialEq, Eq, Debug)]
pub struct PolicyVerdict {
    pub violations: Vec<PolicyViolation>,
}

impl PolicyVerdict {
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }

    pub fn to_json(&self) -> String {
        let violations = self
            .violations
            .iter()
            .map(|violation| {
                format!(
                    "{{\"rule\":{},\"findings\":{}}}",
                    json_string(violation.rule.name()),
                    violation.findings
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"passed\":{},\"violations\":[{violations}]}}",
            self.passed()
        )
    }
}

#[derive(Default, Debug)]
pub struct ReviewReceipt {
    pub authority_added: Vec<AuthorityChange>,
    pub tier_regressions: Vec<TierRegression>,
    pub guarded_added: Vec<GuardedAddition>,
}

impl ReviewReceipt {
    pub fn between(before: &[Checked], after: &[Checked]) -> Self {
        let old = snapshot(before);
        let new = snapshot(after);
        let mut receipt = Self::default();

        for (key, function) in &new {
            let previous_authority = old.get(key).map(|entry| &entry.authority);
            for entry in function.authority.iter().filter(|entry| {
                previous_authority
                    .is_none_or(|previous| !previous.iter().any(|old| covers(old, entry)))
            }) {
                receipt.authority_added.push(AuthorityChange {
                    module: key.0.clone(),
                    declaration: key.1.clone(),
                    authority: authority(entry, &key.0),
                });
            }
            compare_obligations(key, old.get(key), function, &mut receipt);
        }

        receipt
    }

    pub fn is_clean(&self) -> bool {
        self.authority_added.is_empty()
            && self.tier_regressions.is_empty()
            && self.guarded_added.is_empty()
    }

    pub fn evaluate(&self, policy: ReviewPolicy) -> PolicyVerdict {
        let mut violations = Vec::new();
        if policy.deny_new_authority && !self.authority_added.is_empty() {
            violations.push(PolicyViolation {
                rule: PolicyRule::NewAuthority,
                findings: self.authority_added.len(),
            });
        }
        if policy.deny_weaker_promises && !self.tier_regressions.is_empty() {
            violations.push(PolicyViolation {
                rule: PolicyRule::WeakerPromises,
                findings: self.tier_regressions.len(),
            });
        }
        if policy.deny_new_guarded && !self.guarded_added.is_empty() {
            violations.push(PolicyViolation {
                rule: PolicyRule::NewGuarded,
                findings: self.guarded_added.len(),
            });
        }
        PolicyVerdict { violations }
    }

    pub fn to_json(&self) -> String {
        format!("{{{}}}", self.json_fields())
    }

    pub fn to_json_with_policy(&self, verdict: &PolicyVerdict) -> String {
        format!(
            "{{{},\"policy\":{}}}",
            self.json_fields(),
            verdict.to_json()
        )
    }

    fn json_fields(&self) -> String {
        let authority = self
            .authority_added
            .iter()
            .map(|change| {
                format!(
                    "{{\"module\":{},\"declaration\":{},\"authority\":{}}}",
                    json_string(&change.module),
                    json_string(&change.declaration),
                    json_string(&change.authority)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let regressions = self
            .tier_regressions
            .iter()
            .map(|change| {
                format!(
                    "{{\"module\":{},\"declaration\":{},\"subject\":{},\"occurrence\":{},\"before\":{},\"after\":{}}}",
                    json_string(&change.module),
                    json_string(&change.declaration),
                    json_string(&change.subject),
                    change.occurrence,
                    json_string(change.before.name()),
                    json_string(change.after.name())
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let guarded = self
            .guarded_added
            .iter()
            .map(|change| {
                format!(
                    "{{\"module\":{},\"declaration\":{},\"subject\":{},\"occurrence\":{}}}",
                    json_string(&change.module),
                    json_string(&change.declaration),
                    json_string(&change.subject),
                    change.occurrence
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "\"kind\":\"review_receipt\",\"clean\":{},\"authority_added\":[{authority}],\"tier_regressions\":[{regressions}],\"guarded_added\":[{guarded}]",
            self.is_clean()
        )
    }
}

/// Reviews two in-memory module sets and writes the same JSON receipt every
/// compiler surface hands to its caller.
pub fn review_sources(before: &[&str], after: &[&str], policy: Option<ReviewPolicy>) -> String {
    let before = review_side("before", before);
    let after = review_side("after", after);
    let before_refusal = refusal("before", &before);
    let after_refusal = refusal("after", &after);
    if before_refusal.is_some() || after_refusal.is_some() {
        let mut text = before_refusal.unwrap_or_default();
        text.push_str(&after_refusal.unwrap_or_default());
        return text;
    }

    let receipt = ReviewReceipt::between(&before.checks, &after.checks);
    let mut text = match policy {
        Some(policy) => receipt.to_json_with_policy(&receipt.evaluate(policy)),
        None => receipt.to_json(),
    };
    text.push('\n');
    text
}

struct ReviewSide {
    sources: SourceMap,
    checks: Vec<Checked>,
    subjects: usize,
}

fn review_side(label: &str, texts: &[&str]) -> ReviewSide {
    let mut sources = SourceMap::new();
    let mut ids = texts
        .iter()
        .enumerate()
        .map(|(index, text)| sources.add(format!("<{label}/{}.deed>", index + 1), *text))
        .collect::<Vec<_>>();
    let subjects = ids.len();
    for module in shipped_for(texts.iter().copied()) {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("<shipped>/{module}.deed"), text));
    }
    let checks = check_all(&sources, &ids);
    ReviewSide {
        sources,
        checks,
        subjects,
    }
}

fn refusal(label: &str, side: &ReviewSide) -> Option<String> {
    let errors = side.checks.iter().map(Checked::error_count).sum::<usize>();
    let unnamed = side.checks[..side.subjects]
        .iter()
        .filter(|checked| checked.module.name.is_none())
        .count();
    if errors == 0 && unnamed == 0 {
        return None;
    }

    let mut text = json_report(&side.sources, &side.checks, false);
    text.push_str(&format!(
        "{{\"kind\":\"review_refused\",\"side\":{},\"errors\":{errors},\"unnamed\":{unnamed},\"message\":{}}}\n",
        json_string(label),
        json_string("every reviewed source must check and declare a module")
    ));
    Some(text)
}

fn compare_obligations(
    key: &(String, String),
    before: Option<&DeclarationSnapshot>,
    after: &DeclarationSnapshot,
    receipt: &mut ReviewReceipt,
) {
    let mut by_subject: BTreeMap<&str, Vec<(usize, Tier)>> = BTreeMap::new();
    for ((subject, occurrence), tier) in &after.obligations {
        by_subject
            .entry(subject)
            .or_default()
            .push((*occurrence, *tier));
    }

    for (subject, mut after) in by_subject {
        let mut before = before
            .into_iter()
            .flat_map(|function| &function.obligations)
            .filter(|((old_subject, _), _)| old_subject == subject)
            .map(|(_, tier)| *tier)
            .collect::<Vec<_>>();

        // Written order is not identity. Match evidence that stayed at the
        // same tier first, so moving two calls cannot manufacture a change.
        after.retain(|(_, tier)| {
            let Some(at) = before.iter().position(|old| old == tier) else {
                return true;
            };
            before.remove(at);
            false
        });
        before.sort_by_key(|tier| tier_rank(*tier));
        after.sort_by_key(|(_, tier)| tier_rank(*tier));

        let paired = before.len().min(after.len());
        for (before, (occurrence, after)) in before.iter().zip(&after).take(paired) {
            if matches!(
                (*before, *after),
                (Tier::Proven, Tier::Tested | Tier::Guarded) | (Tier::Tested, Tier::Guarded)
            ) {
                receipt.tier_regressions.push(TierRegression {
                    module: key.0.clone(),
                    declaration: key.1.clone(),
                    subject: subject.to_string(),
                    occurrence: *occurrence,
                    before: *before,
                    after: *after,
                });
            }
        }
        for (occurrence, tier) in after.into_iter().skip(paired) {
            if tier == Tier::Guarded {
                receipt.guarded_added.push(GuardedAddition {
                    module: key.0.clone(),
                    declaration: key.1.clone(),
                    subject: subject.to_string(),
                    occurrence,
                });
            }
        }
    }
}

#[derive(Default)]
struct DeclarationSnapshot {
    authority: BTreeSet<RowEntry>,
    obligations: BTreeMap<(String, usize), Tier>,
}

fn snapshot(checks: &[Checked]) -> BTreeMap<(String, String), DeclarationSnapshot> {
    let mut out: BTreeMap<(String, String), DeclarationSnapshot> = BTreeMap::new();
    for checked in checks {
        let Some(module) = checked
            .module
            .name
            .as_ref()
            .map(|name| name.to_string_path())
        else {
            continue;
        };
        let exports = Exports::of(&checked.module);
        for name in exports.names() {
            let export = exports
                .get(name)
                .expect("an exported name should be readable");
            if !matches!(export.kind, ExportKind::Function | ExportKind::Handler) {
                continue;
            }
            out.entry((module.clone(), name.to_string()))
                .or_default()
                .authority
                .extend(export.row.iter().cloned());
        }
        for item in &checked.module.items {
            let Item::Function(function) = item else {
                continue;
            };
            let name = function.sig.name.name.clone();
            let snapshot = out.entry((module.clone(), name)).or_default();

            let mut occurrences: HashMap<&str, usize> = HashMap::new();
            for ObligationReport {
                tier,
                span,
                subject,
                ..
            } in &checked.obligations
            {
                if !function.span.contains_span(*span) {
                    continue;
                }
                let occurrence = occurrences.entry(subject).or_default();
                snapshot
                    .obligations
                    .insert((subject.clone(), *occurrence), *tier);
                *occurrence += 1;
            }
        }
    }
    out
}

fn authority(row: &RowEntry, current_module: &str) -> String {
    if row.variable {
        return format!("row {}", row.effect);
    }
    let effect = if row.module.is_empty() || row.module == current_module {
        row.effect.clone()
    } else {
        format!("{}/{}", row.module, row.effect)
    };
    match &row.operation {
        Some(operation) => format!("{effect}.{operation}"),
        None => effect,
    }
}

fn covers(old: &RowEntry, new: &RowEntry) -> bool {
    old == new
        || (!old.variable
            && !new.variable
            && old.module == new.module
            && old.effect == new.effect
            && old.operation.is_none())
}

fn tier_rank(tier: Tier) -> u8 {
    match tier {
        Tier::Proven => 0,
        Tier::Tested => 1,
        Tier::Guarded => 2,
    }
}
