//! Master playlists (RFC 8216 §4.3.4): the variants to switch between and
//! the audio rendition they share, or the media playlist served in their
//! place.

use std::collections::HashMap;

use url::Url;

use super::{Error, MediaPlaylist, parse_attributes};

/// One variant of a master playlist: the media playlist of its video, with
/// any muxed audio.
#[derive(Clone)]
pub struct Variant {
    /// Peak bits per second, as declared (BANDWIDTH).
    pub bandwidth: u64,
    /// As declared (RESOLUTION), for the log.
    pub resolution: Option<String>,
    pub url: Url,
}

impl Variant {
    /// Lines of its picture, from RESOLUTION.
    pub fn height(&self) -> Option<u32> {
        self.resolution.as_deref()?.split_once(['x', 'X'])?.1.parse().ok()
    }
}

/// The variants of a master playlist to switch between, and the audio
/// rendition they share, if it has a URI of its own.
pub struct Variants {
    /// Lightest first; a media playlist served in place of a master is its
    /// only variant.
    pub video: Vec<Variant>,
    pub audio: Option<Url>,
    /// The media playlist served in place of a master, already read.
    pub media: Option<MediaPlaylist>,
}

/// The variants to switch between in a master playlist: those not declared
/// audio-only, when there are any (one would blank the picture, as hls.js
/// filters them), in
/// the audio group and video codec of the best one, so that switching keeps
/// the audio rendition and the decoder. Variants with the same bandwidth
/// are redundant copies: the first one stands for them. The audio is the
/// default rendition of the group, else its first.
pub fn parse_master(text: &str, master: &Url) -> Result<Variants, Error> {
    let (mut audio_groups, mut declared) = declarations(text, master)?;
    if declared.iter().any(|d| !d.audio_only()) {
        declared.retain(|d| !d.audio_only());
    }
    // The first of the highest bandwidth.
    let Some(best) = declared.iter().rev().max_by_key(|d| d.variant.bandwidth) else {
        let only = Variant { bandwidth: 0, resolution: None, url: master.clone() };
        return Ok(Variants { video: vec![only], audio: None, media: Some(MediaPlaylist::parse(text, master)?) });
    };
    let (group, codec) = (best.audio_group.clone(), best.video_codec());
    let video = variants_like(declared, &group, codec);
    // One without a URI means the audio is muxed into the video segments.
    let audio = group
        .and_then(|g| audio_groups.remove(&g))
        .and_then(|renditions| default_uri(&renditions))
        .map(|uri| master.join(&uri))
        .transpose()?;
    Ok(Variants { video, audio, media: None })
}

/// Audio group -> (default rendition, URI) of its renditions.
type AudioGroups = HashMap<String, Vec<(bool, Option<String>)>>;

/// The audio renditions and the variants a master playlist declares.
fn declarations(text: &str, master: &Url) -> Result<(AudioGroups, Vec<Declared>), Error> {
    let mut audio_groups = AudioGroups::new();
    let mut declared = Vec::new();
    let mut lines = text.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if let Some(media) = line.strip_prefix("#EXT-X-MEDIA:")
            && let Some((group, rendition)) = audio_rendition(media)
        {
            audio_groups.entry(group).or_default().push(rendition);
            continue;
        }
        let Some(attributes) = line.strip_prefix("#EXT-X-STREAM-INF:") else { continue };
        let Some(uri) = lines.find(|l| !l.is_empty() && !l.starts_with('#')) else { break };
        declared.push(Declared::new(parse_attributes(attributes), master.join(uri)?));
    }
    Ok((audio_groups, declared))
}

/// The group and the (default, URI) of an EXT-X-MEDIA tag of audio.
fn audio_rendition(attributes: &str) -> Option<(String, (bool, Option<String>))> {
    let media = parse_attributes(attributes);
    if media.get("TYPE").is_none_or(|t| t != "AUDIO") {
        return None;
    }
    let default = media.get("DEFAULT").is_some_and(|d| d == "YES");
    Some((media.get("GROUP-ID")?.clone(), (default, media.get("URI").cloned())))
}

/// The variants declared in `group` with the video codec family `codec`,
/// lightest first, the first of each bandwidth.
fn variants_like(declared: Vec<Declared>, group: &Option<String>, codec: Option<&str>) -> Vec<Variant> {
    let same_codec = |d: &Declared| d.video_codec().is_none() || codec.is_none() || d.video_codec() == codec;
    let mut video: Vec<Variant> = Vec::new();
    for d in declared.into_iter().filter(|d| &d.audio_group == group && same_codec(d)) {
        if !video.iter().any(|v| v.bandwidth == d.variant.bandwidth) {
            video.push(d.variant);
        }
    }
    video.sort_by_key(|v| v.bandwidth);
    video
}

