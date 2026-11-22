//! HLS playlists (RFC 8216): master and media playlists, attribute lists
//! and keys, parsed from text already downloaded.

use std::collections::HashMap;
use std::time::Duration;

use url::Url;

mod master;

pub use master::{Variant, parse_master};

/// How far behind the live edge playback starts, in target durations: the
/// segments in between are the margin against a slow network (RFC 8216
/// §6.3.3, as hls.js and Media3). Counted in time, not segments, so that
/// video and audio playlists start together even when their segments
/// differ in length.
const LIVE_EDGE_TARGETS: u32 = 3;
/// What a playlist may unpack to: RaiPlay's are 242 KB, a two-hour window
/// of 2 s segments is about 0.6 MB. The JSON a recipe reads is of the same
/// order.
pub(super) const MAX_BYTES: u64 = 4 * 1024 * 1024;
/// The shortest target duration: a lower one, zero included, would have
/// the playlist reloaded without pause (hls.js reads it as 1 s too).
const MIN_TARGET_DURATION: Duration = Duration::from_secs(1);
/// Why a playlist could not be read.
#[derive(Debug)]
pub enum Error {
    /// Not a playlist, or a damaged one: a later download may do better.
    Invalid(String),
    /// HLS features this client does not have.
    Unsupported(String),
}

impl From<url::ParseError> for Error {
    fn from(error: url::ParseError) -> Self {
        Self::Invalid(error.to_string())
    }
}

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
            return Err(Error::Unsupported("segmenti con BYTERANGE".into()));
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
        Err(Error::Unsupported("manifest DASH".into()))
    } else {
        Err(Error::Invalid("playlist non valida".into()))
    }
}

/// The initialization section of an EXT-X-MAP tag.
fn parse_map(attributes: &str, base: &Url) -> Result<Url, Error> {
    let map = parse_attributes(attributes);
    if map.contains_key("BYTERANGE") {
        return Err(Error::Unsupported("sezione di inizializzazione con BYTERANGE".into()));
    }
    let uri = map.get("URI").ok_or_else(|| Error::Invalid("EXT-X-MAP senza URI".into()))?;
    Ok(base.join(uri)?)
}

/// A media or discontinuity sequence number.
fn sequence(value: &str) -> Result<u64, Error> {
    value.parse().map_err(|_| Error::Invalid(format!("sequenza non valida: {value}")))
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
        .ok_or_else(|| Error::Invalid(format!("durata non valida: {value}")))
}

/// An RFC 3339 date and time (RFC 8216 §4.3.2.6), `2026-11-21T10:00:00.5Z`
/// or with an offset as `+01:00`, in seconds since the Unix epoch.
fn parse_date(text: &str) -> Option<f64> {
    let (day, time) = text.trim().split_once(['T', 't'])?;
    let mut day = day.splitn(3, '-').map(|n| n.parse::<i64>().ok());
    let (year, month, date) = (day.next()??, day.next()??, day.next()??);
    let (clock, offset) = split_offset(time)?;
    let mut clock = clock.splitn(3, ':');
    let (hour, minute) = (clock.next()?.parse::<i64>().ok()?, clock.next()?.parse::<i64>().ok()?);
    let second = clock.next()?.parse::<f64>().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&date) || hour > 23 || minute > 59 || !(0.0..61.0).contains(&second) {
        return None;
    }
    let days = days_since_epoch(year, month, date);
    Some((days * 86_400 + hour * 3_600 + minute * 60) as f64 + second - offset)
}

/// The clock of an RFC 3339 time and its offset from UTC, in seconds: `Z`,
/// or `+01:00`, `-0030`.
fn split_offset(time: &str) -> Option<(&str, f64)> {
    if let Some(clock) = time.strip_suffix(['Z', 'z']) {
        return Some((clock, 0.0));
    }
    let at = time.rfind(['+', '-'])?;
    let digits = time[at + 1..].replace(':', "");
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let offset = (digits[..2].parse::<f64>().ok()? * 60.0 + digits[2..].parse::<f64>().ok()?) * 60.0;
    Some((&time[..at], if time[at..].starts_with('-') { -offset } else { offset }))
}

/// Days from 1970-01-01 in the proleptic Gregorian calendar (H. Hinnant).
fn days_since_epoch(year: i64, month: i64, date: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year.rem_euclid(400);
    let of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + date - 1;
    era * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + of_year - 719_468
}

fn parse_key(attributes: &str, base: &Url) -> Result<Option<Key>, Error> {
    let attributes = parse_attributes(attributes);
    match attributes.get("METHOD").map(String::as_str) {
        Some("NONE") => Ok(None),
        Some("AES-128") => {
            let uri = attributes.get("URI").ok_or_else(|| Error::Invalid("chiave senza URI".into()))?;
            let iv = match attributes.get("IV") {
                Some(hex) => {
                    let hex = hex.trim_start_matches("0x").trim_start_matches("0X");
                    let value = u128::from_str_radix(hex, 16).map_err(|_| Error::Invalid(format!("IV non valido: {hex}")))?;
                    Some(value.to_be_bytes())
                }
                None => None,
            };
            Ok(Some(Key { url: base.join(uri)?, iv }))
        }
        other => Err(Error::Unsupported(format!("cifratura {other:?}"))),
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
    String::from_utf8(body).map_err(|_| Error::Invalid("playlist non UTF-8".into()))
}

#[cfg(test)]
mod tests;
