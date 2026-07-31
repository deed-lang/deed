//! Diagnostic codes produced by name resolution.
//!
//! The resolver owns the `DEED3xxx` range. Codes are stable and never reused.

/// A name that does not refer to anything in scope.
pub const UNKNOWN_NAME: &str = "DEED3001";

/// Two declarations of the same name in one module.
pub const DUPLICATE_DEFINITION: &str = "DEED3002";

/// An imported name that is never used.
pub const UNUSED_IMPORT: &str = "DEED3003";

/// A binding that hides another binding.
///
/// This is an error rather than a warning, and that is a real decision. See
/// `design/02-syntax.md`.
pub const SHADOWED_BINDING: &str = "DEED3004";

/// A binding that hides a declaration from the module or the prelude.
pub const SHADOWED_DECLARATION: &str = "DEED3005";

/// A qualified name whose container has no such member.
pub const UNKNOWN_MEMBER: &str = "DEED3006";

/// A `use` of a module that is not among the files being compiled.
pub const UNKNOWN_MODULE: &str = "DEED3007";

/// A `use` of a name the module does not declare.
pub const UNKNOWN_EXPORT: &str = "DEED3008";

/// A binding nobody reads.
///
/// A `let` binding, a pattern binder, a `for` binder or a parameter that no
/// expression mentions. Shadowing is not allowed, so a name is bound once and
/// means one thing for the whole function, which is what makes this decidable
/// by looking rather than by tracking. A leading underscore is how a program
/// says it meant to ignore something.
pub const UNUSED_BINDING: &str = "DEED3009";

/// An alternative in a match arm that would bind a name.
///
/// `Plus | Times` names two variants so that one body can serve both, and it
/// is cheap precisely because neither side binds. A language whose
/// alternatives can bind has to require that every one of them binds the same
/// names, so that the body finds what it reads whichever side matched, and
/// that rule is the entire cost of the feature. Refusing to bind at all is the
/// version with none of it.
///
/// Nothing is lost. A variant with fields can be matched by name alone, which
/// is what every arm that wanted this was already doing with the fields it
/// never read.
pub const BINDING_IN_AN_ALTERNATIVE: &str = "DEED3010";

/// A reference to a declaration marked `deprecated`.
///
/// The declaration still exists for now, so this is a warning, but the warning
/// carries the replacement and may carry a machine-applicable fix.
pub const DEPRECATED_DECLARATION: &str = "DEED3011";
