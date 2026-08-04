//! An HTTP/1.1 client small enough to read in one sitting.
//!
//! This is the interpreter's half of the network capability. A compiled
//! program does not use it: it asks its host for `deed:io.fetch` and whatever
//! runs the component answers, which is the same arrangement every other
//! capability already has. This exists because the interpreter has to be able
//! to answer too, and because a capability nothing can exercise is a
//! capability nothing tests.
//!
//! What it does not do, and why:
//!
//! - **No TLS.** A TLS client is a cryptographic implementation and this
//!   workspace has no dependencies. Writing one to make a test go green would
//!   be the least trustworthy code here. [`crate::reach`] refuses `https` by
//!   name and says this, rather than failing to connect and letting the reader
//!   guess.
//! - **No redirects.** A redirect is a second request to a second host, and
//!   deciding to make it is exactly the decision the capability exists to put
//!   in the caller's hands. The status and the `Location` header come back and
//!   the program follows it or does not.
//! - **No connection reuse.** One request, one connection, `Connection: close`.
//!   A pool is a cache, and a cache is a measurement nobody has taken.
//! - **No streaming.** A response arrives as a string, which is what the
//!   language has. A body that does not decode as UTF-8 is an error rather
//!   than a lossy string, for the same reason `Io.read` gives one.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::reach::Target;

/// How long a request may spend connecting, writing or waiting.
///
/// A capability bounds what a program may reach and says nothing about how
/// long it may wait, so without this a program with an empty row can still
/// hang forever on a host that accepts a connection and never answers. Time is
/// the resource `design/04-capabilities.md` lists as unbounded, and this is one
/// place it can be bounded cheaply.
const TIMEOUT: Duration = Duration::from_secs(30);

/// The most of a response this will hold in memory.
///
/// A string is the only shape the language has for a body, so the whole thing
/// has to fit. Refusing at a stated size beats being killed by the allocator
/// at one nobody wrote down.
const LIMIT: usize = 8 * 1024 * 1024;

/// What came back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    /// The status line's code, so a program can tell 200 from 404 without
    /// parsing anything.
    pub status: u16,
    /// The body, decoded as UTF-8.
    pub body: String,
}

/// Makes one request and reads one response.
///
/// `body` is `None` for a request that carries none, which is what separates a
/// `GET` from a `POST` here rather than the method string alone.
pub fn request(target: &Target, method: &str, body: Option<&str>) -> Result<Response, String> {
    let address = format!("{}:{}", target.host, target.port);
    let mut stream = connect(&address)?;

    let mut head = format!(
        "{method} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: */*\r\n",
        target.path,
        target.authority()
    );
    if let Some(body) = body {
        head.push_str("Content-Type: text/plain; charset=utf-8\r\n");
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");

    stream
        .write_all(head.as_bytes())
        .map_err(|why| format!("could not send the request to `{address}`: {why}"))?;
    if let Some(body) = body {
        stream
            .write_all(body.as_bytes())
            .map_err(|why| format!("could not send the body to `{address}`: {why}"))?;
    }
    stream
        .flush()
        .map_err(|why| format!("could not send the request to `{address}`: {why}"))?;

    let raw = read_all(&mut stream, &address, LIMIT)?;
    parse(&raw, &address)
}

/// Opens a connection with both timeouts already set.
///
/// Separate because `connect_timeout` needs a resolved address and the read
/// and write timeouts are set on the socket afterwards, and getting one of the
/// three and not the others is the shape of a hang.
fn connect(address: &str) -> Result<TcpStream, String> {
    use std::net::ToSocketAddrs;

    let mut addresses = address
        .to_socket_addrs()
        .map_err(|why| format!("could not look up `{address}`: {why}"))?;
    let resolved = addresses
        .next()
        .ok_or_else(|| format!("`{address}` did not resolve to an address"))?;

    let stream = TcpStream::connect_timeout(&resolved, TIMEOUT)
        .map_err(|why| format!("could not reach `{address}`: {why}"))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|why| format!("could not set a timeout on `{address}`: {why}"))?;
    Ok(stream)
}