/// The URI of a group's default rendition, else of its first.
fn default_uri(renditions: &[(bool, Option<String>)]) -> Option<String> {
    renditions.iter().find(|(default, _)| *default).or(renditions.first()).and_then(|(_, uri)| uri.clone())
}

/// A variant as the master playlist declares it.
struct Declared {
    variant: Variant,
    codecs: Option<String>,
    audio_group: Option<String>,
}

impl Declared {
    /// The variant at `url`, from the attributes of its EXT-X-STREAM-INF.
    fn new(mut attributes: HashMap<String, String>, url: Url) -> Self {
        let variant = Variant {
            bandwidth: attributes.get("BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0),
            resolution: attributes.remove("RESOLUTION"),
            url,
        };
        Self { variant, codecs: attributes.remove("CODECS"), audio_group: attributes.remove("AUDIO") }
    }

    /// The family of its video codec, when CODECS lists one.
    fn video_codec(&self) -> Option<&'static str> {
        self.codecs.as_deref().and_then(video_codec)
    }

    /// CODECS without a video codec, and no RESOLUTION.
    fn audio_only(&self) -> bool {
        self.codecs.is_some() && self.video_codec().is_none() && self.variant.resolution.is_none()
    }
}

/// The family of the video codec in a CODECS list, as RFC 6381 names it.
fn video_codec(codecs: &str) -> Option<&'static str> {
    codecs.split(',').map(str::trim).find_map(|codec| match codec.split('.').next()? {
        "avc1" | "avc3" => Some("avc"),
        "hvc1" | "hev1" => Some("hevc"),
        "dvh1" | "dvhe" => Some("dolby-vision"),
        "av01" => Some("av1"),
        "vp09" => Some("vp9"),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_keeps_the_variants_of_the_best_one_and_its_audio_rendition() {
        let master = Url::parse("https://lab.invalid/live3/master.m3u8").unwrap();
        let text = "#EXTM3U\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"it\",DEFAULT=NO,URI=\"audio_it.m3u8\"\n\
            #EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"aud\",NAME=\"en\",DEFAULT=YES,URI=\"audio_en.m3u8\"\n\
            #EXT-X-STREAM-INF:BANDWIDTH=3000000,CODECS=\"avc1.64001f,mp4a.40.2\",RESOLUTION=1920x1080,AUDIO=\"aud\"\nhigh.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=800000,AUDIO=\"aud\"\nlow.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=800000,AUDIO=\"aud\"\nlow-backup.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=1500000,CODECS=\"hvc1.1.6.L93.B0\",AUDIO=\"aud\"\nhevc.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=1200000,CODECS=\"avc1.4d401e\",AUDIO=\"other\"\nother-audio.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=64000,CODECS=\"mp4a.40.5\"\naudio-only.m3u8\n";
        let variants = parse_master(text, &master).unwrap();
        let urls: Vec<&str> = variants.video.iter().map(|v| v.url.path()).collect();
        assert_eq!(urls, ["/live3/low.m3u8", "/live3/high.m3u8"]);
        assert_eq!(variants.video[1].resolution.as_deref(), Some("1920x1080"));
        assert_eq!(variants.audio.unwrap().as_str(), "https://lab.invalid/live3/audio_en.m3u8");

        let muxed = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"main\",DEFAULT=YES\n\
            #EXT-X-STREAM-INF:BANDWIDTH=1,AUDIO=\"a\"\nv.m3u8\n";
        assert!(parse_master(muxed, &master).unwrap().audio.is_none());
        let media = parse_master("#EXTM3U\n#EXTINF:2,\na.ts\n", &master).unwrap();
        assert_eq!(media.video.len(), 1);
        assert_eq!(media.video[0].url, master);
        assert_eq!(media.media.unwrap().segments.len(), 1);
        assert!(parse_master(text, &master).unwrap().media.is_none());
    }

    #[test]
    fn audio_only_variants_stay_when_no_variant_declares_video() {
        let master = Url::parse("https://lab.invalid/radio/master.m3u8").unwrap();
        let text = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=64000,CODECS=\"mp4a.40.5\"\nlow.m3u8\n\
            #EXT-X-STREAM-INF:BANDWIDTH=128000,CODECS=\"mp4a.40.2\"\nhigh.m3u8\n";
        assert_eq!(parse_master(text, &master).unwrap().video.len(), 2);
    }
}
