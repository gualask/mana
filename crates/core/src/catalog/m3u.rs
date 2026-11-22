//! M3U channel lists.

use std::path::Path;

use super::Channel;
use crate::live::source::Source;

/// Why an entry without a URL cannot be played.
pub const NO_URL: &str = "la lista non dà il suo indirizzo";

/// The channels of the M3U list at `path`; None when it cannot be read or
/// lists none.
pub fn load(path: &Path) -> Option<Vec<Channel>> {
    let channels = parse(&std::fs::read_to_string(path).ok()?);
    (!channels.is_empty()).then_some(channels)
}

/// One channel per `#EXTINF` line: the name after its comma, the logo and
/// the group from its `tvg-logo` and `group-title` attributes, the URL from
/// the first line after it that is not a tag; an entry without one cannot
/// be played. Request headers come from the lines in between,
/// `#EXTVLCOPT:http-user-agent=…` and `#EXTVLCOPT:http-referrer=…` as VLC
/// reads them, and from the URL itself, `url|User-Agent=…&Referer=…` as
/// Kodi reads it, values percent-encoded; `|` between headers is accepted
/// too.
pub fn parse(text: &str) -> Vec<Channel> {
    let mut channels = Vec::new();
    let mut current: Option<Entry> = None;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            channels.extend(current.replace(Entry::from_extinf(info)).map(|entry| entry.channel(None)));
        } else if let Some(header) = line.strip_prefix("#EXTVLCOPT:").and_then(vlc_header)
            && let Some(entry) = current.as_mut()
        {
            entry.headers.push(header);
        } else if !line.starts_with('#')
            && let Some(entry) = current.take()
        {
            channels.push(entry.channel(Some(line)));
        }
    }
    channels.extend(current.map(|entry| entry.channel(None)));
    channels
}

/// An entry being read, until its URL.
struct Entry {
    name: String,
    logo: Option<String>,
    group: Option<String>,
    headers: Vec<(String, String)>,
}

impl Entry {
    /// From what follows `#EXTINF:`.
    fn from_extinf(info: &str) -> Self {
        let name = info.rsplit_once(',').map_or("", |(_, name)| name).trim().to_string();
        Self { name, logo: attribute(info, "tvg-logo"), group: attribute(info, "group-title"), headers: Vec::new() }
    }

    /// The channel of the entry, with the URL line that ends it, if any.
    fn channel(mut self, line: Option<&str>) -> Channel {
        let play = match line {
            Some(line) => {
                let (url, headers) = line.split_once('|').unwrap_or((line, ""));
                self.headers.extend(kodi_headers(headers));
                Ok(Source::direct(url.to_string(), self.headers))
            }
            None => Err(NO_URL.to_string()),
        };
        Channel { name: self.name, logo: self.logo, group: self.group, play }
    }
}

/// The value of the attribute `name="…"` of an `#EXTINF` line, if not empty.
fn attribute(info: &str, name: &str) -> Option<String> {
    info.split_once(&format!("{name}=\""))
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(value, _)| value.to_string())
        .filter(|value| !value.is_empty())
}

/// The header of a VLC option that sets one; other options are not ours.
fn vlc_header(option: &str) -> Option<(String, String)> {
    let (name, value) = option.split_once('=')?;
    let name = match name.trim() {
        "http-user-agent" => "User-Agent",
        "http-referrer" => "Referrer",
        _ => return None,
    };
    Some((http_name(name), value.trim().to_string()))
}

/// The `Name=value` pairs after a URL's `|`.
fn kodi_headers(text: &str) -> Vec<(String, String)> {
    url::form_urlencoded::parse(text.replace('|', "&").as_bytes())
        .filter(|(name, _)| !name.trim().is_empty())
        .map(|(name, value)| (http_name(name.trim()), value.trim().to_string()))
        .collect()
}

/// A header's name as HTTP spells it: `Referrer`, as VLC and most lists
/// write it, is `Referer`.
fn http_name(name: &str) -> String {
    if name.eq_ignore_ascii_case("referrer") { "Referer".into() } else { name.into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(name: &str, logo: Option<&str>, url: Option<&str>, headers: &[(&str, &str)]) -> Channel {
        let headers = headers.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect();
        let play = url.map(|url| Source::direct(url.into(), headers)).ok_or(NO_URL.to_string());
        Channel { name: name.into(), logo: logo.map(Into::into), group: None, play }
    }

    #[test]
    fn names_logos_and_urls_come_from_each_entry() {
        let text = "#EXTM3U\n#EXTINF:-1 tvg-id=\"Rai1.it\" tvg-logo=\"https://logo.invalid/rai1.png\",Rai 1 (1080p)\nhttps://a.invalid/1.m3u8\n\
                    #EXTINF:-1 tvg-logo=\"\",Senza logo\nhttps://a.invalid/2.m3u8\n\
                    #EXTINF:-1 group-title=\"Notizie\",Senza URL\n";
        let rai1 = channel("Rai 1 (1080p)", Some("https://logo.invalid/rai1.png"), Some("https://a.invalid/1.m3u8"), &[]);
        let without_url = Channel { group: Some("Notizie".into()), ..channel("Senza URL", None, None, &[]) };
        assert_eq!(parse(text), [rai1, channel("Senza logo", None, Some("https://a.invalid/2.m3u8"), &[]), without_url]);
    }

    #[test]
    fn vlc_options_give_the_headers_of_their_entry_only() {
        // As the iptv-org lists write them, with a `&` and a `#` in the referrer.
        let text = "#EXTM3U\n#EXTINF:-1 http-user-agent=\"HbbTV/1.6.1\",Uno\n#EXTVLCOPT:http-user-agent=HbbTV/1.6.1\n\
                    #EXTVLCOPT:network-caching=1000\nhttp://a.invalid/1.m3u8\n\
                    #EXTINF:-1,Due\n#EXTVLCOPT:http-referrer=https://b.invalid/odeon/#/?playlistId=0&videoId=0\nhttp://a.invalid/2.m3u8\n\
                    #EXTINF:-1,Tre\nhttp://a.invalid/3.m3u8\n";
        assert_eq!(
            parse(text),
            [
                channel("Uno", None, Some("http://a.invalid/1.m3u8"), &[("User-Agent", "HbbTV/1.6.1")]),
                channel(
                    "Due",
                    None,
                    Some("http://a.invalid/2.m3u8"),
                    &[("Referer", "https://b.invalid/odeon/#/?playlistId=0&videoId=0")]
                ),
                channel("Tre", None, Some("http://a.invalid/3.m3u8"), &[]),
            ]
        );
    }

    #[test]
    fn headers_after_the_url_are_decoded_and_dropped_from_it() {
        let text = "#EXTINF:-1,Uno\nhttp://a.invalid/1.m3u8|User-Agent=Mozilla%2F5.0%20(X11)&Referer=https%3A%2F%2Fb.invalid%2F%3Fa%3D1%26b%3D2\n\
                    #EXTINF:-1,Due\nhttp://a.invalid/2.m3u8?x=1|User-Agent=VLC/3.0|referrer=https://c.invalid/\n";
        assert_eq!(
            parse(text),
            [
                channel(
                    "Uno",
                    None,
                    Some("http://a.invalid/1.m3u8"),
                    &[("User-Agent", "Mozilla/5.0 (X11)"), ("Referer", "https://b.invalid/?a=1&b=2")]
                ),
                channel(
                    "Due",
                    None,
                    Some("http://a.invalid/2.m3u8?x=1"),
                    &[("User-Agent", "VLC/3.0"), ("Referer", "https://c.invalid/")]
                ),
            ]
        );
    }
}
