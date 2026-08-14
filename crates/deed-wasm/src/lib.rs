//! The boundary a page calls across.
//!
//! No `wasm-bindgen`: it is a dependency, and it generates glue this crate
//! would rather write by hand, the same way `deed-codegen` writes WebAssembly
//! by hand. The interface is a small number of exported functions over linear
//! memory rather than one entry point with a mode flag, because `check`,
//! `test`, `run` and `fmt` answer different questions and a flag would be
//! deciding that on the caller's behalf.
//!
//! # The shape a page sees
//!
//! - [`deed_alloc`] / [`deed_free`]: the allocator pair a host needs to hand
//!   this module a string and to give back what this module hands out. There
//!   is no other way onto or off of this module's memory.
//! - [`deed_check`]: writes a program's source into a buffer from
//!   [`deed_alloc`], calls this with its pointer and length, then reads
//!   [`deed_result_ptr`] and [`deed_result_len`] to find UTF-8 JSON, one
//!   object a line, in the exact shape `deed check --format json` already
//!   writes (see `deed_driver::json_report`, #587). The caller reads it and
//!   then calls [`deed_free`] on it.
//! - [`deed_test`], [`deed_run`] and [`deed_fmt`]: the other three verbs,
//!   same calling shape, different question. [`deed_fmt`] is the one that
//!   can answer with the program rather than about it. [`deed_review`] takes
//!   two such buffers and compares what a reviewer has to trust.
//! - [`deed_tokens`] and [`deed_explain`]: not verbs. One says how the
//!   compiler's own lexer classified each byte range, so a page can colour
//!   Deed without a second grammar to keep in step. The other hands over
//!   every diagnostic code with its page, so a site hosts an error index
//!   rather than writing one.
//!
//! `usize` throughout rather than packing a pointer and a length into one
//! return value: a pointer is whatever width the target is, and a scheme that
//! assumes 32 bits to fit two of them in a 64 bit return is exactly the kind
//! of thing that is right on the real target and silently wrong on the one
//! this crate's own tests run on, which is 64 bit. Two exported reads cost a
//! call each and are correct on both.
//!
//! Nothing here decides what a page may do with a running program; that is
//! #590 and #591, both scoped separately from the entry point itself.

use std::alloc::{Layout, alloc, dealloc};
use std::cell::Cell;
use std::path::Path;

use deed_ast::Item;
use deed_diagnostics::{Diagnostic, SourceMap, json_string, render_json};
use deed_driver::review::review_sources as review_source_sets;
use deed_driver::{Checked, check_all, json_report, shipped_for, shipped_source};
use deed_fmt::format as format_source;
use deed_interp::{Program, PropertyConfig, run_main, run_properties, run_tests};
use deed_lexer::{TokenKind, TriviaKind};

thread_local! {
    /// Where the last `deed_*` verb's answer is, for [`deed_result_ptr`] and
    /// [`deed_result_len`] to read back.
    ///
    /// Thread local rather than a single global: this module's own tests call
    /// these exports from more than one thread at once, and a page never has
    /// more than the one, so this costs nothing there and saves the tests
    /// from a lock.
    static RESULT: Cell<(*mut u8, usize)> = const { Cell::new((std::ptr::null_mut(), 0)) };
}

/// Allocates `len` bytes the host may write into and must eventually pass to
/// [`deed_free`] with the same `len`.
///
/// # Safety
///
/// Called by the host, not by Deed code, so the usual Rust borrow rules do not
/// apply to it; the contract is entirely in this doc comment. `len` must be
/// the same length passed to the matching [`deed_free`], because the
/// allocator is only told a size at the moment it is asked to free one.
#[unsafe(no_mangle)]
pub extern "C" fn deed_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::NonNull::dangling().as_ptr();
    }
    // Byte alignment: what comes across this boundary is UTF-8 text, which
    // has no alignment requirement of its own.
    let layout = Layout::from_size_align(len, 1).expect("a page-sized length should be valid");
    // SAFETY: `layout` has a nonzero size whenever this returns a real
    // allocation (`len == 0` took the early return above), which is what
    // `alloc` requires.
    unsafe { alloc(layout) }
}

/// Frees a buffer [`deed_alloc`] returned, or that [`deed_result_ptr`] named.
///
/// # Safety
///
/// `ptr` must be a pointer this crate handed out, still valid, and `len` must
/// be the exact length that came with it: the allocator this module uses
/// keys nothing but the length on the pointer, so a wrong one frees the wrong
/// number of bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deed_free(ptr: *mut u8, len: usize) {
    if len == 0 {
        return;
    }
    let layout = Layout::from_size_align(len, 1).expect("a page-sized length should be valid");
    // SAFETY: the caller's contract above is exactly `dealloc`'s contract.
    unsafe { dealloc(ptr, layout) };
}

/// Where the answer to the most recent `deed_*` verb starts.
#[unsafe(no_mangle)]
pub extern "C" fn deed_result_ptr() -> *mut u8 {
    RESULT.get().0
}

/// How many bytes long the answer to the most recent `deed_*` verb is.
#[unsafe(no_mangle)]
pub extern "C" fn deed_result_len() -> usize {
    RESULT.get().1
}

fn set_result(text: String) {
    let len = text.len();
    let out = deed_alloc(len);
    if len > 0 {
        // SAFETY: `out` was just allocated with exactly `len` bytes by the
        // same allocator, and `text.as_ptr()` is valid for `len` bytes.
        unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), out, len) };
    }
    RESULT.set((out, len));
}

