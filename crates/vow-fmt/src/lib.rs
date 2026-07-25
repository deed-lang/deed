//! The one canonical form of a Vow source file.
//!
//! There are no options. Not "no options yet", none. P4 says formatting is not
//! configurable, and the reason is the same reason the language exists: the
//! output is read as a diff, and two spellings of the same program mean every
//! diff carries noise a reviewer has to filter before they can see the change.
//!
//! Formatting reads the tree, not the token stream, so what comes out is a
//! function of what the program means. The one thing the tree does not carry is
//! comments, and a formatter that eats comments is not usable, so they come
//! along separately from the lexer and get put back by span.
//!
//! ```
//! use vow_diagnostics::SourceMap;
//!
//! let mut sources = SourceMap::new();
//! let file = sources.add("demo.vow", "module demo\nfn   f( n:Int )->Int{n+n}\n");
//! let formatted = vow_fmt::format(file, sources.file(file).text()).unwrap();
//! assert_eq!(formatted, "module demo\n\nfn f(n: Int) -> Int {\n    n + n\n}\n");
//! ```

mod printer;

use vow_diagnostics::{Diagnostic, FileId};
use vow_lexer::tokenize;
use vow_parser::parse;

/// Formats one file.
///
/// Fails on a file that does not parse, rather than doing its best. A
/// formatter that reshapes a broken file is guessing at what was meant, and
/// the guess lands in the working tree where the next reader assumes a machine
/// checked it.
pub fn format(file: FileId, source: &str) -> Result<String, Vec<Diagnostic>> {
    let lexed = tokenize(file, source);
    let parsed = parse(file, &lexed.tokens);

    let mut diagnostics = lexed.diagnostics;
    diagnostics.extend(parsed.diagnostics);
    if diagnostics.iter().any(Diagnostic::is_error) {
        return Err(diagnostics);
    }

    Ok(printer::print(source, &parsed.module, &lexed.trivia))
}

/// Whether a file is already in canonical form.
pub fn is_formatted(file: FileId, source: &str) -> Result<bool, Vec<Diagnostic>> {
    format(file, source).map(|formatted| formatted == source)
}
