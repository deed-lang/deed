//! Compiles [`deed_mir`] to WebAssembly, and runs what it produced.
//!
//! See `design/05-backend.md`. WebAssembly rather than native object code
//! first, for two reasons written up there: a module runs inside `deed`
//! without asking the machine for a linker, and its import model says the
//! same thing about a module and its host that a capability says about a
//! function and its caller.
//!
//! Nothing here is a dependency. The encoder writes the binary format by
//! hand and [`run`] is a small runner over the instructions [`compile`]
//! emits, for the same reason `deed-lsp` writes its own JSON: this workspace
//! has no dependencies, and the part of each format a compiler needs is
//! small.

pub mod abi;
pub mod compile;
pub mod layout;
pub mod run;
mod runtime;
pub mod validate;
pub mod wasm;

pub use compile::{Unsupported, compile};
pub use run::{Host, LinkError, Linked, Outcome, Trap, Value, call, call_measured};
pub use validate::{Invalid, validate};
pub use wasm::Module;
