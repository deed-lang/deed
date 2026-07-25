//! What is known about the integers in scope.
//!
//! The Proven tier used to mean "this is a literal and the predicate evaluates
//! to true". That is a small enough slice of the tier that almost every
//! refinement in real code was a runtime check with a paragraph of ceremony
//! around it, and the argument for having refinements at all is that they
//! replace checks rather than decorate them.
//!
//! This is interval reasoning with one relation on top. Each integer-valued
//! name gets a range, ranges come from the things that state one (a `where`
//! clause, an `if`, a guard that leaves, a refined parameter type, the contract
//! of a function being called), and a predicate is discharged by evaluating it
//! over the range rather than over a value.
//!
//! The relation is the difference of two names. An interval has nowhere to put
//! `low < high`, so every contract that says how two arguments relate used to
//! be thrown away, and that is most of what a `where` clause is for. A range
//! per pair of names holds exactly the orderings, which is what comparisons
//! produce and nothing more.
//!
//! # What this cannot do
//!
//! A great deal, and the answer is always "not proven", never a wrong answer.
//!
//! - **Relationships that are not a difference.** `a < b * c` relates three
//!   names through a product, and a pair of bounds has nowhere to put that.
//!   Deciding it is a solver's job.
//! - **A difference larger than an integer.** `low < high` on its own does not
//!   settle `high - low`, because with nothing bounding either name the
//!   subtraction overflows, and an expression with no answer proves nothing
//!   about the answer.
//! - **Anything that is not an integer.** No `String`, no record field, no
//!   variant.
//! - **The payload of a `Result`.** A call that can fail is a `Result` at the
//!   call site, and the range of a `Result` means nothing, so an `ensures` on
//!   a fallible function is not read here.
//! - **Division and remainder.** The sign rules around zero and around
//!   `i64::MIN` are fiddly enough that getting them wrong is worse than not
//!   trying.

use std::collections::{BTreeMap, HashMap};

use vow_ast::{BinaryOp, Expr, UnaryOp};
use vow_resolve::DefId;

/// A closed range of integers, or nothing at all.
///
/// `Empty` is reachable and useful: it is what an impossible branch narrows to,
/// and a predicate over an impossible value is vacuously true. Keeping it in
/// the domain is what stops `if n > 0 { if n < 0 { .. } }` from needing a
/// special case.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Range {
    Empty,
    Bounded { low: i64, high: i64 },
}

impl Range {
    pub const ANY: Range = Range::Bounded {
        low: i64::MIN,
        high: i64::MAX,
    };

    pub fn exactly(value: i64) -> Range {
        Range::Bounded {
            low: value,
            high: value,
        }
    }

    pub fn between(low: i64, high: i64) -> Range {
        if low > high {
            Range::Empty
        } else {
            Range::Bounded { low, high }
        }
    }

    pub fn is_any(self) -> bool {
        self == Range::ANY
    }

    fn bounds(self) -> Option<(i64, i64)> {
        match self {
            Range::Empty => None,
            Range::Bounded { low, high } => Some((low, high)),
        }
    }

    /// Everything in both, which is what narrowing on a condition does.
    pub fn meet(self, other: Range) -> Range {
        match (self.bounds(), other.bounds()) {
            (Some((a_low, a_high)), Some((b_low, b_high))) => {
                Range::between(a_low.max(b_low), a_high.min(b_high))
            }
            _ => Range::Empty,
        }
    }

    /// Everything in either, which is what joining two branches does.
    pub fn join(self, other: Range) -> Range {
        match (self.bounds(), other.bounds()) {
            (Some((a_low, a_high)), Some((b_low, b_high))) => {
                Range::between(a_low.min(b_low), a_high.max(b_high))
            }
            (Some(_), None) => self,
            (None, Some(_)) => other,
            (None, None) => Range::Empty,
        }
    }

    fn add(self, other: Range) -> Range {
        let (Some((a_low, a_high)), Some((b_low, b_high))) = (self.bounds(), other.bounds()) else {
            return Range::Empty;
        };
        match (a_low.checked_add(b_low), a_high.checked_add(b_high)) {
            (Some(low), Some(high)) => Range::between(low, high),
            // Overflow means the answer is not representable, so nothing is
            // known rather than something wrong.
            _ => Range::ANY,
        }
    }

    fn sub(self, other: Range) -> Range {
        match other.bounds() {
            // Subtracting is adding the negation, and negating the bounds
            // swaps them.
            Some((low, high)) => match (low.checked_neg(), high.checked_neg()) {
                (Some(neg_low), Some(neg_high)) => self.add(Range::between(neg_high, neg_low)),
                _ => Range::ANY,
            },
            None => Range::Empty,
        }
    }

