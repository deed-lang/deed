//! Effect rows.
//!
//! A row is the set of things a function may do beyond returning a value. An
//! empty row means the function is pure, which is the default and, in most
//! code, the truth.

use std::collections::BTreeSet;

use vow_resolve::DefId;

/// One entry in a row: an effect, and optionally a single operation of it.
///
/// The operation is kept as a name rather than a definition so that effects
/// from modules the compiler has not loaded can still be written down. They
/// cannot be checked, but they can be represented, and pretending otherwise
/// would mean losing the declaration entirely.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct EffectItem {
    pub effect: DefId,
    /// `None` means every operation of the effect.
    pub operation: Option<String>,
}

impl EffectItem {
    pub fn whole(effect: DefId) -> Self {
        Self {
            effect,
            operation: None,
        }
    }

    pub fn operation(effect: DefId, operation: impl Into<String>) -> Self {
        Self {
            effect,
            operation: Some(operation.into()),
        }
    }

    /// Whether granting `self` also grants `other`.
    pub fn covers(&self, other: &EffectItem) -> bool {
        if self.effect != other.effect {
            return false;
        }
        match (&self.operation, &other.operation) {
            (None, _) => true,
            (Some(mine), Some(theirs)) => mine == theirs,
            (Some(_), None) => false,
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Row {
    items: BTreeSet<EffectItem>,
}

impl Row {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, item: EffectItem) {
        self.items.insert(item);
    }

    pub fn extend(&mut self, other: &Row) {
        self.items.extend(other.items.iter().cloned());
    }

    pub fn iter(&self) -> impl Iterator<Item = &EffectItem> {
        self.items.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Whether some entry in this row permits `item`.
    pub fn covers(&self, item: &EffectItem) -> bool {
        self.items.iter().any(|granted| granted.covers(item))
    }
}

impl FromIterator<EffectItem> for Row {
    fn from_iter<T: IntoIterator<Item = EffectItem>>(iter: T) -> Self {
        Self {
            items: iter.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EffectItem, Row};
    use vow_resolve::DefId;

    #[test]
    fn a_whole_effect_covers_its_operations() {
        let ledger = DefId::from_raw(0);
        let whole = EffectItem::whole(ledger);
        assert!(whole.covers(&EffectItem::operation(ledger, "post")));
        assert!(whole.covers(&EffectItem::whole(ledger)));
    }

    #[test]
    fn an_operation_does_not_cover_the_whole_effect() {
        let ledger = DefId::from_raw(0);
        let post = EffectItem::operation(ledger, "post");
        assert!(!post.covers(&EffectItem::whole(ledger)));
        assert!(!post.covers(&EffectItem::operation(ledger, "balance")));
        assert!(post.covers(&EffectItem::operation(ledger, "post")));
    }

    #[test]
    fn different_effects_never_cover_each_other() {
        let ledger = DefId::from_raw(0);
        let audit = DefId::from_raw(1);
        assert!(!EffectItem::whole(ledger).covers(&EffectItem::whole(audit)));
    }

    #[test]
    fn a_row_covers_what_any_entry_covers() {
        let ledger = DefId::from_raw(0);
        let audit = DefId::from_raw(1);
        let row: Row = [
            EffectItem::operation(ledger, "post"),
            EffectItem::whole(audit),
        ]
        .into_iter()
        .collect();

        assert!(row.covers(&EffectItem::operation(ledger, "post")));
        assert!(row.covers(&EffectItem::operation(audit, "append")));
        assert!(!row.covers(&EffectItem::operation(ledger, "balance")));
    }
}
