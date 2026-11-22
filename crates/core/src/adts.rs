//! ADTS (ISO/IEC 13818-7) frame headers, read by `live` to time packed
//! audio and by `playback` to make silent frames in the stream's format.

/// Sample rates by the header's sampling frequency index.
const RATES: [u64; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
pub const HEADER_BYTES: usize = 7;

pub struct Header {
    /// Of the whole frame, header included.
    pub length: usize,
    pub rate: u64,
    pub samples: u64,
}

/// The header of the ADTS frame at the start of `data`, if one is there.
pub fn header(data: &[u8]) -> Option<Header> {
    if data.len() < HEADER_BYTES || data[0] != 0xFF || data[1] & 0xF6 != 0xF0 {
        return None;
    }
    let rate = *RATES.get(((data[2] >> 2) & 0x0F) as usize)?;
    let length = ((data[3] as usize & 0x03) << 11) | ((data[4] as usize) << 3) | (data[5] as usize >> 5);
    let blocks = (data[6] & 0x03) as u64 + 1;
    (length >= HEADER_BYTES).then_some(Header { length, rate, samples: 1024 * blocks })
}