/// Reads until the server closes, refusing past `limit`.
///
/// The limit is a parameter rather than a read of [`LIMIT`] so that a test can
/// hand it a small one. A threshold nothing can cross in a test is a threshold
/// nothing checks.
fn read_all(stream: &mut TcpStream, address: &str, limit: usize) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|why| format!("could not read the answer from `{address}`: {why}"))?;
        if read == 0 {
            break;
        }
        if raw.len() + read > limit {
            return Err(format!(
                "the answer from `{address}` is larger than {limit} bytes, which is the most one \
                 can be held as a string"
            ));
        }
        raw.extend_from_slice(&chunk[..read]);
    }
    Ok(raw)
}

/// Splits a response into its status, its headers and its body.
fn parse(raw: &[u8], address: &str) -> Result<Response, String> {
    let split = find(raw, b"\r\n\r\n")
        .ok_or_else(|| format!("`{address}` answered with something that is not HTTP"))?;
    let head = std::str::from_utf8(&raw[..split])
        .map_err(|_| format!("the headers from `{address}` are not text"))?;
    let body = &raw[split + 4..];

    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| format!("`{address}` answered with an empty status line"))?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| format!("`{address}` answered with `{status_line}`, which has no status"))?;

    let chunked = lines.any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("transfer-encoding")
                && value.to_ascii_lowercase().contains("chunked")
        })
    });

    let body = if chunked {
        dechunk(body, address)?
    } else {
        body.to_vec()
    };

    let body = String::from_utf8(body).map_err(|_| {
        format!("the answer from `{address}` is not UTF-8, so it is not a `String`")
    })?;
    Ok(Response { status, body })
}

/// Reassembles a chunked body.
///
/// Servers pick this without being asked, so a client that does not understand
/// it reads the framing as content and hands back a string with hexadecimal
/// lengths in it, which is worse than an error.
fn dechunk(mut body: &[u8], address: &str) -> Result<Vec<u8>, String> {
    let malformed =
        || format!("the chunked answer from `{address}` ended in the middle of a chunk");
    let mut out = Vec::new();
    loop {
        let end = find(body, b"\r\n").ok_or_else(malformed)?;
        let header = std::str::from_utf8(&body[..end]).map_err(|_| malformed())?;
        // A chunk header may carry extensions after a semicolon, which say
        // nothing about the length.
        let size = header.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size, 16).map_err(|_| malformed())?;
        body = &body[end + 2..];
        if size == 0 {
            break;
        }
        // The chunk and the newline that ends it are asked for together. Two
        // checks would leave one of them unreachable: a body exactly as long
        // as the chunk has no room for the newline either, so nothing could
        // tell the two refusals apart.
        let rest = body.get(size + 2..).ok_or_else(malformed)?;
        out.extend_from_slice(&body[..size]);
        body = rest;
    }
    Ok(out)
}

