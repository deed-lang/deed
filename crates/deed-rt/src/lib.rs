//! What a compiled Deed program needs from whoever runs it.
//!
//! A WebAssembly module cannot open a file. It declares what it wants from
//! its host and the host decides whether to hand it over, which is the same
//! shape the language's capabilities already have and most of why WASM was
//! the right target. See `design/05-backend.md`.
//!
//! This crate is the host half. The interpreter is one host, an embedder
//! loading a `.wasm` is another, and the rules a `Dir` enforces have to be
//! the same for both or the guarantee is only about one of them. That is
//! what [`sandbox`] is doing here rather than inside `deed-interp`: the
//! interpreter uses it, and it is not the interpreter's.
//!
//! Nothing else has moved. List and string helpers are still emitted inline
//! by the backend, and putting them here would be duplication until
//! something outside a module calls them.

pub mod sandbox;

pub use sandbox::{Refused, resolve, resolve_new, root};
