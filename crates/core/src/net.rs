//! The HTTP base every request goes through: rustls with the public roots,
//! timeouts, User-Agent, gzip answers accepted and unpacked (RaiPlay's
//! 242 KB playlists travel as 7 KB), bodies read whole up to the bound the
//! caller gives or as they arrive, and URLs logged without tokens.
//! Request headers and an extra trusted CA belong to one source: only the
//! client made for that source carries them, and its credentials go only
//! to the origins the source names itself.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use url::{Origin, Url};

/// The longest value sent in one line of a request, a URL or a header
/// value: room for a JWT or a session cookie, well within the 8 KB a
/// server usually takes for a line.
pub const MAX_LINE_VALUE: usize = 4096;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// Desktop Chrome's: the requests are those of a browser's HLS player,
/// and servers refuse or treat differently clients they do not know. The
/// version is kept recent, as a years-old one stands out too.
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/156.0.0.0 Safari/537.36";
/// Headers that carry credentials (RFC 9110 §11.6.2, §11.7.2, RFC 6265):
/// sent only to the origins the source names, as browsers keep them to
/// their protection space; the others (User-Agent, Referer, Origin) go
/// with every request, as the channels need them on their CDNs too.
const CREDENTIALS: [&str; 3] = ["Authorization", "Proxy-Authorization", "Cookie"];

/// A failed request; it names the URL as `loggable_path`.
#[derive(Debug)]
pub struct Error {
    /// The HTTP status of an answer that was not a success.
    pub status: Option<u16>,
    /// An answer longer than its bound: asking again brings the same.
    oversized: bool,
    message: String,
}

impl Error {
    /// Whether the server refused the URL's token (expired, as RaiPlay's
    /// after 24 h): only new URLs can help.
    pub fn is_refused(&self) -> bool {
        matches!(self.status, Some(401 | 403 | 410))
    }

    /// Whether the answer was longer than what it may be.
    pub fn is_oversized(&self) -> bool {
        self.oversized
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<Error> for String {
    fn from(error: Error) -> Self {
        error.message
    }
}

#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    headers: Vec<(String, String)>,
    /// Where the CREDENTIALS among the headers go.
    credentials_for: Vec<Origin>,
}

impl Client {
    /// A client trusting the public roots only, without headers of its own.
    pub fn new() -> Result<Self, String> {
        Self::for_source(Vec::new(), &[], None)
    }

