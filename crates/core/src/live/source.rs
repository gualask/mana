//! A stream to play, as a `.stream` file or a channel list describes it.

use std::path::{Path, PathBuf};

use url::Url;

use super::recipe::Step;
use crate::log::log;
use crate::net::{self, loggable};

/// The extension of the files that describe a stream to play.
pub const EXTENSION: &str = "stream";
/// The longest header value sent.
const MAX_HEADER_VALUE: usize = net::MAX_LINE_VALUE;

/// A stream to play, read from a `.stream` file: `url = …`, or the
/// `resolve = …` steps of a recipe that end on the URL (see recipe.rs), any
/// number of `header = Name: value`, optional `ca = file.pem` (relative to
/// the file) trusted in addition to the public roots.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub url: Option<String>,
    pub resolve: Vec<Step>,
    pub headers: Vec<(String, String)>,
    pub root_certificate: Option<PathBuf>,
}

impl Source {
    /// The stream at `url`, requested with `headers`, as a channel list
    /// gives them.
    pub fn direct(url: String, headers: Vec<(String, String)>) -> Self {
        Self { url: Some(url), resolve: Vec::new(), headers: checked(headers), root_certificate: None }
    }

    /// The stream the recipe `resolve` ends on, requested with `headers`,
    /// as a channel list gives them.
    pub fn recipe(resolve: Vec<Step>, headers: Vec<(String, String)>) -> Self {
        Self { url: None, resolve, headers: checked(headers), root_certificate: None }
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let (mut url, mut resolve, mut headers, mut root_certificate) = (None, Vec::new(), Vec::new(), None);
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            match key.trim() {
                "url" => url = Some(value.to_string()),
                "resolve" => resolve.push(Step::parse(value)?),
                "header" => {
                    // Not the value in the message: it may be a token.
                    let (name, header) =
                        value.split_once(':').ok_or_else(|| format!("{}: header senza `Nome: valore`", path.display()))?;
                    headers.push((name.trim().to_string(), header.trim().to_string()));
                }
                "ca" => root_certificate = Some(path.parent().unwrap_or(Path::new(".")).join(value)),
                other => return Err(format!("chiave sconosciuta in {}: {other}", path.display())),
            }
        }
        if url.is_none() == resolve.is_empty() {
            return Err(format!("{}: serve `url` oppure una ricetta `resolve`, non entrambi", path.display()));
        }
        Ok(Self { url, resolve, headers: checked(headers), root_certificate })
    }

    /// The URLs the source gives itself, to which its credentials go: its
    /// `url`, or those its recipe downloads as written.
    pub fn named_urls(&self) -> Result<Vec<Url>, String> {
        let written = self.resolve.iter().filter_map(|step| match step {
            Step::Get(Some(url)) => Some(url),
            _ => None,
        });
        self.url.iter().chain(written).map(|url| Url::parse(url).map_err(|e| format!("{url}: {e}"))).collect()
    }

    /// For the log: host and path without what may carry a token.
    pub fn describe(&self) -> String {
        match &self.url {
            Some(url) => Url::parse(url).map_or_else(|_| "URL non valido".into(), |u| loggable(&u)),
            None => format!("ricetta di {} passi", self.resolve.len()),
        }
    }
}

/// `headers` without those a request cannot carry as they are, each logged
/// with its name and why, never its value, which may be a token.
fn checked(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    headers
        .into_iter()
        .filter(|(name, value)| match header_problem(name, value) {
            Some(problem) => {
                log!("header {name:?} scartato: {problem}");
                false
            }
            None => true,
        })
        .collect()
}

/// Why a header cannot be sent, as ureq, OkHttp and browsers refuse it: a
/// name that is not an RFC 9110 token, a value with other than visible
/// ASCII, spaces and tabs, or one longer than MAX_HEADER_VALUE.
fn header_problem(name: &str, value: &str) -> Option<String> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)) {
        return Some("nome non valido".into());
    }
    if !value.bytes().all(|b| b == b'\t' || (0x20..=0x7e).contains(&b)) {
        return Some("valore con caratteri di controllo o non ASCII".into());
    }
    if value.len() > MAX_HEADER_VALUE {
        return Some(format!("valore di {} byte, oltre i {MAX_HEADER_VALUE}", value.len()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(name: &str, value: &str) -> (String, String) {
        (name.into(), value.into())
    }

    #[test]
    fn headers_a_request_cannot_carry_are_named_with_the_reason() {
        assert_eq!(header_problem("User-Agent", "Mozilla/5.0 (X11; Linux)\tok"), None);
        assert_eq!(header_problem("Bad Name", "x"), Some("nome non valido".into()));
        assert_eq!(header_problem("", "x"), Some("nome non valido".into()));
        let control = Some("valore con caratteri di controllo o non ASCII".into());
        assert_eq!(header_problem("Referer", "https://a.invalid/\r\nX-Injected: 1"), control);
        assert_eq!(header_problem("Referer", "caff\u{e8}"), control);
        assert_eq!(header_problem("Cookie", &"a".repeat(MAX_HEADER_VALUE)), None);
        let long = header_problem("Cookie", &"a".repeat(MAX_HEADER_VALUE + 1));
        assert_eq!(long, Some(format!("valore di {} byte, oltre i {MAX_HEADER_VALUE}", MAX_HEADER_VALUE + 1)));
    }

    #[test]
    fn a_channel_list_s_bad_headers_are_left_out() {
        let headers = vec![header("User-Agent", "VLC/3.0"), header("Referer", "a\u{7}b"), header("Origin", "https://b.invalid")];
        let source = Source::direct("https://a.invalid/live.m3u8".into(), headers);
        assert_eq!(source.headers, [header("User-Agent", "VLC/3.0"), header("Origin", "https://b.invalid")]);
    }

    #[test]
    fn a_stream_file_s_bad_headers_are_left_out() {
        let dir = std::env::temp_dir().join(format!("mana-source-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("headers.stream");
        let long = "a".repeat(MAX_HEADER_VALUE + 1);
        let text = format!("url = https://a.invalid/live.m3u8\nheader = User-Agent: VLC/3.0\nheader = Cookie: {long}\nheader = X\u{1}Y: z\n");
        std::fs::write(&path, text).unwrap();
        assert_eq!(Source::load(&path).unwrap().headers, [header("User-Agent", "VLC/3.0")]);
    }
}