    fn mul(self, other: Range) -> Range {
        let (Some((a_low, a_high)), Some((b_low, b_high))) = (self.bounds(), other.bounds()) else {
            return Range::Empty;
        };

        // The extremes of a product are at the corners, and any overflow makes
        // the whole thing unknown rather than wrong.
        let corners = [
            a_low.checked_mul(b_low),
            a_low.checked_mul(b_high),
            a_high.checked_mul(b_low),
            a_high.checked_mul(b_high),
        ];
        if corners.iter().any(Option::is_none) {
            return Range::ANY;
        }

        let values: Vec<i64> = corners.into_iter().flatten().collect();
        Range::between(
            *values.iter().min().expect("four corners"),
            *values.iter().max().expect("four corners"),
        )
    }

    fn negate(self) -> Range {
        match self.bounds() {
            Some((low, high)) => match (low.checked_neg(), high.checked_neg()) {
                (Some(neg_low), Some(neg_high)) => Range::between(neg_high, neg_low),
                _ => Range::ANY,
            },
            None => Range::Empty,
        }
    }

    /// The difference of two ranges, clamped rather than given up on.
    ///
    /// [`Range::sub`] answers what the program would compute, and a subtraction
    /// with no answer has to say so. This one answers which side of zero a
    /// difference falls on, which is all a comparison ever asks, and clamping
    /// keeps that answer while `i64::MIN` and `i64::MAX` stand in for "further
    /// than this, in that direction".
    fn spread(self, other: Range) -> Range {
        match (self.bounds(), other.bounds()) {
            (Some((a_low, a_high)), Some((b_low, b_high))) => {
                Range::between(a_low.saturating_sub(b_high), a_high.saturating_sub(b_low))
            }
            _ => Range::Empty,
        }
    }

    /// The sum of two ranges, clamped for the same reason [`Range::spread`] is.
    ///
    /// Two differences chained together, or a name moved by a difference. A
    /// bound that clamps is weaker than the true one and never wrong: every
    /// value being described is an `i64`, so "at least `i64::MIN`" is something
    /// already known about all of them.
    fn shift(self, other: Range) -> Range {
        match (self.bounds(), other.bounds()) {
            (Some((a_low, a_high)), Some((b_low, b_high))) => {
                Range::between(a_low.saturating_add(b_low), a_high.saturating_add(b_high))
            }
            _ => Range::Empty,
        }
    }

    /// Everything at or below `bound`, and everything above it.
    ///
    /// Not used by the checker, which narrows through [`Range::meet`], but it
    /// is the operation the split in an `if` is made of and the test below is
    /// what says the two halves cover the whole.
    #[cfg(test)]
    fn split_at(self, bound: i64) -> (Range, Range) {
        let below = self.meet(Range::between(i64::MIN, bound));
        let above = match bound.checked_add(1) {
            Some(next) => self.meet(Range::between(next, i64::MAX)),
            None => Range::Empty,
        };
        (below, above)
    }
}

/// What a comparison worked out to, when it worked out at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Truth {
    Always,
    Never,
    Unknown,
}

impl Truth {
    fn of(value: bool) -> Truth {
        if value { Truth::Always } else { Truth::Never }
    }

    fn negate(self) -> Truth {
        match self {
            Truth::Always => Truth::Never,
            Truth::Never => Truth::Always,
            Truth::Unknown => Truth::Unknown,
        }
    }

    fn and(self, other: Truth) -> Truth {
        match (self, other) {
            (Truth::Never, _) | (_, Truth::Never) => Truth::Never,
            (Truth::Always, Truth::Always) => Truth::Always,
            _ => Truth::Unknown,
        }
    }

    fn or(self, other: Truth) -> Truth {
        match (self, other) {
            (Truth::Always, _) | (_, Truth::Always) => Truth::Always,
            (Truth::Never, Truth::Never) => Truth::Never,
            _ => Truth::Unknown,
        }
    }
}

/// Ranges for the names in scope, and for the differences between them.
///
/// Keyed by definition, which resolution already made unique, so a shadowed
/// name cannot pick up a fact about the one it hid. There is no shadowing in
/// Vow anyway, and relying on that rather than on the key would be relying on
/// something that could change.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    known: HashMap<DefId, Range>,
    /// The range of `a - b`, for the pair `(a, b)`.
    ///
    /// Both orders are stored. Reading a difference the wrong way round means
    /// negating it, negating `i64::MIN` is not a thing that can be done, and
    /// the honest answer in that case is that nothing is known. Recording both
    /// orders keeps that answer where it belongs, on the order that could not
    /// be worked out, instead of on every lookup.
    differences: HashMap<(DefId, DefId), Range>,
    /// The range `value` stands for, while a refinement predicate is being
    /// evaluated. `value` has no definition of its own.
    subject: Option<Range>,
}