/// The first index of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(raw: &str) -> Result<Response, String> {
        parse(raw.as_bytes(), "test")
    }

    #[test]
    fn a_status_and_a_body_come_back() {
        let answer = response("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi").unwrap();
        assert_eq!(answer.status, 200);
        assert_eq!(answer.body, "hi");
    }

    /// A status other than 200 is an answer, not an error. Whether a 404 is a
    /// problem is the caller's question, and this cannot know.
    #[test]
    fn a_failure_status_is_still_an_answer() {
        let answer = response("HTTP/1.1 404 Not Found\r\n\r\nno").unwrap();
        assert_eq!(answer.status, 404);
        assert_eq!(answer.body, "no");
    }

    #[test]
    fn a_chunked_body_is_reassembled() {
        let answer = response(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nabcd\r\n2\r\nef\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(answer.body, "abcdef");
    }

    #[test]
    fn a_chunk_extension_is_not_part_of_the_length() {
        let answer = response(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2;name=value\r\nok\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(answer.body, "ok");
    }

    #[test]
    fn the_header_name_is_read_without_regard_to_case() {
        let answer =
            response("HTTP/1.1 200 OK\r\ntransfer-encoding: Chunked\r\n\r\n2\r\nok\r\n0\r\n\r\n")
                .unwrap();
        assert_eq!(answer.body, "ok");
    }

    /// Both halves of the header decide. Reading the name alone would dechunk
    /// a body that was framed some other way, and reading the value alone
    /// would dechunk one because a different header happened to say the word.
    #[test]
    fn a_body_is_dechunked_only_when_the_header_says_both_things() {
        let plain = response("HTTP/1.1 200 OK\r\nTransfer-Encoding: identity\r\n\r\n2\r\nok")
            .expect("identity framing is not chunked");
        assert_eq!(plain.body, "2\r\nok");

        let elsewhere = response("HTTP/1.1 200 OK\r\nContent-Type: text/chunked\r\n\r\n2\r\nok")
            .expect("a content type is not a framing");
        assert_eq!(elsewhere.body, "2\r\nok");
    }

    /// The failure this exists to prevent: framing read as content.
    #[test]
    fn a_truncated_chunked_body_is_an_error_rather_than_a_string_with_lengths_in_it() {
        let why = response("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nab")
            .expect_err("a truncated chunk should not decode");
        assert!(why.contains("middle of a chunk"), "{why}");
    }

    #[test]
    fn something_that_is_not_http_is_said_to_be() {
        let why = response("hello").expect_err("that is not a response");
        assert!(why.contains("not HTTP"), "{why}");
    }

    #[test]
    fn a_status_line_with_no_code_is_refused() {
        let why = response("HTTP/1.1\r\n\r\n").expect_err("there is no status");
        assert!(why.contains("no status"), "{why}");
    }

    #[test]
    fn a_body_that_is_not_utf8_is_an_error_rather_than_a_lossy_string() {
        let mut raw = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        raw.push(0xff);
        let why = parse(&raw, "test").expect_err("that is not text");
        assert!(why.contains("not UTF-8"), "{why}");
    }

    /// The size limit, exercised at a size a test can reach. A body one byte
    /// over is refused and one exactly at it is not, which is the pair that
    /// says where the line is rather than that there is one.
    #[test]
    fn a_body_past_the_limit_is_refused_and_one_at_it_is_not() {
        assert_eq!(read_with(64, 64).expect("exactly at the limit").len(), 64);
        let why = read_with(64, 65).expect_err("one byte over the limit");
        assert!(why.contains("larger than 64 bytes"), "{why}");
    }

    /// The limit a released build actually carries, rather than one a test
    /// chose. Without this the constant is a number nothing reads, and
    /// arithmetic on it that produced a kilobyte would look the same from here
    /// as eight megabytes.
    ///
    /// Read in megabytes because that is the unit the number was chosen in. A
    /// copy of the expression would be the same arithmetic written twice and
    /// would agree with itself however it was wrong.
    #[test]
    fn the_shipped_limit_holds_an_ordinary_answer() {
        let megabyte = 1024 * 1024;
        assert_eq!(
            LIMIT / megabyte,
            8,
            "the limit is eight megabytes, and {LIMIT} bytes is not that"
        );
        assert_eq!(
            read_with(LIMIT, megabyte)
                .expect("a megabyte is ordinary")
                .len(),
            megabyte
        );
    }

    /// Reads `bytes` from a loopback server, refusing past `limit`.
    fn read_with(limit: usize, bytes: usize) -> Result<Vec<u8>, String> {
        use std::io::Write;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().expect("a bound address").port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.write_all(&vec![b'x'; bytes]);
            }
        });
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("a connection");
        read_all(&mut stream, "test", limit)
    }
}
