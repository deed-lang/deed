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

use deed_ast::{ChoiceDecl, FnDecl, Item, Module, RecordDecl, Type, TypeAlias};
use deed_diagnostics::{Diagnostic, FileId, Span};
use deed_resolve::{DefId, DefKind, Resolutions};

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

/// What happened when one generated input was handed to a runtime.
pub enum PropertyAttempt {
    Passed(Value),
    /// The input violated a precondition, which makes the generator a bad
    /// caller rather than the function a bad function.
    Rejected,
    Failed(Diagnostic),
}

/// The reference implementation of contract checking for generated inputs.
pub struct PropertyInterpreter<'a> {
    interp: Interp<'a>,
}

impl<'a> PropertyInterpreter<'a> {
    pub fn new(program: &Program<'a>, file: FileId) -> Self {
        Self {
            interp: Interp::make(program, file),
        }
    }

    pub fn attempt(&mut self, function: &'a FnDecl, args: &[Value]) -> PropertyAttempt {
        attempt(&mut self.interp, function, args)
    }
}

/// Inputs generated for one function.
pub struct GeneratedInputs {
    /// Inputs that satisfied the function's preconditions.
    pub cases: Vec<Vec<Value>>,
    /// Inputs discarded because they violated a `where` clause or failed while
    /// being exercised for generation.
    pub rejected: usize,
    pub seed: u64,
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
    let mut interp = PropertyInterpreter::new(program, file);
    run_properties_with(
        program,
        file,
        module,
        resolutions,
        config,
        |function, args| interp.attempt(function, args),
    )
}

/// Runs generated properties, handing each input to the runtime supplied by
/// the caller.
pub fn run_properties_with<'a, F>(
    program: &Program<'a>,
    file: FileId,
    module: &'a Module,
    resolutions: &'a Resolutions,
    config: PropertyConfig,
    mut attempt: F,
) -> Vec<PropertyOutcome>
where
    F: FnMut(&'a FnDecl, &[Value]) -> PropertyAttempt,
{
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) if is_testable(function, module, resolutions) => {
                Some(function)
            }
            _ => None,
        })
        .map(|function| {
            run_property_with(
                program,
                file,
                module,
                resolutions,
                function,
                config,
                &mut attempt,
            )
        })
        .collect()
}

/// Generates inputs for one function, using the same generator and precondition
/// filtering as property tests.
pub fn generate_inputs<'a>(
    program: &Program<'a>,
    file: FileId,
    module: &'a Module,
    resolutions: &'a Resolutions,
    function: &'a FnDecl,
    config: PropertyConfig,
) -> GeneratedInputs {
    let types = TypeIndex::new(module, resolutions);
    let mut rng = Rng::new(config.seed);
    let mut interp = Interp::make(program, file);
    let mut cases = Vec::new();
    let mut rejected = 0usize;
    let budget = config.cases * 20;

    for _ in 0..budget {
        if cases.len() >= config.cases {
            break;
        }
        let Some(args) = generate_arguments(&types, &mut rng, function, &mut interp) else {
            rejected += 1;
            continue;
        };
        match attempt(&mut interp, function, &args) {
            PropertyAttempt::Passed(_) => cases.push(args),
            PropertyAttempt::Rejected | PropertyAttempt::Failed(_) => rejected += 1,
        }
    }

    GeneratedInputs {
        cases,
        rejected,
        seed: config.seed,
    }
}