impl Facts {
    /// How many times [`Facts::settle`] goes round.
    ///
    /// Bounded rather than run to a real fixpoint. A couple of rounds cover
    /// anything anyone writes, and a body full of related names must not cost
    /// the checker something surprising, which is P9.
    const ROUNDS: usize = 4;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, def: DefId) -> Range {
        self.known.get(&def).copied().unwrap_or(Range::ANY)
    }

    /// Replaces what is known about `def`.
    ///
    /// Any difference involving `def` goes with it. This is what a binding
    /// does, and a fact about the old meaning of a name is not a fact about the
    /// new one.
    pub fn set(&mut self, def: DefId, range: Range) {
        self.differences
            .retain(|(left, right), _| *left != def && *right != def);
        self.known.insert(def, range);
        self.settle();
    }

    /// Narrows what is known about `def`, keeping anything already known.
    pub fn narrow(&mut self, def: DefId, range: Range) {
        if self.tighten(def, range) {
            self.settle();
        }
    }

    /// The range of `a - b`.
    pub fn difference(&self, a: DefId, b: DefId) -> Range {
        if a == b {
            return Range::exactly(0);
        }
        self.differences.get(&(a, b)).copied().unwrap_or(Range::ANY)
    }

    /// Narrows what is known about `a - b`, keeping anything already known.
    pub fn narrow_difference(&mut self, a: DefId, b: DefId, range: Range) {
        if self.tighten_difference(a, b, range) {
            self.settle();
        }
    }

    fn tighten(&mut self, def: DefId, range: Range) -> bool {
        let current = self.get(def);
        let narrowed = current.meet(range);
        if narrowed == current {
            return false;
        }
        self.known.insert(def, narrowed);
        true
    }

    fn tighten_difference(&mut self, a: DefId, b: DefId, range: Range) -> bool {
        if a == b {
            return false;
        }

        let current = self.difference(a, b);
        let narrowed = current.meet(range);
        // The other order is narrowed rather than replaced. Negating a bound
        // can lose it, and a fact already recorded the other way round must not
        // be thrown away by one that could not be worked out.
        let opposite = self.difference(b, a);
        let mirrored = opposite.meet(narrowed.negate());

        if narrowed == current && mirrored == opposite {
            return false;
        }
        self.differences.insert((a, b), narrowed);
        self.differences.insert((b, a), mirrored);
        true
    }

    /// Works out what the ranges and the differences say about each other.
    ///
    /// Three things, until nothing changes: a difference bounds the two names
    /// in it, the two names bound the difference, and two differences that
    /// share a name make a third. The last is what makes `a < b` and `b < c`
    /// enough to settle `a < c`.
    fn settle(&mut self) {
        for _ in 0..Self::ROUNDS {
            if !self.settle_once() {
                break;
            }
        }
    }

    fn settle_once(&mut self) -> bool {
        let entries: Vec<((DefId, DefId), Range)> =
            self.differences.iter().map(|(k, v)| (*k, *v)).collect();
        let mut changed = false;

        for ((a, b), difference) in &entries {
            // `a - b` is in `difference`, so `a` is in `b + difference` and `b`
            // is in `a - difference`. Both names are integers whatever the
            // difference turns out to be, so the arithmetic clamps.
            let from_b = self.get(*b).shift(*difference);
            changed |= self.tighten(*a, from_b);
            let from_a = self.get(*a).shift(Range::exactly(0).spread(*difference));
            changed |= self.tighten(*b, from_a);

            let from_both = self.get(*a).spread(self.get(*b));
            changed |= self.tighten_difference(*a, *b, from_both);
        }

        for ((a, b), first) in &entries {
            for ((c, d), second) in &entries {
                if b == c && a != d {
                    let chained = first.shift(*second);
                    changed |= self.tighten_difference(*a, *d, chained);
                }
            }
        }

        changed
    }

    pub fn with_subject(&self, range: Range) -> Facts {
        Facts {
            known: self.known.clone(),
            differences: self.differences.clone(),
            subject: Some(range),
        }
    }

    /// Everything both sides agree on, for joining two branches.
    pub fn join(&self, other: &Facts) -> Facts {
        let mut known = HashMap::new();
        for (def, range) in &self.known {
            let combined = range.join(other.get(*def));
            if !combined.is_any() {
                known.insert(*def, combined);
            }
        }

        let mut differences = HashMap::new();
        for ((a, b), range) in &self.differences {
            let combined = range.join(other.difference(*a, *b));
            if !combined.is_any() {
                differences.insert((*a, *b), combined);
            }
        }

        Facts {
            known,
            differences,
            subject: None,
        }
    }
}

/// What a call promises about the value it hands back.
///
/// A range on its own is what a callee can say without mentioning its
/// arguments, and most contracts worth writing mention them. `ensures ok =>
/// result == n` is the ordinary shape of a promise and it says nothing at all
/// as a pair of bounds, so what travels is the range plus the range of
/// `result - argument` for each argument the clause ties the result to.
///
/// That is the same relation the body reasoning uses, which is the point: a
/// difference is small enough to cross a module boundary as two numbers, and a
/// predicate is not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Guarantee {
    /// Where the result lands, whatever it was called with.
    pub range: Range,
    /// The range of `result - argument`, by the argument's position.
    pub offsets: BTreeMap<usize, Range>,
}

impl Guarantee {
    /// A promise about the result alone.
    pub fn of(range: Range) -> Guarantee {
        Guarantee {
            range,
            offsets: BTreeMap::new(),
        }
    }

    /// Promises nothing.
    pub fn any() -> Guarantee {
        Guarantee::of(Range::ANY)
    }

