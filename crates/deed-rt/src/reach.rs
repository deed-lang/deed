//! Resolving a URL inside a network capability, without letting it out.
//!
//! This is [`sandbox`](crate::sandbox) for the other resource. A `Dir` is
//! worth something because a function holding one rooted at `cache` has no way
//! to reach its parent; a `Net` is worth something for exactly the same
//! reason, and the thing it must not be able to reach is a host nobody granted
//! it.
//!
//! The rules are deliberately narrower than a URL parser allows:
//!
//! - the set of reachable hosts is given by whoever starts the program, and it
//!   starts empty rather than starting as "everything"
//! - a host matches by equality, never by suffix, so granting `example.com`
//!   does not grant `evil-example.com` or `example.com.evil.net`
//! - narrowing may only ever remove hosts, so there is no way back out
//! - a scheme other than `http` is refused by name rather than attempted
//!
//! The second rule is the one that matters and the one that is easy to leave
//! out. Suffix matching is how a host allowlist becomes a host suggestion.
//!
//! ## Why the default is empty and `--dir` defaults to the working directory
//!
//! They look inconsistent and they are not the same question. Running a
//! command in a directory is already a choice about that directory: the
//! working directory is one a person navigated to. There is no equivalent
//! ambient choice about the network, so there is nothing for a default to
//! inherit, and "the network" is not a place anyone is standing.

use std::collections::BTreeSet;

/// The hosts a `Net` may reach.
///
/// Ordered and deduplicated so that two capabilities granted the same hosts in
/// different orders compare equal, which is what makes a narrowed `Net`
/// comparable to the one it came from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reach {
    hosts: BTreeSet<String>,
}

impl Reach {
    /// A capability that reaches nothing. The default a run starts from.
    pub fn none() -> Self {
        Reach::default()
    }

    /// A capability reaching exactly `hosts`.
    ///
    /// Each entry is either a bare host (`example.com`, which grants every
    /// port on it, the way a `Dir` grants everything under it) or a host with
    /// a port (`example.com:8080`, which grants only that one).
    pub fn granting<I, S>(hosts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Reach {
            hosts: hosts
                .into_iter()
                .map(|host| host.as_ref().trim().to_ascii_lowercase())
                .filter(|host| !host.is_empty())
                .collect(),
        }
    }

    /// Whether this reaches nothing at all.
    pub fn is_empty(&self) -> bool {
        self.hosts.is_empty()
    }

    /// The hosts, in order, for a message that has to name them.
    pub fn hosts(&self) -> impl Iterator<Item = &str> {
        self.hosts.iter().map(String::as_str)
    }

    /// Whether `authority` (a host, or a host and port) is reachable.
    ///
    /// Equality on the whole authority, or equality on the host alone when the
    /// grant named no port. Never a suffix: `example.com` does not grant
    /// `evil-example.com`, and that is the entire point of this function.
    fn admits(&self, host: &str, port: u16) -> bool {
        self.hosts.contains(host) || self.hosts.contains(&format!("{host}:{port}"))
    }
}

/// Where a URL points, once it has been allowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    /// The host, lowercased.
    pub host: String,
    /// The port, defaulted from the scheme when the URL left it out.
    pub port: u16,
    /// Everything from the path onwards, including any query.
    pub path: String,
}

