//! Property tests generated from contracts.
//!
//! This is the `Tested` tier in `design/02-syntax.md`. The other two have
//! existed for a while: `Proven` discharges constant expressions statically and
//! `Guarded` checks at runtime. This one is the distinctive claim, that you
//! write the contract and the language produces the tests.
//!
//! The runner does one thing: it supplies inputs. Judging them is already the
//! interpreter's job, since contracts are enforced on every call, so a
//! postcondition failure surfaces here as an ordinary diagnostic and there is
//! no second implementation of the check to disagree with the first.
//!
//! The same trick handles preconditions. A generated input that violates a
//! `where` clause makes the caller wrong, and the runtime already says so with
//! its own code. The runner reads that code as "discard this input", which
//! means there is no separate precondition path either.

use std::collections::HashMap;
use std::rc::Rc;

use vow_ast::{ChoiceDecl, FnDecl, Item, Module, RecordDecl, Type, TypeAlias};
use vow_diagnostics::{Diagnostic, FileId, Span};
use vow_resolve::{DefId, DefKind, Resolutions};

use crate::codes;
use crate::interp::{Interp, Program};
use crate::value::{Fields, Value};

/// How hard to try.
#[derive(Clone, Copy, Debug)]
pub struct PropertyConfig {
    pub cases: usize,
    /// Fixed by default, and reported, because a property test you cannot
    /// reproduce is a rumour.
    pub seed: u64,
}

impl Default for PropertyConfig {
    fn default() -> Self {
        Self {
            cases: 100,
            seed: 0x5EED_1234_ABCD_0001,
        }
    }
}

pub struct PropertyOutcome {
    pub function: String,
    pub span: Span,
    /// Inputs that satisfied the preconditions and were actually run.
    pub cases: usize,
    /// Inputs discarded because they violated a `where` clause.
    pub rejected: usize,
    pub seed: u64,
    pub failure: Option<Diagnostic>,
}

impl PropertyOutcome {
    pub fn passed(&self) -> bool {
        self.failure.is_none()
    }
}

/// Whether a function's contract can be exercised by generated inputs.
///
/// Pure, has something to prove, and every parameter has a type the generator
/// understands. Effectful functions are excluded because running one needs a
/// handler, and inventing a handler means inventing the behaviour the property
/// would then be checking against itself.
pub fn is_testable(function: &FnDecl, module: &Module, resolutions: &Resolutions) -> bool {
    if !function.contract.is_pure() || function.contract.ensures.is_empty() {
        return false;
    }
    let types = TypeIndex::new(module, resolutions);
    function
        .sig
        .params
        .iter()
        .all(|param| matches!(&param.ty, Some(ty) if types.can_generate(ty, 0)))
}

/// Runs a generated property test for every function that has one.
pub fn run_properties<'a>(
    program: &Program<'a>,
    file: FileId,
    module: &'a Module,
    resolutions: &'a Resolutions,
    config: PropertyConfig,
) -> Vec<PropertyOutcome> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) if is_testable(function, module, resolutions) => {
                Some(function)
            }
            _ => None,
        })
        .map(|function| run_property(program, file, module, resolutions, function, config))
        .collect()
}

fn run_property<'a>(
    program: &Program<'a>,
    file: FileId,
    module: &'a Module,
    resolutions: &'a Resolutions,
    function: &'a FnDecl,
    config: PropertyConfig,
) -> PropertyOutcome {
    let types = TypeIndex::new(module, resolutions);
    let mut rng = Rng::new(config.seed);
    let mut interp = Interp::make(program, file);

    let mut cases = 0usize;
    let mut rejected = 0usize;
    // A generator that keeps producing inputs the preconditions throw away is
    // not testing anything, so give up rather than run forever.
    let budget = config.cases * 20;

    for _ in 0..budget {
        if cases >= config.cases {
            break;
        }

        let Some(args) = generate_arguments(&types, &mut rng, function, &mut interp) else {
            rejected += 1;
            continue;
        };

        match attempt(&mut interp, function, &args) {
            Attempt::Passed => cases += 1,
            Attempt::Rejected => rejected += 1,
            Attempt::Failed(diagnostic) => {
                let (args, diagnostic) = shrink(&mut interp, function, args, diagnostic);
                return PropertyOutcome {
                    function: function.sig.name.name.clone(),
                    span: function.sig.name.span,
                    cases,
                    rejected,
                    seed: config.seed,
                    failure: Some(with_counterexample(
                        diagnostic,
                        function,
                        &args,
                        config.seed,
                    )),
                };
            }
        }
    }

    let failure = (cases < config.cases).then(|| {
        Diagnostic::warning(
            codes::NOT_ENOUGH_CASES,
            file,
            function.sig.name.span,
            format!(
                "only {cases} of {} generated inputs got past the preconditions",
                config.cases
            ),
        )
        .with_primary_label("not enough cases")
        .with_note(
            "a property that only tested a handful of inputs is worse than no property, because it looks like one",
        )
    });

    PropertyOutcome {
        function: function.sig.name.name.clone(),
        span: function.sig.name.span,
        cases,
        rejected,
        seed: config.seed,
        failure,
    }
}

