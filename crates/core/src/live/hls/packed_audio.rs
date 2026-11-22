//! HLS packed audio (RFC 8216 §3.4): each segment is raw ADTS frames, timed
//! only by an ID3 PRIV frame at its start. The demuxer reads a playlist as
//! one continuous byte stream, where those per-segment timestamps would be
//! lost, so each segment is rewrapped as MPEG-TS with every frame stamped.

use super::ts;
use crate::adts;

/// ID3 PRIV owner of the segment's first timestamp (90 kHz MPEG-TS clock).
const TIMESTAMP_OWNER: &[u8] = b"com.apple.streaming.transportStreamTimestamp\0";
const PAT_PID: u16 = 0;
const PMT_PID: u16 = 0x1000;
const AUDIO_PID: u16 = 0x100;
/// ISO/IEC 13818-7 audio with ADTS transport syntax.
const STREAM_TYPE_ADTS: u8 = 0x0F;

/// Whether a downloaded segment is packed audio rather than MPEG-TS or MP4.
pub fn is_packed_audio(segment: &[u8]) -> bool {
    segment.starts_with(b"ID3") || adts::header(segment).is_some()
}

/// Rewraps packed-audio segments of one playlist; continuity counters run on
/// across segments, as the demuxer sees them back to back.
#[derive(Default)]
pub struct TsRewrapper {
    continuity: [u8; 3],
}

impl TsRewrapper {
    /// Errors are segments this rewrapper cannot read: retrying cannot help.
    pub fn rewrap(&mut self, segment: &[u8]) -> Result<Vec<u8>, String> {
        let (timestamp, mut rest) = read_id3_tags(segment);
        let start = timestamp.ok_or("audio packed senza timestamp ID3")?;
        let mut out = Vec::with_capacity(segment.len() * 5 / 4);
        self.psi(&mut out, PAT_PID, 0, &pat());
        self.psi(&mut out, PMT_PID, 1, &pmt());
        let mut samples = 0u64;
        while !rest.is_empty() {
            if rest.starts_with(b"ID3") {
                rest = read_id3_tags(rest).1;
                continue;
            }
            let Some(adts::Header { length, rate, samples: frame_samples }) = adts::header(rest) else {
                if samples == 0 {
                    return Err("audio packed non AAC".into());
                }
                rest = &rest[1..]; // damaged frame: look for the next one
                continue;
            };
            let length = length.min(rest.len());
            let pts = (start + samples * 90_000 / rate) & ((1 << 33) - 1);
            self.pes(&mut out, pts, &rest[..length]);
            samples += frame_samples;
            rest = &rest[length..];
        }
        Ok(out)
    }

    /// One PSI section in a single packet, stuffed with 0xFF.
    fn psi(&mut self, out: &mut Vec<u8>, pid: u16, counter: usize, section: &[u8]) {
        let start = out.len();
        self.header(out, pid, counter, true, 0b01);
        out.push(0); // pointer field
        out.extend_from_slice(section);
        out.resize(start + ts::PACKET, 0xFF);
    }

    fn pes(&mut self, out: &mut Vec<u8>, pts: u64, frame: &[u8]) {
        let mut pes = Vec::with_capacity(frame.len() + 14);
        pes.extend_from_slice(&[0, 0, 1, 0xC0]);
        pes.extend_from_slice(&((frame.len() + 8) as u16).to_be_bytes());
        pes.extend_from_slice(&[0x80, 0x80, 5]);
        pes.extend_from_slice(&[
            0x21 | ((pts >> 29) as u8 & 0x0E),
            (pts >> 22) as u8,
            (pts >> 14) as u8 | 1,
            (pts >> 7) as u8,
            (pts << 1) as u8 | 1,
        ]);
        pes.extend_from_slice(frame);
        for (index, chunk) in pes.chunks(ts::PACKET - 4).enumerate() {
            let full = chunk.len() == ts::PACKET - 4;
            self.header(out, AUDIO_PID, 2, index == 0, if full { 0b01 } else { 0b11 });
            if !full {
                // Adaptation field stuffing fills the last packet.
                let field = ts::PACKET - 5 - chunk.len();
                out.push(field as u8);
                if field > 0 {
                    out.push(0);
                    out.extend(std::iter::repeat_n(0xFF, field - 1));
                }
            }
            out.extend_from_slice(chunk);
        }
    }