/// Shrinks a failing generated input with the existing property shrinker.
///
/// `still_failing` should return true exactly when the candidate still
/// reproduces the finding being shrunk.
pub fn shrink_inputs<'a, F>(
    program: &Program<'a>,
    file: FileId,
    module: &'a Module,
    resolutions: &'a Resolutions,
    mut args: Vec<Value>,
    mut still_failing: F,
) -> Vec<Value>
where
    F: FnMut(&[Value]) -> bool,
{
    if !still_failing(&args) {
        return args;
    }

    let types = TypeIndex::new(module, resolutions);
    let mut interp = Interp::make(program, file);
    let simpler = Simpler::of(&types, &mut interp);
    let mut budget = 300usize;

    'outer: while budget > 0 {
        for index in 0..args.len() {
            for candidate in simpler.smaller(&args[index]) {
                budget = budget.saturating_sub(1);
                if budget == 0 {
                    break 'outer;
                }
                let mut attempt = args.clone();
                attempt[index] = candidate;
                if still_failing(&attempt) {
                    args = attempt;
                    continue 'outer;
                }
            }
        }
        break;
    }

    args
}

fn run_property_with<'a, F>(
    program: &Program<'a>,
    file: FileId,
    module: &'a Module,
    resolutions: &'a Resolutions,
    function: &'a FnDecl,
    config: PropertyConfig,
    attempt: &mut F,
) -> PropertyOutcome
where
    F: FnMut(&'a FnDecl, &[Value]) -> PropertyAttempt,
{
    let types = TypeIndex::new(module, resolutions);
    let mut rng = Rng::new(config.seed);
    let mut generator = Interp::make(program, file);

    let mut cases = 0usize;
    let mut attempts = 0usize;
    let mut rejected = 0usize;
    // A generator that keeps producing inputs the preconditions throw away is
    // not testing anything, so give up rather than run forever.
    let budget = config.cases * 20;

    for _ in 0..budget {
        if cases >= config.cases {
            break;
        }

        let Some(args) = generate_arguments(&types, &mut rng, function, &mut generator) else {
            continue;
        };
        attempts += 1;

        match attempt(function, &args) {
            PropertyAttempt::Passed(_) => cases += 1,
            PropertyAttempt::Rejected => rejected = attempts - cases,
            PropertyAttempt::Failed(diagnostic) => {
                let simpler = Simpler::of(&types, &mut generator);
                let (args, diagnostic) = shrink(function, args, diagnostic, &simpler, attempt);
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

fn attempt<'a>(interp: &mut Interp<'a>, function: &'a FnDecl, args: &[Value]) -> PropertyAttempt {
    let call_args = args
        .iter()
        .cloned()
        .map(|value| (value, function.sig.name.span))
        .collect();

    match interp.call_from_outside(function, call_args, function.sig.name.span) {
        Ok(value) => PropertyAttempt::Passed(value),
        Err(diagnostic) if diagnostic.code == codes::PRECONDITION_FAILED => {
            PropertyAttempt::Rejected
        }
        Err(diagnostic) => PropertyAttempt::Failed(*diagnostic),
    }
}

/// Shrinks a failing input.
///
/// Integers get a binary search toward zero. Greedy halving overshoots a
/// boundary and then crawls towards it one step at a time, which is how a
/// counterexample ends up being `8968765479210500849` when `4611686018427387904`
/// was the answer.
///
/// Everything else shrinks greedily: try each smaller form of each argument,
/// keep the first that still fails, and go round again. That covers every shape
/// the generator can produce, which is the property this has to have. A
/// counterexample built out of something nothing shrinks is a counterexample
/// nobody reads, and it looks exactly like a small one that happens to be
/// awkward.
fn shrink<'a, F>(
    function: &'a FnDecl,
    mut args: Vec<Value>,
    mut failure: Diagnostic,
    simpler: &Simpler,
    attempt: &mut F,
) -> (Vec<Value>, Diagnostic)
where
    F: FnMut(&'a FnDecl, &[Value]) -> PropertyAttempt,
{
    let mut budget = 300usize;

    for index in 0..args.len() {
        if matches!(args[index], Value::Int(_)) {
            shrink_int(
                function,
                &mut args,
                index,
                &mut failure,
                &mut budget,
                attempt,
            );
        }
    }

    'outer: while budget > 0 {
        for index in 0..args.len() {
            if matches!(args[index], Value::Int(_)) {
                continue;
            }
            for candidate in simpler.smaller(&args[index]) {
                budget = budget.saturating_sub(1);
                if budget == 0 {
                    break 'outer;
                }

                let mut attempt_args = args.clone();
                attempt_args[index] = candidate;
                if let PropertyAttempt::Failed(diagnostic) = attempt(function, &attempt_args) {
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

fn shrink_int<'a, F>(
    function: &'a FnDecl,
    args: &mut [Value],
    index: usize,
    failure: &mut Diagnostic,
    budget: &mut usize,
    attempt: &mut F,
) where
    F: FnMut(&'a FnDecl, &[Value]) -> PropertyAttempt,
{
    let Value::Int(start) = args[index] else {
        return;
    };
    if start == 0 {
        return;
    }

    let signed = |magnitude: i64| {
        if start.is_negative() {
            0i64.saturating_sub(magnitude)
        } else {
            magnitude
        }
    };
    let magnitude = start.checked_abs().unwrap_or(i64::MAX);

    // If zero fails too, there is no smaller counterexample to look for.
    if let Some(diagnostic) = try_value(function, args, index, 0, budget, attempt) {
        args[index] = Value::Int(0);
        *failure = diagnostic;
        return;
    }

    let mut good = 0i64;
    let mut bad = magnitude;

    for _ in 0..*budget {
        if bad - good == 1 {
            break;
        }
        let middle = good + (bad - good) / 2;
        match try_value(function, args, index, signed(middle), budget, attempt) {
            Some(diagnostic) => {
                bad = middle;
                *failure = diagnostic;
            }
            None => good = middle,
        }
    }

    args[index] = Value::Int(signed(bad));
    if let Some(diagnostic) = attempt_with(function, args, attempt) {
        *failure = diagnostic;
    }
}

/// Runs the function with one argument replaced, restoring it afterwards.
fn try_value<'a, F>(
    function: &'a FnDecl,
    args: &mut [Value],
    index: usize,
    candidate: i64,
    budget: &mut usize,
    attempt: &mut F,
) -> Option<Diagnostic>
where
    F: FnMut(&'a FnDecl, &[Value]) -> PropertyAttempt,
{
    *budget = budget.saturating_sub(1);
    let original = args[index].clone();
    args[index] = Value::Int(candidate);
    let outcome = attempt_with(function, args, attempt);
    args[index] = original;
    outcome
}

fn attempt_with<'a, F>(function: &'a FnDecl, args: &[Value], attempt: &mut F) -> Option<Diagnostic>
where
    F: FnMut(&'a FnDecl, &[Value]) -> PropertyAttempt,
{
    match attempt(function, args) {
        PropertyAttempt::Failed(diagnostic) => Some(diagnostic),
        _ => None,
    }
}

/// A variant's identity: the module that declared it, and its name there.
///
/// The same identity [`Value::variant`] carries, for the same reason. A
/// `DefId` is an index into one module's table, so it would not survive the
/// trip out here.
type VariantId = (Rc<str>, String);

/// Which variants a value could be replaced by, worked out once.
///
/// A variant cannot be shrunk from the value alone: `One { n: 0 }` says nothing
/// about there being a `Nothing` next to it. So the choices are walked once and
/// every variant is told which of its siblings carry no fields, which are the
/// only ones that can be built here without inventing values for them.
///
/// Everything else shrinks from the value, so this is the only table.
#[derive(Default)]
struct Simpler {
    /// Variant identity to the fieldless variants of the same choice.
    fieldless: HashMap<VariantId, Vec<VariantId>>,
}

impl Simpler {
    fn of<'a>(types: &TypeIndex<'a>, interp: &mut Interp<'a>) -> Simpler {
        let mut fieldless = HashMap::new();

        for choice in types.choices.values() {
            let mut identities = Vec::new();
            let mut empty = Vec::new();
            for variant in &choice.variants {
                let Some(def) = types.resolutions.resolution(variant.name.span) else {
                    continue;
                };
                let Some(identity) = interp.variant_identity(def) else {
                    continue;
                };
                if variant.fields.iter().flatten().next().is_none() {
                    empty.push(identity.clone());
                }
                identities.push(identity);
            }

            for identity in identities {
                // A fieldless variant is not simpler than itself, and a list
                // that held it would make the shrinker go round for nothing.
                let others: Vec<_> = empty
                    .iter()
                    .filter(|it| **it != identity)
                    .cloned()
                    .collect();
                if !others.is_empty() {
                    fieldless.insert(identity, others);
                }
            }
        }

        Simpler { fieldless }
    }

    /// Smaller forms of a value, best first.
    ///
    /// Best first matters: the loop keeps the first candidate that still fails
    /// and starts again, so putting the emptiest form at the front is what gets
    /// a counterexample down in a few steps rather than a few hundred.
    fn smaller(&self, value: &Value) -> Vec<Value> {
        match value {
            Value::Record(fields) => self
                .shrink_fields(fields)
                .into_iter()
                .map(Value::record)
                .collect(),

            Value::Variant(variant) => {
                let mut out: Vec<Value> = self
                    .fieldless
                    .get(&(Rc::clone(&variant.origin), variant.name.clone()))
                    .into_iter()
                    .flatten()
                    .map(|(origin, name)| {
                        Value::variant(Rc::clone(origin), name.clone(), Fields::new())
                    })
                    .collect();
                out.extend(
                    self.shrink_fields(&variant.fields)
                        .into_iter()
                        .map(|fields| {
                            Value::variant(Rc::clone(&variant.origin), variant.name.clone(), fields)
                        }),
                );
                out
            }

            // Fewer elements first, then simpler elements. A list that is too
            // long and a list whose contents are too big are two different
            // complaints, and the first one is the one worth answering.
            Value::List(elements) if !elements.is_empty() => {
                let mut out = vec![Value::list(Vec::new())];
                if elements.len() > 1 {
                    out.push(Value::list(elements[..elements.len() / 2].to_vec()));
                    for skip in 0..elements.len() {
                        let mut shorter = elements.as_ref().clone();
                        shorter.remove(skip);
                        out.push(Value::list(shorter));
                    }
                }
                for (index, element) in elements.iter().enumerate() {
                    for candidate in self.smaller(element) {
                        let mut copy = elements.as_ref().clone();
                        copy[index] = candidate;
                        out.push(Value::list(copy));
                    }
                }
                out
            }

            // Shorter first, then plainer. `"a"` is the emptiest thing a
            // non-empty string can be, and a counterexample that says the
            // letters did not matter is worth as much as one that says the
            // length did not.
            Value::Str(text) if !text.is_empty() => {
                let characters: Vec<char> = text.chars().collect();
                let mut out = vec![Value::str("")];
                if characters.len() > 1 {
                    let half: String = characters[..characters.len() / 2].iter().collect();
                    out.push(Value::str(half));
                    for skip in 0..characters.len() {
                        let shorter: String = characters
                            .iter()
                            .enumerate()
                            .filter(|(index, _)| *index != skip)
                            .map(|(_, c)| *c)
                            .collect();
                        out.push(Value::str(shorter));
                    }
                }
                for (index, character) in characters.iter().enumerate() {
                    if *character != 'a' {
                        let mut copy = characters.clone();
                        copy[index] = 'a';
                        out.push(Value::str(copy.into_iter().collect::<String>()));
                    }
                }
                out
            }

            // The payload, and only the payload. `ok` and `err` are two
            // outcomes rather than a big one and a small one, and turning one
            // into the other means inventing a value for the other side, which
            // is the generator's job.
            Value::Result { ok, value } => self
                .smaller(value)
                .into_iter()
                .map(|inner| {
                    if *ok {
                        Value::ok(inner)
                    } else {
                        Value::err(inner)
                    }
                })
                .collect(),

            // `false` is the emptier of the two, the way zero is for an
            // integer and the empty list is for a list.
            Value::Bool(true) => vec![Value::Bool(false)],

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

    fn shrink_fields(&self, fields: &Fields) -> Vec<Fields> {
        let mut out = Vec::new();
        for (name, value) in fields {
            for candidate in self.smaller(value) {
                let mut copy = fields.clone();
                copy.insert(name.clone(), candidate);
                out.push(copy);
            }
        }
        out
    }
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
            // Nothing sensible to invent. A generated function would be a
            // constant one, and a property tested against a constant is a
            // property about nothing.
            Type::Fn { .. } => false,
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
                        "List" => args.len() == 1 && self.can_generate(&args[0], depth + 1),
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
            Type::Fn { .. } => None,
            Type::Named { name, args, .. } => {
                let def = self.resolutions.resolution(name.span)?;
                match self.resolutions.def(def).kind {
                    DefKind::Builtin => match name.name.as_str() {
                        "Int" => Some(Value::Int(rng.int())),
                        "Bool" => Some(Value::Bool(rng.next().is_multiple_of(2))),
                        "String" => Some(Value::Str(rng.word().into())),
                        "Result" => {
                            let ok = rng.next().is_multiple_of(2);
                            let inner =
                                self.generate(args.get(usize::from(!ok))?, rng, interp, depth + 1)?;
                            Some(if ok {
                                Value::ok(inner)
                            } else {
                                Value::err(inner)
                            })
                        }
                        // Short on purpose. A counterexample with forty
                        // elements in it is a counterexample nobody reads,
                        // and the empty list is the case worth hitting often.
                        "List" => {
                            let element = args.first()?;
                            let count = rng.next() % 4;
                            let mut elements = Vec::new();
                            for _ in 0..count {
                                elements.push(self.generate(element, rng, interp, depth + 1)?);
                            }
                            Some(Value::list(elements))
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
                                    if interp.satisfies(def, predicate, &candidate) {
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
    use super::{Rng, Simpler};
    use crate::value::Value;

    fn smaller(value: &Value) -> Vec<Value> {
        Simpler::default().smaller(value)
    }

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
    fn a_list_gets_shorter_before_its_elements_get_smaller() {
        let list = Value::list(vec![Value::Int(9), Value::Int(8)]);
        let candidates = smaller(&list);

        // The emptiest form first, because the loop keeps the first candidate
        // that still fails and starts again.
        assert_eq!(candidates[0], Value::list(Vec::new()));
        assert!(candidates.contains(&Value::list(vec![Value::Int(9)])));
        assert!(candidates.contains(&Value::list(vec![Value::Int(8)])));
        assert!(candidates.contains(&Value::list(vec![Value::Int(0), Value::Int(8)])));

        assert!(smaller(&Value::list(Vec::new())).is_empty());
    }

    #[test]
    fn a_string_gets_shorter_and_then_plainer() {
        let candidates = smaller(&Value::str("qx"));

        assert_eq!(candidates[0], Value::str(""));
        assert!(candidates.contains(&Value::str("q")));
        assert!(candidates.contains(&Value::str("x")));
        assert!(candidates.contains(&Value::str("ax")));
        assert!(candidates.contains(&Value::str("qa")));

        assert!(smaller(&Value::str("")).is_empty());
        // Already as plain as it gets, so there is nothing left to try beyond
        // making it shorter.
        assert_eq!(smaller(&Value::str("a")), vec![Value::str("")]);
    }

    #[test]
    fn true_is_bigger_than_false() {
        assert_eq!(smaller(&Value::Bool(true)), vec![Value::Bool(false)]);
        assert!(smaller(&Value::Bool(false)).is_empty());
    }

    #[test]
    fn things_with_no_smaller_form_do_not_shrink() {
        assert!(smaller(&Value::Unit).is_empty());
    }
}