    /// A client for one source: its fixed request headers on every request,
    /// those with credentials only to the origins of `named`, the URLs the
    /// source gives itself (not those its playlists or answers lead to),
    /// and `root_certificate` (PEM) trusted in addition to the public roots.
    pub fn for_source(headers: Vec<(String, String)>, named: &[Url], root_certificate: Option<&Path>) -> Result<Self, String> {
        let mut roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        if let Some(path) = root_certificate {
            let pem = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            for certificate in rustls_pemfile::certs(&mut pem.as_slice()) {
                roots.add(certificate.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            }
        }
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(crypto_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
        let agent = ureq::AgentBuilder::new()
            .tls_config(Arc::new(tls))
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .user_agent(USER_AGENT)
            .build();
        Ok(Self { agent, headers, credentials_for: named.iter().map(Url::origin).collect() })
    }

    /// The body, of at most `limit` bytes: past it, the answer is refused
    /// rather than cut, also after decompression. Only the connection and
    /// each read have a time limit: a large segment may take long as a whole.
    pub fn get(&self, url: &Url, limit: u64) -> Result<Vec<u8>, Error> {
        Ok(self.fetch(url, limit, None)?.1)
    }

    /// The URL that served the body, after redirects, and the body, of at
    /// most `limit` bytes. A document's relative URLs resolve against the
    /// former: a CDN may redirect to a path that carries its token
    /// (RaiPlay's on netrw.it).
    pub fn get_with_final_url(&self, url: &Url, limit: u64) -> Result<(Url, Vec<u8>), Error> {
        self.fetch(url, limit, None)
    }

    /// The body, of at most `limit` bytes, given up when the whole request
    /// takes longer than `within`: a host that trickles a small resource
    /// must not hold the caller.
    pub fn get_small(&self, url: &Url, limit: u64, within: Duration) -> Result<Vec<u8>, Error> {
        Ok(self.fetch(url, limit, Some(within))?.1)
    }

    /// The URL that served the answer, after redirects, and its body to
    /// read as it arrives: for a stream that never ends. Only each read
    /// has a time limit.
    pub fn open(&self, url: &Url) -> Result<(Url, Box<dyn Read + Send>), Error> {
        self.request(url, None)
    }

    fn fetch(&self, url: &Url, limit: u64, within: Option<Duration>) -> Result<(Url, Vec<u8>), Error> {
        let (served, body) = self.request(url, within)?;
        Ok((served, read_rest(url, body, limit)?))
    }

    fn request(&self, url: &Url, within: Option<Duration>) -> Result<(Url, Box<dyn Read + Send>), Error> {
        let mut request = self.agent.request_url("GET", url);
        if let Some(within) = within {
            request = request.timeout(within);
        }
        let named = self.credentials_for.contains(&url.origin());
        for (name, value) in &self.headers {
            if named || !CREDENTIALS.iter().any(|c| c.eq_ignore_ascii_case(name)) {
                request = request.set(name, value);
            }
        }
        let response = request.call().map_err(|e| request_error(url, e))?;
        let served = Url::parse(response.get_url()).unwrap_or_else(|_| url.clone());
        Ok((served, response.into_reader()))
    }
}

/// ring with its cipher suites in the order the CPU runs fastest. On 32-bit
/// ARM ring has no hardware AES, and ChaCha20-Poly1305 on NEON costs about
/// a third of AES-GCM on NEON: offered first, servers that follow the
/// client's preference pick it (Akamai's do, CloudFront's keep AES-128).
/// AES-128 comes before AES-256, which takes 14 rounds instead of 10.
fn crypto_provider() -> rustls::crypto::CryptoProvider {
    use rustls::CipherSuite::*;
    let mut provider = rustls::crypto::ring::default_provider();
    if cfg!(target_arch = "arm") {
        provider.cipher_suites.sort_by_key(|suite| match suite.suite() {
            TLS13_CHACHA20_POLY1305_SHA256
            | TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256
            | TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256 => 0,
            TLS13_AES_128_GCM_SHA256 | TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 | TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 => 1,
            _ => 2,
        });
    }
    provider
}

/// The rest of an answer that `open` gave, of at most `limit` bytes, as
/// `get` reads one: a byte past it tells a longer answer from one that
/// ends there.
pub fn read_rest(url: &Url, body: impl Read, limit: u64) -> Result<Vec<u8>, Error> {
    let mut data = Vec::new();
    body.take(limit.saturating_add(1)).read_to_end(&mut data).map_err(|e| read_error(url, &e))?;
    if data.len() as u64 > limit {
        let message = format!("{}: risposta oltre {} byte", loggable_path(url), limit);
        return Err(Error { status: None, oversized: true, message });
    }
    Ok(data)
}

/// Reading the body of an answer from `url` failed.
pub fn read_error(url: &Url, error: &std::io::Error) -> Error {
    Error { status: None, oversized: false, message: format!("{}: {error}", loggable_path(url)) }
}

/// A failed request to `url`: the server's status, or what the transport
/// reports and its cause.
fn request_error(url: &Url, error: ureq::Error) -> Error {
    match error {
        ureq::Error::Status(code, _) => Error { status: Some(code), oversized: false, message: format!("HTTP {code} per {}", loggable_path(url)) },
        ureq::Error::Transport(transport) => {
            let cause = std::error::Error::source(&transport).map(|c| format!(": {c}")).unwrap_or_default();
            Error { status: None, oversized: false, message: format!("{}: {}{cause}", loggable_path(url), transport.kind()) }
        }
    }
}

/// The URL's path for the log, without what may carry a token: the query
/// string, and path segments with `=` in them (Akamai puts
/// `exp=…~hmac=…` in the path, as RaiPlay's playlists do).
pub fn loggable_path(url: &Url) -> String {
    url.path().split('/').map(|segment| if segment.contains('=') { "…" } else { segment }).collect::<Vec<_>>().join("/")
}

/// Host and path for the log, as `loggable_path`.
pub fn loggable(url: &Url) -> String {
    format!("{}{}", url.host_str().unwrap_or(""), loggable_path(url))
}

#[cfg(test)]
mod tests;
