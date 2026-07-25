//! Runtime values.
//!
//! Everything is immutable and cheap to clone. The only mutable thing in the
//! language is handler state, which lives in the interpreter rather than in a
//! value, so nothing here needs interior mutability and structural equality is
//! just `==`.
//!
//! That last part matters more than it looks: `unchanged(E)` is implemented by
//! comparing a handler's state before and after, and it only works because
//! values compare structurally.

use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

use vow_resolve::DefId;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Value {
    Unit,
    Int(i64),
    Str(Rc<str>),
    Bool(bool),
    Record(Rc<Fields>),
    Variant(Rc<VariantValue>),
    /// `ok(v)` or `err(e)`.
    Result {
        ok: bool,
        value: Rc<Value>,
    },
}

/// Field values, ordered by name so that two records built in different orders
/// compare equal and print the same.
pub type Fields = BTreeMap<String, Value>;

#[derive(PartialEq, Eq, Debug)]
pub struct VariantValue {
    pub def: DefId,
    pub name: String,
    pub fields: Fields,
}

impl Value {
    pub fn record(fields: Fields) -> Self {
        Value::Record(Rc::new(fields))
    }

    pub fn variant(def: DefId, name: impl Into<String>, fields: Fields) -> Self {
        Value::Variant(Rc::new(VariantValue {
            def,
            name: name.into(),
            fields,
        }))
    }

    pub fn ok(value: Value) -> Self {
        Value::Result {
            ok: true,
            value: Rc::new(value),
        }
    }

    pub fn err(value: Value) -> Self {
        Value::Result {
            ok: false,
            value: Rc::new(value),
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// What kind of thing this is, for a diagnostic.
    pub fn describe(&self) -> &'static str {
        match self {
            Value::Unit => "()",
            Value::Int(_) => "an Int",
            Value::Str(_) => "a String",
            Value::Bool(_) => "a Bool",
            Value::Record(_) => "a record",
            Value::Variant(_) => "a variant",
            Value::Result { .. } => "a Result",
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Unit => write!(f, "()"),
            Value::Int(value) => write!(f, "{value}"),
            Value::Str(value) => write!(f, "{value:?}"),
            Value::Bool(value) => write!(f, "{value}"),
            Value::Record(fields) => write_fields(f, "", fields),
            Value::Variant(variant) => {
                if variant.fields.is_empty() {
                    write!(f, "{}", variant.name)
                } else {
                    write_fields(f, &variant.name, &variant.fields)
                }
            }
            Value::Result { ok, value } => {
                write!(f, "{}({value})", if *ok { "ok" } else { "err" })
            }
        }
    }
}

fn write_fields(f: &mut fmt::Formatter<'_>, prefix: &str, fields: &Fields) -> fmt::Result {
    if !prefix.is_empty() {
        write!(f, "{prefix} ")?;
    }
    write!(f, "{{ ")?;
    for (index, (name, value)) in fields.iter().enumerate() {
        if index > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{name}: {value}")?;
    }
    write!(f, " }}")
}

#[cfg(test)]
mod tests {
    use super::{Fields, Value};
    use vow_resolve::DefId;

    #[test]
    fn records_compare_by_content_not_by_insertion_order() {
        let mut one = Fields::new();
        one.insert("a".into(), Value::Int(1));
        one.insert("b".into(), Value::Int(2));

        let mut other = Fields::new();
        other.insert("b".into(), Value::Int(2));
        other.insert("a".into(), Value::Int(1));

        assert_eq!(Value::record(one), Value::record(other));
    }

    #[test]
    fn variants_of_different_shapes_differ() {
        let empty = Value::variant(DefId::from_raw(0), "A", Fields::new());
        let mut fields = Fields::new();
        fields.insert("n".into(), Value::Int(1));
        assert_ne!(empty, Value::variant(DefId::from_raw(0), "A", fields));
    }

    #[test]
    fn display_is_readable() {
        let mut fields = Fields::new();
        fields.insert("units".into(), Value::Int(40));
        assert_eq!(Value::record(fields).to_string(), "{ units: 40 }");
        assert_eq!(
            Value::variant(DefId::from_raw(1), "Bare", Fields::new()).to_string(),
            "Bare"
        );
        assert_eq!(Value::Bool(true).to_string(), "true");
    }
}
