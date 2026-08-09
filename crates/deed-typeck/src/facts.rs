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
//! A name multiplied by a number is still a name, counted more than once, so
//! `n + n` and `n * 2` are read as two of one name rather than as a shape with
//! nowhere to go. A name multiplied by a name is not, and that is where this
//! stops and a solver would start.
//!
//! # What this cannot do
//!
//! A great deal, and the answer is always "not proven", never a wrong answer.
//!
//! - **Two names multiplied together.** `a < b * c` is not linear, and a pair
//!   of bounds has nowhere to put it. Deciding it is a solver's job.
//! - **Anything that is not an integer.** No `String`, no record field, no
//!   variant.
//! - **The payload of a call that can fail, until it is taken out.** A call
//!   that can fail promises things about the number inside the `ok`, and the
//!   expression is the `Result` around it. The two only meet where the payload
//!   comes out: at a `?`, at an `ok(..)` pattern, and where a `Result` is
//!   assigned into one with a refined success type.
//! - **Division and remainder.** The sign rules around zero and around
//!   `i64::MIN` are fiddly enough that getting them wrong is worse than not
//!   trying.

use std::collections::{BTreeMap, HashMap};

use deed_ast::{BinaryOp, Expr, UnaryOp};
use deed_diagnostics::Span;
use deed_resolve::DefId;

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

    /// Arithmetic saturates rather than giving up, and that is sound because
    /// overflow is an error here rather than a wrap.
    ///
    /// `checked_add` in the interpreter means `n + 1` either produces a value
    /// or stops the program with `DEED6004`. It never produces a wrong one. So
    /// every value this expression can produce is inside `i64`, and a bound
    /// that leaves `i64` should be clamped to the edge rather than thrown
    /// away.
    ///
    /// Throwing it away was the old answer and it was not merely conservative,
    /// it was wrong in the direction that matters: `Positive` is `[1, MAX]`, so
    /// `n + 1` collapsed to "anything at all" and a provable obligation became
    /// a runtime check that could never fire.
    ///
    /// This is the same argument [`Range::Empty`] already rests on. A claim
    /// about a value that does not exist is vacuously true, which is why an
    /// impossible branch needs no special case.
    fn add(self, other: Range) -> Range {
        let (Some((a_low, a_high)), Some((b_low, b_high))) = (self.bounds(), other.bounds()) else {
            return Range::Empty;
        };
        Range::between(a_low.saturating_add(b_low), a_high.saturating_add(b_high))
    }

    fn sub(self, other: Range) -> Range {
        match other.bounds() {
            // Subtracting is adding the negation, and negating the bounds
            // swaps them.
            Some((low, high)) => {
                self.add(Range::between(high.saturating_neg(), low.saturating_neg()))
            }
            None => Range::Empty,
        }
    }

    fn mul(self, other: Range) -> Range {
        let (Some((a_low, a_high)), Some((b_low, b_high))) = (self.bounds(), other.bounds()) else {
            return Range::Empty;
        };

        // The extremes of a product are at the corners, and a corner outside
        // `i64` is clamped to the edge for the same reason a sum is: nothing
        // outside `i64` is ever a value.
        let corners = [
            a_low.saturating_mul(b_low),
            a_low.saturating_mul(b_high),
            a_high.saturating_mul(b_low),
            a_high.saturating_mul(b_high),
        ];
        Range::between(
            *corners.iter().min().expect("four corners"),
            *corners.iter().max().expect("four corners"),
        )
    }

    fn negate(self) -> Range {
        match self.bounds() {
            Some((low, high)) => Range::between(high.saturating_neg(), low.saturating_neg()),
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

    /// Every value multiplied, clamped for the same reason [`Range::shift`] is.
    ///
    /// A negative factor swaps the ends, which is the trap here.
    fn times(self, factor: i64) -> Range {
        let Some((low, high)) = self.bounds() else {
            return Range::Empty;
        };
        let (one, other) = (low.saturating_mul(factor), high.saturating_mul(factor));
        Range::between(one.min(other), one.max(other))
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
///
/// `Unknown` carries [`Reason`] rather than nothing, because it is the answer
/// a reader sees the most: it is what produces `Guarded`. A reason with no
/// content is a checker that says "I don't know" and nothing else, which is
/// not something a reader can act on. There is no `Unknown` value without one:
/// every place in this file and in `check.rs` that used to write the bare
/// variant now has to say why, and the type does not compile until it does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Truth {
    Always,
    Never,
    Unknown(Reason),
}

/// Why a `Truth` came back `Unknown`, said about the obligation rather than
/// about the checker.
///
/// "The solver gave up" tells a reader nothing they can act on; these are
/// meant to. Each one is the answer to "what would make this Proven instead":
/// narrow the name, establish the length, give the value a name, keep the
/// clause on this side of a module boundary, or write the condition in a
/// shape this checker reasons about at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reason {
    /// A name is being compared, but nothing in scope narrowed its range
    /// enough to settle the comparison.
    NothingNarrowedThisName,
    /// A `length(name)` is being compared, but nothing in scope established a
    /// bound on it.
    NothingEstablishedThisLength,
    /// Neither side of the comparison is a name or a `length(name)`, so there
    /// is nothing in `Facts` this could be keyed on.
    NothingNamesThisValue,
    /// The clause was read across a module boundary, where a nested call
    /// answers `Promise::any()` instead of whatever its own `ensures` says.
    CrossedAModuleBoundary,
    /// The condition is not `and`, `or`, `not`, or a comparison the interval
    /// machinery understands, so there is nothing here to evaluate at all.
    NotAShapeTheCheckerReasonsAbout,
    /// Nothing attempted this one.
    ///
    /// The others are the checker having looked and come back without an
    /// answer. This is the checker not having looked, which is a different
    /// thing to tell a reader and was previously told by saying nothing at
    /// all. An `ensures` clause is the case: it is checked on every call, so
    /// its floor is `Guarded` whatever the body looks like, and no pass tries
    /// to settle one ahead of time.
    NothingTriesToProveThis,
}

impl Reason {
    /// A sentence a diagnostic, a hover or an inlay hint can show as-is.
    pub fn text(self) -> &'static str {
        match self {
            Reason::NothingNarrowedThisName => "nothing narrowed this name",
            Reason::NothingEstablishedThisLength => "nothing established this length",
            Reason::NothingNamesThisValue => "nothing names this value",
            Reason::CrossedAModuleBoundary => {
                "this clause crossed a module boundary and arrived thinner"
            }
            Reason::NotAShapeTheCheckerReasonsAbout => {
                "this predicate is not the shape the checker reasons about"
            }
            Reason::NothingTriesToProveThis => {
                "nothing tries to prove this one ahead of time, so it is checked on every call"
            }
        }
    }
}

impl Truth {
    fn of(value: bool) -> Truth {
        if value { Truth::Always } else { Truth::Never }
    }

    fn negate(self) -> Truth {
        match self {
            Truth::Always => Truth::Never,
            Truth::Never => Truth::Always,
            Truth::Unknown(reason) => Truth::Unknown(reason),
        }
    }

    fn and(self, other: Truth) -> Truth {
        match (self, other) {
            (Truth::Never, _) | (_, Truth::Never) => Truth::Never,
            (Truth::Always, Truth::Always) => Truth::Always,
            (Truth::Unknown(reason), _) | (_, Truth::Unknown(reason)) => Truth::Unknown(reason),
        }
    }

    fn or(self, other: Truth) -> Truth {
        match (self, other) {
            (Truth::Always, _) | (_, Truth::Always) => Truth::Always,
            (Truth::Never, Truth::Never) => Truth::Never,
            (Truth::Unknown(reason), _) | (_, Truth::Unknown(reason)) => Truth::Unknown(reason),
        }
    }
}