enum Attempt {
    Passed,
    /// The input violated a precondition, which makes the generator a bad
    /// caller rather than the function a bad function.
    Rejected,
    Failed(Diagnostic),
}

fn attempt<'a>(interp: &mut Interp<'a>, function: &'a FnDecl, args: &[Value]) -> Attempt {
    let call_args = args
        .iter()
        .cloned()
        .map(|value| (value, function.sig.name.span))
        .collect();

    match interp.call_from_outside(function, call_args, function.sig.name.span) {
        Ok(_) => Attempt::Passed,
        Err(diagnostic) if diagnostic.code == codes::PRECONDITION_FAILED => Attempt::Rejected,
        Err(diagnostic) => Attempt::Failed(*diagnostic),
    }
}

/// Shrinks a failing input.
///
/// Integers get a binary search toward zero. Greedy halving overshoots a
/// boundary and then crawls towards it one step at a time, which is how a
/// counterexample ends up being `8968765479210500849` when `4611686018427387904`
/// was the answer.
///
/// Fields of records and variants shrink greedily, one at a time. Nothing else
/// shrinks. That is a real limitation and is written down rather than left to
/// be discovered.
fn shrink<'a>(
    interp: &mut Interp<'a>,
    function: &'a FnDecl,
    mut args: Vec<Value>,
    mut failure: Diagnostic,
) -> (Vec<Value>, Diagnostic) {
    let mut budget = 300usize;

    for index in 0..args.len() {
        if matches!(args[index], Value::Int(_)) {
            shrink_int(
                interp,
                function,
                &mut args,
                index,
                &mut failure,
                &mut budget,
            );
        }
    }

    'outer: while budget > 0 {
        for index in 0..args.len() {
            if matches!(args[index], Value::Int(_)) {
                continue;
            }
            for candidate in smaller(&args[index]) {
                budget = budget.saturating_sub(1);
                if budget == 0 {
                    break 'outer;
                }

                let mut attempt_args = args.clone();
                attempt_args[index] = candidate;
                if let Attempt::Failed(diagnostic) = attempt(interp, function, &attempt_args) {
                    args = attempt_args;
                    failure = diagnostic;
                    continue 'outer;
                }
            }
        }
        break;
    }

    (args, failure)
}

fn shrink_int<'a>(
    interp: &mut Interp<'a>,
    function: &'a FnDecl,
    args: &mut [Value],
    index: usize,
    failure: &mut Diagnostic,
    budget: &mut usize,
) {
    let Value::Int(start) = args[index] else {
        return;
    };
    if start == 0 {
        return;
    }

    let sign = if start > 0 { 1i64 } else { -1 };
    let magnitude = start.checked_abs().unwrap_or(i64::MAX);

    // If zero fails too, there is no smaller counterexample to look for.
    if let Some(diagnostic) = try_value(interp, function, args, index, 0, budget) {
        args[index] = Value::Int(0);
        *failure = diagnostic;
        return;
    }

    let mut good = 0i64;
    let mut bad = magnitude;

    while bad - good > 1 && *budget > 0 {
        let middle = good + (bad - good) / 2;
        match try_value(interp, function, args, index, middle * sign, budget) {
            Some(diagnostic) => {
                bad = middle;
                *failure = diagnostic;
            }
            None => good = middle,
        }
    }

    args[index] = Value::Int(bad * sign);
    if let Some(diagnostic) = attempt_with(interp, function, args) {
        *failure = diagnostic;
    }
}

/// Runs the function with one argument replaced, restoring it afterwards.
fn try_value<'a>(
    interp: &mut Interp<'a>,
    function: &'a FnDecl,
    args: &mut [Value],
    index: usize,
    candidate: i64,
    budget: &mut usize,
) -> Option<Diagnostic> {
    *budget = budget.saturating_sub(1);
    let original = args[index].clone();
    args[index] = Value::Int(candidate);
    let outcome = attempt_with(interp, function, args);
    args[index] = original;
    outcome
}

fn attempt_with<'a>(
    interp: &mut Interp<'a>,
    function: &'a FnDecl,
    args: &[Value],
) -> Option<Diagnostic> {
    match attempt(interp, function, args) {
        Attempt::Failed(diagnostic) => Some(diagnostic),
        _ => None,
    }
}