    /// Everything both promises say, for a function with more than one clause.
    pub fn meet(mut self, other: Guarantee) -> Guarantee {
        self.range = self.range.meet(other.range);
        for (index, range) in other.offsets {
            let combined = match self.offsets.get(&index) {
                Some(existing) => existing.meet(range),
                None => range,
            };
            self.offsets.insert(index, combined);
        }
        self
    }

    /// Where the result lands, given what the arguments were.
    fn applied(&self, args: &[Expr], facts: &Facts, env: &Env<'_>) -> Range {
        let mut range = self.range;
        for (index, offset) in &self.offsets {
            let Some(arg) = args.get(*index) else {
                continue;
            };
            // Clamped, because the result is an integer whatever the promise
            // adds up to, so `i64::MIN` and `i64::MAX` are bounds it already
            // satisfies rather than ones this could get wrong.
            range = range.meet(range_of(arg, facts, env).shift(*offset));
        }
        range
    }
}

/// What an `ensures` clause promises, read as a range and a set of differences.
///
/// `subject` is the name standing for the thing being described, which has no
/// definition of its own, and `names` are the parameters it is allowed to be
/// related to. Both get a definition invented here, so the narrowing a function
/// body gets can be run over a contract without one.
///
/// A parameter sharing the subject's name is left out of the relation rather
/// than confused with it. There is nothing useful to say about `result == result`.
pub fn promised_by(condition: &Expr, subject: &str, names: &[&str]) -> Guarantee {
    let result = DefId::from_raw(0);
    let params: Vec<(usize, DefId)> = names
        .iter()
        .enumerate()
        .filter(|(_, name)| **name != subject)
        .map(|(index, _)| (index, DefId::from_raw(index as u32 + 1)))
        .collect();

    let def_of = |expr: &Expr| match expr {
        Expr::Ident(ident) if ident.name == subject => Some(result),
        Expr::Ident(ident) => params
            .iter()
            .find(|(index, _)| names[*index] == ident.name)
            .map(|(_, def)| *def),
        _ => None,
    };
    let env = Env {
        def_of: &def_of,
        call: &|_| Guarantee::any(),
    };

    let mut facts = Facts::new();
    apply_narrowing(condition, &mut facts, &env, true);

    Guarantee {
        range: facts.get(result),
        offsets: params
            .into_iter()
            .map(|(index, def)| (index, facts.difference(result, def)))
            .filter(|(_, range)| !range.is_any())
            .collect(),
    }
}

/// What the fact machinery needs to know that it cannot read off the syntax.
///
/// Closures rather than a borrow of the checker, so this module stays
/// independent of how the checker stores things and can be tested without one.
pub struct Env<'a> {
    /// The definition an identifier refers to.
    pub def_of: &'a dyn Fn(&Expr) -> Option<DefId>,
    /// What a call to this callee promises, from its declared return type and
    /// its `ensures` clause. Whoever answers this is responsible for only
    /// answering for contracts that are themselves checked, since a promise
    /// nobody keeps is not a fact.
    pub call: &'a dyn Fn(&Expr) -> Guarantee,
}

impl Env<'_> {
    /// An environment that knows nothing, for reading a predicate on its own.
    pub fn blind() -> Env<'static> {
        Env {
            def_of: &|_| None,
            call: &|_| Guarantee::any(),
        }
    }
}

/// Whether an expression is a name a fact can be attached to.
fn def_of(expr: &Expr, env: &Env<'_>) -> Option<DefId> {
    match expr {
        Expr::Ident(_) => (env.def_of)(expr),
        _ => None,
    }
}

/// An expression rewritten as names added, names subtracted, and a range.
///
/// The only shape worth recovering is `a - b + k`, because that is what a
/// difference fact is about. Everything else collapses into the range, where
/// the interval reasoning answers it as it always did.
#[derive(Clone, Debug)]
struct Linear {
    positive: Vec<DefId>,
    negative: Vec<DefId>,
    offset: Range,
}

impl Linear {
    fn constant(offset: Range) -> Linear {
        Linear {
            positive: Vec::new(),
            negative: Vec::new(),
            offset,
        }
    }

    fn name(def: DefId) -> Linear {
        Linear {
            positive: vec![def],
            negative: Vec::new(),
            offset: Range::exactly(0),
        }
    }

    fn negate(self) -> Linear {
        Linear {
            positive: self.negative,
            negative: self.positive,
            offset: self.offset.negate(),
        }
    }

    fn add(self, other: Linear) -> Linear {
        let mut positive = self.positive;
        positive.extend(other.positive);
        let mut negative = self.negative;
        negative.extend(other.negative);
        let offset = self.offset.add(other.offset);

        // A name on both sides cancels, so `n - n` is zero rather than a
        // difference nobody recorded.
        let mut kept = Vec::with_capacity(positive.len());
        for def in positive {
            match negative.iter().position(|other| *other == def) {
                Some(at) => {
                    negative.remove(at);
                }
                None => kept.push(def),
            }
        }

        Linear {
            positive: kept,
            negative,
            offset,
        }
    }

