//! Gives every Deed expression a type.
//!
//! ```
//! use deed_diagnostics::SourceMap;
//! use deed_lexer::tokenize;
//! use deed_parser::parse;
//! use deed_resolve::{Universe, resolve};
//! use deed_typeck::{World, check};
//!
//! let source = "module demo\n\nfn twice(n: Int) -> Int { n + n }\n";
//! let mut sources = SourceMap::new();
//! let file = sources.add("demo.deed", source);
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

pub use check::{Checked, check, io_signatures, is_capability};
pub use facts::{Facts, Range, Truth};
pub use surface::{PRELUDE_MODULE, Surface, SurfaceItem, SurfaceVariant, World, surface};
pub use ty::{FieldTy, FnRow, Nominal, Obligation, Tier, Ty, Types, VariantTy};
