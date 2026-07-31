#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Non-UTF-8 bytes are not a valid source file. Skip them rather than
    // testing the UTF-8 decoder, which is not written here.
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut sources = deed_diagnostics::SourceMap::new();
    deed_driver::check_text(&mut sources, "<fuzz>", text);
});