    /// The pair this is the difference of, when it is the difference of a pair.
    fn pair(&self) -> Option<(DefId, DefId)> {
        match (self.positive.as_slice(), self.negative.as_slice()) {
            ([a], [b]) => Some((*a, *b)),
            _ => None,
        }
    }
}

/// Reads `expr` as names and an offset, as far as it goes.
fn linear(expr: &Expr, facts: &Facts, env: &Env<'_>) -> Linear {
    match expr {
        // `value` stands for whatever is being described and has no definition,
        // so it is a range and never a name.
        Expr::Ident(ident) if ident.name == "value" => {
            Linear::constant(facts.subject.unwrap_or(Range::ANY))
        }
        Expr::Ident(_) => match def_of(expr, env) {
            Some(def) => Linear::name(def),
            None => Linear::constant(interval_of(expr, facts, env)),
        },
        Expr::Unary {
            op: UnaryOp::Neg,
            operand,
            ..
        } => linear(operand, facts, env).negate(),
        Expr::Binary {
            op: BinaryOp::Add,
            lhs,
            rhs,
            ..
        } => linear(lhs, facts, env).add(linear(rhs, facts, env)),
        Expr::Binary {
            op: BinaryOp::Sub,
            lhs,
            rhs,
            ..
        } => linear(lhs, facts, env).add(linear(rhs, facts, env).negate()),
        // Not a sum of names. Whatever it is, the interval answers it, and
        // asking for the interval rather than the range avoids going round
        // again through the same expression.
        _ => Linear::constant(interval_of(expr, facts, env)),
    }
}

/// The range `left - right` falls in, using what is known about the pair.
///
/// Clamped throughout, because the only caller is a comparison, and a
/// comparison asks which side of zero this lands on rather than what the
/// program would compute.
fn difference_of(left: &Expr, right: &Expr, facts: &Facts, env: &Env<'_>) -> Range {
    let spread = range_of(left, facts, env).spread(range_of(right, facts, env));
    let form = linear(left, facts, env).add(linear(right, facts, env).negate());
    match form.pair() {
        Some((a, b)) => spread.meet(facts.difference(a, b).shift(form.offset)),
        None => spread,
    }
}

/// The range an expression can take, given what is known.
pub fn range_of(expr: &Expr, facts: &Facts, env: &Env<'_>) -> Range {
    let interval = interval_of(expr, facts, env);
    match related(expr, facts, env) {
        Some(range) => interval.meet(range),
        None => interval,
    }
}

/// What the difference facts add to an expression, when it is a difference.
///
/// Only worth the walk for a sum, since nothing else can reduce to one. The
/// arithmetic here is checked rather than clamped: this is the value the
/// program computes, and a subtraction that overflows does not have one.
fn related(expr: &Expr, facts: &Facts, env: &Env<'_>) -> Option<Range> {
    if !matches!(
        expr,
        Expr::Binary {
            op: BinaryOp::Add | BinaryOp::Sub,
            ..
        }
    ) {
        return None;
    }
    let form = linear(expr, facts, env);
    let (a, b) = form.pair()?;
    Some(facts.difference(a, b).add(form.offset))
}

/// The range an expression can take, from the ranges of the names in it alone.
fn interval_of(expr: &Expr, facts: &Facts, env: &Env<'_>) -> Range {
    match expr {
        Expr::Int { value, .. } => Range::exactly(*value),
        Expr::Ident(ident) if ident.name == "value" => facts.subject.unwrap_or(Range::ANY),
        Expr::Ident(_) => match def_of(expr, env) {
            Some(def) => facts.get(def),
            None => Range::ANY,
        },
        // The contract of whatever is being called. This is the one place the
        // reasoning leaves the function it is looking at, and it is why a
        // proof inside one function is worth anything to its callers.
        Expr::Call { callee, args, .. } => (env.call)(callee).applied(args, facts, env),
        Expr::Unary {
            op: UnaryOp::Neg,
            operand,
            ..
        } => range_of(operand, facts, env).negate(),
        Expr::Binary { op, lhs, rhs, .. } => {
            let left = range_of(lhs, facts, env);
            let right = range_of(rhs, facts, env);
            match op {
                BinaryOp::Add => left.add(right),
                BinaryOp::Sub => left.sub(right),
                BinaryOp::Mul => left.mul(right),
                // Division and remainder are left alone on purpose. See the
                // note at the top of the file.
                _ => Range::ANY,
            }
        }
        _ => Range::ANY,
    }
}

/// Whether a condition holds, given what is known.
pub fn holds(condition: &Expr, facts: &Facts, env: &Env<'_>) -> Truth {
    match condition {
        Expr::Bool { value, .. } => Truth::of(*value),
        Expr::Unary {
            op: UnaryOp::Not,
            operand,
            ..
        } => holds(operand, facts, env).negate(),
        Expr::Binary { op, lhs, rhs, .. } => match op {
            BinaryOp::And => holds(lhs, facts, env).and(holds(rhs, facts, env)),
            BinaryOp::Or => holds(lhs, facts, env).or(holds(rhs, facts, env)),
            _ => compare(*op, lhs, rhs, facts, env),
        },
        _ => Truth::Unknown,
    }
}

