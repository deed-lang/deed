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
//! [`reach`] is the same argument about the other resource. A `Dir` is worth
//! something because there is no way out of it, and a `Net` is worth the same
//! only if the set of hosts it reaches can shrink and never grow. Both live
//! here for the same reason: the rule has to be one rule, or the guarantee is
//! about one runtime rather than about the language.
//!
//! [`http`] is the interpreter's answer to a network capability, and it is
//! deliberately the smallest one that can be exercised. A compiled program
//! never reaches it: a component asks its host for `deed:io.fetch` and the
//! host answers, the same way it answers `deed:io.read`.
//!
//! [`hashing`] is here for a third version of the same argument. There are two
//! implementations of the hash walk and there have to be, because one reads a
//! `Value` and the other reads linear memory. What cannot be written twice is
//! the arithmetic: a hash is an `Int` a program can assert on, so two engines
//! computing it differently is two engines disagreeing about what a program
//! means. The constants live here and both read them.
//!
//! Nothing else has moved. List and string helpers are still emitted inline
//! by the backend, and putting them here would be duplication until
//! something outside a module calls them.

pub mod hashing;
pub mod http;
pub mod reach;
pub mod sandbox;

pub use hashing::Hash;
pub use http::{Response, request};
pub use reach::{Reach, Target};
pub use sandbox::{Refused, resolve, resolve_new, root};