/// A thing a fact can be about.
///
/// A name is the obvious one and was the only one for a while. A length is the
/// other, and it is here because `index < length(items)` is a relation between
/// two quantities and the machinery that holds `low < high` could not see it.
/// A call came back as a range, and a range cannot be one side of a difference.
///
/// `Length` carries the definition of the thing being measured rather than a
/// definition of its own, so `length(items)` written twice is one term. Nothing
/// else in this module knows the difference between the two: [`Linear`], the
/// differences and [`Facts::settle`] all work on keys.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Term {
    /// An integer-valued name.
    Name(DefId),
    /// How many things are in the list or string that name refers to.
    Length(DefId),
}

/// Ranges for the names in scope, and for the differences between them.
///
/// Keyed by definition, which resolution already made unique, so a shadowed
/// name cannot pick up a fact about the one it hid. There is no shadowing in
/// Deed anyway, and relying on that rather than on the key would be relying on
/// something that could change.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    known: HashMap<Term, Range>,
    /// The range of `a - b`, for the pair `(a, b)`.
    ///
    /// Both orders are stored. Reading a difference the wrong way round means
    /// negating it, negating `i64::MIN` is not a thing that can be done, and
    /// the honest answer in that case is that nothing is known. Recording both
    /// orders keeps that answer where it belongs, on the order that could not
    /// be worked out, instead of on every lookup.
    differences: HashMap<(Term, Term), Range>,
    /// The range of `a + b`, for the pair `(a, b)`.
    ///
    /// The other half a pair of names can be constrained by, and the one a
    /// `where` clause reaches for most: `where count + delivered > 0` is a
    /// fact about neither name and about their total. Without somewhere to put
    /// it, the clause was read, found to be about no term this could be keyed
    /// on, and dropped, so a function whose precondition was word for word its
    /// own obligation came back Guarded.
    ///
    /// Both orders are stored, and unlike a difference they hold the same
    /// range, because addition does not care which way round it is read.
    totals: HashMap<(Term, Term), Range>,
    /// What `value` stands for, while a refinement predicate is being read.
    subject: Option<Subject>,
}

/// What is known about the thing a refinement predicate is about.
///
/// `value` has no definition of its own, so for a long time it was a range and
/// nothing else. That answers `value > 0` and cannot answer
/// `length(value) > 0`, which is a question about a term, and a term needs
/// something to be keyed by. So a subject carries what is known three ways and
/// each shape in the predicate reads the one that fits.
#[derive(Clone, Copy, Debug)]
pub struct Subject {
    /// The range the value itself lands in.
    pub range: Range,
    /// How long it is, for a predicate that asks. A string written on the spot
    /// has a length nobody has to work out, and refusing to count it would be
    /// refusing a fact for want of somewhere to put it.
    pub length: Range,
    /// The name the value was given, when it has one.
    ///
    /// Stronger than either range, because a name is a term the body has been
    /// narrowing all along: `if length(s) > 0` and `length(value) > 0` end up
    /// asking about the same entry, which is why a refinement and a `where`
    /// clause saying the same thing now agree.
    pub name: Option<DefId>,
}

impl Subject {
    /// A subject known only by the range it lands in.
    pub fn of(range: Range) -> Subject {
        Subject {
            range,
            length: Range::between(0, i64::MAX),
            name: None,
        }
    }

    #[must_use]
    pub fn with_length(mut self, length: Range) -> Subject {
        self.length = length;
        self
    }

    #[must_use]
    pub fn with_name(mut self, name: Option<DefId>) -> Subject {
        self.name = name;
        self
    }
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

    /// What is known about a term.
    ///
    /// The default is where "a length is not negative" lives. It is not a fact
    /// anybody states and not one a call has to hand back: there is no list
    /// with fewer than no things in it, so the domain says so and every
    /// length starts from there.
    pub fn get(&self, term: Term) -> Range {
        self.known.get(&term).copied().unwrap_or(match term {
            Term::Name(_) => Range::ANY,
            Term::Length(_) => Range::between(0, i64::MAX),
        })
    }

    /// Replaces what is known about `def`.
    ///
    /// Any difference involving `def` goes with it, and so does anything known
    /// about its length. This is what a binding does, and a fact about the old
    /// meaning of a name is not a fact about the new one.
    pub fn set(&mut self, def: DefId, range: Range) {
        self.forget(def);
        self.known.insert(Term::Name(def), range);
        self.settle();
    }

    /// Records what is known about one term, leaving the rest alone.
    ///
    /// Unlike [`Facts::set`], which is what a binding does and forgets
    /// everything about the old meaning of the name. This is for saying a
    /// second thing about something already in scope, such as how long a
    /// parameter whose type is refined over its length has to be.
    pub fn note(&mut self, term: Term, range: Range) {
        let current = self.get(term);
        self.known.insert(term, current.meet(range));
        self.settle();
    }

    /// Drops everything known about a definition, under either reading.
    pub fn forget(&mut self, def: DefId) {
        let gone = |term: &Term| match term {
            Term::Name(other) | Term::Length(other) => *other == def,
        };
        self.known.retain(|term, _| !gone(term));
        self.differences
            .retain(|(left, right), _| !gone(left) && !gone(right));
        self.totals
            .retain(|(left, right), _| !gone(left) && !gone(right));
    }

    /// The range of `a + b`.
    pub fn total(&self, a: Term, b: Term) -> Range {
        self.totals.get(&(a, b)).copied().unwrap_or(Range::ANY)
    }

    /// Narrows what is known about `a + b`, keeping anything already known.
    ///
    /// No settling afterwards, unlike a difference. What a total says about
    /// either name on its own needs a bound on the other, and where there is
    /// one the interval reasoning already has it, so there is nothing here to
    /// propagate and no reason to walk the whole table to find that out.
    pub fn narrow_total(&mut self, a: Term, b: Term, range: Range) {
        let narrowed = self.total(a, b).meet(range);
        self.totals.insert((a, b), narrowed);
        self.totals.insert((b, a), narrowed);
    }

    /// The range of `a - b`.
    pub fn difference(&self, a: Term, b: Term) -> Range {
        if a == b {
            return Range::exactly(0);
        }
        self.differences.get(&(a, b)).copied().unwrap_or(Range::ANY)
    }

    /// Narrows what is known about `a - b`, keeping anything already known.
    pub fn narrow_difference(&mut self, a: Term, b: Term, range: Range) {
        if self.tighten_difference(a, b, range) {
            self.settle();
        }
    }

    /// Narrows what is known about a term, keeping anything already known.
    pub fn narrow(&mut self, term: Term, range: Range) {
        if self.tighten(term, range) {
            self.settle();
        }
    }

    fn tighten(&mut self, term: Term, range: Range) -> bool {
        let current = self.get(term);
        let narrowed = current.meet(range);
        if narrowed == current {
            return false;
        }
        self.known.insert(term, narrowed);
        true
    }