impl Target {
    /// The authority as it is written in a `Host:` header and in a message.
    pub fn authority(&self) -> String {
        if self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// Why a URL or a host was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// The empty string names nothing.
    Empty,
    /// No scheme, no host, or something else that is not a URL at all.
    NotAUrl,
    /// A scheme this runtime does not speak, carried so the message can name
    /// it. `https` lands here and gets its own sentence.
    NotHttp(String),
    /// The port was not a number, or did not fit in one.
    NotAPort(String),
    /// A well formed URL pointing somewhere this capability does not reach.
    NotGranted(String),
}

impl Refused {
    /// The message a program sees, which is the rule that was hit.
    pub fn message(&self, subject: &str) -> String {
        match self {
            Refused::Empty => "the empty string names nothing".to_string(),
            Refused::NotAUrl => {
                format!("`{subject}` is not a URL, and a `Net` reaches things by URL")
            }
            // Named rather than attempted. A TLS client is a cryptographic
            // implementation, this compiler has no dependencies, and writing
            // one to get a green test would be the least trustworthy code in
            // the repository. A compiled program does not hit this: `https`
            // is the host's to speak, and the host is whatever runs the
            // component.
            Refused::NotHttp(scheme) if scheme == "https" => format!(
                "`{subject}` is `https`, and this runtime speaks `http` only, because TLS needs \
                 a cryptographic implementation and this compiler carries no dependencies; a \
                 compiled component asks its host for `deed:io.fetch` and the host may speak \
                 whatever it likes"
            ),
            Refused::NotHttp(scheme) => {
                format!("`{subject}` is `{scheme}`, and a `Net` speaks `http`")
            }
            Refused::NotAPort(port) => format!("`{port}` is not a port number"),
            Refused::NotGranted(host) => format!(
                "`{host}` is not one of the hosts this `Net` reaches, and there is no way to \
                 widen one"
            ),
        }
    }
}

/// Splits a URL into scheme, authority and path without validating anything.
///
/// Separate from [`resolve`] for the reason [`crate::sandbox`] splits its name
/// check out: these are the rules a reader has to be able to verify, and
/// mixing them with a decision about authority makes that harder.
fn parts(url: &str) -> Result<(String, &str, String), Refused> {
    if url.is_empty() {
        return Err(Refused::Empty);
    }

    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(Refused::NotAUrl);
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme.is_empty()
        || !scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '+')
    {
        return Err(Refused::NotAUrl);
    }

    // A userinfo section (`user@host`) is refused rather than stripped. It
    // carries a credential, and a capability that quietly drops one is a
    // capability whose reach depends on something the caller cannot see.
    let (authority, path) = match rest.find(['/', '?', '#']) {
        Some(at) => (&rest[..at], rest[at..].to_string()),
        None => (rest, "/".to_string()),
    };
    if authority.is_empty() || authority.contains('@') {
        return Err(Refused::NotAUrl);
    }

    Ok((
        scheme,
        authority,
        if path.is_empty() { "/".into() } else { path },
    ))
}

/// Resolves `url` against `reach`, or says which rule refused it.
///
/// The check that decides is the last one: a URL this capability was not
/// granted is refused however well formed it is.
pub fn resolve(reach: &Reach, url: &str) -> Result<Target, Refused> {
    let (scheme, authority, path) = parts(url)?;
    if scheme != "http" {
        return Err(Refused::NotHttp(scheme));
    }

    let (host, port) = split_authority(authority, 80)?;
    if !reach.admits(&host, port) {
        return Err(Refused::NotGranted(host));
    }

    Ok(Target { host, port, path })
}

/// Narrows `reach` to `host`, or says why it cannot.
///
/// The returned capability reaches a subset of what went in, never more, which
/// is the same one-way property `sandbox::resolve` gives a `Dir`. Asking for a
/// host that was not already reachable is refused rather than granted, because
/// a narrowing operation that can add a host is not a narrowing operation.
pub fn narrow(reach: &Reach, host: &str) -> Result<Reach, Refused> {
    let host = host.trim();
    if host.is_empty() {
        return Err(Refused::Empty);
    }
    if host.contains("://") || host.contains('/') || host.contains('@') {
        return Err(Refused::NotAUrl);
    }

    let (name, port) = split_authority(host, 0)?;
    let asked = if port == 0 {
        name.clone()
    } else {
        format!("{name}:{port}")
    };

    // Reachable either because the exact authority was granted, or because a
    // bare host was granted and this narrows it to one port. Both directions
    // only ever remove.
    let kept: BTreeSet<String> = reach
        .hosts
        .iter()
        .filter(|granted| **granted == asked || (port != 0 && **granted == name))
        .map(|_| asked.clone())
        .collect();

    if kept.is_empty() {
        return Err(Refused::NotGranted(asked));
    }
    Ok(Reach { hosts: kept })
}

