//! What is known about the integers in scope.
//!
//! The Proven tier used to mean "this is a literal and the predicate evaluates
//! to true". That is a small enough slice of the tier that almost every
//! refinement in real code was a runtime check with a paragraph of ceremony
//! around it, and the argument for having refinements at all is that they
//! replace checks rather than decorate them.
//!
//! This is interval reasoning and nothing more. Each integer-valued name gets a
//! range, ranges come from the things that state one (a `where` clause, an
//! `if`, a guard that leaves, a refined parameter type), and a predicate is
//! discharged by evaluating it over the range rather than over a value.
//!
//! # What this cannot do
//!
//! A great deal, and the answer is always "not proven", never a wrong answer.
//!
//! - **Relationships between variables.** `a < b` is not something an interval
//!   can hold, so a `where a < b` proves nothing about `b - a`. This is the
//!   biggest limitation and the first thing anyone will hit.
//! - **Anything that is not an integer.** No `String`, no record field, no
//!   variant.
//! - **Reasoning across a call.** A function's `ensures` clause is not
//!   consulted, so the result of any call is unknown.
//! - **Division and remainder.** The sign rules around zero and around
//!   `i64::MIN` are fiddly enough that getting them wrong is worse than not
//!   trying.

use std::collections::HashMap;

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

/// Ranges for the names in scope.
///
/// Keyed by definition, which resolution already made unique, so a shadowed
/// name cannot pick up a fact about the one it hid. There is no shadowing in
/// Vow anyway, and relying on that rather than on the key would be relying on
/// something that could change.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    known: HashMap<DefId, Range>,
    /// The range `value` stands for, while a refinement predicate is being
    /// evaluated. `value` has no definition of its own.
    subject: Option<Range>,
}

impl Facts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, def: DefId) -> Range {
        self.known.get(&def).copied().unwrap_or(Range::ANY)
    }

    pub fn set(&mut self, def: DefId, range: Range) {
        self.known.insert(def, range);
    }

    /// Narrows what is known about `def`, keeping anything already known.
    pub fn narrow(&mut self, def: DefId, range: Range) {
        let narrowed = self.get(def).meet(range);
        self.known.insert(def, narrowed);
    }

    pub fn with_subject(&self, range: Range) -> Facts {
        Facts {
            known: self.known.clone(),
            subject: Some(range),
        }
    }

    /// Everything both sides agree on, for joining two branches.
    pub fn join(&self, other: &Facts) -> Facts {
        let mut joined = HashMap::new();
        for (def, range) in &self.known {
            let combined = range.join(other.get(*def));
            if !combined.is_any() {
                joined.insert(*def, combined);
            }
        }
        Facts {
            known: joined,
            subject: None,
        }
    }
}

/// Whether an expression is a name a fact can be attached to.
fn def_of<'a>(expr: &'a Expr, resolve: &dyn Fn(&'a Expr) -> Option<DefId>) -> Option<DefId> {
    match expr {
        Expr::Ident(_) => resolve(expr),
        _ => None,
    }
}

