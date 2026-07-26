//! A language server for Vow.
//!
//! The compiler already produces everything an editor wants. Diagnostics are
//! structured data with spans on them, `Types::type_of` can say what an
//! expression turned out to be, and the formatter has one canonical answer
//! with no options. What was missing was a way for an editor to ask, which is
//! all this is: framing, a small JSON reader, and a translation between byte
//! offsets and the positions the protocol uses.
//!
//! No dependencies, for the same reason the rest of the compiler has none. The
//! protocol needed here is a header, a blank line and a handful of object
//! shapes, and a serialiser would be a larger thing to audit than the code it
//! replaced.
//!
//! ```
//! use vow_lsp::{Json, Next, Server};
//!
//! let mut server = Server::new();
//! let open = vow_lsp::json::parse(
//!     r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":
//!        {"textDocument":{"uri":"file:///a.vow","text":"module a\n"}}}"#,
//! )
//! .unwrap();
//!
//! server.handle(&vow_lsp::json::parse(r#"{"id":1,"method":"initialize"}"#).unwrap());
//! let (sent, next) = server.handle(&open);
//!
//! assert_eq!(next, Next::Continue);
//! assert_eq!(
//!     sent[0].at(&["method"]).and_then(Json::as_str),
//!     Some("textDocument/publishDiagnostics")
//! );
//! ```

pub mod json;
pub mod position;
pub mod protocol;
mod server;
pub mod uri;
pub mod workspace;

use std::io::{BufRead, Write};

pub use json::Json;
pub use position::{Lines, Position};
pub use protocol::ReadError;
pub use server::{Next, Server};

/// Runs a server until the editor says `exit` or the stream closes.
///
/// Malformed framing ends the loop rather than being skipped. Once a message
/// has been read wrongly the position in the stream is no longer known, so
/// carrying on means reading the middle of one message as the start of the
/// next, and an editor talking to a server in that state gets answers to
/// questions it did not ask.
pub fn serve(input: &mut impl BufRead, output: &mut impl Write) -> std::io::Result<()> {
    let mut server = Server::new();

    loop {
        let message = match protocol::read_message(input) {
            Ok(message) => message,
            Err(ReadError::Closed) => return Ok(()),
            Err(ReadError::Io(error)) => return Err(error),
            Err(ReadError::Malformed(why)) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("the editor sent something this server could not read: {why}"),
                ));
            }
        };

        let (replies, next) = server.handle(&message);
        for reply in &replies {
            protocol::write_message(output, reply)?;
        }
        if next == Next::Stop {
            return Ok(());
        }
    }
}