/// Whether `left op right` holds, which is a question about `left - right`.
///
/// Writing it once against the difference rather than once per operator against
/// two intervals is what lets a recorded relationship count. Where there is no
/// relationship the difference is the one the two intervals imply, which is the
/// same answer as before.
fn compare(op: BinaryOp, lhs: &Expr, rhs: &Expr, facts: &Facts, env: &Env<'_>) -> Truth {
    if range_of(lhs, facts, env) == Range::Empty || range_of(rhs, facts, env) == Range::Empty {
        // One side cannot happen, so the comparison never does either.
        return Truth::Never;
    }

    let Some((low, high)) = difference_of(lhs, rhs, facts, env).bounds() else {
        return Truth::Never;
    };

    match op {
        BinaryOp::Lt => {
            if high < 0 {
                Truth::Always
            } else if low >= 0 {
                Truth::Never
            } else {
                Truth::Unknown
            }
        }
        BinaryOp::Le => {
            if high <= 0 {
                Truth::Always
            } else if low > 0 {
                Truth::Never
            } else {
                Truth::Unknown
            }
        }
        BinaryOp::Gt => {
            if low > 0 {
                Truth::Always
            } else if high <= 0 {
                Truth::Never
            } else {
                Truth::Unknown
            }
        }
        BinaryOp::Ge => {
            if low >= 0 {
                Truth::Always
            } else if high < 0 {
                Truth::Never
            } else {
                Truth::Unknown
            }
        }
        BinaryOp::Eq => {
            if low == 0 && high == 0 {
                Truth::Always
            } else if low > 0 || high < 0 {
                Truth::Never
            } else {
                Truth::Unknown
            }
        }
        BinaryOp::Ne => compare(BinaryOp::Eq, lhs, rhs, facts, env).negate(),
        _ => Truth::Unknown,
    }
}

/// What `condition` tells you about the names in it, when it holds.
///
/// Only comparisons of a name against something with a known range, and
/// conjunctions of those. An `||` tells you nothing about either side on its
/// own, which is why it is absent rather than approximated.
pub fn narrowed(condition: &Expr, facts: &Facts, env: &Env<'_>, when_true: bool) -> Facts {
    let mut result = facts.clone();
    apply_narrowing(condition, &mut result, env, when_true);
    result
}

fn apply_narrowing(condition: &Expr, facts: &mut Facts, env: &Env<'_>, when_true: bool) {
    match condition {
        Expr::Unary {
            op: UnaryOp::Not,
            operand,
            ..
        } => apply_narrowing(operand, facts, env, !when_true),

        // `a && b` holding means both hold. `a && b` failing means at least one
        // failed, which says nothing about either.
        Expr::Binary {
            op: BinaryOp::And,
            lhs,
            rhs,
            ..
        } if when_true => {
            apply_narrowing(lhs, facts, env, true);
            apply_narrowing(rhs, facts, env, true);
        }

        // The mirror image: `a || b` failing means both failed.
        Expr::Binary {
            op: BinaryOp::Or,
            lhs,
            rhs,
            ..
        } if !when_true => {
            apply_narrowing(lhs, facts, env, false);
            apply_narrowing(rhs, facts, env, false);
        }

        Expr::Binary { op, lhs, rhs, .. } => {
            let effective = if when_true { Some(*op) } else { negated(*op) };
            let Some(effective) = effective else { return };

            // Both directions, so `0 < n` narrows `n` the same way `n > 0`
            // does.
            narrow_side(effective, lhs, rhs, facts, env);
            if let Some(flipped) = flipped(effective) {
                narrow_side(flipped, rhs, lhs, facts, env);
            }

            narrow_relation(effective, lhs, rhs, facts, env);
        }

        _ => {}
    }
}

/// Records what `left op right` says about the difference of two names.
///
/// This is the half an interval cannot hold. `low < high` says nothing about
/// either name on its own, and everything about `low - high`.
fn narrow_relation(op: BinaryOp, left: &Expr, right: &Expr, facts: &mut Facts, env: &Env<'_>) {
    let form = linear(left, facts, env).add(linear(right, facts, env).negate());
    let Some((a, b)) = form.pair() else {
        return;
    };
    let Some((low, high)) = form.offset.bounds() else {
        return;
    };

    // The constraint is `(a - b) + offset op 0`, and the offset is only known
    // to be somewhere in its own range, so a bound has to hold for the whole of
    // it: the upper one comes from the smallest offset and the lower one from
    // the largest. An offset at the edge of what an `i64` holds is not worth a
    // special case, so nothing is learned there rather than something wrong.
    let (Some(least), Some(most)) = (high.checked_neg(), low.checked_neg()) else {
        return;
    };

    let bound = match op {
        BinaryOp::Lt => match most.checked_sub(1) {
            Some(bound) => Range::between(i64::MIN, bound),
            None => Range::Empty,
        },
        BinaryOp::Le => Range::between(i64::MIN, most),
        BinaryOp::Gt => match least.checked_add(1) {
            Some(bound) => Range::between(bound, i64::MAX),
            None => Range::Empty,
        },
        BinaryOp::Ge => Range::between(least, i64::MAX),
        BinaryOp::Eq => Range::between(least, most),
        _ => return,
    };

    facts.narrow_difference(a, b, bound);
}