/// Which released compiler built this module, at
/// [`deed_result_ptr`]/[`deed_result_len`].
///
/// #594: the artifact carries its own version rather than trusting a
/// filename, because a filename is a copy away from being wrong and this
/// is not. A page pins a tag and compares this against it before trusting
/// anything else the module says.
#[unsafe(no_mangle)]
pub extern "C" fn deed_version() {
    set_result(env!("CARGO_PKG_VERSION").to_string());
}

/// Checks one file's worth of Deed source, resolving `use std/x` against the
/// shipped library the way the CLI and the language server already do
/// (#589), and returns the same JSON `deed check --format json` writes
/// (#587).
///
/// A single file rather than a workspace: a page has one editor and no
/// filesystem to resolve a second file's import against, so the only import
/// this can answer is one the compiler already carries.
pub fn check_source(source: &str) -> String {
    let (sources, checks) = checked_of(source);
    // Only the file the page wrote is a subject; the shipped modules behind
    // it are context, the same distinction `deed check` draws between a named
    // file and what it imports.
    json_report(&sources, &checks[..1], true)
}

/// What [`check_source`] found, as diagnostics rather than as JSON.
///
/// `deed_driver::fix::fix` wants to re-check text between rounds and reads
/// diagnostics, not a report. Rendering to JSON and parsing it back would be
/// the same answer with a round trip in the middle.
pub fn diagnostics_of(source: &str) -> Vec<Diagnostic> {
    let (_, mut checks) = checked_of(source);
    checks.swap_remove(0).diagnostics
}

/// Compares one module before and after a patch using the same receipt as
/// `deed review` and the MCP `deed_review` tool.
pub fn review_source(before: &str, after: &str) -> String {
    review_source_sets(&[before], &[after], None)
}

/// Parses and checks `source` plus whatever it imports from the shipped
/// library, the way [`check_source`] does, and hands back what the rest of
/// this crate's verbs need to run or test it.
///
/// The page's own file is always `checks[0]`.
fn checked_of(source: &str) -> (SourceMap, Vec<Checked>) {
    let mut sources = SourceMap::new();
    let main = sources.add("main.deed".to_string(), source.to_string());
    let mut ids = vec![main];

    for module in shipped_for([source]) {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("{module}.deed"), text.to_string()));
    }

    let checks = check_all(&sources, &ids);
    (sources, checks)
}

/// Every `Io` operation `main`'s row mentions that a page host does not
/// answer for, as a sentence rather than a code.
///
/// #591's decision: a page offers exactly `Io.write` (buffered, already
/// in-memory) and `Io.now` (a deterministic counter, never the real clock).
/// Nothing else, for now: `Io.epoch` has no fallible form on this target, so
/// the standard library call behind it traps rather than returning a
/// `Result`, and the directory operations have a real host answer (an
/// in-memory directory) that nobody has built yet. Refusing before running
/// turns both into one honest message instead of a trap or an OS error
/// nobody chose.
pub fn unsupported_capabilities(checked: &Checked) -> Vec<String> {
    let Some(io) = checked.resolutions.builtin("Io") else {
        return Vec::new();
    };
    let Some(main_span) = checked.module.items.iter().find_map(|item| match item {
        Item::Function(function) if function.sig.name.name == "main" => {
            Some(function.sig.name.span)
        }
        _ => None,
    }) else {
        return Vec::new();
    };

    let rows = checked.rows();
    let Some(row) = rows.get(&main_span) else {
        return Vec::new();
    };

    row.iter()
        .filter(|item| item.effect == io)
        .filter_map(|item| match item.operation.as_deref() {
            Some("write") | Some("now") => None,
            Some(operation) => Some(format!("this page does not offer `Io.{operation}` yet")),
            None => Some(
                "this page only offers `Io.write` and `Io.now`, not the whole `Io` effect yet"
                    .to_string(),
            ),
        })
        .collect()
}

/// Runs every `test` block in the page's file, and every property its
/// contracts generate.
///
/// No capability decision applies here: a `test` block takes no parameters,
/// so there is no way for one to hold a capability at all, the same reason
/// `deed test` never asks for a directory to run one in.
///
/// Both kinds, because `deed test` runs both and a surface that ran one of
/// them would be answering a narrower question under the same name. A
/// property is on its own line kind rather than mixed in with the written
/// tests: one of them is something a person wrote and the other is generated
/// from a contract, and nobody should have to work out where a failing test
/// they never wrote came from. The default configuration's seed is fixed and
/// reported, so this stays as reproducible here as it is from a terminal.
///
/// The summary line is always written, including when nothing ran. Silence
/// already means "well formed" on this surface, so letting it also mean
/// "nothing to run" would leave a reader unable to tell a clean pass from a
/// file with no tests in it.
pub fn test_source(source: &str) -> String {
    let (sources, checks) = checked_of(source);
    if let Some(refusal) = refuse_unchecked(&checks) {
        return refusal;
    }
    let subject = &checks[0];

    let mut program = Program::new();
    for checked in &checks {
        program.add(
            checked.file,
            &checked.module,
            &checked.resolutions,
            checked.guards(),
            checked.rows(),
            checked.operators(),
        );
    }

    let mut out = String::new();
    let (mut passed, mut failed) = (0usize, 0usize);
    for outcome in run_tests(&program, subject.file) {
        match outcome.failure {
            None => {
                passed += 1;
                out.push_str(&format!(
                    "{{\"kind\":\"test\",\"name\":{},\"passed\":true}}\n",
                    json_string(&outcome.name)
                ));
            }
            Some(failure) => {
                failed += 1;
                out.push_str(&format!(
                    "{{\"kind\":\"test\",\"name\":{},\"passed\":false,\"diagnostic\":{}}}\n",
                    json_string(&outcome.name),
                    render_json(&sources, &failure)
                ));
            }
        }
    }

    for outcome in run_properties(
        &program,
        subject.file,
        &subject.module,
        &subject.resolutions,
        PropertyConfig::default(),
    ) {
        let head = format!(
            "\"kind\":\"property\",\"function\":{},\"cases\":{},\"seed\":\"{:#018x}\"",
            json_string(&outcome.function),
            outcome.cases,
            outcome.seed
        );
        match outcome.failure {
            None => {
                passed += 1;
                out.push_str(&format!("{{{head},\"passed\":true}}\n"));
            }
            Some(failure) => {
                failed += 1;
                out.push_str(&format!(
                    "{{{head},\"passed\":false,\"diagnostic\":{}}}\n",
                    render_json(&sources, &failure)
                ));
            }
        }
    }

    out.push_str(&format!(
        "{{\"kind\":\"summary\",\"passed\":{passed},\"failed\":{failed}}}\n"
    ));
    out
}