/// Splits `host:port`, defaulting the port when there is none.
fn split_authority(authority: &str, default: u16) -> Result<(String, u16), Refused> {
    // Only the last colon separates a port, and a host containing any colon at
    // all is an IPv6 literal this runtime does not take. Refusing it by name
    // beats guessing which colon meant what.
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => {
            if host.contains(':') {
                return Err(Refused::NotAUrl);
            }
            let parsed = port
                .parse::<u16>()
                .map_err(|_| Refused::NotAPort(port.to_string()))?;
            if parsed == 0 {
                return Err(Refused::NotAPort(port.to_string()));
            }
            (host, parsed)
        }
        None => (authority, default),
    };

    if host.is_empty() || host.contains(':') {
        return Err(Refused::NotAUrl);
    }
    Ok((host.to_ascii_lowercase(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn granting(hosts: &[&str]) -> Reach {
        Reach::granting(hosts.iter().copied())
    }

    #[test]
    fn a_new_capability_reaches_nothing() {
        assert!(Reach::none().is_empty());
        assert_eq!(
            resolve(&Reach::none(), "http://example.com/x"),
            Err(Refused::NotGranted("example.com".to_string()))
        );
    }

    #[test]
    fn a_granted_host_resolves() {
        let target = resolve(&granting(&["example.com"]), "http://example.com/a?b=1")
            .expect("the host was granted");
        assert_eq!(target.host, "example.com");
        assert_eq!(target.port, 80);
        assert_eq!(target.path, "/a?b=1");
    }

    #[test]
    fn a_url_with_no_path_reaches_the_root() {
        let target = resolve(&granting(&["example.com"]), "http://example.com")
            .expect("the host was granted");
        assert_eq!(target.path, "/");
    }

    /// The rule the whole allowlist rests on. Anything that matches by suffix
    /// or by prefix turns a grant into a suggestion.
    #[test]
    fn a_host_matches_by_equality_and_never_by_suffix() {
        let reach = granting(&["example.com"]);
        for pretender in [
            "http://evil-example.com/x",
            "http://example.com.evil.net/x",
            "http://notexample.com/x",
            "http://sub.example.com/x",
        ] {
            assert!(
                matches!(resolve(&reach, pretender), Err(Refused::NotGranted(_))),
                "{pretender} should not be reachable from a grant of example.com"
            );
        }
    }

    #[test]
    fn a_host_is_matched_without_regard_to_case() {
        let reach = granting(&["Example.COM"]);
        assert!(resolve(&reach, "http://EXAMPLE.com/x").is_ok());
    }

    #[test]
    fn a_grant_naming_a_port_reaches_only_that_port() {
        let reach = granting(&["example.com:8080"]);
        assert!(resolve(&reach, "http://example.com:8080/x").is_ok());
        assert_eq!(
            resolve(&reach, "http://example.com:9090/x"),
            Err(Refused::NotGranted("example.com".to_string()))
        );
        assert_eq!(
            resolve(&reach, "http://example.com/x"),
            Err(Refused::NotGranted("example.com".to_string()))
        );
    }

    #[test]
    fn a_grant_naming_no_port_reaches_every_port_on_that_host() {
        let reach = granting(&["example.com"]);
        assert!(resolve(&reach, "http://example.com:8080/x").is_ok());
        assert!(resolve(&reach, "http://example.com/x").is_ok());
    }

    #[test]
    fn https_is_refused_by_name_with_the_reason() {
        let reach = granting(&["example.com"]);
        let Err(refused) = resolve(&reach, "https://example.com/x") else {
            panic!("https should be refused");
        };
        assert_eq!(refused, Refused::NotHttp("https".to_string()));
        let message = refused.message("https://example.com/x");
        assert!(message.contains("TLS"), "{message}");
        assert!(message.contains("no dependencies"), "{message}");
    }

    #[test]
    fn a_scheme_that_is_not_http_is_named() {
        let reach = granting(&["example.com"]);
        assert_eq!(
            resolve(&reach, "ftp://example.com/x"),
            Err(Refused::NotHttp("ftp".to_string()))
        );
        assert_eq!(
            resolve(&reach, "file://example.com/x"),
            Err(Refused::NotHttp("file".to_string()))
        );
    }

    #[test]
    fn something_that_is_not_a_url_is_refused_as_one() {
        let reach = granting(&["example.com"]);
        assert_eq!(resolve(&reach, ""), Err(Refused::Empty));
        assert_eq!(resolve(&reach, "example.com/x"), Err(Refused::NotAUrl));
        assert_eq!(resolve(&reach, "http:///x"), Err(Refused::NotAUrl));
    }

    /// A URL carrying a credential is refused rather than stripped, because a
    /// capability that quietly drops one reaches somewhere the caller cannot see.
    #[test]
    fn userinfo_is_refused_rather_than_dropped() {
        let reach = granting(&["example.com"]);
        assert_eq!(
            resolve(&reach, "http://user:secret@example.com/x"),
            Err(Refused::NotAUrl)
        );
    }

    #[test]
    fn a_port_that_is_not_a_number_is_refused() {
        let reach = granting(&["example.com"]);
        assert_eq!(
            resolve(&reach, "http://example.com:http/x"),
            Err(Refused::NotAPort("http".to_string()))
        );
        assert_eq!(
            resolve(&reach, "http://example.com:0/x"),
            Err(Refused::NotAPort("0".to_string()))
        );
    }

    /// The other half of the `Dir` guarantee, on the other resource: what
    /// comes back reaches less than what went in, and never more.
    #[test]
    fn narrowing_keeps_only_the_host_asked_for() {
        let reach = granting(&["a.example", "b.example"]);
        let narrowed = narrow(&reach, "a.example").expect("a.example was granted");
        assert!(resolve(&narrowed, "http://a.example/x").is_ok());
        assert_eq!(
            resolve(&narrowed, "http://b.example/x"),
            Err(Refused::NotGranted("b.example".to_string()))
        );
    }

    #[test]
    fn narrowing_to_a_host_that_was_not_granted_is_refused() {
        let reach = granting(&["a.example"]);
        assert_eq!(
            narrow(&reach, "b.example"),
            Err(Refused::NotGranted("b.example".to_string()))
        );
        assert_eq!(
            narrow(&Reach::none(), "a.example"),
            Err(Refused::NotGranted("a.example".to_string()))
        );
    }

    /// Narrowing twice cannot climb back out, which is the property that makes
    /// handing a narrowed capability onwards safe.
    #[test]
    fn there_is_no_way_back_out_of_a_narrowed_capability() {
        let reach = granting(&["a.example", "b.example"]);
        let narrowed = narrow(&reach, "a.example").expect("a.example was granted");
        assert_eq!(
            narrow(&narrowed, "b.example"),
            Err(Refused::NotGranted("b.example".to_string()))
        );
        let again = narrow(&narrowed, "a.example").expect("still reachable");
        assert_eq!(again, narrowed);
    }

    #[test]
    fn narrowing_a_bare_host_to_one_port_is_allowed() {
        let reach = granting(&["example.com"]);
        let narrowed = narrow(&reach, "example.com:8080").expect("the host was granted");
        assert!(resolve(&narrowed, "http://example.com:8080/x").is_ok());
        assert_eq!(
            resolve(&narrowed, "http://example.com:9090/x"),
            Err(Refused::NotGranted("example.com".to_string()))
        );
    }

    /// The direction that must not work: a grant of one port cannot be widened
    /// to the whole host by asking for the bare name.
    #[test]
    fn narrowing_cannot_widen_a_port_grant_to_the_whole_host() {
        let reach = granting(&["example.com:8080"]);
        assert_eq!(
            narrow(&reach, "example.com"),
            Err(Refused::NotGranted("example.com".to_string()))
        );
    }

    #[test]
    fn a_url_is_not_a_host() {
        let reach = granting(&["example.com"]);
        assert_eq!(narrow(&reach, "http://example.com"), Err(Refused::NotAUrl));
        assert_eq!(narrow(&reach, "example.com/x"), Err(Refused::NotAUrl));
        assert_eq!(narrow(&reach, ""), Err(Refused::Empty));
    }

    #[test]
    fn an_authority_carrying_two_colons_is_refused() {
        let reach = granting(&["example.com"]);
        assert_eq!(resolve(&reach, "http://[::1]:80/x"), Err(Refused::NotAUrl));
    }

    #[test]
    fn the_authority_leaves_out_the_default_port() {
        let target = resolve(&granting(&["example.com"]), "http://example.com/x").unwrap();
        assert_eq!(target.authority(), "example.com");
        let target = resolve(&granting(&["example.com"]), "http://example.com:8080/x").unwrap();
        assert_eq!(target.authority(), "example.com:8080");
    }
}