/// Narrows the left side of `left op right`.
fn narrow_side(op: BinaryOp, left: &Expr, right: &Expr, facts: &mut Facts, env: &Env<'_>) {
    let Some(def) = def_of(left, env) else {
        return;
    };
    let Some((low, high)) = range_of(right, facts, env).bounds() else {
        return;
    };

    let narrowed = match op {
        // `left < right` means left is below the largest right could be.
        BinaryOp::Lt => match high.checked_sub(1) {
            Some(bound) => Range::between(i64::MIN, bound),
            None => Range::Empty,
        },
        BinaryOp::Le => Range::between(i64::MIN, high),
        BinaryOp::Gt => match low.checked_add(1) {
            Some(bound) => Range::between(bound, i64::MAX),
            None => Range::Empty,
        },
        BinaryOp::Ge => Range::between(low, i64::MAX),
        BinaryOp::Eq => Range::between(low, high),
        // `!=` only says something when the other side is a single value at
        // the edge of what is known, and that is rare enough to skip.
        _ => return,
    };

    facts.narrow(def, narrowed);
}

fn negated(op: BinaryOp) -> Option<BinaryOp> {
    match op {
        BinaryOp::Lt => Some(BinaryOp::Ge),
        BinaryOp::Le => Some(BinaryOp::Gt),
        BinaryOp::Gt => Some(BinaryOp::Le),
        BinaryOp::Ge => Some(BinaryOp::Lt),
        BinaryOp::Eq => Some(BinaryOp::Ne),
        BinaryOp::Ne => Some(BinaryOp::Eq),
        _ => None,
    }
}

/// The same comparison with its sides swapped.
fn flipped(op: BinaryOp) -> Option<BinaryOp> {
    match op {
        BinaryOp::Lt => Some(BinaryOp::Gt),
        BinaryOp::Le => Some(BinaryOp::Ge),
        BinaryOp::Gt => Some(BinaryOp::Lt),
        BinaryOp::Ge => Some(BinaryOp::Le),
        BinaryOp::Eq => Some(BinaryOp::Eq),
        BinaryOp::Ne => Some(BinaryOp::Ne),
        _ => None,
    }
}

/// The range a refinement predicate admits, when it is a simple comparison of
/// `value` against a constant.
///
/// This is what turns a parameter already of a refined type into a fact,
/// without anyone writing a `where` clause saying the same thing again.
pub fn range_admitted_by(predicate: &Expr) -> Range {
    range_of_subject(predicate, "value")
}

/// The range a condition pins its subject to, reading nothing else.
///
/// `value` in a refinement and `result` in an `ensures` clause are the same
/// idea twice: a name that stands for the thing being described and has no
/// definition of its own. Everything else in the condition is unknown, so a
/// clause that mentions an argument tells us nothing, which is the honest
/// answer rather than a wrong one.
pub fn range_of_subject(condition: &Expr, subject: &str) -> Range {
    let mut narrowed = Facts::new();
    let env = Env::blind();
    apply_subject_narrowing(condition, &mut narrowed, &env, subject, true);
    narrowed.subject.unwrap_or(Range::ANY)
}

/// The same walk as [`apply_narrowing`], for a subject with no definition.
fn apply_subject_narrowing(
    predicate: &Expr,
    facts: &mut Facts,
    env: &Env<'_>,
    subject: &str,
    when_true: bool,
) {
    match predicate {
        Expr::Unary {
            op: UnaryOp::Not,
            operand,
            ..
        } => apply_subject_narrowing(operand, facts, env, subject, !when_true),
        Expr::Binary {
            op: BinaryOp::And,
            lhs,
            rhs,
            ..
        } if when_true => {
            apply_subject_narrowing(lhs, facts, env, subject, true);
            apply_subject_narrowing(rhs, facts, env, subject, true);
        }
        Expr::Binary { op, lhs, rhs, .. } => {
            let Some(effective) = (if when_true { Some(*op) } else { negated(*op) }) else {
                return;
            };
            narrow_subject(effective, lhs, rhs, facts, env, subject);
            if let Some(flipped) = flipped(effective) {
                narrow_subject(flipped, rhs, lhs, facts, env, subject);
            }
        }
        _ => {}
    }
}