fn smaller(value: &Value) -> Vec<Value> {
    match value {
        Value::Record(fields) => shrink_fields(fields)
            .into_iter()
            .map(Value::record)
            .collect(),
        Value::Variant(variant) => shrink_fields(&variant.fields)
            .into_iter()
            .map(|fields| Value::variant(Rc::clone(&variant.origin), variant.name.clone(), fields))
            .collect(),
        Value::Int(0) => Vec::new(),
        Value::Int(n) => {
            let mut candidates = vec![Value::Int(0)];
            if n.abs() > 1 {
                candidates.push(Value::Int(n / 2));
            }
            candidates.push(Value::Int(if *n > 0 { n - 1 } else { n + 1 }));
            candidates
        }
        _ => Vec::new(),
    }
}

fn shrink_fields(fields: &Fields) -> Vec<Fields> {
    let mut out = Vec::new();
    for (name, value) in fields {
        for candidate in smaller(value) {
            let mut copy = fields.clone();
            copy.insert(name.clone(), candidate);
            out.push(copy);
        }
    }
    out
}

fn with_counterexample(
    diagnostic: Diagnostic,
    function: &FnDecl,
    args: &[Value],
    seed: u64,
) -> Diagnostic {
    let shown: Vec<String> = function
        .sig
        .params
        .iter()
        .zip(args)
        .map(|(param, value)| format!("{} = {value}", param.name.name))
        .collect();

    diagnostic
        .with_note(format!("generated input: {}", shown.join(", ")))
        .with_note(format!("seed {seed:#x}, so this reproduces"))
}

// -- generating ------------------------------------------------------------

fn generate_arguments<'a>(
    types: &TypeIndex<'a>,
    rng: &mut Rng,
    function: &FnDecl,
    interp: &mut Interp<'a>,
) -> Option<Vec<Value>> {
    function
        .sig
        .params
        .iter()
        .map(|param| types.generate(param.ty.as_ref()?, rng, interp, 0))
        .collect()
}

/// What the generator knows about the types in a module.
struct TypeIndex<'a> {
    resolutions: &'a Resolutions,
    records: HashMap<DefId, &'a RecordDecl>,
    choices: HashMap<DefId, &'a ChoiceDecl>,
    aliases: HashMap<DefId, &'a TypeAlias>,
}

impl<'a> TypeIndex<'a> {
    fn new(module: &'a Module, resolutions: &'a Resolutions) -> Self {
        let mut index = TypeIndex {
            resolutions,
            records: HashMap::new(),
            choices: HashMap::new(),
            aliases: HashMap::new(),
        };

        for item in &module.items {
            match item {
                Item::Record(record) => {
                    if let Some(def) = resolutions.resolution(record.name.span) {
                        index.records.insert(def, record);
                    }
                }
                Item::Choice(choice) => {
                    if let Some(def) = resolutions.resolution(choice.name.span) {
                        index.choices.insert(def, choice);
                    }
                }
                Item::TypeAlias(alias) => {
                    if let Some(def) = resolutions.resolution(alias.name.span) {
                        index.aliases.insert(def, alias);
                    }
                }
                _ => {}
            }
        }

        index
    }

    fn can_generate(&self, ty: &Type, depth: usize) -> bool {
        if depth > 4 {
            return false;
        }
        match ty {
            Type::Unit(_) => true,
            Type::Error(_) => false,
            Type::Named { name, args, .. } => {
                let Some(def) = self.resolutions.resolution(name.span) else {
                    return false;
                };
                match self.resolutions.def(def).kind {
                    DefKind::Builtin => match name.name.as_str() {
                        "Int" | "Bool" | "String" => true,
                        "Result" => {
                            args.len() == 2 && args.iter().all(|a| self.can_generate(a, depth + 1))
                        }
                        _ => false,
                    },
                    DefKind::Record => self.records.get(&def).is_some_and(|record| {
                        record
                            .fields
                            .iter()
                            .all(|field| self.can_generate(&field.ty, depth + 1))
                    }),
                    DefKind::Choice => self.choices.get(&def).is_some_and(|choice| {
                        !choice.variants.is_empty()
                            && choice.variants.iter().all(|variant| {
                                variant
                                    .fields
                                    .iter()
                                    .flatten()
                                    .all(|field| self.can_generate(&field.ty, depth + 1))
                            })
                    }),
                    DefKind::Type => self
                        .aliases
                        .get(&def)
                        .is_some_and(|alias| self.can_generate(&alias.ty, depth + 1)),
                    _ => false,
                }
            }
        }
    }

