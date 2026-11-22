//! Demuxing, section by section: one input, or a video input and an audio
//! rendition of its own merged in timestamp order, and the formats the
//! decoder must be loaded for.

use super::Error;
use super::ffmpeg::{self, Media, Read};
use crate::log::{debug, log};

/// Start gap between an audio rendition and the video that is not trimmed:
/// segment boundaries of the two differ by up to an audio frame, and video
/// decode timestamps run ahead of presentation by a few frames.
const ALIGN_TOLERANCE_US: i64 = 200_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VideoCodec {
    H264,
    H265,
}

/// Audio the decoder takes as it is. MPEG audio arrives transcoded to AAC
/// by the shim.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AudioCodec {
    Aac,
    Ac3,
    Eac3,
}

/// What the decoder is loaded for.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Formats {
    pub video: VideoCodec,
    pub width: i32,
    pub height: i32,
    /// None when the stream has no audio the decoder takes.
    pub audio: Option<AudioCodec>,
}

impl Formats {
    /// The formats of the demuxer's current sections, and their description.
    pub fn of(demux: &Demux) -> Result<(Self, String), Error> {
        let info = &demux.main.info;
        let video = match info.video_codec {
            ffmpeg::CODEC_H264 => VideoCodec::H264,
            ffmpeg::CODEC_HEVC => VideoCodec::H265,
            _ => return Err(Error::Unsupported(format!("codec video non supportato: {}", info.describe()))),
        };
        // An audio rendition of its own replaces any muxed audio.
        let audio_info = demux.audio.as_ref().map_or(info, |a| &a.info);
        let audio = match audio_info.audio_codec {
            ffmpeg::CODEC_AAC => Some(AudioCodec::Aac),
            ffmpeg::CODEC_AC3 => Some(AudioCodec::Ac3),
            ffmpeg::CODEC_EAC3 => Some(AudioCodec::Eac3),
            _ => None,
        };
        let description = match &demux.audio {
            Some(audio) => format!("{} · traccia audio separata {}", info.describe_video(), audio.info.describe_audio()),
            None => info.describe(),
        };
        if audio_info.has_audio() && audio.is_none() {
            log!("audio non supportato, riproduzione senza audio: {description}");
        }
        Ok((Self { video, width: info.width, height: info.height, audio }, description))
    }

    /// Whether `next` needs a decoder this one was not loaded with. A new
    /// size does not (the decoder adapts), nor audio going away (silence
    /// takes its place).
    pub fn need_reload_for(&self, next: &Self) -> bool {
        next.video != self.video || next.audio.is_some_and(|audio| self.audio != Some(audio))
    }
}

/// A demuxed packet, copied out of FFmpeg's buffer so that packets of two
/// demuxers can be held at once. `section` is the discontinuity sequence
/// number of the section the packet comes from: timestamps compare only
/// within one.
pub enum Packet {
    Video { data: Vec<u8>, pts_us: Option<i64>, dts_us: Option<i64>, keyframe: bool, section: u64 },
    Audio { data: Vec<u8>, pts_us: Option<i64>, section: u64 },
    Eof,
}

impl From<Read<'_>> for Packet {
    fn from(read: Read<'_>) -> Self {
        match read {
            Read::Video { data, pts_us, dts_us, keyframe } => {
                Self::Video { data: data.to_vec(), pts_us, dts_us, keyframe, section: 0 }
            }
            Read::Audio { data, pts_us } => Self::Audio { data: data.to_vec(), pts_us, section: 0 },
            Read::Eof => Self::Eof,
        }
    }
}

impl Packet {
    fn in_section(mut self, sequence: u64) -> Self {
        if let Self::Video { section, .. } | Self::Audio { section, .. } = &mut self {
            *section = sequence;
        }
        self
    }
}

/// The main demuxer and, for an HLS audio rendition, a second one whose
/// packets are merged with the video in timestamp order.
pub struct Demux {
    main: Media,
    audio: Option<Media>,
    next_video: Option<Packet>,
    next_audio: Option<Packet>,
    /// Whether the rendition and the video now start together.
    aligned: bool,
    /// Whether an input moved on to a new section since the last check.
    new_section: bool,
}

impl Demux {
    pub fn new(main: Media, audio: Option<Media>) -> Self {
        Self { main, audio, next_video: None, next_audio: None, aligned: false, new_section: false }
    }

