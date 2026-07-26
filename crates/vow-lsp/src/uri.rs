//! From a `file://` URI to a path on this machine.
//!
//! An editor names documents with URIs and the filesystem does not, so
//! something has to convert, and the conversion is full of things that look
//! like they do not matter. A space in a directory name arrives as `%20`. A
//! Windows drive letter arrives as `/c%3A/...`, with a leading slash that is
//! not part of the path and a colon that has been escaped. A path with a
//! Turkish letter in it arrives as several escapes that are bytes of UTF-8
//! rather than characters.
//!
//! Every one of those fails the same quiet way: the server looks for a file
//! that is not there, finds nothing, and reports errors about a program that
//! is fine. So anything this does not understand is refused rather than
//! guessed at, and the caller falls back to the one document it was handed.
//!
//! There is no path to URI direction, because nothing needs one. Every URI
//! this server sends back is a URI an editor sent it first.

use std::path::PathBuf;

/// The path a `file://` URI names, if it names one on this machine.
///
/// `None` for anything else, which includes the URIs an editor invents for
/// unsaved buffers and for documents that live inside an archive or on a
/// remote.
pub fn to_path(uri: &str) -> Option<PathBuf> {
    // `file:///a` has an empty authority. `file://host/a` names another
    // machine, and there is nothing on this one to read.
    let path = uri.strip_prefix("file:///")?;
    let decoded = percent_decode(path)?;

    // `/C:/work` is how a Windows path arrives. The leading slash belongs to
    // the URI rather than to the path, and leaving it on produces something
    // that looks absolute and resolves nowhere.
    if drive_letter(&decoded) {
        return Some(PathBuf::from(decoded));
    }
    Some(PathBuf::from(format!("/{decoded}")))
}

/// Whether a decoded path starts with something like `C:`.
fn drive_letter(path: &str) -> bool {
    let mut chars = path.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic()) && chars.next() == Some(':')
}

/// Decodes `%XX` escapes as bytes, then reads the result as UTF-8.
///
/// Bytes rather than characters, because that is what percent encoding is. A
/// decoder that took each escape for a character would turn every non-ASCII
/// path into something else and then fail to find it.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] != b'%' {
            out.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 2 >= bytes.len() {
            return None;
        }
        let high = (bytes[index + 1] as char).to_digit(16)?;
        let low = (bytes[index + 2] as char).to_digit(16)?;
        out.push((high * 16 + low) as u8);
        index += 3;
    }

    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::to_path;
    use std::path::PathBuf;

    #[test]
    fn a_plain_path_arrives_as_itself() {
        assert_eq!(
            to_path("file:///work/vow/examples/hello.vow"),
            Some(PathBuf::from("/work/vow/examples/hello.vow"))
        );
    }

    #[test]
    fn a_windows_drive_keeps_its_colon_and_loses_the_leading_slash() {
        // The leading slash belongs to the URI. Leaving it on produces
        // something that looks absolute and resolves nowhere, which shows up
        // as errors about a program that is fine.
        assert_eq!(
            to_path("file:///c%3A/work/a.vow"),
            Some(PathBuf::from("c:/work/a.vow"))
        );
        assert_eq!(
            to_path("file:///C:/work/a.vow"),
            Some(PathBuf::from("C:/work/a.vow"))
        );
    }

    #[test]
    fn escapes_are_bytes_rather_than_characters() {
        // `ü` is two bytes and arrives as two escapes. A decoder that took
        // each escape for a character would produce two wrong ones and then
        // look for a file nobody has.
        assert_eq!(
            to_path("file:///work/g%C3%BCn/a.vow"),
            Some(PathBuf::from("/work/gün/a.vow"))
        );
        assert_eq!(
            to_path("file:///work/my%20files/a.vow"),
            Some(PathBuf::from("/work/my files/a.vow"))
        );
    }

    #[test]
    fn anything_that_is_not_a_local_file_is_refused() {
        // Refusing is the point. An editor invents URIs for unsaved buffers
        // and for documents on a remote, and guessing at a path for one of
        // those means reporting on a file that is not the one on the screen.
        for uri in [
            "untitled:Untitled-1",
            "http://example.com/a.vow",
            "file:",
            "file://",
            "file://host/a.vow",
            "vscode-vfs://github/onatozmenn/vow/a.vow",
        ] {
            assert_eq!(to_path(uri), None, "should have refused {uri}");
        }
    }

    #[test]
    fn a_broken_escape_is_refused_rather_than_guessed_at() {
        for uri in ["file:///a%.vow", "file:///a%zz.vow", "file:///a%2"] {
            assert_eq!(to_path(uri), None, "should have refused {uri}");
        }
    }
}
