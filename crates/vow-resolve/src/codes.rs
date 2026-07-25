//! Diagnostic codes produced by name resolution.
//!
//! The resolver owns the `VOW3xxx` range. Codes are stable and never reused.

/// A name that does not refer to anything in scope.
pub const UNKNOWN_NAME: &str = "VOW3001";

/// Two declarations of the same name in one module.
pub const DUPLICATE_DEFINITION: &str = "VOW3002";

/// An imported name that is never used.
pub const UNUSED_IMPORT: &str = "VOW3003";

/// A binding that hides another binding.
///
/// This is an error rather than a warning, and that is a real decision. See
/// `design/02-syntax.md`.
pub const SHADOWED_BINDING: &str = "VOW3004";

/// A binding that hides a declaration from the module or the prelude.
pub const SHADOWED_DECLARATION: &str = "VOW3005";

/// A qualified name whose container has no such member.
pub const UNKNOWN_MEMBER: &str = "VOW3006";

/// A `use` of a module that is not among the files being compiled.
pub const UNKNOWN_MODULE: &str = "VOW3007";

/// A `use` of a name the module does not declare.
pub const UNKNOWN_EXPORT: &str = "VOW3008";