    fn header(&mut self, out: &mut Vec<u8>, pid: u16, counter: usize, unit_start: bool, adaptation: u8) {
        let continuity = self.continuity[counter];
        self.continuity[counter] = (continuity + 1) & 0x0F;
        out.extend_from_slice(&[
            ts::SYNC,
            (u8::from(unit_start) << 6) | (pid >> 8) as u8,
            pid as u8,
            (adaptation << 4) | continuity,
        ]);
    }
}

/// Skips the ID3v2 tags at the start of `data`, returning the timestamp
/// found in them, if any, and what follows.
fn read_id3_tags(mut data: &[u8]) -> (Option<u64>, &[u8]) {
    let mut timestamp = None;
    while data.len() >= 10 && data.starts_with(b"ID3") {
        let version = data[3];
        let footer = if data[5] & 0x10 != 0 { 10 } else { 0 };
        let end = (10 + syncsafe(&data[6..10]) + footer).min(data.len());
        let mut frames = &data[10..end];
        while frames.len() >= 10 && frames[0] != 0 {
            let size = if version >= 4 { syncsafe(&frames[4..8]) } else { u32_be(&frames[4..8]) };
            let body = &frames[10..(10 + size).min(frames.len())];
            if &frames[..4] == b"PRIV" && body.len() >= TIMESTAMP_OWNER.len() + 8 && body.starts_with(TIMESTAMP_OWNER) {
                let value = &body[TIMESTAMP_OWNER.len()..TIMESTAMP_OWNER.len() + 8];
                timestamp = Some(u64::from_be_bytes(value.try_into().unwrap()) & ((1 << 33) - 1));
            }
            frames = &frames[(10 + size).min(frames.len())..];
        }
        data = &data[end..];
    }
    (timestamp, data)
}

fn syncsafe(bytes: &[u8]) -> usize {
    bytes.iter().fold(0, |value, b| (value << 7) | (*b as usize & 0x7F))
}

fn u32_be(bytes: &[u8]) -> usize {
    u32::from_be_bytes(bytes.try_into().unwrap()) as usize
}

fn pat() -> Vec<u8> {
    section(0x00, &[0x00, 0x01, 0xE0 | (PMT_PID >> 8) as u8, PMT_PID as u8])
}

fn pmt() -> Vec<u8> {
    section(
        0x02,
        &[0xFF, 0xFF, 0xF0, 0x00, STREAM_TYPE_ADTS, 0xE0 | (AUDIO_PID >> 8) as u8, AUDIO_PID as u8, 0xF0, 0x00],
    )
}

/// A PSI section of program/stream 1, version 0, with its CRC.
fn section(table: u8, payload: &[u8]) -> Vec<u8> {
    let length = 5 + payload.len() + 4;
    let mut section = vec![table, 0xB0 | (length >> 8) as u8, length as u8, 0x00, 0x01, 0xC1, 0x00, 0x00];
    section.extend_from_slice(payload);
    let crc = crc32_mpeg(&section);
    section.extend_from_slice(&crc.to_be_bytes());
    section
}

