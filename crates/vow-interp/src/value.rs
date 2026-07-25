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
use std::path::Path;
use std::rc::Rc;

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
    /// Authority to do something to something.
    ///
    /// Opaque on purpose. There is nothing to know about a capability except
    /// that you were handed it, and the only way to get one is to be passed
    /// one, which is what makes the absence of an argument mean something.
    Capability(Capability),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Capability {
    /// The root. The runtime hands out exactly one, to `main`.
    System,
    Console,
    Clock,
    /// A directory, and everything under it, and nothing else.
    ///
    /// The path is always canonical, because every way of getting one of these
    /// canonicalizes. A `Dir` that held a path with a `..` still in it would be
    /// a `Dir` whose reach depended on when you looked.
    Dir(Rc<Path>),
}

impl Capability {
    pub fn name(&self) -> &'static str {
        match self {
            Capability::System => "System",
            Capability::Console => "Console",
            Capability::Clock => "Clock",
            Capability::Dir(_) => "Dir",
        }
    }
}

/// Field values, ordered by name so that two records built in different orders
/// compare equal and print the same.
pub type Fields = BTreeMap<String, Value>;

/// A value of a `choice`.
///
/// Identity is the module that declared the variant together with its name,
/// not a `DefId`. A `DefId` is an index into one module's resolution table, so
/// the same variant reached through an import would compare unequal to itself,
/// and structural equality is what `unchanged(E)` and every `assert` are built
/// on.
#[derive(PartialEq, Eq, Debug)]
pub struct VariantValue {
    pub origin: Rc<str>,
    pub name: String,
    pub fields: Fields,
}

impl Value {
    pub fn record(fields: Fields) -> Self {
        Value::Record(Rc::new(fields))
    }

    pub fn variant(origin: Rc<str>, name: impl Into<String>, fields: Fields) -> Self {
        Value::Variant(Rc::new(VariantValue {
            origin,
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

    pub fn str(text: impl AsRef<str>) -> Self {
        Value::Str(Rc::from(text.as_ref()))
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
            Value::Capability(_) => "a capability",
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
            Value::Capability(capability) => write!(f, "<{}>", capability.name()),
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
    use std::rc::Rc;

    use super::{Fields, Value};

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
        let empty = Value::variant(Rc::from("m"), "A", Fields::new());
        let mut fields = Fields::new();
        fields.insert("n".into(), Value::Int(1));
        assert_ne!(empty, Value::variant(Rc::from("m"), "A", fields));
    }

    #[test]
    fn a_variant_is_the_same_variant_however_it_was_reached() {
        // Two modules can each declare a `Loud`, and one module can reach
        // another's through an import. Only the second pair is one variant.
        let here = Value::variant(Rc::from("one"), "Loud", Fields::new());
        let elsewhere = Value::variant(Rc::from("two"), "Loud", Fields::new());
        assert_ne!(here, elsewhere);

        let reached_again = Value::variant(Rc::from("one"), "Loud", Fields::new());
        assert_eq!(here, reached_again);
    }

    #[test]
    fn display_is_readable() {
        let mut fields = Fields::new();
        fields.insert("units".into(), Value::Int(40));
        assert_eq!(Value::record(fields).to_string(), "{ units: 40 }");
        assert_eq!(
            Value::variant(Rc::from("m"), "Bare", Fields::new()).to_string(),
            "Bare"
        );
        assert_eq!(Value::Bool(true).to_string(), "true");
    }
}