fn narrow_subject(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    facts: &mut Facts,
    env: &Env<'_>,
    subject: &str,
) {
    if !matches!(left, Expr::Ident(ident) if ident.name == subject) {
        return;
    }
    let Some((low, high)) = range_of(right, facts, env).bounds() else {
        return;
    };

    let narrowed = match op {
        BinaryOp::Lt => match high.checked_sub(1) {
            Some(bound) => Range::between(i64::MIN, bound),
            None => Range::Empty,
        },
        BinaryOp::Le => Range::between(i64::MIN, high),
        BinaryOp::Gt => match low.checked_add(1) {
            Some(bound) => Range::between(bound, i64::MAX),
            None => Range::Empty,
        },
        BinaryOp::Ge => Range::between(low, i64::MAX),
        BinaryOp::Eq => Range::between(low, high),
        _ => return,
    };

    let current = facts.subject.unwrap_or(Range::ANY);
    facts.subject = Some(current.meet(narrowed));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrowing_keeps_the_tighter_bound() {
        let wide = Range::between(0, 100);
        assert_eq!(wide.meet(Range::between(50, 200)), Range::between(50, 100));
        assert_eq!(wide.meet(Range::between(200, 300)), Range::Empty);
    }

    #[test]
    fn joining_two_branches_covers_both() {
        assert_eq!(
            Range::exactly(1).join(Range::exactly(9)),
            Range::between(1, 9)
        );
        assert_eq!(Range::Empty.join(Range::exactly(3)), Range::exactly(3));
    }

    #[test]
    fn arithmetic_that_overflows_gives_up_rather_than_wrapping() {
        let huge = Range::exactly(i64::MAX);
        assert_eq!(huge.add(Range::exactly(1)), Range::ANY);
        assert_eq!(huge.mul(Range::exactly(2)), Range::ANY);
    }

    #[test]
    fn adding_two_ranges_adds_the_bounds() {
        assert_eq!(
            Range::between(1, 5).add(Range::between(10, 20)),
            Range::between(11, 25)
        );
        assert_eq!(
            Range::between(1, 5).sub(Range::between(10, 20)),
            Range::between(-19, -5)
        );
    }

    #[test]
    fn multiplying_takes_the_extreme_corners() {
        // The trap is signs: the smallest product of two ranges that straddle
        // zero is not the product of the two smallest bounds.
        assert_eq!(
            Range::between(-3, 2).mul(Range::between(-4, 5)),
            Range::between(-15, 12)
        );
    }

    #[test]
    fn splitting_at_a_bound_covers_everything() {
        let (below, above) = Range::between(0, 10).split_at(4);
        assert_eq!(below, Range::between(0, 4));
        assert_eq!(above, Range::between(5, 10));
        assert_eq!(below.join(above), Range::between(0, 10));
    }

    #[test]
    fn a_clamped_difference_still_says_which_side_of_zero_it_is_on() {
        // The subtraction has no answer, because the answer is larger than an
        // integer. Which side of zero it falls on is not in doubt, and that is
        // the only thing a comparison asks.
        let below = Range::between(i64::MIN, -1);
        let above = Range::between(0, i64::MAX);
        assert_eq!(below.sub(above), Range::ANY);

        let (_, highest) = below.spread(above).bounds().expect("not empty");
        assert!(highest < 0, "the difference is negative whatever its size");
    }

    #[test]
    fn two_differences_that_share_a_name_make_a_third() {
        let (a, b, c) = (DefId::from_raw(0), DefId::from_raw(1), DefId::from_raw(2));
        let mut facts = Facts::new();
        facts.narrow_difference(a, b, Range::between(i64::MIN, -1));
        facts.narrow_difference(b, c, Range::between(i64::MIN, -1));

        // `a < b` and `b < c`, so `a < c`, however far apart any of them are.
        let (_, highest) = facts.difference(a, c).bounds().expect("not empty");
        assert!(highest < 0);
    }

    #[test]
    fn a_difference_carries_a_bound_from_one_name_to_the_other() {
        let (n, limit) = (DefId::from_raw(0), DefId::from_raw(1));
        let mut facts = Facts::new();
        facts.narrow_difference(n, limit, Range::between(i64::MIN, -1));

        // Nothing bounded `limit` when the difference was recorded, so this is
        // the fact arriving after the reasoning that needed it.
        facts.narrow(limit, Range::between(0, 100));
        let (_, highest) = facts.get(n).bounds().expect("not empty");
        assert_eq!(highest, 99);
    }

    #[test]
    fn a_binding_does_not_inherit_the_differences_of_the_name_it_replaces() {
        let (a, b) = (DefId::from_raw(0), DefId::from_raw(1));
        let mut facts = Facts::new();
        facts.narrow_difference(a, b, Range::between(i64::MIN, -1));
        facts.set(a, Range::ANY);
        assert_eq!(facts.difference(a, b), Range::ANY);
    }

    #[test]
    fn joining_two_branches_keeps_only_the_differences_both_agree_on() {
        let (a, b) = (DefId::from_raw(0), DefId::from_raw(1));
        let mut left = Facts::new();
        left.narrow_difference(a, b, Range::between(-10, -1));
        let mut right = Facts::new();
        right.narrow_difference(a, b, Range::between(-4, 8));

        assert_eq!(left.join(&right).difference(a, b), Range::between(-10, 8));
        assert_eq!(left.join(&Facts::new()).difference(a, b), Range::ANY);
    }
}