/// The range an expression can take, given what is known.
///
/// `resolve` turns an identifier into the definition it refers to. It is a
/// closure rather than a borrow of the resolution table so this module stays
/// independent of how the checker stores things.
pub fn range_of<'a>(
    expr: &'a Expr,
    facts: &Facts,
    resolve: &dyn Fn(&'a Expr) -> Option<DefId>,
) -> Range {
    match expr {
        Expr::Int { value, .. } => Range::exactly(*value),
        Expr::Ident(ident) if ident.name == "value" => facts.subject.unwrap_or(Range::ANY),
        Expr::Ident(_) => match def_of(expr, resolve) {
            Some(def) => facts.get(def),
            None => Range::ANY,
        },
        Expr::Unary {
            op: UnaryOp::Neg,
            operand,
            ..
        } => range_of(operand, facts, resolve).negate(),
        Expr::Binary { op, lhs, rhs, .. } => {
            let left = range_of(lhs, facts, resolve);
            let right = range_of(rhs, facts, resolve);
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
pub fn holds<'a>(
    condition: &'a Expr,
    facts: &Facts,
    resolve: &dyn Fn(&'a Expr) -> Option<DefId>,
) -> Truth {
    match condition {
        Expr::Bool { value, .. } => Truth::of(*value),
        Expr::Unary {
            op: UnaryOp::Not,
            operand,
            ..
        } => holds(operand, facts, resolve).negate(),
        Expr::Binary { op, lhs, rhs, .. } => match op {
            BinaryOp::And => holds(lhs, facts, resolve).and(holds(rhs, facts, resolve)),
            BinaryOp::Or => holds(lhs, facts, resolve).or(holds(rhs, facts, resolve)),
            _ => compare(*op, lhs, rhs, facts, resolve),
        },
        _ => Truth::Unknown,
    }
}

fn compare<'a>(
    op: BinaryOp,
    lhs: &'a Expr,
    rhs: &'a Expr,
    facts: &Facts,
    resolve: &dyn Fn(&'a Expr) -> Option<DefId>,
) -> Truth {
    let left = range_of(lhs, facts, resolve);
    let right = range_of(rhs, facts, resolve);

    let (Some((a_low, a_high)), Some((b_low, b_high))) = (left.bounds(), right.bounds()) else {
        // One side cannot happen, so the comparison never does either.
        return Truth::Never;
    };

    match op {
        BinaryOp::Lt => {
            if a_high < b_low {
                Truth::Always
            } else if a_low >= b_high {
                Truth::Never
            } else {
                Truth::Unknown
            }
        }
        BinaryOp::Le => {
            if a_high <= b_low {
                Truth::Always
            } else if a_low > b_high {
                Truth::Never
            } else {
                Truth::Unknown
            }
        }
        BinaryOp::Gt => compare(BinaryOp::Lt, rhs, lhs, facts, resolve),
        BinaryOp::Ge => compare(BinaryOp::Le, rhs, lhs, facts, resolve),
        BinaryOp::Eq => {
            if a_low == a_high && b_low == b_high && a_low == b_low {
                Truth::Always
            } else if a_high < b_low || b_high < a_low {
                Truth::Never
            } else {
                Truth::Unknown
            }
        }
        BinaryOp::Ne => compare(BinaryOp::Eq, lhs, rhs, facts, resolve).negate(),
        _ => Truth::Unknown,
    }
}

/// What `condition` tells you about the names in it, when it holds.
///
/// Only comparisons of a name against something with a known range, and
/// conjunctions of those. An `||` tells you nothing about either side on its
/// own, which is why it is absent rather than approximated.
pub fn narrowed<'a>(
    condition: &'a Expr,
    facts: &Facts,
    resolve: &dyn Fn(&'a Expr) -> Option<DefId>,
    when_true: bool,
) -> Facts {
    let mut result = facts.clone();
    apply_narrowing(condition, &mut result, resolve, when_true);
    result
}

fn apply_narrowing<'a>(
    condition: &'a Expr,
    facts: &mut Facts,
    resolve: &dyn Fn(&'a Expr) -> Option<DefId>,
    when_true: bool,
) {
    match condition {
        Expr::Unary {
            op: UnaryOp::Not,
            operand,
            ..
        } => apply_narrowing(operand, facts, resolve, !when_true),

        // `a && b` holding means both hold. `a && b` failing means at least one
        // failed, which says nothing about either.
        Expr::Binary {
            op: BinaryOp::And,
            lhs,
            rhs,
            ..
        } if when_true => {
            apply_narrowing(lhs, facts, resolve, true);
            apply_narrowing(rhs, facts, resolve, true);
        }

        // The mirror image: `a || b` failing means both failed.
        Expr::Binary {
            op: BinaryOp::Or,
            lhs,
            rhs,
            ..
        } if !when_true => {
            apply_narrowing(lhs, facts, resolve, false);
            apply_narrowing(rhs, facts, resolve, false);
        }

        Expr::Binary { op, lhs, rhs, .. } => {
            let effective = if when_true { Some(*op) } else { negated(*op) };
            let Some(effective) = effective else { return };

            // Both directions, so `0 < n` narrows `n` the same way `n > 0`
            // does.
            narrow_side(effective, lhs, rhs, facts, resolve);
            if let Some(flipped) = flipped(effective) {
                narrow_side(flipped, rhs, lhs, facts, resolve);
            }
        }

        _ => {}
    }
}

/// Narrows the left side of `left op right`.
fn narrow_side<'a>(
    op: BinaryOp,
    left: &'a Expr,
    right: &'a Expr,
    facts: &mut Facts,
    resolve: &dyn Fn(&'a Expr) -> Option<DefId>,
) {
    let Some(def) = def_of(left, resolve) else {
        return;
    };
    let Some((low, high)) = range_of(right, facts, resolve).bounds() else {
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
    let facts = Facts::new();
    let resolve = |_: &Expr| None;
    let mut narrowed = facts.clone();
    apply_subject_narrowing(predicate, &mut narrowed, &resolve, true);
    narrowed.subject.unwrap_or(Range::ANY)
}

/// The same walk as [`apply_narrowing`], for the implicit `value`.
fn apply_subject_narrowing<'a>(
    predicate: &'a Expr,
    facts: &mut Facts,
    resolve: &dyn Fn(&'a Expr) -> Option<DefId>,
    when_true: bool,
) {
    match predicate {
        Expr::Unary {
            op: UnaryOp::Not,
            operand,
            ..
        } => apply_subject_narrowing(operand, facts, resolve, !when_true),
        Expr::Binary {
            op: BinaryOp::And,
            lhs,
            rhs,
            ..
        } if when_true => {
            apply_subject_narrowing(lhs, facts, resolve, true);
            apply_subject_narrowing(rhs, facts, resolve, true);
        }
        Expr::Binary { op, lhs, rhs, .. } => {
            let Some(effective) = (if when_true { Some(*op) } else { negated(*op) }) else {
                return;
            };
            narrow_subject(effective, lhs, rhs, facts, resolve);
            if let Some(flipped) = flipped(effective) {
                narrow_subject(flipped, rhs, lhs, facts, resolve);
            }
        }
        _ => {}
    }
}

fn narrow_subject<'a>(
    op: BinaryOp,
    left: &'a Expr,
    right: &'a Expr,
    facts: &mut Facts,
    resolve: &dyn Fn(&'a Expr) -> Option<DefId>,
) {
    if !matches!(left, Expr::Ident(ident) if ident.name == "value") {
        return;
    }
    let Some((low, high)) = range_of(right, facts, resolve).bounds() else {
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
}