    fn tighten_difference(&mut self, a: Term, b: Term, range: Range) -> bool {
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
        let entries: Vec<((Term, Term), Range)> =
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

    pub fn with_subject(&self, subject: Subject) -> Facts {
        Facts {
            known: self.known.clone(),
            differences: self.differences.clone(),
            totals: self.totals.clone(),
            subject: Some(subject),
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

        let mut totals = HashMap::new();
        for ((a, b), range) in &self.totals {
            let combined = range.join(other.total(*a, *b));
            if !combined.is_any() {
                totals.insert((*a, *b), combined);
            }
        }

        Facts {
            known,
            differences,
            totals,
            subject: None,
        }
    }
}

/// What a promise says about the result next to one of the arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scaled {
    /// How many of the argument the result is measured against.
    ///
    /// One, nearly always. `ensures ok => result == n + n` is where the rest
    /// comes from: a name counted twice is still a name, and refusing to count
    /// it would be refusing the clause over a detail of how it was written.
    pub factor: i64,
    /// The range of `result - factor * argument`.
    pub range: Range,
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
    /// What the result is worth next to each argument, by its position.
    pub offsets: BTreeMap<usize, Scaled>,
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
        for (index, scaled) in other.offsets {
            record_scaled(&mut self.offsets, index, scaled);
        }
        self
    }

    /// Where the result lands, given what the arguments were.
    fn applied(&self, args: &[Expr], facts: &Facts, env: &Env<'_>) -> Range {
        let mut range = self.range;
        for (index, scaled) in &self.offsets {
            let Some(arg) = args.get(*index) else {
                continue;
            };
            // Clamped at every step, because the result is an integer whatever
            // the promise adds up to, so `i64::MIN` and `i64::MAX` are bounds
            // it already satisfies rather than ones this could get wrong.
            let counted = range_of(arg, facts, env).times(scaled.factor);
            range = range.meet(counted.shift(scaled.range));
        }
        range
    }
}

/// Keeps the tighter of two things a promise says about one argument.
///
/// Two clauses counting the same argument a different number of times cannot
/// be merged into one entry, and the one counting it once composes with
/// everything else, so that is the one kept.
fn record_scaled(offsets: &mut BTreeMap<usize, Scaled>, index: usize, scaled: Scaled) {
    match offsets.get(&index) {
        None => {
            offsets.insert(index, scaled);
        }
        Some(existing) if existing.factor == scaled.factor => {
            let range = existing.range.meet(scaled.range);
            offsets.insert(index, Scaled { range, ..scaled });
        }
        Some(existing) if existing.factor != 1 && scaled.factor == 1 => {
            offsets.insert(index, scaled);
        }
        Some(_) => {}
    }
}

/// What a call promises, and which value it is about.
///
/// A function that can fail promises things about the number inside the `ok`,
/// and the call site is holding the `Result` around it. Those are two different
/// values and treating them as one was a quiet mistake: a `Result` was picking
/// up the range of a number it contained.
///
/// Which of these is the useful one is decided by what the caller does next.
/// Nothing, for `f()`, and everything, for `f()?`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Promise {
    /// What the expression is worth, when the call cannot fail.
    pub value: Guarantee,
    /// What is inside the `ok`, when it can.
    pub ok: Guarantee,
}

impl Promise {
    /// Promises nothing either way.
    pub fn any() -> Promise {
        Promise {
            value: Guarantee::any(),
            ok: Guarantee::any(),
        }
    }

    /// A promise about the value a call hands back.
    pub fn value(guarantee: Guarantee) -> Promise {
        Promise {
            value: guarantee,
            ok: Guarantee::any(),
        }
    }

    /// A promise about the number inside the `ok` of a call that can fail.
    pub fn ok(guarantee: Guarantee) -> Promise {
        Promise {
            value: Guarantee::any(),
            ok: guarantee,
        }
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
        length: None,
        call: &|_| Promise::any(),
    };

    let mut facts = Facts::new();
    apply_narrowing(condition, &mut facts, &env, true);

    let mut offsets = BTreeMap::new();
    collect_scaled(condition, &facts, &env, result, &params, &mut offsets, true);

    Guarantee {
        range: facts.get(Term::Name(result)),
        offsets,
    }
}

/// What a clause says about `result` next to each of the arguments.
///
/// The same walk [`apply_narrowing`] does, reading `result - factor * argument`
/// out of every comparison rather than narrowing anything. It is separate
/// because a difference between two names is not the only useful shape here:
/// `result == n + n` counts `n` twice and there is nowhere in the facts to put
/// that, while there is somewhere in a promise.
///
/// The parameter count is what a recursive walk over a tree costs when it
/// carries both what it is looking for and where it is putting the answer.
/// Bundling them into a struct would name the same six things one level down.
#[allow(clippy::too_many_arguments)]
fn collect_scaled(
    condition: &Expr,
    facts: &Facts,
    env: &Env<'_>,
    subject: DefId,
    params: &[(usize, DefId)],
    offsets: &mut BTreeMap<usize, Scaled>,
    when_true: bool,
) {
    match condition {
        Expr::Unary {
            op: UnaryOp::Not,
            operand,
            ..
        } => collect_scaled(operand, facts, env, subject, params, offsets, !when_true),

        Expr::Binary {
            op: BinaryOp::And,
            lhs,
            rhs,
            ..
        } if when_true => {
            collect_scaled(lhs, facts, env, subject, params, offsets, true);
            collect_scaled(rhs, facts, env, subject, params, offsets, true);
        }

        Expr::Binary {
            op: BinaryOp::Or,
            lhs,
            rhs,
            ..
        } if !when_true => {
            collect_scaled(lhs, facts, env, subject, params, offsets, false);
            collect_scaled(rhs, facts, env, subject, params, offsets, false);
        }

        Expr::Binary { op, lhs, rhs, .. } => {
            let Some(op) = (if when_true { Some(*op) } else { negated(*op) }) else {
                return;
            };

            // Both ways round, since `result` has to come out counted once and
            // `n == result` puts it on the wrong side.
            let form = linear(lhs, facts, env).add(linear(rhs, facts, env).negate());
            scaled_from(op, form, subject, params, offsets);
            if let Some(flipped) = flipped(op) {
                let form = linear(rhs, facts, env).add(linear(lhs, facts, env).negate());
                scaled_from(flipped, form, subject, params, offsets);
            }
        }

        _ => {}
    }
}

/// Reads one comparison as a bound on `result - factor * argument`.
fn scaled_from(
    op: BinaryOp,
    form: Linear,
    subject: DefId,
    params: &[(usize, DefId)],
    offsets: &mut BTreeMap<usize, Scaled>,
) {
    let mut terms = form.terms.clone();
    // One `result`, no more and no less. Two of it is a shape with nowhere to
    // go, and none of it is a clause about something else.
    if terms.remove(&Term::Name(subject)) != Some(1) {
        return;
    }

    let [(term, count)] = terms.into_iter().collect::<Vec<_>>()[..] else {
        return;
    };
    let Term::Name(def) = term else {
        // A promise about how long one of the arguments is. There is nowhere
        // in a `Guarantee` to put that: what crosses a call is a range and a
        // difference per argument, and a length is neither of those.
        return;
    };
    let Some(factor) = count.checked_neg() else {
        return;
    };
    let Some((index, _)) = params.iter().find(|(_, param)| *param == def) else {
        return;
    };
    // The constraint is `(result - factor * argument) + offset op 0`.
    let Some(range) = bound_from(op, form.offset) else {
        return;
    };

    record_scaled(offsets, *index, Scaled { factor, range });
}

/// What the fact machinery needs to know that it cannot read off the syntax.
///
/// Closures rather than a borrow of the checker, so this module stays
/// independent of how the checker stores things and can be tested without one.
pub struct Env<'a> {
    /// The definition an identifier refers to.
    pub def_of: &'a dyn Fn(&Expr) -> Option<DefId>,
    /// The `length` the language provides, when it is in scope.
    ///
    /// A definition rather than a spelling, because a name is only that
    /// builtin if it resolves to it and a module is free to declare a `length`
    /// of its own.
    pub length: Option<DefId>,
    /// What a call to this callee promises, from its declared return type and
    /// its `ensures` clause. Whoever answers this is responsible for only
    /// answering for contracts that are themselves checked, since a promise
    /// nobody keeps is not a fact.
    pub call: &'a dyn Fn(&Expr) -> Promise,
}