/// One line saying the file did not check, when it did not.
///
/// One line rather than the diagnostics themselves: [`check_source`] is the
/// verb that answers what is wrong, and repeating its answer here would give a
/// reader two places to look for one list. The count is in the line so that
/// "go and check" is a claim the caller can see the size of.
fn refuse_unchecked(checks: &[Checked]) -> Option<String> {
    let errors: usize = checks.iter().map(Checked::error_count).sum();
    if errors == 0 {
        return None;
    }
    Some(format!(
        "{{\"kind\":\"refused\",\"errors\":{errors},\"message\":{}}}\n",
        json_string(deed_driver::DOES_NOT_CHECK)
    ))
}

/// Runs the page's `main`, refusing first if its row asks for a capability
/// #591 decided a page does not offer.
///
/// `root` is a placeholder path rather than a real directory: there is no
/// filesystem under `wasm32-unknown-unknown` to root one in, and a program
/// whose row could reach it was already refused above.
pub fn run_source(source: &str) -> String {
    let (sources, checks) = checked_of(source);
    if let Some(refusal) = refuse_unchecked(&checks) {
        return refusal;
    }
    let subject = &checks[0];

    let refused = unsupported_capabilities(subject);
    if !refused.is_empty() {
        let mut out = String::new();
        for message in refused {
            out.push_str(&format!(
                "{{\"kind\":\"capability\",\"message\":{}}}\n",
                json_string(&message)
            ));
        }
        return out;
    }

    let mut program = Program::new();
    for checked in &checks {
        program.add(
            checked.file,
            &checked.module,
            &checked.resolutions,
            checked.guards(),
            checked.rows(),
            checked.operators(),
        );
    }

    let Some(run) = run_main(&program, subject.file, Path::new("/sandbox"), &[]) else {
        return format!(
            "{{\"kind\":\"result\",\"ok\":false,\"message\":{}}}\n",
            json_string(deed_driver::NOTHING_TO_RUN)
        );
    };

    let mut out = String::new();
    for line in &run.output {
        out.push_str(&format!(
            "{{\"kind\":\"output\",\"line\":{}}}\n",
            json_string(line)
        ));
    }
    match run.result {
        Ok(_) => out.push_str("{\"kind\":\"result\",\"ok\":true}\n"),
        Err(failure) => out.push_str(&format!(
            "{{\"kind\":\"result\",\"ok\":false,\"diagnostic\":{}}}\n",
            render_json(&sources, &failure)
        )),
    }
    out
}

/// Formats the page's file, or says why it cannot.
///
/// One `{"kind":"formatted","text":...}` line when the file parses, and the
/// same `{"kind":"diagnostic",...}` lines [`check_source`] writes when it
/// does not. Refusing rather than reshaping is `deed fmt`'s own rule: a file
/// that does not parse has no layout to choose, and picking one would be
/// guessing at what was meant.
///
/// Only the page's file, with no shipped modules behind it. Formatting reads
/// one file's own text, so an import would be a file this cannot rewrite
/// anyway.
pub fn fmt_source(source: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add("main.deed".to_string(), source.to_string());

    match format_source(file, source) {
        Ok(text) => format!(
            "{{\"kind\":\"formatted\",\"text\":{}}}\n",
            json_string(&text)
        ),
        Err(diagnostics) => diagnostics
            .iter()
            .map(|diagnostic| {
                format!(
                    "{{\"kind\":\"diagnostic\",\"diagnostic\":{}}}\n",
                    render_json(&sources, diagnostic)
                )
            })
            .collect(),
    }
}

