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
//! The other direction exists because a definition can be in a file the editor
//! has not opened, so the server has to name one it was never handed.

use std::path::{Path, PathBuf};

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

/// The `file://` URI for a path.
///
/// Used to name a file the editor has not opened, which is what a jump into
/// another module produces. Escaping is the conservative direction: anything
/// outside the unreserved set plus the two separators becomes an escape, and
/// an editor that receives more escaping than it would have written still
/// resolves it to the same file.
pub fn from_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
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
    use super::{from_path, to_path};
    use std::path::PathBuf;

    #[test]
    fn a_path_round_trips_through_a_uri() {
        // The direction that matters for a jump into another file: the URI the
        // server sends has to name the file it meant.
        for path in [
            "/work/deed/examples/hello.deed",
            "/work/gün/a.deed",
            "/work/my files/a.deed",
        ] {
            let path = PathBuf::from(path);
            assert_eq!(to_path(&from_path(&path)), Some(path.clone()), "{path:?}");
        }
    }

    #[test]
    fn a_windows_path_becomes_a_uri_with_forward_slashes() {
        assert_eq!(
            from_path(&PathBuf::from("C:\\work\\a.deed")),
            "file:///C:/work/a.deed"
        );
    }

    #[test]
    fn a_plain_path_arrives_as_itself() {
        assert_eq!(
            to_path("file:///work/deed/examples/hello.deed"),
            Some(PathBuf::from("/work/deed/examples/hello.deed"))
        );
    }

    #[test]
    fn a_windows_drive_keeps_its_colon_and_loses_the_leading_slash() {
        // The leading slash belongs to the URI. Leaving it on produces
        // something that looks absolute and resolves nowhere, which shows up
        // as errors about a program that is fine.
        assert_eq!(
            to_path("file:///c%3A/work/a.deed"),
            Some(PathBuf::from("c:/work/a.deed"))
        );
        assert_eq!(
            to_path("file:///C:/work/a.deed"),
            Some(PathBuf::from("C:/work/a.deed"))
        );
    }

    #[test]
    fn escapes_are_bytes_rather_than_characters() {
        // `ü` is two bytes and arrives as two escapes. A decoder that took
        // each escape for a character would produce two wrong ones and then
        // look for a file nobody has.
        assert_eq!(
            to_path("file:///work/g%C3%BCn/a.deed"),
            Some(PathBuf::from("/work/gün/a.deed"))
        );
        assert_eq!(
            to_path("file:///work/my%20files/a.deed"),
            Some(PathBuf::from("/work/my files/a.deed"))
        );
    }

    #[test]
    fn anything_that_is_not_a_local_file_is_refused() {
        // Refusing is the point. An editor invents URIs for unsaved buffers
        // and for documents on a remote, and guessing at a path for one of
        // those means reporting on a file that is not the one on the screen.
        for uri in [
            "untitled:Untitled-1",
            "http://example.com/a.deed",
            "file:",
            "file://",
            "file://host/a.deed",
            "vscode-vfs://github/deed-lang/deed/a.deed",
        ] {
            assert_eq!(to_path(uri), None, "should have refused {uri}");
        }
    }

    #[test]
    fn a_broken_escape_is_refused_rather_than_guessed_at() {
        for uri in ["file:///a%.deed", "file:///a%zz.deed", "file:///a%2"] {
            assert_eq!(to_path(uri), None, "should have refused {uri}");
        }
    }
}
