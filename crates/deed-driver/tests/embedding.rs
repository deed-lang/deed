//! The numbers an embedder is given are the numbers the compiler uses.
//!
//! `how-to/embed-a-compiled-program.md` is the one document somebody outside
//! this repository has to trust: a host reads and writes the module's memory
//! directly, so a stale sentence there is not a confusing page, it is a host
//! that reads eight bytes from the wrong place and hands the program a value
//! it never wrote.
//!
//! Every other document in this repository can be checked by reading it
//! beside the code. This one is checked by asking the code, because the
//! reader who needs it most is the one who cannot.

use std::path::PathBuf;

use deed_codegen::layout;

fn page() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../how-to/embed-a-compiled-program.md");
    std::fs::read_to_string(&path).unwrap_or_else(|why| {
        panic!("the embedding guide should be at {}: {why}", path.display())
    })
}

/// The three columns of one row of the layout table, by its first column.
fn row(what: &str) -> (String, String) {
    let page = page();
    let line = page
        .lines()
        .find(|line| line.starts_with(&format!("| {what} |")))
        .unwrap_or_else(|| panic!("the layout table should have a row for {what}"));
    let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
    assert_eq!(
        cells.len(),
        3,
        "a row of the layout table is what it is, where it is, and how big: {line}"
    );
    (cells[1].to_string(), cells[2].to_string())
}

/// The table is not decoration: a host that reads the bump pointer from the
/// wrong address allocates over whatever is there.
#[test]
fn the_addresses_the_guide_gives_are_the_addresses_the_compiler_uses() {
    let (where_, size) = row("the bump pointer");
    assert_eq!(where_, layout::BUMP.to_string());
    assert_eq!(size, layout::WORD.to_string());

    let (where_, _) = row("the code of a failed contract");
    assert_eq!(where_, layout::FAILURE_CODE.to_string());

    let (where_, _) = row("the message of a failed contract");
    assert_eq!(where_, layout::FAILURE_MESSAGE.to_string());

    let (_, size) = row("a word");
    assert_eq!(size, layout::WORD.to_string());
}

/// A string's header is two words and the guide says so in bytes. Getting
/// this wrong reads the byte count where the character count is, which is a
/// host that truncates every string with an accent in it.
#[test]
fn the_string_header_the_guide_describes_is_the_one_the_compiler_writes() {
    let (shape, size) = row("a string");
    assert_eq!(shape, "`[characters][bytes][the bytes]`");

    let header = layout::string_size(0);
    assert_eq!(
        size,
        format!("{header} + bytes rounded up to 8"),
        "an empty string is its header, and that is what the first number is"
    );
    assert_eq!(
        layout::string_size(1),
        header + layout::WORD,
        "one byte still leaves the next allocation on a word"
    );
    assert_eq!(layout::string_size(8), header + layout::WORD);
    assert_eq!(layout::string_size(9), header + 2 * layout::WORD);
}

#[test]
fn the_list_header_the_guide_describes_is_the_one_the_compiler_writes() {
    let (shape, size) = row("a list");
    assert_eq!(shape, "`[length][element 0]...`");

    let header = layout::list_size(0);
    assert_eq!(size, format!("{header} + {} per element", layout::WORD));
    assert_eq!(layout::list_size(3), header + 3 * layout::WORD);
    assert_eq!(layout::element_offset(0), header);
}

/// The half a host gets wrong by assuming every aggregate has a tag. A record
/// has nothing to tell apart and does not carry one, so its first field sits
/// where a choice's tag would.
#[test]
fn a_record_has_no_tag_and_a_choice_does_on_the_page_as_well_as_in_the_code() {
    let (shape, size) = row("a record");
    assert_eq!(shape, "`[field 0]...`");
    assert_eq!(size, format!("{} per field", layout::WORD));
    assert_eq!(layout::aggregate_size(false, 2), 2 * layout::WORD);
    assert_eq!(layout::field_offset(false, 0), 0);

    let (shape, size) = row("a choice");
    assert_eq!(shape, "`[tag][field 0]...`");
    assert_eq!(
        size,
        format!("{} + {} per field", layout::WORD, layout::WORD)
    );
    assert_eq!(layout::aggregate_size(true, 2), 3 * layout::WORD);
    assert_eq!(layout::field_offset(true, 0), layout::WORD);
}

/// Which tag is which, in the one type a host has to build to answer
/// anything that can fail. Inverted, this is a host that reports every
/// success as a failure and the reader has no way to tell.
#[test]
fn the_guide_names_the_result_tags_the_compiler_gives_them() {
    let page = page();
    assert!(
        page.contains("`ok` is tag 0 and `err` is tag 1"),
        "the guide should say which tag is which"
    );
    assert_eq!(deed_mir::result_variant("ok"), Some(0));
    assert_eq!(deed_mir::result_variant("err"), Some(1));
}

/// The guide tells a host to read the memory by name. If the compiler stops
/// exporting it under that name, every host written from this page breaks at
/// the first string.
#[test]
fn the_name_the_guide_reads_the_memory_under_is_the_one_that_is_exported() {
    let page = page();
    assert!(
        page.contains("exports its memory as `memory`"),
        "the guide should name the export a host reads"
    );

    let mut sources = deed_diagnostics::SourceMap::new();
    let id = sources.add(
        "embedding.deed".to_string(),
        "module a\n\nfn answer() -> Int { 2 + 2 }\n".to_string(),
    );
    let mut all = deed_driver::check_all(&sources, &[id]);
    let one = all.pop().expect("one file in, one result out");
    let lowered =
        deed_mir::lower(&one.module, &one.resolutions, &one.types).expect("this lowers");
    let module = deed_codegen::compile(&lowered).expect("this compiles");

    assert_eq!(module.exported_memory.as_deref(), Some("memory"));
}
