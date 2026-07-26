//! The base protocol: `Content-Length`, a blank line, then the message.
//!
//! This is the whole of the framing the language server protocol defines, and
//! it is worth doing carefully because every failure mode is silent. A header
//! read as text rather than as bytes truncates the first message with a
//! multi-byte character in it, and the editor then waits forever for a reply
//! it will never get.

use std::io::{BufRead, Write};

use crate::json::{Json, parse};

/// Why a message could not be read.
#[derive(Debug)]
pub enum ReadError {
    /// The stream ended between messages, which is how an editor says goodbye
    /// when it did not get to send `exit`.
    Closed,
    Io(std::io::Error),
    /// The framing or the body was wrong. Not recoverable: the stream position
    /// is no longer known, so carrying on would be reading the middle of
    /// something as the start of something else.
    Malformed(String),
}

impl From<std::io::Error> for ReadError {
    fn from(error: std::io::Error) -> Self {
        ReadError::Io(error)
    }
}

/// Reads one message, headers and all.
pub fn read_message(input: &mut impl BufRead) -> Result<Json, ReadError> {
    let mut length: Option<usize> = None;

    loop {
        let mut line = String::new();
        // Headers are ASCII by definition, so reading them as text is safe.
        // The body is not, and is read as bytes below.
        if input.read_line(&mut line)? == 0 {
            return Err(ReadError::Closed);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(ReadError::Malformed(format!(
                "header without a `:`: {line}"
            )));
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            match value.trim().parse::<usize>() {
                Ok(parsed) => length = Some(parsed),
                Err(_) => {
                    return Err(ReadError::Malformed(format!(
                        "`Content-Length` is not a number: {}",
                        value.trim()
                    )));
                }
            }
        }
        // Everything else, `Content-Type` included, is ignored. There is one
        // encoding and it is UTF-8.
    }

    let Some(length) = length else {
        return Err(ReadError::Malformed("no `Content-Length`".to_string()));
    };

    let mut body = vec![0u8; length];
    input.read_exact(&mut body)?;
    let text = String::from_utf8(body)
        .map_err(|_| ReadError::Malformed("the body was not UTF-8".to_string()))?;

    parse(&text).map_err(|error| {
        ReadError::Malformed(format!("{} at offset {}", error.message, error.offset))
    })
}

/// Writes one message, headers and all.
///
/// The length counts bytes rather than characters, which is the same mistake
/// as the one at the top of this file wearing different clothes.
pub fn write_message(output: &mut impl Write, message: &Json) -> std::io::Result<()> {
    let body = message.to_text();
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(body.as_bytes())?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use std::io::BufReader;

    use super::{ReadError, read_message, write_message};
    use crate::json::Json;

    fn framed(body: &str) -> String {
        format!("Content-Length: {}\r\n\r\n{body}", body.len())
    }

    #[test]
    fn a_message_survives_being_written_and_read_back() {
        let message = Json::object(vec![
            ("jsonrpc", Json::string("2.0")),
            ("id", Json::number(1)),
            ("method", Json::string("initialize")),
        ]);

        let mut written = Vec::new();
        write_message(&mut written, &message).unwrap();

        let mut input = BufReader::new(written.as_slice());
        assert_eq!(read_message(&mut input).unwrap(), message);
    }

    #[test]
    fn several_messages_come_back_in_order() {
        let text = format!("{}{}", framed("{\"id\":1}"), framed("{\"id\":2}"));
        let mut input = BufReader::new(text.as_bytes());

        assert_eq!(
            read_message(&mut input)
                .unwrap()
                .at(&["id"])
                .unwrap()
                .as_i64(),
            Some(1)
        );
        assert_eq!(
            read_message(&mut input)
                .unwrap()
                .at(&["id"])
                .unwrap()
                .as_i64(),
            Some(2)
        );
        assert!(matches!(read_message(&mut input), Err(ReadError::Closed)));
    }

    #[test]
    fn the_length_is_bytes_and_not_characters() {
        // The one that breaks the first time somebody writes a comment in a
        // language that is not English, and breaks by hanging rather than by
        // saying anything.
        let body = "{\"text\":\"gün\"}";
        assert_ne!(body.len(), body.chars().count());

        let mut written = Vec::new();
        let message = Json::object(vec![("text", Json::string("gün"))]);
        write_message(&mut written, &message).unwrap();

        let mut input = BufReader::new(written.as_slice());
        assert_eq!(
            read_message(&mut input)
                .unwrap()
                .at(&["text"])
                .unwrap()
                .as_str(),
            Some("gün")
        );
    }

    #[test]
    fn headers_are_case_insensitive_and_extra_ones_are_ignored() {
        let body = "{\"id\":7}";
        let text = format!(
            "content-length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{body}",
            body.len()
        );
        let mut input = BufReader::new(text.as_bytes());
        assert_eq!(
            read_message(&mut input)
                .unwrap()
                .at(&["id"])
                .unwrap()
                .as_i64(),
            Some(7)
        );
    }

    #[test]
    fn broken_framing_is_refused_rather_than_guessed_at() {
        for text in [
            "\r\n{}",
            "Content-Length: nonsense\r\n\r\n{}",
            "no colon here\r\n\r\n{}",
        ] {
            let mut input = BufReader::new(text.as_bytes());
            assert!(
                matches!(read_message(&mut input), Err(ReadError::Malformed(_))),
                "should have refused {text:?}"
            );
        }
    }

    #[test]
    fn a_body_that_is_not_json_is_malformed_rather_than_ignored() {
        let text = framed("{not json}");
        let mut input = BufReader::new(text.as_bytes());
        assert!(matches!(
            read_message(&mut input),
            Err(ReadError::Malformed(_))
        ));
    }
}