    pub fn read(&mut self) -> Result<Packet, String> {
        if self.audio.is_none() {
            return read_sections(&mut self.main, Input::Whole, &mut self.new_section);
        }
        self.fill()?;
        if !self.aligned {
            self.align()?;
        }
        // Timestamps restart at a discontinuity: across one, the input still
        // before it goes first, until it crosses too.
        let sections = (self.main.discontinuity_sequence(), self.audio.as_ref().map_or(0, Media::discontinuity_sequence));
        let video_first = match (&self.next_video, &self.next_audio) {
            (Some(Packet::Eof), Some(Packet::Eof)) => return Ok(Packet::Eof),
            (Some(Packet::Eof), _) => false,
            (Some(Packet::Video { .. }), Some(Packet::Audio { .. })) if sections.0 != sections.1 => sections.0 < sections.1,
            (Some(Packet::Video { pts_us, dts_us, .. }), Some(Packet::Audio { pts_us: audio, .. })) => {
                dts_us.or(*pts_us).unwrap_or(i64::MIN) <= audio.unwrap_or(i64::MIN)
            }
            _ => true,
        };
        let next = if video_first { &mut self.next_video } else { &mut self.next_audio };
        Ok(next.take().unwrap_or(Packet::Eof))
    }

    /// Back to the start of a local file, to loop it.
    pub fn rewind(&mut self) -> Result<(), String> {
        self.main.rewind()
    }

    /// Whether an input moved on to a new section, whose formats may differ,
    /// since the last call.
    pub fn take_new_section(&mut self) -> bool {
        std::mem::take(&mut self.new_section)
    }

    /// Whether the current section has no audio, although the stream had.
    pub fn section_without_audio(&self) -> bool {
        !self.audio.as_ref().unwrap_or(&self.main).info.has_audio()
    }

    /// Separate playlists can start a segment apart, when one gained a
    /// segment between the two reads. Drops the head of the earlier input
    /// until both start together: audio up to the first video packet, video
    /// up to the keyframe where the audio starts.
    fn align(&mut self) -> Result<(), String> {
        let mut dropped = (0, 0);
        loop {
            let video = match &self.next_video {
                Some(Packet::Video { pts_us, dts_us, keyframe, .. }) => dts_us.or(*pts_us).map(|t| (t, *keyframe)),
                _ => None,
            };
            let audio = match &self.next_audio {
                Some(Packet::Audio { pts_us, .. }) => *pts_us,
                _ => None,
            };
            let (Some((video, keyframe)), Some(audio)) = (video, audio) else { break };
            if audio + ALIGN_TOLERANCE_US < video {
                self.next_audio = None;
                dropped.1 += 1;
            } else if video + ALIGN_TOLERANCE_US < audio || (dropped.0 > 0 && !keyframe) {
                self.next_video = None;
                dropped.0 += 1;
            } else {
                break;
            }
            self.fill()?;
        }
        if dropped != (0, 0) {
            log!("traccia audio e video allineate all'avvio: scartati {} pacchetti video e {} audio", dropped.0, dropped.1);
        }
        self.aligned = true;
        Ok(())
    }

    /// Reads the next packet of each input that has none waiting.
    fn fill(&mut self) -> Result<(), String> {
        let Some(audio) = self.audio.as_mut() else { return Ok(()) };
        if self.next_video.is_none() {
            // Muxed audio: the rendition replaces it.
            let muxed_audio = |p: &Packet| matches!(p, Packet::Audio { .. });
            self.next_video = Some(read_skipping(&mut self.main, Input::Video, &mut self.new_section, muxed_audio)?);
        }
        if self.next_audio.is_none() {
            let video = |p: &Packet| matches!(p, Packet::Video { .. });
            self.next_audio = Some(read_skipping(audio, Input::Audio, &mut self.new_section, video)?);
        }
        Ok(())
    }
}

/// The next packet of `media` that `skipped` does not drop.
fn read_skipping(media: &mut Media, input: Input, new_section: &mut bool, skipped: fn(&Packet) -> bool) -> Result<Packet, String> {
    loop {
        let packet = read_sections(media, input, new_section)?;
        if !skipped(&packet) {
            return Ok(packet);
        }
    }
}

/// What an input carries of the stream.
#[derive(Clone, Copy)]
enum Input {
    /// Video and any muxed audio.
    Whole,
    /// The video, beside an audio rendition that replaces any muxed audio.
    Video,
    /// An audio rendition.
    Audio,
}

impl Input {
    /// The formats of `media`'s section that the stream plays.
    fn describe(self, media: &Media) -> String {
        match self {
            Self::Whole => media.info.describe(),
            Self::Video => format!("video {}", media.info.describe_video()),
            Self::Audio => format!("traccia audio separata {}", media.info.describe_audio()),
        }
    }
}

/// Reads the next packet of `media`, moving on through the sections of its
/// input: at a discontinuity streams and formats may change (Pluto TV's ad
/// clips come with their own PIDs, some without audio).
fn read_sections(media: &mut Media, input: Input, new_section: &mut bool) -> Result<Packet, String> {
    loop {
        match Packet::from(media.read()?) {
            Packet::Eof if media.next_section()? => {
                debug!("nuova sezione: {}", input.describe(media));
                *new_section = true;
            }
            packet => return Ok(packet.in_section(media.discontinuity_sequence())),
        }
    }
}