    fn generate(
        &self,
        ty: &Type,
        rng: &mut Rng,
        interp: &mut Interp<'a>,
        depth: usize,
    ) -> Option<Value> {
        if depth > 4 {
            return None;
        }

        match ty {
            Type::Unit(_) => Some(Value::Unit),
            Type::Error(_) => None,
            Type::Named { name, args, .. } => {
                let def = self.resolutions.resolution(name.span)?;
                match self.resolutions.def(def).kind {
                    DefKind::Builtin => match name.name.as_str() {
                        "Int" => Some(Value::Int(rng.int())),
                        "Bool" => Some(Value::Bool(rng.next() % 2 == 0)),
                        "String" => Some(Value::Str(rng.word().into())),
                        "Result" => {
                            let ok = rng.next() % 2 == 0;
                            let inner =
                                self.generate(args.get(usize::from(!ok))?, rng, interp, depth + 1)?;
                            Some(if ok {
                                Value::ok(inner)
                            } else {
                                Value::err(inner)
                            })
                        }
                        _ => None,
                    },

                    DefKind::Record => {
                        let record = self.records.get(&def)?;
                        let mut fields = Fields::new();
                        for field in &record.fields {
                            fields.insert(
                                field.name.name.clone(),
                                self.generate(&field.ty, rng, interp, depth + 1)?,
                            );
                        }
                        Some(Value::record(fields))
                    }

                    DefKind::Choice => {
                        let choice = self.choices.get(&def)?;
                        let index = (rng.next() as usize) % choice.variants.len();
                        let variant = &choice.variants[index];
                        let variant_def = self.resolutions.resolution(variant.name.span)?;
                        let (origin, name) = interp.variant_identity(variant_def)?;

                        let mut fields = Fields::new();
                        for field in variant.fields.iter().flatten() {
                            fields.insert(
                                field.name.name.clone(),
                                self.generate(&field.ty, rng, interp, depth + 1)?,
                            );
                        }
                        Some(Value::variant(origin, name, fields))
                    }

                    DefKind::Type => {
                        let alias = self.aliases.get(&def)?;
                        match &alias.refinement {
                            None => self.generate(&alias.ty, rng, interp, depth + 1),
                            // Rejection sampling. Good enough for the shapes a
                            // refinement usually takes, and honest about
                            // giving up rather than returning something that
                            // does not satisfy the predicate.
                            Some(predicate) => {
                                for _ in 0..64 {
                                    let candidate =
                                        self.generate(&alias.ty, rng, interp, depth + 1)?;
                                    if interp.satisfies(predicate, &candidate) {
                                        return Some(candidate);
                                    }
                                }
                                None
                            }
                        }
                    }

                    _ => None,
                }
            }
        }
    }
}

// -- randomness ------------------------------------------------------------

/// xorshift64. Small, deterministic, and nowhere near a real generator, which
/// is fine for choosing test inputs and would not be for anything else.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Biased towards small values, because that is where the interesting
    /// cases are and where a counterexample is readable.
    fn int(&mut self) -> i64 {
        match self.next() % 10 {
            0 => 0,
            1 => (self.next() % 3) as i64 - 1,
            2..=7 => (self.next() % 201) as i64 - 100,
            8 => (self.next() % 20_001) as i64 - 10_000,
            _ => self.next() as i64,
        }
    }

    fn word(&mut self) -> String {
        let length = (self.next() % 6) as usize;
        (0..length)
            .map(|_| (b'a' + (self.next() % 26) as u8) as char)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Rng, smaller};
    use crate::value::Value;

    #[test]
    fn the_generator_is_deterministic() {
        let mut one = Rng::new(7);
        let mut other = Rng::new(7);
        for _ in 0..50 {
            assert_eq!(one.int(), other.int());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut one = Rng::new(7);
        let mut other = Rng::new(8);
        let a: Vec<i64> = (0..20).map(|_| one.int()).collect();
        let b: Vec<i64> = (0..20).map(|_| other.int()).collect();
        assert_ne!(a, b);
    }

    #[test]
    fn shrinking_moves_integers_towards_zero() {
        assert!(smaller(&Value::Int(0)).is_empty());

        let candidates = smaller(&Value::Int(100));
        assert!(candidates.contains(&Value::Int(0)));
        assert!(candidates.contains(&Value::Int(50)));
        assert!(candidates.contains(&Value::Int(99)));

        let negative = smaller(&Value::Int(-100));
        assert!(negative.contains(&Value::Int(-50)));
        assert!(negative.contains(&Value::Int(-99)));
    }

    #[test]
    fn things_with_no_smaller_form_do_not_shrink() {
        assert!(smaller(&Value::Bool(true)).is_empty());
        assert!(smaller(&Value::Unit).is_empty());
    }
}
