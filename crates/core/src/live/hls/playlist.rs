//! HLS playlists (RFC 8216): master and media playlists, attribute lists
//! and keys, parsed from text already downloaded.

use std::collections::HashMap;
use std::time::Duration;

use url::Url;

mod date;
mod error;
mod master;

use date::parse_date;
pub use error::{Error, Invalid, Unsupported};
pub use master::{Variant, parse_master};

/// How far behind the live edge playback starts, in target durations: the
/// segments in between are the margin against a slow network (RFC 8216
/// §6.3.3, as hls.js and Media3). Counted in time, not segments, so that
/// video and audio playlists start together even when their segments
/// differ in length.
const LIVE_EDGE_TARGETS: u32 = 3;
/// The shortest target duration: a lower one, zero included, would have
/// the playlist reloaded without pause (hls.js reads it as 1 s too).
const MIN_TARGET_DURATION: Duration = Duration::from_secs(1);
pub struct MediaPlaylist {
    /// At least MIN_TARGET_DURATION.
    pub target_duration: Duration,
    pub media_sequence: u64,
    pub segments: Vec<Segment>,
    pub ended: bool,
}

pub struct Segment {
    pub sequence: u64,
    pub url: Url,
    pub key: Option<Key>,
    /// Preceded by EXT-X-DISCONTINUITY: timestamps may restart.
    pub discontinuity: bool,
    /// EXT-X-DISCONTINUITY-SEQUENCE plus the discontinuities up to here.
    pub discontinuity_sequence: u64,
    /// fMP4 initialization section (EXT-X-MAP) in effect.
    pub init: Option<Url>,
    /// From EXTINF.
    pub duration: Duration,
    /// When its first sample was taken, in seconds since the Unix epoch:
    /// from the last EXT-X-PROGRAM-DATE-TIME and the durations since, when
    /// the playlist has one.
    pub date: Option<f64>,
}

#[derive(Clone)]
pub struct Key {
    pub url: Url,
    pub iv: Option<[u8; 16]>,
}

/// Whether `head`, the start of an answer, is a playlist: `#EXTM3U` first,
/// after a BOM or blank lines.
pub fn is_playlist(head: &[u8]) -> bool {
    let text = head.strip_prefix("\u{feff}".as_bytes()).unwrap_or(head);
    text.trim_ascii_start().starts_with(b"#EXTM3U")
}

impl MediaPlaylist {
    pub fn parse(text: &str, base: &Url) -> Result<Self, Error> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        check_playlist(text)?;
        let mut playlist = Self { target_duration: Duration::from_secs(2), media_sequence: 0, segments: Vec::new(), ended: false };
        let mut next = NextSegment::default();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if line.starts_with('#') {
                playlist.read_tag(line, base, &mut next)?;
            } else {
                let sequence = playlist.media_sequence + playlist.segments.len() as u64;
                playlist.segments.push(next.segment(sequence, base.join(line)?));
            }
        }
        Ok(playlist)
    }

    /// Applies the tag on `line`, to the playlist or to the segments after
    /// it (`next`); other tags and comments are skipped.
    fn read_tag(&mut self, line: &str, base: &Url, next: &mut NextSegment) -> Result<(), Error> {
        if let Some(value) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            self.target_duration = seconds(value)?.max(MIN_TARGET_DURATION);
        } else if let Some(value) = line.strip_prefix("#EXT-X-DISCONTINUITY-SEQUENCE:") {
            next.discontinuity_sequence = sequence(value)?;
        } else if let Some(value) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            self.media_sequence = sequence(value)?;
        } else if let Some(value) = line.strip_prefix("#EXT-X-KEY:") {
            next.key = parse_key(value, base)?;
        } else if let Some(value) = line.strip_prefix("#EXT-X-PROGRAM-DATE-TIME:") {
            // A date this client cannot read only loses the alignment it
            // would give.
            next.date = parse_date(value);
        } else if line == "#EXT-X-ENDLIST" {
            self.ended = true;
        } else if let Some(value) = line.strip_prefix("#EXT-X-MAP:") {
            next.init = Some(parse_map(value, base)?);
        } else if line.starts_with("#EXT-X-BYTERANGE:") {
            return Err(Error::Unsupported(Unsupported::ByteRange));
        } else if let Some(value) = line.strip_prefix("#EXTINF:") {
            next.duration = seconds(value.split(',').next().unwrap_or_default())?;
        } else if line == "#EXT-X-DISCONTINUITY" {
            next.discontinuity = true;
            next.discontinuity_sequence += 1;
        }
        Ok(())
    }

    /// Sequence number of the last segment that starts at least
    /// LIVE_EDGE_TARGETS target durations from the end; the first one when
    /// the playlist is shorter.
    pub fn live_start(&self) -> u64 {
        let margin = self.target_duration.saturating_mul(LIVE_EDGE_TARGETS);
        let mut behind = Duration::ZERO;
        for segment in self.segments.iter().rev() {
            behind = behind.saturating_add(segment.duration);
            if behind >= margin {
                return segment.sequence;
            }
        }
        self.media_sequence
    }
}