impl Env<'_> {
    /// An environment that knows nothing, for reading a predicate on its own.
    pub fn blind() -> Env<'static> {
        Env {
            def_of: &|_| None,
            length: None,
            call: &|_| Promise::any(),
        }
    }
}

/// What is known about how long something is.
///
/// A name has a term to hang the answer on. A list or a string written on the
/// spot has none, and it also has a length nobody has to work out: refusing to
/// count what the source says out loud would be refusing a fact for want of a
/// place to put it.
pub fn length_of(expr: &Expr, facts: &Facts, env: &Env<'_>) -> Range {
    match expr {
        Expr::List { elements, .. } => Range::exactly(elements.len() as i64),
        Expr::Str { value, .. } => Range::exactly(value.chars().count() as i64),
        _ => match term_of(expr, env) {
            Some(Term::Name(def)) => facts.get(Term::Length(def)),
            _ => Range::between(0, i64::MAX),
        },
    }
}

/// Whether an expression is the `value` a refinement predicate is about.
fn is_subject(expr: &Expr) -> bool {
    matches!(expr, Expr::Ident(ident) if ident.name == "value")
}

/// The term `value` stands for in a predicate, when it stands for one.
///
/// `value` itself is the name the value was given, and `length(value)` is that
/// name's length. Both are entries the body has been narrowing all along,
/// which is the whole reason for looking.
fn subject_term(expr: &Expr, facts: &Facts, env: &Env<'_>) -> Option<Term> {
    let name = facts.subject.and_then(|subject| subject.name)?;
    match expr {
        _ if is_subject(expr) => Some(Term::Name(name)),
        Expr::Call { callee, args, .. }
            if is_length_call(callee, args, env) && is_subject(&args[0]) =>
        {
            Some(Term::Length(name))
        }
        _ => None,
    }
}

/// What the predicate is asking about `value`, for a subject with no name.
///
/// A range for `value`, and the length for `length(value)`. A literal has no
/// term to hang either on and both are still known.
fn subject_range(expr: &Expr, facts: &Facts, env: &Env<'_>) -> Option<Range> {
    let subject = facts.subject?;
    match expr {
        _ if is_subject(expr) => Some(subject.range),
        Expr::Call { callee, args, .. }
            if is_length_call(callee, args, env) && is_subject(&args[0]) =>
        {
            Some(subject.length)
        }
        _ => None,
    }
}

/// Whether an expression is a call to the `length` the language provides.
fn is_length_call(callee: &Expr, args: &[Expr], env: &Env<'_>) -> bool {
    env.length.is_some() && args.len() == 1 && (env.def_of)(callee) == env.length
}

/// Whether `expr` contains a call the checker cannot see through, other than
/// one to `length`.
///
/// `length(x)` crosses a module boundary intact: `Origin::Elsewhere` still
/// knows which import is `length`. Any other call does not, because the
/// promise a call keeps comes from its own `ensures`, and reading a clause
/// across a boundary answers every such call with `Promise::any()` rather
/// than looking one up. Walking only as far as `holds` itself does, because a
/// clause is `and`/`or`/`not` over comparisons and nothing deeper needs this.
fn mentions_an_opaque_call(expr: &Expr, env: &Env<'_>) -> bool {
    match expr {
        Expr::Unary { operand, .. } => mentions_an_opaque_call(operand, env),
        Expr::Binary { lhs, rhs, .. } => {
            mentions_an_opaque_call(lhs, env) || mentions_an_opaque_call(rhs, env)
        }
        Expr::Call { callee, args, .. } => !is_length_call(callee, args, env),
        _ => false,
    }
}

/// Blames a boundary rather than a name, when a boundary is why.
///
/// An unsettled clause read across a module boundary that contains an opaque
/// call would sometimes have settled read locally, where the call's own
/// `ensures` is available. Nothing else changes: a clause with no such call
/// crossed the boundary exactly as thick as it was written, and keeps
/// whichever reason [`holds`] already gave it.
pub(crate) fn thinned_by_boundary(clause: &Expr, env: &Env<'_>, outcome: Truth) -> Truth {
    match outcome {
        Truth::Unknown(_) if mentions_an_opaque_call(clause, env) => {
            Truth::Unknown(Reason::CrossedAModuleBoundary)
        }
        other => other,
    }
}

