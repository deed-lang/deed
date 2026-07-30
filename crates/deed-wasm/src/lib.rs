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

use deed_diagnostics::SourceMap;
use deed_driver::{check_all, json_report, shipped_for, shipped_source};

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

/// Checks one file's worth of Deed source, resolving `use std/x` against the
/// shipped library the way the CLI and the language server already do
/// (#589), and returns the same JSON `deed check --format json` writes
/// (#587).
///
/// A single file rather than a workspace: a page has one editor and no
/// filesystem to resolve a second file's import against, so the only import
/// this can answer is one the compiler already carries.
pub fn check_source(source: &str) -> String {
    let mut sources = SourceMap::new();
    let main = sources.add("main.deed".to_string(), source.to_string());
    let mut ids = vec![main];

    for module in shipped_for([source]) {
        let text = shipped_source(module).expect("a module that ships has a source");
        ids.push(sources.add(format!("{module}.deed"), text.to_string()));
    }

    // Only the file the page wrote is a subject; the shipped modules behind
    // it are context, the same distinction `deed check` draws between a named
    // file and what it imports.
    let checks = check_all(&sources, &ids);
    json_report(&sources, &checks[..1], true)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Calls the exports the way a page would: allocate, write, call, read
    /// the result back through the two accessors, then free both buffers.
    fn call_check(source: &str) -> String {
        let input = source.as_bytes();
        let in_ptr = deed_alloc(input.len());
        // SAFETY: `in_ptr` was allocated above with `input.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(input.as_ptr(), in_ptr, input.len()) };

        // SAFETY: `in_ptr`/`input.len()` name the buffer just filled in.
        unsafe { deed_check(in_ptr, input.len()) };

        let out_ptr = deed_result_ptr();
        let out_len = deed_result_len();
        // SAFETY: `out_ptr`/`out_len` are exactly what `deed_check` just set,
        // and this module never hands out a pointer it does not also own.
        let text = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
        let result = String::from_utf8(text.to_vec()).expect("deed_check should write UTF-8");

        // SAFETY: `in_ptr`/`input.len()` and `out_ptr`/`out_len` are exactly
        // what was allocated above and what `deed_check` just set.
        unsafe {
            deed_free(in_ptr, input.len());
            deed_free(out_ptr, out_len);
        }
        result
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
}