/// What the tags read so far give the next segment.
#[derive(Default)]
struct NextSegment {
    key: Option<Key>,
    /// Preceded by EXT-X-DISCONTINUITY.
    discontinuity: bool,
    discontinuity_sequence: u64,
    init: Option<Url>,
    duration: Duration,
    date: Option<f64>,
}

impl NextSegment {
    /// The segment at `url`, numbered `sequence`; the one after it follows
    /// it in time and starts no section of its own.
    fn segment(&mut self, sequence: u64, url: Url) -> Segment {
        let segment = Segment {
            sequence,
            url,
            key: self.key.clone(),
            discontinuity: self.discontinuity,
            discontinuity_sequence: self.discontinuity_sequence,
            init: self.init.clone(),
            duration: self.duration,
            date: self.date,
        };
        self.date = self.date.map(|date| date + self.duration.as_secs_f64());
        self.discontinuity = false;
        segment
    }
}

/// An error unless `text` is a playlist; a DASH manifest is a format of its
/// own, not a damaged answer worth retrying.
fn check_playlist(text: &str) -> Result<(), Error> {
    if is_playlist(text.as_bytes()) {
        Ok(())
    } else if text.contains("<MPD") {
        Err(Error::Unsupported(Unsupported::Dash))
    } else {
        Err(Error::Invalid(Invalid::NotPlaylist))
    }
}

/// The initialization section of an EXT-X-MAP tag.
fn parse_map(attributes: &str, base: &Url) -> Result<Url, Error> {
    let map = parse_attributes(attributes);
    if map.contains_key("BYTERANGE") {
        return Err(Error::Unsupported(Unsupported::MapByteRange));
    }
    let uri = map.get("URI").ok_or(Error::Invalid(Invalid::MapWithoutUri))?;
    Ok(base.join(uri)?)
}

/// A media or discontinuity sequence number.
fn sequence(value: &str) -> Result<u64, Error> {
    value.parse().map_err(|_| Error::Invalid(Invalid::Sequence(value.into())))
}

/// A duration in seconds (RFC 8216 §4.3.2.1, §4.3.3.1: a decimal integer
/// or floating point, never negative). Media3 refuses a playlist with
/// another, and so does this: a later reload may do better. A value too
/// large to be a Duration is refused too.
fn seconds(value: &str) -> Result<Duration, Error> {
    let value = value.trim();
    value
        .parse()
        .ok()
        .and_then(|seconds| Duration::try_from_secs_f64(seconds).ok())
        .ok_or_else(|| Error::Invalid(Invalid::Duration(value.into())))
}

fn parse_key(attributes: &str, base: &Url) -> Result<Option<Key>, Error> {
    let attributes = parse_attributes(attributes);
    match attributes.get("METHOD").map(String::as_str) {
        Some("NONE") => Ok(None),
        Some("AES-128") => {
            let uri = attributes.get("URI").ok_or(Error::Invalid(Invalid::KeyWithoutUri))?;
            let iv = match attributes.get("IV") {
                Some(hex) => {
                    let hex = hex.trim_start_matches("0x").trim_start_matches("0X");
                    let value = u128::from_str_radix(hex, 16).map_err(|_| Error::Invalid(Invalid::Iv(hex.into())))?;
                    Some(value.to_be_bytes())
                }
                None => None,
            };
            Ok(Some(Key { url: base.join(uri)?, iv }))
        }
        other => Err(Error::Unsupported(Unsupported::Encryption(other.unwrap_or("?").into()))),
    }
}

/// `NAME=value,NAME="quoted, value"` attribute lists.
fn parse_attributes(text: &str) -> HashMap<String, String> {
    let mut attributes = HashMap::new();
    let mut rest = text;
    while let Some((name, after)) = rest.split_once('=') {
        let (value, next) = match after.strip_prefix('"') {
            Some(quoted) => {
                let end = quoted.find('"').unwrap_or(quoted.len());
                (&quoted[..end], quoted[end..].trim_start_matches('"'))
            }
            None => after.split_once(',').map_or((after, ""), |(value, next)| (value, next)),
        };
        attributes.insert(name.trim().to_string(), value.to_string());
        rest = next.trim_start_matches(',');
    }
    attributes
}

pub fn text(body: Vec<u8>) -> Result<String, Error> {
    String::from_utf8(body).map_err(|_| Error::Invalid(Invalid::NotUtf8))
}

#[cfg(test)]
mod tests;