/// Whether an expression is something a fact can be attached to.
///
/// Two shapes. A name, which is the obvious one, and `length(x)` where `x` is
/// a name, which is the one that makes an index and a bound comparable. Only a
/// name inside the call: `length(f(xs))` names nothing that stays put, and a
/// term that could mean two things across two calls is worse than no term.
pub fn term_of(expr: &Expr, env: &Env<'_>) -> Option<Term> {
    match expr {
        Expr::Ident(_) => (env.def_of)(expr).map(Term::Name),
        Expr::Call { callee, args, .. } if is_length_call(callee, args, env) => match &args[0] {
            Expr::Ident(_) => (env.def_of)(&args[0]).map(Term::Length),
            _ => None,
        },
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
    /// How many of each term, with a zero coefficient dropped rather than kept.
    terms: BTreeMap<Term, i64>,
    offset: Range,
}

impl Linear {
    fn constant(offset: Range) -> Linear {
        Linear {
            terms: BTreeMap::new(),
            offset,
        }
    }

    fn name(term: Term) -> Linear {
        Linear {
            terms: BTreeMap::from([(term, 1)]),
            offset: Range::exactly(0),
        }
    }

    fn negate(self) -> Linear {
        // Negating cannot lose a coefficient unless one reached `i64::MIN`,
        // which takes more names than a body has. Nothing known is the answer
        // if it ever happens, and nothing known is never wrong.
        self.scale(-1)
            .unwrap_or_else(|| Linear::constant(Range::ANY))
    }

    /// The same form with every coefficient multiplied, when that is a number.
    fn scale(self, factor: i64) -> Option<Linear> {
        let mut terms = BTreeMap::new();
        for (def, count) in self.terms {
            let scaled = count.checked_mul(factor)?;
            if scaled != 0 {
                terms.insert(def, scaled);
            }
        }
        Some(Linear {
            terms,
            offset: self.offset.mul(Range::exactly(factor)),
        })
    }

    /// The single number this is, when it is one.
    fn constant_value(&self) -> Option<i64> {
        if !self.terms.is_empty() {
            return None;
        }
        match self.offset {
            Range::Bounded { low, high } if low == high => Some(low),
            _ => None,
        }
    }

    fn add(self, other: Linear) -> Linear {
        let mut terms = self.terms;
        for (def, count) in other.terms {
            // A name on both sides cancels, so `n - n` is zero rather than a
            // difference nobody recorded, and `n + n` is two of one name rather
            // than a shape with nowhere to go.
            let total = terms.get(&def).copied().unwrap_or(0).saturating_add(count);
            if total == 0 {
                terms.remove(&def);
            } else {
                terms.insert(def, total);
            }
        }

        Linear {
            terms,
            offset: self.offset.add(other.offset),
        }
    }

    /// The pair this is the difference of, when it is the difference of a pair.
    fn pair(&self) -> Option<(Term, Term)> {
        let mut positive = None;
        let mut negative = None;
        for (term, count) in &self.terms {
            match count {
                1 if positive.is_none() => positive = Some(*term),
                -1 if negative.is_none() => negative = Some(*term),
                _ => return None,
            }
        }
        Some((positive?, negative?))
    }

    /// The pair this is the total of, when it is the total of a pair.
    ///
    /// The other shape two names come in. `count + delivered` is not a
    /// difference and not one name counted twice, and it is what a `where`
    /// clause about two parameters nearly always says.
    fn total(&self) -> Option<(Term, Term)> {
        let mut both = self.terms.iter().filter(|(_, count)| **count == 1);
        let (first, second) = (both.next()?.0, both.next()?.0);
        (self.terms.len() == 2).then_some((*first, *second))
    }

    /// The one name in this, and how many of it, when there is exactly one.
    fn scaled_name(&self) -> Option<(Term, i64)> {
        match self.terms.iter().next() {
            Some((term, count)) if self.terms.len() == 1 => Some((*term, *count)),
            _ => None,
        }
    }
}

/// Reads `expr` as names and an offset, as far as it goes.
fn linear(expr: &Expr, facts: &Facts, env: &Env<'_>) -> Linear {
    // `value` stands for whatever is being described. When that is a name it
    // is a term like any other, and when it is not it is a range.
    if let Some(term) = subject_term(expr, facts, env) {
        return Linear::name(term);
    }
    if let Some(range) = subject_range(expr, facts, env) {
        return Linear::constant(range);
    }
    match expr {
        Expr::Ident(_) => match term_of(expr, env) {
            Some(term) => Linear::name(term),
            None => Linear::constant(interval_of(expr, facts, env)),
        },
        // `length(items)` is a quantity with a name, so it can be one side of
        // a difference. Anything else that is called is whatever its contract
        // says and stays a range.
        Expr::Call { .. } => match term_of(expr, env) {
            Some(term) => Linear::name(term),
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
        // A name multiplied by a number is still a name, counted more than
        // once. A name multiplied by a name is not, and that is where linear
        // arithmetic stops and a solver would start.
        Expr::Binary {
            op: BinaryOp::Mul,
            lhs,
            rhs,
            ..
        } => {
            let left = linear(lhs, facts, env);
            let right = linear(rhs, facts, env);
            let scaled = match (left.constant_value(), right.constant_value()) {
                (Some(factor), None) => right.scale(factor),
                (None, Some(factor)) => left.scale(factor),
                // Two constants, which the interval already multiplies, and
                // two names, which nothing here can.
                _ => None,
            };
            scaled.unwrap_or_else(|| Linear::constant(interval_of(expr, facts, env)))
        }
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

/// The range the value inside the `ok` of `expr` lands in.
///
/// A call that can fail promises things about the number inside the `ok`, and
/// the expression is the `Result` around it, so the promise is only readable
/// where the payload is actually taken out: at a `?`, at an `ok(..)` pattern,
/// and where a `Result` is assigned into one with a refined success type.
/// Nothing names that value in the source, which is why this exists at all.
pub fn ok_range_of(expr: &Expr, facts: &Facts, env: &Env<'_>) -> Range {
    match expr {
        Expr::Call { callee, args, .. } => (env.call)(callee).ok.applied(args, facts, env),
        _ => Range::ANY,
    }
}

/// Where the arithmetic in `expr` can have no answer, if anywhere.
///
/// This is not a check, it is an explanation. `n / d` where `d` could be zero
/// is not provably anything, and a reader looking at that has every right to
/// think the reasoning is weak. It is not: the quotient has no answer when `d`
/// is zero, so there is no value to prove anything about. Saying which
/// operation, rather than leaving it to be worked out, is the difference
/// between a diagnostic and a shrug.
///
/// Only dividing, and only because adding, subtracting and multiplying stopped
/// being able to defeat a proof. Overflow is an error rather than a wrap, so
/// every value a sum can produce is inside `i64`, and [`Range::add`] saturates
/// rather than giving up. `n + 1` where `n` is `Positive` is provably positive
/// now, and a note saying it might not be would be a note about something that
/// no longer happens.
pub fn overflowing(expr: &Expr, facts: &Facts, env: &Env<'_>) -> Option<Span> {
    match expr {
        Expr::Unary {
            op: UnaryOp::Neg,
            operand,
            ..
        } => overflowing(operand, facts, env),

        Expr::Binary {
            op, lhs, rhs, span, ..
        } => {
            // The innermost one, because that is the one that has to be fixed
            // and the outer one is only a consequence of it.
            if let Some(inner) = overflowing(lhs, facts, env) {
                return Some(inner);
            }
            if let Some(inner) = overflowing(rhs, facts, env) {
                return Some(inner);
            }

            match op {
                // Dividing by zero has no answer, and neither does the
                // smallest integer divided by minus one.
                BinaryOp::Div | BinaryOp::Rem => {
                    let (low, high) = range_of(rhs, facts, env).bounds()?;
                    (low <= 0 && high >= 0).then_some(*span)
                }
                _ => None,
            }
        }

        _ => None,
    }
}

/// What the difference facts add to an expression, when it is a difference.
///
/// Or the total facts, when it is a total. Only worth the walk for a sum,
/// since nothing else can reduce to either. The arithmetic here is checked
/// rather than clamped: this is the value the program computes, and a
/// subtraction that overflows does not have one.
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
    if let Some((a, b)) = form.pair() {
        return Some(facts.difference(a, b).add(form.offset));
    }
    let (a, b) = form.total()?;
    Some(facts.total(a, b).add(form.offset))
}

/// The range an expression can take, from the ranges of the names in it alone.
fn interval_of(expr: &Expr, facts: &Facts, env: &Env<'_>) -> Range {
    // The same fact from two directions: what the caller worked out about the
    // value it is handing over, and what the body knows about the name it gave
    // it. Both are true, so they meet.
    if let Some(term) = subject_term(expr, facts, env) {
        let known = facts.get(term);
        return match subject_range(expr, facts, env) {
            Some(range) => known.meet(range),
            None => known,
        };
    }
    if let Some(range) = subject_range(expr, facts, env) {
        return range;
    }
    match expr {
        Expr::Int { value, .. } => Range::exactly(*value),
        // `Int.max` is the number, so a clause naming it is one the checker
        // can settle rather than guard. Every other field read is a value
        // nothing here knows.
        Expr::Field { .. } => match expr.int_limit() {
            Some(value) => Range::exactly(value),
            None => Range::ANY,
        },
        Expr::Ident(_) => match term_of(expr, env) {
            Some(term) => facts.get(term),
            None => Range::ANY,
        },
        // The contract of whatever is being called. This is the one place the
        // reasoning leaves the function it is looking at, and it is why a
        // proof inside one function is worth anything to its callers.
        //
        // A `length` is both: it has a contract like any other call and it is
        // a term the body may have narrowed, so the two are met rather than
        // one of them winning. A list written on the spot has no term and an
        // exact length, which is the other half of the same answer.
        Expr::Call { callee, args, .. } => {
            let promised = (env.call)(callee).value.applied(args, facts, env);
            if is_length_call(callee, args, env) {
                return promised.meet(length_of(&args[0], facts, env));
            }
            promised
        }
        // `f()?` is the number inside the `ok`, which is what the promise on a
        // fallible function was about all along.
        Expr::Try { operand, .. } => ok_range_of(operand, facts, env),
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
        _ => Truth::Unknown(Reason::NotAShapeTheCheckerReasonsAbout),
    }
}

/// What to blame when a comparison between `lhs` and `rhs` did not settle.
///
/// A `length(name)` on either side means the missing bound is about a length,
/// not a name; a bare name on either side means it is a name nothing narrowed;
/// neither means there was nothing here to key a fact on in the first place.
fn unresolved_comparison(lhs: &Expr, rhs: &Expr, env: &Env<'_>) -> Reason {
    match (term_of(lhs, env), term_of(rhs, env)) {
        (Some(Term::Length(_)), _) | (_, Some(Term::Length(_))) => {
            Reason::NothingEstablishedThisLength
        }
        (Some(Term::Name(_)), _) | (_, Some(Term::Name(_))) => Reason::NothingNarrowedThisName,
        (None, None) => Reason::NothingNamesThisValue,
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
    let unclear = || Truth::Unknown(unresolved_comparison(lhs, rhs, env));

    match op {
        BinaryOp::Lt => {
            if high < 0 {
                Truth::Always
            } else if low >= 0 {
                Truth::Never
            } else {
                unclear()
            }
        }
        BinaryOp::Le => {
            if high <= 0 {
                Truth::Always
            } else if low > 0 {
                Truth::Never
            } else {
                unclear()
            }
        }
        BinaryOp::Gt => {
            if low > 0 {
                Truth::Always
            } else if high <= 0 {
                Truth::Never
            } else {
                unclear()
            }
        }
        BinaryOp::Ge => {
            if low >= 0 {
                Truth::Always
            } else if high < 0 {
                Truth::Never
            } else {
                unclear()
            }
        }
        BinaryOp::Eq => {
            if low == 0 && high == 0 {
                Truth::Always
            } else if low > 0 || high < 0 {
                Truth::Never
            } else {
                unclear()
            }
        }
        BinaryOp::Ne => compare(BinaryOp::Eq, lhs, rhs, facts, env).negate(),
        _ => Truth::Unknown(Reason::NotAShapeTheCheckerReasonsAbout),
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
            rule_out_a_value(effective, lhs, rhs, facts, env);
            if let Some(flipped) = flipped(effective) {
                rule_out_a_value(flipped, rhs, lhs, facts, env);
            }

            narrow_scaled(effective, lhs, rhs, facts, env);
            narrow_relation(effective, lhs, rhs, facts, env);
        }

        _ => {}
    }
}

/// The range `x` is pinned to by `x + offset op 0`.
///
/// The offset is only known to be somewhere in its own range, so a bound has to
/// hold for the whole of it: the upper one comes from the smallest offset and
/// the lower one from the largest. An offset at the edge of what an `i64` holds
/// is not worth a special case, so nothing is learned there rather than
/// something wrong.
fn bound_from(op: BinaryOp, offset: Range) -> Option<Range> {
    let (low, high) = offset.bounds()?;
    let least = high.checked_neg()?;
    let most = low.checked_neg()?;

    Some(match op {
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
        // `!=` is answered in `rule_out_a_value`, where the term whose range
        // would be trimmed is in hand. Here the value has an offset on it, so
        // the edge to compare against is not the one this function was given.
        _ => return None,
    })
}

/// Records what `left op right` says about a pair of names.
///
/// This is the half an interval cannot hold. `low < high` says nothing about
/// either name on its own, and everything about `low - high`; `a + b > 0` says
/// nothing about either and everything about their total.
fn narrow_relation(op: BinaryOp, left: &Expr, right: &Expr, facts: &mut Facts, env: &Env<'_>) {
    let form = linear(left, facts, env).add(linear(right, facts, env).negate());
    // The constraint is `<the names> + offset op 0`, whichever shape the names
    // came in.
    let Some(bound) = bound_from(op, form.offset) else {
        return;
    };
    if let Some((a, b)) = form.pair() {
        facts.narrow_difference(a, b, bound);
    } else if let Some((a, b)) = form.total() {
        facts.narrow_total(a, b, bound);
    }
}

/// Narrows a name from a comparison, however many times it appears in it.
///
/// `n * 2 <= 100` says `n <= 50`, `n - 5 > 0` says `n > 5`, and a bare `n > 0`
/// is the same form with a count of one. Every comparison that bounds a name
/// is answered here; what a range cannot hold, and what is therefore somewhere
/// else, is a value ruled out rather than a bound.
fn narrow_scaled(op: BinaryOp, left: &Expr, right: &Expr, facts: &mut Facts, env: &Env<'_>) {
    let form = linear(left, facts, env).add(linear(right, facts, env).negate());
    let Some((term, count)) = form.scaled_name() else {
        return;
    };
    // The constraint is `(count * term) + offset op 0`.
    let Some(bound) = bound_from(op, form.offset) else {
        return;
    };
    let Some(narrowed) = divided(bound, count) else {
        return;
    };
    facts.narrow(term, narrowed);
}

/// The range of `d`, given the range of `count * d`.
///
/// Rounded inwards, which is the only direction that is not a lie: `2 * d <= 5`
/// admits `d <= 2` and nothing between two and three is an integer. Dividing by
/// a negative swaps the ends, which is the trap here and the reason this is not
/// written inline.
fn divided(bound: Range, count: i64) -> Option<Range> {
    if count == 0 {
        return None;
    }
    let Some((low, high)) = bound.bounds() else {
        return Some(Range::Empty);
    };

    let (low, high) = if count > 0 {
        (round_up(low, count)?, round_down(high, count)?)
    } else {
        (round_up(high, count)?, round_down(low, count)?)
    };
    Some(Range::between(low, high))
}

/// The largest integer `d` with `d * by <= value`, when `by` is positive, and
/// the mirror of that when it is negative. Plain floor division, which Rust
/// does not have.
fn round_down(value: i64, by: i64) -> Option<i64> {
    // The one division that overflows, and the value it overflows on is how
    // "no bound" is written, so the answer is that there is still no bound.
    // Returning nothing instead threw away the whole comparison: `0 < n - 5`
    // reaches here and used to learn less than `n - 5 > 0` did.
    if value == i64::MIN && by == -1 {
        return Some(i64::MAX);
    }
    let quotient = value.checked_div(by)?;
    let exact = value % by == 0;
    if exact || (value < 0) == (by < 0) {
        Some(quotient)
    } else {
        quotient.checked_sub(1)
    }
}

/// The smallest integer `d` with `d * by >= value`, and the mirror of that for
/// a negative `by`.
fn round_up(value: i64, by: i64) -> Option<i64> {
    // The mirror of the case in `round_down`. Nothing an `i64` holds satisfies
    // this one, so any answer is wider than the truth, and wider is the safe
    // direction for a bound.
    if value == i64::MIN && by == -1 {
        return Some(i64::MAX);
    }
    let quotient = value.checked_div(by)?;
    let exact = value % by == 0;
    if exact || (value < 0) != (by < 0) {
        Some(quotient)
    } else {
        quotient.checked_add(1)
    }
}

/// Rules a single value out of what a name could be, when it sits at an edge.
///
/// This used to narrow from every comparison, and for a while nobody could say
/// whether it or [`narrow_scaled`] owned that question. Both did: a bare name
/// compared with something is that function's linear form with a count of one.
/// Disabling arms here and running the whole suite said as much, except for
/// `Gt`, and the asymmetry was written down as possibly the shape of the
/// corpus.
///
/// It was not. `0 < n` came through `divided` with a count of minus one and an
/// unbounded end, `i64::MIN / -1` overflows, and the bound was dropped; this
/// function happened to catch the same clause the other way round. With that
/// division fixed, all four ordering arms here are dead across 2394 tests, so
/// they are gone and `narrow_scaled` owns bounding a name.
///
/// What is left is a different question, and one an interval genuinely cannot
/// ask the linear form: `!=` says something exactly when the other side is a
/// single value sitting at an edge of what is already known, because
/// everything the term could have been, minus one end, is still a range.
///
/// That was skipped as rare until #929 measured it. `if n == Int.min` and
/// `if n <= Int.min` say the same thing about the else branch, and only the
/// second one narrowed, so one of them proved an obligation and the other left
/// it guarded. Two spellings of one claim answering differently is worse than
/// either answer.
fn rule_out_a_value(op: BinaryOp, left: &Expr, right: &Expr, facts: &mut Facts, env: &Env<'_>) {
    if op != BinaryOp::Ne {
        return;
    }
    let Some(term) = term_of(left, env) else {
        return;
    };
    let Some((low, high)) = range_of(right, facts, env).bounds() else {
        return;
    };
    // A range on the other side rules nothing out. One value does.
    if low != high {
        return;
    }
    let Some((known_low, known_high)) = facts.get(term).bounds() else {
        return;
    };

    let narrowed = if low == known_low {
        match known_low.checked_add(1) {
            Some(bound) => Range::between(bound, known_high),
            None => Range::Empty,
        }
    } else if high == known_high {
        match known_high.checked_sub(1) {
            Some(bound) => Range::between(known_low, bound),
            None => Range::Empty,
        }
    } else {
        // Somewhere in the middle. What is left is two ranges, and a range is
        // what this holds.
        return;
    };

    facts.narrow(term, narrowed);
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
    admitted_by(predicate, &Env::blind()).range
}

/// Everything a refinement predicate admits about the thing it describes.
///
/// The range and the length, because `length(value) > 0` is as much a fact
/// about a parameter as `value > 0` is, and it belongs on the entry everything
/// else reads rather than nowhere. Needs a real [`Env`] to recognise `length`,
/// since a name is only that builtin if it resolves to it.
pub fn admitted_by(predicate: &Expr, env: &Env<'_>) -> Subject {
    let mut narrowed = Facts::new();
    apply_subject_narrowing(predicate, &mut narrowed, env, "value", true);
    narrowed.subject.unwrap_or(Subject::of(Range::ANY))
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
    narrowed
        .subject
        .map(|subject| subject.range)
        .unwrap_or(Range::ANY)
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

/// Which half of the subject a comparison is about.
#[derive(Clone, Copy, PartialEq, Eq)]
enum About {
    Value,
    Length,
}

fn narrow_subject(
    op: BinaryOp,
    left: &Expr,
    right: &Expr,
    facts: &mut Facts,
    env: &Env<'_>,
    subject: &str,
) {
    let names_subject = |expr: &Expr| matches!(expr, Expr::Ident(ident) if ident.name == subject);
    let about = match left {
        _ if names_subject(left) => About::Value,
        Expr::Call { callee, args, .. }
            if is_length_call(callee, args, env) && names_subject(&args[0]) =>
        {
            About::Length
        }
        _ => return,
    };
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

    let current = facts.subject.unwrap_or(Subject::of(Range::ANY));
    facts.subject = Some(match about {
        About::Value => Subject {
            range: current.range.meet(narrowed),
            ..current
        },
        About::Length => Subject {
            length: current.length.meet(narrowed),
            ..current
        },
    });
}

#[cfg(test)]
mod tests {
    use deed_ast::Ident;

    use super::*;

    /// An integer-valued name, for the tests that only need a key.
    fn name(raw: u32) -> Term {
        Term::Name(DefId::from_raw(raw))
    }

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
    fn arithmetic_that_leaves_the_integers_clamps_to_the_edge() {
        // Not because clamping is close enough, but because it is exact.
        // Overflow is an error rather than a wrap, so `i64::MAX + 1` produces
        // no value at all, and the only value this expression can produce is
        // `i64::MAX` itself, from nothing.
        let huge = Range::exactly(i64::MAX);
        assert_eq!(huge.add(Range::exactly(1)), Range::exactly(i64::MAX));
        assert_eq!(huge.mul(Range::exactly(2)), Range::exactly(i64::MAX));

        // The interesting one: an unbounded name plus one is everything except
        // the smallest integer, which used to be "anything at all".
        assert_eq!(
            Range::ANY.add(Range::exactly(1)),
            Range::between(i64::MIN + 1, i64::MAX)
        );
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
        // The subtraction can be larger than an integer, so the bound clamps
        // at the edge. Which side of zero it falls on is not in doubt, and
        // that is the only thing a comparison asks.
        let below = Range::between(i64::MIN, -1);
        let above = Range::between(0, i64::MAX);
        assert_eq!(below.sub(above), Range::between(i64::MIN, -1));

        let (_, highest) = below.spread(above).bounds().expect("not empty");
        assert!(highest < 0, "the difference is negative whatever its size");
    }

    #[test]
    fn two_differences_that_share_a_name_make_a_third() {
        let (a, b, c) = (name(0), name(1), name(2));
        let mut facts = Facts::new();
        facts.narrow_difference(a, b, Range::between(i64::MIN, -1));
        facts.narrow_difference(b, c, Range::between(i64::MIN, -1));

        // `a < b` and `b < c`, so `a < c`, however far apart any of them are.
        let (_, highest) = facts.difference(a, c).bounds().expect("not empty");
        assert!(highest < 0);
    }

    #[test]
    fn a_difference_carries_a_bound_from_one_name_to_the_other() {
        let (n, limit) = (name(0), name(1));
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
        let (a, b) = (name(0), name(1));
        let mut facts = Facts::new();
        facts.narrow_difference(a, b, Range::between(i64::MIN, -1));
        facts.set(DefId::from_raw(0), Range::ANY);
        assert_eq!(facts.difference(a, b), Range::ANY);
    }

    /// And a total is a fact about the old meaning of the name just as much.
    /// Both orders, because a total is stored both ways round and keeping
    /// either one would keep the fact.
    #[test]
    fn a_binding_does_not_inherit_the_totals_of_the_name_it_replaces() {
        let (a, b) = (name(0), name(1));
        let mut facts = Facts::new();
        facts.narrow_total(a, b, Range::between(1, i64::MAX));
        facts.set(DefId::from_raw(0), Range::ANY);
        assert_eq!(facts.total(a, b), Range::ANY);
        assert_eq!(facts.total(b, a), Range::ANY);
    }

    /// What both branches of an `if` know about a total is still known after
    /// it, which is the whole point of joining rather than dropping.
    #[test]
    fn a_total_both_branches_know_survives_the_join() {
        let (a, b) = (name(0), name(1));
        let mut left = Facts::new();
        left.narrow_total(a, b, Range::between(1, 10));
        let mut right = Facts::new();
        right.narrow_total(a, b, Range::between(5, 20));

        assert_eq!(left.join(&right).total(a, b), Range::between(1, 20));
    }

    #[test]
    fn a_length_is_not_negative_without_anybody_saying_so() {
        // The default is the fact. A list with fewer than no things in it does
        // not exist, so nothing has to state it and no call has to hand it
        // back for the domain to know.
        let facts = Facts::new();
        assert_eq!(
            facts.get(Term::Length(DefId::from_raw(0))),
            Range::between(0, i64::MAX)
        );
        assert_eq!(facts.get(name(0)), Range::ANY);
    }

    #[test]
    fn an_index_below_a_length_is_a_difference_like_any_other() {
        // The point of a length being a term. `index < length(items)` is the
        // same shape as `low < high`, and the machinery that already held one
        // holds the other with no new arithmetic behind it.
        let index = name(0);
        let length = Term::Length(DefId::from_raw(1));
        let mut facts = Facts::new();
        facts.narrow_difference(index, length, Range::between(i64::MIN, -1));
        facts.narrow(length, Range::between(0, 3));

        let (_, highest) = facts.get(index).bounds().expect("not empty");
        assert_eq!(highest, 2);
    }

    #[test]
    fn a_binding_forgets_how_long_the_old_value_was() {
        // `set` is what a binding does, and a length recorded against the name
        // it replaces is a fact about something that is no longer there.
        let def = DefId::from_raw(0);
        let mut facts = Facts::new();
        facts.narrow(Term::Length(def), Range::exactly(3));
        facts.set(def, Range::ANY);
        assert_eq!(facts.get(Term::Length(def)), Range::between(0, i64::MAX));
    }

    #[test]
    fn joining_two_branches_keeps_only_the_differences_both_agree_on() {
        let (a, b) = (name(0), name(1));
        let mut left = Facts::new();
        left.narrow_difference(a, b, Range::between(-10, -1));
        let mut right = Facts::new();
        right.narrow_difference(a, b, Range::between(-4, 8));

        assert_eq!(left.join(&right).difference(a, b), Range::between(-10, 8));
        assert_eq!(left.join(&Facts::new()).difference(a, b), Range::ANY);
    }

    #[test]
    fn dividing_a_bound_rounds_inwards() {
        // `3 * d` somewhere in `[-2, 7]` puts `d` in `[0, 2]`. Rounding either
        // end outwards would admit a `d` the constraint does not, which is the
        // whole risk in doing this at all.
        assert_eq!(
            divided(Range::between(-2, 7), 3),
            Some(Range::between(0, 2))
        );

        // A negative multiplier swaps the ends, which is the other trap.
        assert_eq!(
            divided(Range::between(-2, 7), -3),
            Some(Range::between(-2, 0))
        );

        assert_eq!(divided(Range::between(4, 4), 2), Some(Range::exactly(2)));
        assert_eq!(divided(Range::between(1, 1), 2), Some(Range::Empty));
        assert_eq!(divided(Range::ANY, 0), None);
    }

    /// The end of what an `i64` holds is how "no bound" is written, and it
    /// used to take the whole range with it.
    ///
    /// `i64::MIN / -1` is the one division that overflows, and a bound with an
    /// open end and a count of minus one is what `0 < n - 5` comes to, so that
    /// clause learned nothing while `n - 5 > 0` learned everything. Nothing
    /// noticed, because `0 < n` on its own was caught by a second function
    /// that has since been reduced to the question only it can answer.
    #[test]
    fn an_open_end_divided_by_minus_one_stays_open() {
        assert_eq!(
            divided(Range::between(i64::MIN, -6), -1),
            Some(Range::between(6, i64::MAX))
        );

        // The mirror does not overflow, so it stays exact rather than opening
        // up: `-d >= i64::MAX` puts `d` one above the smallest `i64`, and
        // saying so is a tighter bound than saying there is none.
        assert_eq!(
            divided(Range::between(6, i64::MAX), -1),
            Some(Range::between(-i64::MAX, -6))
        );
    }

    /// A bare identifier, so a test can name a value without a real resolver.
    fn ident(text: &str) -> Expr {
        Expr::Ident(Ident::new(text, Span::at(0)))
    }

    fn less_than(lhs: Expr, rhs: Expr) -> Expr {
        Expr::Binary {
            op: BinaryOp::Lt,
            op_span: Span::at(0),
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
            span: Span::at(0),
        }
    }

    fn call(callee: Expr, args: Vec<Expr>) -> Expr {
        Expr::Call {
            callee: Box::new(callee),
            args,
            span: Span::at(0),
        }
    }

    /// An environment where `n` resolves to a name and `length` resolves to
    /// the builtin, and nothing else does.
    fn env_with_a_name() -> Env<'static> {
        Env {
            def_of: &|expr| match expr {
                Expr::Ident(ident) if ident.name == "n" => Some(DefId::from_raw(0)),
                Expr::Ident(ident) if ident.name == "length" => Some(DefId::from_raw(1)),
                _ => None,
            },
            length: Some(DefId::from_raw(1)),
            call: &|_| Promise::any(),
        }
    }

    fn unknown_reason(truth: Truth) -> Reason {
        match truth {
            Truth::Unknown(reason) => reason,
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn a_shape_the_checker_does_not_evaluate_says_so() {
        // A call used as a whole condition, rather than `and`/`or`/`not`/a
        // comparison: `holds` never reasons about what a call returns.
        let condition = call(ident("truthy"), vec![]);
        let facts = Facts::new();
        assert_eq!(
            unknown_reason(holds(&condition, &facts, &Env::blind())),
            Reason::NotAShapeTheCheckerReasonsAbout
        );
    }

    #[test]
    fn a_name_with_no_narrowing_blames_the_name() {
        let condition = less_than(
            ident("n"),
            Expr::Int {
                value: 10,
                span: Span::at(0),
            },
        );
        let facts = Facts::new();
        assert_eq!(
            unknown_reason(holds(&condition, &facts, &env_with_a_name())),
            Reason::NothingNarrowedThisName
        );
    }

    #[test]
    fn a_length_with_no_bound_blames_the_length() {
        let condition = less_than(
            call(ident("length"), vec![ident("n")]),
            Expr::Int {
                value: 10,
                span: Span::at(0),
            },
        );
        let facts = Facts::new();
        assert_eq!(
            unknown_reason(holds(&condition, &facts, &env_with_a_name())),
            Reason::NothingEstablishedThisLength
        );
    }

    #[test]
    fn neither_side_a_name_blames_the_value() {
        // `unknown < unknown`: two identifiers this environment cannot
        // resolve, so there is no term on either side to key a fact on.
        let condition = less_than(ident("a"), ident("b"));
        let facts = Facts::new();
        assert_eq!(
            unknown_reason(holds(&condition, &facts, &Env::blind())),
            Reason::NothingNamesThisValue
        );
    }

    #[test]
    fn an_opaque_call_inside_a_boundary_clause_blames_the_boundary() {
        let clause = less_than(
            call(ident("check"), vec![ident("n")]),
            Expr::Int {
                value: 10,
                span: Span::at(0),
            },
        );
        let outcome = Truth::Unknown(Reason::NothingNamesThisValue);
        assert_eq!(
            thinned_by_boundary(&clause, &env_with_a_name(), outcome),
            Truth::Unknown(Reason::CrossedAModuleBoundary)
        );
    }

    #[test]
    fn a_boundary_clause_with_no_opaque_call_keeps_its_own_reason() {
        let clause = less_than(
            ident("n"),
            Expr::Int {
                value: 10,
                span: Span::at(0),
            },
        );
        let outcome = Truth::Unknown(Reason::NothingNarrowedThisName);
        assert_eq!(
            thinned_by_boundary(&clause, &env_with_a_name(), outcome),
            Truth::Unknown(Reason::NothingNarrowedThisName)
        );
    }

    #[test]
    fn a_length_call_at_a_boundary_is_not_an_opaque_one() {
        // `length(n)` crosses a boundary intact (`Env::length` still resolves
        // it), so it should never be blamed on the boundary itself.
        let clause = less_than(
            call(ident("length"), vec![ident("n")]),
            Expr::Int {
                value: 10,
                span: Span::at(0),
            },
        );
        let outcome = Truth::Unknown(Reason::NothingEstablishedThisLength);
        assert_eq!(
            thinned_by_boundary(&clause, &env_with_a_name(), outcome),
            Truth::Unknown(Reason::NothingEstablishedThisLength)
        );
    }

    #[test]
    fn every_reason_renders_a_sentence_someone_can_read() {
        for reason in [
            Reason::NothingNarrowedThisName,
            Reason::NothingEstablishedThisLength,
            Reason::NothingNamesThisValue,
            Reason::CrossedAModuleBoundary,
            Reason::NotAShapeTheCheckerReasonsAbout,
        ] {
            assert!(!reason.text().is_empty());
        }
    }
}
