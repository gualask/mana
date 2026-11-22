//! Silent AAC frames for the stretches where a stream that has audio
//! carries none (Pluto TV's ad clips, skipped audio segments): the TV's
//! pipeline, loaded with an audio track, beeps when it runs out of audio.

use crate::adts::{self, HEADER_BYTES};

/// AAC-LC frames that decode to silence, by channel configuration, as
/// hls.js inserts them (`AAC.getSilentFrame`, Apache 2.0).
const SILENT_MONO: &[u8] = &[0x00, 0xc8, 0x00, 0x80, 0x23, 0x80];
const SILENT_STEREO: &[u8] = &[0x21, 0x00, 0x49, 0x90, 0x02, 0x19, 0x00, 0x23, 0x80];
const SAMPLES_PER_FRAME: u64 = 1024;

/// One silent ADTS frame matching the format of the stream's audio.
pub struct Silence {
    frame: Vec<u8>,
    duration_us: i64,
}

impl Silence {
    /// From a real ADTS frame of the stream; None for formats without a
    /// silent frame (more than two channels, a channel layout in the
    /// stream itself).
    pub fn matching(adts: &[u8]) -> Option<Self> {
        let rate = adts::header(adts)?.rate;
        let rate_index = (adts[2] >> 2) & 0x0F;
        let channels = ((adts[2] & 0x01) << 2) | (adts[3] >> 6);
        let payload = match channels {
            1 => SILENT_MONO,
            2 => SILENT_STEREO,
            _ => return None,
        };
        let length = HEADER_BYTES + payload.len();
        let mut frame = vec![
            0xFF,
            0xF1, // MPEG-4, no CRC
            (1 << 6) | (rate_index << 2) | (channels >> 2), // AAC-LC
            ((channels & 0x03) << 6) | (length >> 11) as u8,
            (length >> 3) as u8,
            ((length & 0x07) << 5) as u8 | 0x1F, // buffer fullness 0x7FF: variable rate
            0xFC,                                // one raw data block
        ];
        frame.extend_from_slice(payload);
        Some(Self { frame, duration_us: (SAMPLES_PER_FRAME * 1_000_000 / rate) as i64 })
    }

    /// Whether `adts` is a frame of the same sample rate and channels.
    pub fn matches(&self, adts: &[u8]) -> bool {
        // Sampling index and channel bits of the ADTS header.
        adts.len() >= HEADER_BYTES && adts[2] & 0x3D == self.frame[2] & 0x3D && adts[3] >> 6 == self.frame[3] >> 6
    }

    pub fn frame(&self) -> &[u8] {
        &self.frame
    }

    pub fn duration_us(&self) -> i64 {
        self.duration_us
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADTS header of a 48 kHz frame with the given channel configuration.
    fn header(channels: u8) -> [u8; 7] {
        [0xFF, 0xF1, (1 << 6) | (3 << 2) | (channels >> 2), (channels & 3) << 6, 0x10, 0x1F, 0xFC]
    }

    #[test]
    fn silent_frame_keeps_the_stream_format() {
        let silence = Silence::matching(&header(2)).unwrap();
        let frame = silence.frame();
        assert_eq!(&frame[..3], &header(2)[..3]);
        assert_eq!(frame[3] >> 6, 2);
        let length = ((frame[3] as usize & 0x03) << 11) | ((frame[4] as usize) << 3) | (frame[5] as usize >> 5);
        assert_eq!(length, frame.len());
        assert_eq!(&frame[7..], SILENT_STEREO);
        assert_eq!(silence.duration_us(), 21_333);
        assert!(silence.matches(&header(2)));
        assert!(!silence.matches(&header(1)));
    }

    #[test]
    fn no_silent_frame_for_other_layouts_or_non_adts() {
        assert!(Silence::matching(&header(6)).is_none());
        assert!(Silence::matching(&header(0)).is_none());
        assert!(Silence::matching(b"\x0b\x77ac3 frame").is_none());
    }
}
