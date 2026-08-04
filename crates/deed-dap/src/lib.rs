//! A debug adapter for Deed.
//!
//! The compiler could already say what a program means and the interpreter
//! could already run it. What neither could do was stop, which is the only
//! thing a debugger is: a place where a running program is held still and can
//! be asked about itself.
//!
//! The place is [`deed_interp::Watcher`], a hook the evaluator calls before
//! each statement. It decides nothing. Everything about what a breakpoint is,
//! what a step means, and when to carry on lives here, because those are
//! protocol questions and the interpreter would only be a second place to
//! answer them differently. See
//! `design/decisions/2026-08-04-a-place-to-stand.md`.
//!
//! # Shape
//!
//! The program runs on its own thread. When its watcher decides to stop, it
//! sends what it can see and waits to be told what to do next, so the host
//! stack is left exactly where it was and the state a client reads is the
//! state the program is in. Nothing is re-run and nothing is simulated.
//!
//! [`Session::handle`] is synchronous: a request that resumes execution comes
//! back with the events up to the next stop. That is what makes a session a
//! stream of messages in and a stream of messages out, which is what
//! `crates/deed-dap/tests/session.rs` reads.
//!
//! No dependencies, and no second copy of the framing either. The Debug
//! Adapter Protocol is `Content-Length`-framed JSON, the same as the language
//! server protocol, so `deed_lsp::protocol` and `deed_lsp::json` are what
//! reads and writes it.
//!
//! ```
//! use deed_dap::{Next, Session};
//! use deed_lsp::Json;
//!
//! let mut session = Session::new();
//! let (replies, next) = session.handle(
//!     &deed_lsp::json::parse(r#"{"seq":1,"type":"request","command":"initialize"}"#).unwrap(),
//! );
//!
//! assert_eq!(next, Next::Continue);
//! assert_eq!(
//!     replies[0].at(&["body", "supportsConfigurationDoneRequest"]),
//!     Some(&Json::Bool(true))
//! );
//! assert_eq!(replies[1].at(&["event"]).and_then(Json::as_str), Some("initialized"));
//! ```

mod running;
mod session;
mod stepper;

use std::io::{BufRead, Write};

pub use session::{Next, Session};

use deed_lsp::ReadError;
use deed_lsp::protocol;

/// Runs an adapter until the client disconnects or the stream closes.
///
/// Malformed framing ends the loop rather than being skipped, for the reason
/// `deed_lsp::serve` gives: once one message has been read wrongly the
/// position in the stream is no longer known, and carrying on means answering
/// questions nobody asked.
pub fn serve(input: &mut impl BufRead, output: &mut impl Write) -> std::io::Result<()> {
    let mut session = Session::new();

    loop {
        let message = match protocol::read_message(input) {
            Ok(message) => message,
            Err(ReadError::Closed) => return Ok(()),
            Err(ReadError::Io(error)) => return Err(error),
            Err(ReadError::Malformed(why)) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("the client sent something this adapter could not read: {why}"),
                ));
            }
        };

        let (replies, next) = session.handle(&message);
        for reply in &replies {
            protocol::write_message(output, reply)?;
        }
        if next == Next::Stop {
            return Ok(());
        }
    }
}