fn crc32_mpeg(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= (*byte as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id3_with_timestamp(timestamp: u64) -> Vec<u8> {
        let mut body = TIMESTAMP_OWNER.to_vec();
        body.extend_from_slice(&timestamp.to_be_bytes());
        let mut frame = b"PRIV".to_vec();
        frame.extend_from_slice(&[0, 0, 0, body.len() as u8, 0, 0]);
        frame.extend_from_slice(&body);
        let mut tag = b"ID3\x04\x00\x00\x00\x00\x00".to_vec();
        tag.push(frame.len() as u8);
        tag.extend_from_slice(&frame);
        tag
    }

    /// An ADTS frame of `length` bytes at 24 kHz (index 6), as HE-AAC sends it.
    fn adts(length: usize) -> Vec<u8> {
        let mut frame = vec![0xFF, 0xF1, 0x58, 0x80 | (length >> 11) as u8, (length >> 3) as u8, ((length as u8) << 5) | 0x1F, 0xFC];
        frame.resize(length, 0xAA);
        frame
    }

    fn pes_timestamps(ts: &[u8]) -> Vec<u64> {
        ts.chunks(ts::PACKET)
            .filter(|p| p[1] & 0x40 != 0 && (((p[1] as u16 & 0x1F) << 8) | p[2] as u16) == AUDIO_PID)
            .map(|p| {
                let start = if p[3] & 0x20 != 0 { 5 + p[4] as usize } else { 4 };
                let h = &p[start + 9..start + 14];
                ((h[0] as u64 >> 1) & 0x07) << 30
                    | (h[1] as u64) << 22
                    | (h[2] as u64 >> 1) << 15
                    | (h[3] as u64) << 7
                    | h[4] as u64 >> 1
            })
            .collect()
    }

    #[test]
    fn segment_becomes_ts_with_every_frame_stamped() {
        let mut segment = id3_with_timestamp(3_214_726_240);
        for length in [300, 7, 400] {
            segment.extend_from_slice(&adts(length));
        }
        assert!(is_packed_audio(&segment));
        let ts = TsRewrapper::default().rewrap(&segment).unwrap();
        assert_eq!(ts.len() % ts::PACKET, 0);
        assert!(ts.chunks(ts::PACKET).all(|p| p[0] == ts::SYNC));
        // 1024 samples at 24 kHz are 3840 ticks of the 90 kHz clock.
        assert_eq!(pes_timestamps(&ts), [3_214_726_240, 3_214_730_080, 3_214_733_920]);
        let payload: usize = ts.chunks(ts::PACKET).filter(|p| p[2] as u16 == AUDIO_PID & 0xFF && p[1] & 0x1F == 1).count();
        assert_eq!(payload, 2 + 1 + 3); // 300+14, 7+14, 400+14 bytes of PES
    }

    #[test]
    fn continuity_runs_on_across_segments() {
        let mut segment = id3_with_timestamp(0);
        segment.extend_from_slice(&adts(100));
        let mut rewrapper = TsRewrapper::default();
        let first = rewrapper.rewrap(&segment).unwrap();
        let second = rewrapper.rewrap(&segment).unwrap();
        let counters: Vec<u8> =
            first.chunks(ts::PACKET).chain(second.chunks(ts::PACKET)).map(|p| p[3] & 0x0F).collect();
        assert_eq!(counters, [0, 0, 0, 1, 1, 1]);
    }

    #[test]
    fn psi_crc_matches_the_standard() {
        // The PAT of a single program 1 on PID 0x1000, as FFmpeg writes it.
        assert_eq!(pat(), [0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xF0, 0x00, 0x2A, 0xB1, 0x04, 0xB2]);
    }

    #[test]
    fn rejects_segments_it_cannot_time_or_read() {
        let mut rewrapper = TsRewrapper::default();
        assert!(rewrapper.rewrap(&adts(100)).is_err_and(|e| e.contains("timestamp")));
        let mut ac3 = id3_with_timestamp(0);
        ac3.extend_from_slice(&[0x0B, 0x77, 0, 0, 0, 0, 0, 0]);
        assert!(rewrapper.rewrap(&ac3).is_err());
        assert!(!is_packed_audio(&[ts::SYNC; ts::PACKET]));
    }
}