/// How the compiler's own lexer classified each byte range, as JSON, one
/// object a line: `{"class":...,"start":N,"end":M}`.
///
/// A page that colours Deed has two choices: reimplement the grammar, or ask
/// the thing that already has one. The second is the only one that cannot
/// drift, and it is why this returns classes rather than a rendering: what a
/// keyword looks like is the page's business, what is a keyword is not.
///
/// The classes are what a lexer knows and no more. There is no `type` class,
/// because at this stage `Console` and `console` are both an identifier, and
/// a highlighter that guessed from the capital letter would be claiming
/// something the compiler has not concluded yet.
///
/// Comments come from the same pass, as `comment`, so the ranges together
/// cover everything but whitespace. Ranges are byte offsets into the source
/// that was passed in, and they never overlap.
pub fn token_classes(source: &str) -> String {
    let mut sources = SourceMap::new();
    let file = sources.add("main.deed".to_string(), source.to_string());
    let lexed = deed_lexer::tokenize(file, source);

    let mut spans: Vec<(u32, u32, &'static str)> = lexed
        .tokens
        .iter()
        .filter_map(|token| {
            let class = match &token.kind {
                TokenKind::Keyword(_) => "keyword",
                TokenKind::Ident(_) => "name",
                TokenKind::Int(_) | TokenKind::IntAtLimit => "number",
                TokenKind::Str(_) => "string",
                TokenKind::Error => "error",
                TokenKind::Eof => return None,
                _ => "punctuation",
            };
            Some((token.span.start, token.span.end, class))
        })
        .collect();

    spans.extend(lexed.trivia.iter().map(|trivia| {
        let class = match trivia.kind {
            TriviaKind::Line | TriviaKind::Block => "comment",
        };
        (trivia.span.start, trivia.span.end, class)
    }));

    spans.sort_by_key(|(start, _, _)| *start);
    spans
        .iter()
        .map(|(start, end, class)| {
            format!("{{\"class\":\"{class}\",\"start\":{start},\"end\":{end}}}\n")
        })
        .collect()
}

/// Every diagnostic code with its page, as JSON, one object a line.
///
/// The same pages `deed explain` prints, generated in `deed-explain` from the
/// doc comments above each code and an example taken out of the test that
/// already had to exist for it. A site that wants an error index therefore
/// hosts one rather than writing one, and it cannot document a code this
/// compiler does not have or miss one it does.
pub fn explain_all() -> String {
    deed_explain::all_pages()
        .iter()
        .map(|page| {
            let example = match page.example {
                Some(text) => json_string(text),
                None => "null".to_string(),
            };
            let from = match page.example_source {
                Some(text) => json_string(text),
                None => "null".to_string(),
            };
            format!(
                "{{\"code\":{},\"name\":{},\"text\":{},\"example\":{},\"example_source\":{}}}\n",
                json_string(page.code),
                json_string(page.name),
                json_string(page.text),
                example,
                from,
            )
        })
        .collect()
}

/// The `check` entry point: reads `len` UTF-8 bytes starting at `ptr` as a
/// Deed program, checks it, and leaves the JSON where [`deed_result_ptr`]
/// and [`deed_result_len`] find it.
///
/// # Safety
///
/// `ptr`/`len` must describe a buffer this module itself allocated (through
/// [`deed_alloc`]) and the host has finished writing into.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deed_check(ptr: *const u8, len: usize) {
    // SAFETY: forwarded from this function's own contract.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let source = String::from_utf8_lossy(bytes);
    set_result(check_source(&source));
}

/// The `review` entry point: reads two separately allocated UTF-8 source
/// buffers and leaves one review receipt, or the diagnostics and refusal for
/// a side that did not check.
///
/// # Safety
///
/// Each pointer/length pair must describe its own buffer allocated through
/// [`deed_alloc`], and the host must have finished writing both buffers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deed_review(
    before_ptr: *const u8,
    before_len: usize,
    after_ptr: *const u8,
    after_len: usize,
) {
    // SAFETY: forwarded from this function's own contract.
    let before = unsafe { std::slice::from_raw_parts(before_ptr, before_len) };
    // SAFETY: forwarded from this function's own contract.
    let after = unsafe { std::slice::from_raw_parts(after_ptr, after_len) };
    let before = String::from_utf8_lossy(before);
    let after = String::from_utf8_lossy(after);
    set_result(review_source(&before, &after));
}

/// The `test` entry point: same input shape as [`deed_check`], and leaves
/// one JSON object a line at [`deed_result_ptr`]/[`deed_result_len`], one per
/// `test` block: `{"kind":"test","name":...,"passed":bool}`, then one
/// `{"kind":"property","function":...,"cases":N,"seed":...,"passed":bool}`
/// per property a contract generates, with a `"diagnostic"` key added to
/// either when it failed, and then one
/// `{"kind":"summary","passed":N,"failed":M}` line. A file that does not
/// check answers with one `{"kind":"refused",...}` line instead, and nothing
/// runs.
///
/// # Safety
///
/// Same contract as [`deed_check`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deed_test(ptr: *const u8, len: usize) {
    // SAFETY: forwarded from this function's own contract.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let source = String::from_utf8_lossy(bytes);
    set_result(test_source(&source));
}

/// The `run` entry point: same input shape as [`deed_check`], and leaves
/// JSON at [`deed_result_ptr`]/[`deed_result_len`]: zero or more
/// `{"kind":"output",...}` lines, then one `{"kind":"result",...}` line, or
/// one `{"kind":"capability",...}` line per capability #591 does not offer
/// when `main`'s row asks for one, in which case nothing runs. A file that
/// does not check answers with one `{"kind":"refused",...}` line, also
/// without running.
///
/// # Safety
///
/// Same contract as [`deed_check`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deed_run(ptr: *const u8, len: usize) {
    // SAFETY: forwarded from this function's own contract.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let source = String::from_utf8_lossy(bytes);
    set_result(run_source(&source));
}

/// The `fmt` entry point: same input shape as [`deed_check`], and leaves
/// either one `{"kind":"formatted","text":...}` line or the
/// `{"kind":"diagnostic",...}` lines saying why there was nothing to format.
///
/// # Safety
///
/// Same contract as [`deed_check`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deed_fmt(ptr: *const u8, len: usize) {
    // SAFETY: forwarded from this function's own contract.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let source = String::from_utf8_lossy(bytes);
    set_result(fmt_source(&source));
}

/// The colouring entry point: same input shape as [`deed_check`], and leaves
/// one `{"class":...,"start":N,"end":M}` line per token and comment.
///
/// Not a verb. `check`, `test`, `run` and `fmt` answer questions about a
/// program; this answers one about its text, and a page needs it on every
/// keystroke rather than on a button.
///
/// # Safety
///
/// Same contract as [`deed_check`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn deed_tokens(ptr: *const u8, len: usize) {
    // SAFETY: forwarded from this function's own contract.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let source = String::from_utf8_lossy(bytes);
    set_result(token_classes(&source));
}

