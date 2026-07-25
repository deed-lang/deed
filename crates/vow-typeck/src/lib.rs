//! Gives every Vow expression a type.
//!
//! ```
//! use vow_diagnostics::SourceMap;
//! use vow_lexer::tokenize;
//! use vow_parser::parse;
//! use vow_resolve::{Universe, resolve};
//! use vow_typeck::{World, check};
//!
//! let source = "module demo\n\nfn twice(n: Int) -> Int { n + n }\n";
//! let mut sources = SourceMap::new();
//! let file = sources.add("demo.vow", source);
//!
//! let lexed = tokenize(file, sources.file(file).text());
//! let parsed = parse(file, &lexed.tokens);
//! let resolved = resolve(file, &parsed.module, &Universe::new());
//! let checked = check(file, &parsed.module, &resolved.resolutions, &World::new());
//!
//! assert!(!checked.has_errors());
//! ```

pub mod check;
pub mod codes;
pub mod facts;
pub mod surface;
pub mod ty;

pub use check::{Checked, check};
pub use facts::{Facts, Range, Truth};
pub use surface::{PRELUDE_MODULE, Surface, SurfaceItem, SurfaceVariant, World, surface};
pub use ty::{FieldTy, Nominal, Obligation, Tier, Ty, Types, VariantTy};