/// Every diagnostic code and its page, at
/// [`deed_result_ptr`]/[`deed_result_len`].
///
/// Takes no source, because it is not about a program: it is what this
/// compiler can say and why.
#[unsafe(no_mangle)]
pub extern "C" fn deed_explain() {
    set_result(explain_all());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Calls one of the exports the way a page would: allocate, write, call,
    /// read the result back through the two accessors, then free both
    /// buffers.
    fn call_export(verb: unsafe extern "C" fn(*const u8, usize), source: &str) -> String {
        let input = source.as_bytes();
        let in_ptr = deed_alloc(input.len());
        // SAFETY: `in_ptr` was allocated above with `input.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(input.as_ptr(), in_ptr, input.len()) };

        // SAFETY: `in_ptr`/`input.len()` name the buffer just filled in.
        unsafe { verb(in_ptr, input.len()) };

        let out_ptr = deed_result_ptr();
        let out_len = deed_result_len();
        // SAFETY: `out_ptr`/`out_len` are exactly what the verb just set, and
        // this module never hands out a pointer it does not also own.
        let text = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
        let result = String::from_utf8(text.to_vec()).expect("a verb should write UTF-8");

        // SAFETY: `in_ptr`/`input.len()` and `out_ptr`/`out_len` are exactly
        // what was allocated above and what the verb just set.
        unsafe {
            deed_free(in_ptr, input.len());
            deed_free(out_ptr, out_len);
        }
        result
    }

    fn call_review(before: &str, after: &str) -> String {
        let before = before.as_bytes();
        let after = after.as_bytes();
        let before_ptr = deed_alloc(before.len());
        let after_ptr = deed_alloc(after.len());
        unsafe {
            std::ptr::copy_nonoverlapping(before.as_ptr(), before_ptr, before.len());
            std::ptr::copy_nonoverlapping(after.as_ptr(), after_ptr, after.len());
            deed_review(before_ptr, before.len(), after_ptr, after.len());
        }

        let out_ptr = deed_result_ptr();
        let out_len = deed_result_len();
        let text = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
        let result = String::from_utf8(text.to_vec()).expect("review should write UTF-8");
        unsafe {
            deed_free(before_ptr, before.len());
            deed_free(after_ptr, after.len());
            deed_free(out_ptr, out_len);
        }
        result
    }

    fn call_check(source: &str) -> String {
        call_export(deed_check, source)
    }

    #[test]
    fn a_page_gets_the_same_review_receipt_as_the_other_surfaces() {
        let before = "module review/sample\n\neffect Store { fn write(value: Int) -> () }\n";
        let after = "module review/sample\n\neffect Store { fn write(value: Int) -> () }\n\nfn sync(value: Int) -> () uses Store.write, { Store.write(value) }\n";
        assert_eq!(
            call_review(before, after),
            "{\"kind\":\"review_receipt\",\"clean\":false,\"authority_added\":[{\"module\":\"review/sample\",\"declaration\":\"sync\",\"authority\":\"Store.write\"}],\"tier_regressions\":[],\"guarded_added\":[]}\n"
        );
    }

    /// The `(start, end, class)` triples `token_classes` wrote, in order.
    fn classes(source: &str) -> Vec<(usize, usize, String)> {
        call_export(deed_tokens, source)
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                (
                    json_number(line, "start"),
                    json_number(line, "end"),
                    json_field(line, "class"),
                )
            })
            .collect()
    }

    fn json_number(line: &str, key: &str) -> usize {
        let needle = format!("\"{key}\":");
        let start = line
            .find(&needle)
            .unwrap_or_else(|| panic!("{line:?} has no {key}"))
            + needle.len();
        line[start..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap_or_else(|_| panic!("{key} in {line:?} is not a number"))
    }

    /// Whatever a page does with the classes, it has to be able to lay the
    /// source out from them alone: covering everything but whitespace, in
    /// order, without overlapping.
    #[test]
    fn the_classes_tile_the_source_and_leave_only_whitespace() {
        let source = "module main\n\n// a comment\nfn main() -> Int {\n    \"hi\"\n    1\n}\n";
        let mut at = 0;

        for (start, end, class) in classes(source) {
            assert!(start >= at, "{class} at {start} overlaps what came before");
            assert!(
                source[at..start].trim().is_empty(),
                "nothing classified {:?}, which is not whitespace",
                &source[at..start]
            );
            at = end;
        }
        assert!(source[at..].trim().is_empty(), "the tail went unclassified");
    }

    #[test]
    fn each_class_is_the_one_the_lexer_decided() {
        let source = "// c\nmodule a\n\nfn f() -> Int {\n    \"s\" == 1\n}\n";
        let found: Vec<(String, String)> = classes(source)
            .into_iter()
            .map(|(start, end, class)| (class, source[start..end].to_string()))
            .collect();

        let of = |text: &str| {
            found
                .iter()
                .find(|(_, seen)| seen == text)
                .map(|(class, _)| class.clone())
                .unwrap_or_else(|| panic!("nothing classified {text:?}: {found:?}"))
        };

        assert_eq!(of("// c"), "comment");
        assert_eq!(of("module"), "keyword");
        assert_eq!(of("f"), "name");
        assert_eq!(of("\"s\""), "string");
        assert_eq!(of("1"), "number");
        assert_eq!(of("=="), "punctuation");
    }

    /// Two arms cargo-mutants could delete without a test noticing, and both
    /// are things a page can see: text the lexer could not read should look
    /// unread, and end-of-file is not a thing on screen.
    #[test]
    fn what_the_lexer_could_not_read_says_so_and_the_end_of_file_is_not_drawn() {
        let source = "module a\n\nfn f() -> Int {\n    1 @ 2\n}\n";
        let found = classes(source);

        assert!(
            found
                .iter()
                .any(|(start, end, class)| class == "error" && &source[*start..*end] == "@"),
            "a character the lexer could not read should be classed error: {found:?}"
        );
        assert!(
            found.iter().all(|(start, end, _)| start < end),
            "an empty range draws nothing and is a page's problem to skip: {found:?}"
        );
    }

    /// A highlighter that coloured `Console` differently would be claiming
    /// something no pass has concluded at this point: to the lexer a capital
    /// letter is a letter.
    #[test]
    fn a_capital_letter_is_not_a_type_yet() {
        let source = "module a\n\nfn f(out: Console, n: Int) -> Int {\n    n\n}\n";
        let found: Vec<(String, String)> = classes(source)
            .into_iter()
            .map(|(start, end, class)| (class, source[start..end].to_string()))
            .collect();

        for name in ["Console", "out", "Int", "n"] {
            let class = found
                .iter()
                .find(|(_, seen)| seen == name)
                .map(|(class, _)| class.as_str())
                .unwrap_or_else(|| panic!("nothing classified {name:?}"));
            assert_eq!(class, "name", "{name} should be a plain name");
        }
    }

    /// Most of the corpus is libraries, so this is the answer a page is most
    /// likely to show first, and it has to say why rather than only what.
    #[test]
    fn a_library_is_told_why_there_is_nothing_to_run() {
        let json = call_export(deed_run, "module a\n\nfn f() -> Int {\n    1\n}\n");
        assert_eq!(
            json_field(&json, "message"),
            deed_driver::NOTHING_TO_RUN,
            "the artifact and the CLI should give the same answer: {json}"
        );
    }

    #[test]
    fn every_code_this_compiler_has_arrives_with_its_page() {
        deed_explain();
        let out_ptr = deed_result_ptr();
        let out_len = deed_result_len();
        // SAFETY: exactly what `deed_explain` just set.
        let text = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
        let json = String::from_utf8(text.to_vec()).expect("the index is UTF-8");
        // SAFETY: the same buffer, freed once.
        unsafe { deed_free(out_ptr, out_len) };

        let lines: Vec<&str> = json.lines().filter(|line| !line.is_empty()).collect();
        assert_eq!(
            lines.len(),
            deed_explain::all_pages().len(),
            "one line per page, no more and no fewer"
        );

        for page in deed_explain::all_pages() {
            let line = lines
                .iter()
                .find(|line| json_field(line, "code") == page.code)
                .unwrap_or_else(|| panic!("{} is missing from the index", page.code));
            assert_eq!(json_field(line, "name"), page.name);
            assert!(
                !json_field(line, "text").is_empty(),
                "{} arrived with an empty page",
                page.code
            );
        }
    }

    #[test]
    fn the_version_export_matches_the_crate_that_built_it() {
        deed_version();
        let out_ptr = deed_result_ptr();
        let out_len = deed_result_len();
        // SAFETY: exactly what `deed_version` just set.
        let text = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
        let version = String::from_utf8(text.to_vec()).expect("a version is UTF-8");
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn a_clean_program_produces_no_diagnostic_lines() {
        let json = call_check("module main\n\nfn main() -> Int {\n    1 + 1\n}\n");
        assert_eq!(
            json, "",
            "a program with nothing to say should write nothing"
        );
    }

    #[test]
    fn a_broken_program_reports_a_diagnostic_the_same_way_the_cli_does() {
        let json = call_check("module main\n\nfn main() -> Int {\n    nonesuch\n}\n");
        let lines: Vec<&str> = json.lines().collect();
        assert_eq!(
            lines.len(),
            1,
            "one error should be one JSON line, got {json:?}"
        );
        assert!(
            lines[0].starts_with("{\"kind\":\"diagnostic\","),
            "a diagnostic line should say so first, got {:?}",
            lines[0]
        );
    }

    #[test]
    fn using_the_shipped_library_checks_cleanly_with_no_filesystem() {
        // #589: `use std/list` resolves against the embedded copy, not a
        // directory, so this needs nothing on disk to pass.
        let json = call_check(
            "module main\n\nuse std/list.{map}\n\nfn main() -> List<Int> {\n    map([1, 2, 3], |n: Int| { n + 1 })\n}\n",
        );
        assert_eq!(
            json, "",
            "a program that only uses the shipped library should check cleanly, got {json:?}"
        );
    }

    #[test]
    fn allocating_zero_bytes_does_not_crash_the_allocator() {
        let ptr = deed_alloc(0);
        // SAFETY: `ptr`/`0` are exactly what was just allocated.
        unsafe { deed_free(ptr, 0) };
    }

    #[test]
    fn check_source_agrees_with_the_exported_boundary() {
        // The safe function and the unsafe exports are one path, not two: a
        // divergence here would mean the boundary added or dropped something
        // on its way through raw memory.
        let source = "module main\n\nfn main() -> Int {\n    1\n}\n";
        assert_eq!(check_source(source), call_check(source));
    }

    #[test]
    fn a_passing_test_is_reported_as_passing() {
        let json = test_source("module main\n\ntest \"one is one\" {\n    assert 1 == 1\n}\n");
        assert_eq!(
            json,
            "{\"kind\":\"test\",\"name\":\"one is one\",\"passed\":true}\n\
             {\"kind\":\"summary\",\"passed\":1,\"failed\":0}\n"
        );
    }

    /// The verb next to this one uses silence to mean the program is well
    /// formed. Letting silence also mean nothing ran would leave a reader
    /// holding two opposite readings of the same empty answer.
    #[test]
    fn a_file_with_no_tests_says_so_rather_than_answering_with_nothing() {
        let json = test_source("module main\n\nfn f() -> Int {\n    1\n}\n");
        assert_eq!(json, "{\"kind\":\"summary\",\"passed\":0,\"failed\":0}\n");
    }

    #[test]
    fn a_failing_test_carries_its_diagnostic() {
        let json = test_source("module main\n\ntest \"never\" {\n    assert 1 == 2\n}\n");
        assert!(
            json.contains("\"passed\":false") && json.contains("\"diagnostic\":"),
            "a failing test should carry a diagnostic, got {json:?}"
        );
        assert!(
            json.contains("{\"kind\":\"summary\",\"passed\":0,\"failed\":1}"),
            "the summary should count it, got {json:?}"
        );
    }

    /// The bad answer this replaced: the program did not check, and the
    /// server said the test passed.
    #[test]
    fn a_file_that_does_not_check_is_refused_rather_than_tested() {
        let json = test_source(
            "module main\n\nfn f() -> Int {\n    nonesuch\n}\n\ntest \"t\" {\n    assert 1 == 1\n}\n",
        );
        assert_eq!(
            json,
            format!(
                "{{\"kind\":\"refused\",\"errors\":1,\"message\":{}}}\n",
                json_string(deed_driver::DOES_NOT_CHECK)
            )
        );
    }

    /// Running it produced DEED6006, whose own note offers the reader two
    /// explanations: the file was not checked, or the check has a hole. Only
    /// the first was ever true here, and nothing said which.
    #[test]
    fn a_file_that_does_not_check_is_refused_rather_than_run() {
        let json = run_source("module main\n\nfn main() -> Int {\n    nonesuch\n}\n");
        assert!(
            json.starts_with("{\"kind\":\"refused\","),
            "the refusal should come first and alone, got {json:?}"
        );
        assert!(
            !json.contains("DEED6006"),
            "nothing should have reached the interpreter, got {json:?}"
        );
    }

    /// A warning is not a refusal. The corpus produces them on purpose, and a
    /// guarded obligation is a thing to read rather than a thing to fix.
    #[test]
    fn a_warning_does_not_stop_a_program_from_running() {
        let json = run_source(
            "module main\n\ntype Positive = Int where value > 0\n\n\
             fn keep(n: Int) -> Positive {\n    n\n}\n\n\
             fn main() -> Positive {\n    keep(1)\n}\n",
        );
        assert_eq!(json, "{\"kind\":\"result\",\"ok\":true}\n");
    }

    /// The contract's own test, which nobody wrote and which is the one that
    /// finds this.
    ///
    /// `twice` looks right and passes the test written under it. `deed test`
    /// reports it as failing, because `n + n` overflows near the top of the
    /// range and `ensures` claims a result for every `n > 0`. This surface
    /// used to run only the written test and answer that the program was
    /// fine.
    #[test]
    fn a_property_a_contract_generates_is_run_and_can_disagree_with_the_written_test() {
        let json = test_source(
            "module main\n\n\
             fn twice(n: Int) -> Int\n  where\n    n > 0,\n  ensures\n    ok  => result > n,\n{\n    n + n\n}\n\n\
             test \"twice doubles\" {\n    assert twice(3) == 6\n}\n",
        );
        assert!(
            json.contains("\"kind\":\"test\",\"name\":\"twice doubles\",\"passed\":true"),
            "the written test still passes, which is the point: {json}"
        );
        assert!(
            json.contains("\"kind\":\"property\",\"function\":\"twice\"")
                && json.contains("\"passed\":false"),
            "the generated property should fail: {json}"
        );
        assert!(
            json.contains("{\"kind\":\"summary\",\"passed\":1,\"failed\":1}"),
            "and it should be counted: {json}"
        );
    }

    /// A property is reproducible or it is a rumour, and a reader who cannot
    /// see the seed cannot ask for the same run twice.
    #[test]
    fn a_property_reports_the_seed_it_ran_under() {
        let json = test_source(
            "module main\n\n\
             fn keep(n: Int) -> Int\n  ensures\n    ok  => result == n,\n{\n    n\n}\n",
        );
        assert!(
            json.contains("\"kind\":\"property\",\"function\":\"keep\"")
                && json.contains("\"passed\":true"),
            "a contract that holds should pass: {json}"
        );
        assert!(
            json.contains(&format!(
                "\"seed\":\"{:#018x}\"",
                PropertyConfig::default().seed
            )),
            "the seed should be on the line: {json}"
        );
        assert!(
            json.contains("{\"kind\":\"summary\",\"passed\":1,\"failed\":0}"),
            "a property that passed counts like anything else that ran: {json}"
        );
    }

    #[test]
    fn running_a_clean_program_reports_its_output_and_a_true_result() {
        let json = run_source("module main\n\nfn main() -> Int {\n    1 + 1\n}\n");
        assert_eq!(json, "{\"kind\":\"result\",\"ok\":true}\n");
    }

    #[test]
    fn a_program_that_can_never_stop_terminates_with_a_depth_error_rather_than_hanging() {
        // #590: there is no `while`, so the only way to not stop is
        // recursion, and the interpreter's own depth limit (`MAX_DEPTH`,
        // deed-interp) already turns that into an error rather than a hang.
        // A page is never left waiting on a call that cannot return.
        //
        // `main` declares `Diverge` too. It used to leave it off and the
        // artifact ran the program anyway, so this test was reaching the
        // depth limit through a file the checker had already rejected.
        let json = run_source(
            "module main\n\n\
             fn a() -> Int\n  uses\n    Diverge,\n{\n    b()\n}\n\n\
             fn b() -> Int\n  uses\n    Diverge,\n{\n    a()\n}\n\n\
             fn main() -> Int\n  uses\n    Diverge,\n{\n    a()\n}\n",
        );
        assert!(
            json.contains("\"kind\":\"result\",\"ok\":false"),
            "a call that can never return should fail rather than hang, got {json:?}"
        );
        assert!(
            json.contains("more than 128 deep") || json.contains("DEED6009"),
            "the failure should be the depth limit, not some other mistake, got {json:?}"
        );
    }

    #[test]
    fn a_program_using_only_write_and_now_is_not_refused() {
        let (_, checks) = checked_of(
            "module main\n\nfn main(sys: System) -> Int\n  uses\n    Io.write,\n    Io.now,\n{\n    Io.write(sys.console, \"hi\")\n    Io.now(sys.clock)\n}\n",
        );
        assert_eq!(unsupported_capabilities(&checks[0]), Vec::<String>::new());
    }

    #[test]
    fn a_program_asking_to_save_a_file_is_refused_with_a_message() {
        let (_, checks) = checked_of(
            "module main\n\nfn main(sys: System) -> Result<(), String>\n  uses\n    Io.save,\n{\n    Io.save(sys.files, \"x\", \"y\")\n}\n",
        );
        assert_eq!(
            unsupported_capabilities(&checks[0]),
            vec!["this page does not offer `Io.save` yet".to_string()]
        );
    }

    #[test]
    fn running_a_program_that_asks_to_save_a_file_is_refused_before_it_runs() {
        let json = run_source(
            "module main\n\nfn main(sys: System) -> Result<(), String>\n  uses\n    Io.save,\n{\n    Io.save(sys.files, \"x\", \"y\")\n}\n",
        );
        assert_eq!(
            json,
            "{\"kind\":\"capability\",\"message\":\"this page does not offer `Io.save` yet\"}\n"
        );
    }

    /// Reads `"<key>":"..."` out of one JSON line the way `JSON.parse` would,
    /// which is the only reader this boundary actually has.
    ///
    /// Comparing against a hand-built string would agree with whatever this
    /// module emits, including something no page could read.
    fn json_field(line: &str, key: &str) -> String {
        let needle = format!("\"{key}\":\"");
        let start = line
            .find(&needle)
            .unwrap_or_else(|| panic!("{line:?} has no {key}"))
            + needle.len();
        let mut out = String::new();
        let mut chars = line[start..].chars();
        while let Some(ch) = chars.next() {
            match ch {
                '"' => return out,
                '\\' => match chars.next().expect("an escape needs a character after it") {
                    '"' => out.push('"'),
                    '\\' => out.push('\\'),
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'u' => {
                        let hex: String = chars.by_ref().take(4).collect();
                        let code =
                            u32::from_str_radix(&hex, 16).expect("`\\u` takes four hex digits");
                        out.push(char::from_u32(code).expect("a Unicode scalar value"));
                    }
                    other => panic!("{line:?} contains the unknown escape `\\{other}`"),
                },
                c => out.push(c),
            }
        }
        panic!("{line:?} never closed its {key} string")
    }

    #[test]
    fn what_a_program_prints_survives_the_trip_through_json() {
        let json = run_source(
            "module main\n\nfn main(sys: System) -> Int\n  uses\n    Io.write,\n{\n    Io.write(sys.console, \"he said \\\"hi\\\" \\\\ then stopped\")\n    0\n}\n",
        );
        let printed: Vec<&str> = json
            .lines()
            .filter(|line| line.contains("\"kind\":\"output\""))
            .collect();
        assert_eq!(printed.len(), 1, "one write is one line, got {json:?}");
        assert_eq!(
            json_field(printed[0], "line"),
            "he said \"hi\" \\ then stopped"
        );
    }

    #[test]
    fn a_newline_a_program_prints_does_not_become_a_second_line() {
        let json = run_source(
            "module main\n\nfn main(sys: System) -> Int\n  uses\n    Io.write,\n{\n    Io.write(sys.console, \"over\\ntwo\")\n    0\n}\n",
        );
        // One `Io.write` and one result: a newline inside the text must not
        // add a line, because the caller splits this on newlines.
        assert_eq!(json.lines().count(), 2, "got {json:?}");
        assert_eq!(
            json_field(json.lines().next().unwrap(), "line"),
            "over\ntwo"
        );
    }

    #[test]
    fn a_formatted_program_survives_the_trip_through_json() {
        let messy = "module main\n\n\n\n\nfn main( ) -> Int {\n        1 + 1\n}\n";
        let json = fmt_source(messy);
        assert_eq!(json.lines().count(), 1, "one answer, got {json:?}");

        let text = json_field(json.lines().next().unwrap(), "text");
        // A whole program is the value here, so it carries the newlines that
        // would otherwise split this answer into lines nobody wrote.
        assert!(text.contains('\n'), "got {text:?}");
        assert_ne!(text, messy, "this input was not already canonical");

        // Formatting what came back changes nothing, which is the property
        // that says the text arrived intact rather than merely parsed.
        let again = fmt_source(&text);
        assert_eq!(json_field(again.lines().next().unwrap(), "text"), text);
    }

    #[test]
    fn a_file_that_does_not_parse_is_refused_rather_than_reshaped() {
        let json = fmt_source("module main\n\nfn main( -> Int {\n");
        assert!(
            !json.is_empty()
                && json
                    .lines()
                    .all(|line| line.contains("\"kind\":\"diagnostic\"")),
            "a file with no layout to choose should only say why, got {json:?}"
        );
    }

    #[test]
    fn fmt_source_agrees_with_the_exported_boundary() {
        let source = "module main\n\nfn main( ) -> Int {\n  1\n}\n";
        assert_eq!(fmt_source(source), call_export(deed_fmt, source));
    }
}
