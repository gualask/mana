//! Safe wrapper over `decoder.c`: software decoding of the access units the
//! feeder hands a backend, for the desktop. The TV decodes in Starfish, so
//! only the `decoder` feature builds it.

use std::ffi::{c_int, c_void};

use super::{CODEC_AAC, CODEC_AC3, CODEC_EAC3, CODEC_H264, CODEC_HEVC, error_text};
use crate::playback::{AudioCodec, VideoCodec};

unsafe extern "C" {
    fn mana_decoder_open(codec: c_int, error: *mut c_int) -> *mut c_void;
    fn mana_decoder_close(decoder: *mut c_void);
    fn mana_decoder_send(decoder: *mut c_void, data: *const u8, size: c_int, pts_ns: i64) -> c_int;
    fn mana_decoder_drain(decoder: *mut c_void) -> c_int;
    fn mana_decoder_receive(decoder: *mut c_void, width: *mut c_int, height: *mut c_int, pts_ns: *mut i64, samples: *mut c_int) -> c_int;
    fn mana_decoder_nv12(decoder: *mut c_void, luma: *mut u8, chroma: *mut u8) -> c_int;
    fn mana_decoder_stereo(decoder: *mut c_void, rate: c_int, out: *mut f32, capacity: c_int) -> c_int;
    fn mana_decoder_stereo_capacity(decoder: *mut c_void, rate: c_int) -> c_int;
}

/// A picture in NV12: the luma plane, then the interleaved chroma one.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub pts_ns: i64,
    pub luma: Vec<u8>,
    pub chroma: Vec<u8>,
}

/// Interleaved stereo samples at the rate asked for.
pub struct Sound {
    pub pts_ns: i64,
    pub samples: Vec<f32>,
}

pub struct Decoder {
    raw: *mut c_void,
}

// SAFETY: the decoder is used by one thread at a time; it moves to the
// thread that decodes.
unsafe impl Send for Decoder {}

impl Decoder {
    pub fn video(codec: VideoCodec) -> Result<Self, String> {
        Self::open(match codec {
            VideoCodec::H264 => CODEC_H264,
            VideoCodec::H265 => CODEC_HEVC,
        })
    }

    pub fn audio(codec: AudioCodec) -> Result<Self, String> {
        Self::open(match codec {
            AudioCodec::Aac => CODEC_AAC,
            AudioCodec::Ac3 => CODEC_AC3,
            AudioCodec::Eac3 => CODEC_EAC3,
        })
    }

    fn open(codec: i32) -> Result<Self, String> {
        let mut error = 0;
        // SAFETY: the shim returns a decoder or null with the error set.
        let raw = unsafe { mana_decoder_open(codec, &mut error) };
        if raw.is_null() { Err(error_text(error)) } else { Ok(Self { raw }) }
    }

    /// Hands over one access unit to present at `pts_ns`; a damaged one is
    /// dropped. False when the frames decoded must be taken first, then the
    /// unit sent again.
    pub fn send(&mut self, data: &[u8], pts_ns: i64) -> Result<bool, String> {
        let size = c_int::try_from(data.len()).map_err(|_| "access unit too large".to_string())?;
        // SAFETY: the shim reads `size` bytes of `data` before returning.
        match unsafe { mana_decoder_send(self.raw, data.as_ptr(), size, pts_ns) } {
            r if r < 0 => Err(error_text(r)),
            r => Ok(r > 0),
        }
    }

    /// No more units: the frames the decoder holds back come out.
    pub fn drain(&mut self) -> Result<(), String> {
        // SAFETY: the decoder is open.
        match unsafe { mana_decoder_drain(self.raw) } {
            r if r < 0 => Err(error_text(r)),
            _ => Ok(()),
        }
    }

    /// The next picture decoded, if one is ready.
    pub fn picture(&mut self) -> Result<Option<Picture>, String> {
        let Some((width, height, pts_ns, _)) = self.receive()? else { return Ok(None) };
        let (w, h) = (width as usize, height as usize);
        let (mut luma, mut chroma) = (vec![0; w * h], vec![0; w * h / 2]);
        // SAFETY: the planes hold a picture of the size the shim reported.
        let r = unsafe { mana_decoder_nv12(self.raw, luma.as_mut_ptr(), chroma.as_mut_ptr()) };
        if r < 0 {
            return Err(error_text(r));
        }
        Ok(Some(Picture { width, height, pts_ns, luma, chroma }))
    }

    /// The next audio frame decoded, as stereo at `rate`, if one is ready.
    pub fn sound(&mut self, rate: u32) -> Result<Option<Sound>, String> {
        let Some((_, _, pts_ns, _)) = self.receive()? else { return Ok(None) };
        let rate = rate as c_int;
        // SAFETY: the decoder holds the frame just received.
        let capacity = unsafe { mana_decoder_stereo_capacity(self.raw, rate) }.max(0);
        let mut samples = vec![0.0; capacity as usize * 2];
        // SAFETY: `samples` has room for `capacity` stereo frames.
        let written = unsafe { mana_decoder_stereo(self.raw, rate, samples.as_mut_ptr(), capacity) };
        if written < 0 {
            return Err(error_text(written));
        }
        samples.truncate(written as usize * 2);
        Ok(Some(Sound { pts_ns, samples }))
    }

    fn receive(&mut self) -> Result<Option<(u32, u32, i64, i32)>, String> {
        let (mut width, mut height, mut pts_ns, mut samples) = (0, 0, 0, 0);
        // SAFETY: the shim writes the frame's numbers into the four places.
        match unsafe { mana_decoder_receive(self.raw, &mut width, &mut height, &mut pts_ns, &mut samples) } {
            0 => Ok(None),
            r if r < 0 => Err(error_text(r)),
            _ => Ok(Some((width as u32, height as u32, pts_ns, samples))),
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: `raw` came from mana_decoder_open and is closed once.
        unsafe { mana_decoder_close(self.raw) };
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::playback::ffmpeg::{Media, Read};

    /// One second of 25 fps H.264 at 320×240 and a 1 kHz tone in AAC, in
    /// MPEG-TS, made by the `ffmpeg` command.
    fn clip() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("mana-decoder-{}.ts", std::process::id()));
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=25:duration=1"])
            .args(["-f", "lavfi", "-i", "sine=frequency=1000:sample_rate=44100:duration=1"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
            .arg(&path)
            .status()
            .expect("the ffmpeg command");
        assert!(made.success());
        path
    }

    #[test]
    fn a_clip_decodes_to_nv12_pictures_and_stereo_at_the_rate_asked() {
        let path = clip();
        let mut media = Media::open(&path).unwrap();
        let (mut video, mut audio) = (Decoder::video(VideoCodec::H264).unwrap(), Decoder::audio(AudioCodec::Aac).unwrap());
        let (mut pictures, mut samples, mut first_pts) = (Vec::new(), 0, None);
        loop {
            match media.read().unwrap() {
                Read::Video { data, pts_us, .. } => {
                    assert!(video.send(data, pts_us.unwrap() * 1000).unwrap());
                    while let Some(picture) = video.picture().unwrap() {
                        pictures.push((picture.width, picture.height, picture.pts_ns, picture.luma.len(), picture.chroma.len()));
                    }
                }
                Read::Audio { data, pts_us } => {
                    assert!(audio.send(data, pts_us.unwrap() * 1000).unwrap());
                    while let Some(sound) = audio.sound(48_000).unwrap() {
                        first_pts.get_or_insert(sound.pts_ns);
                        samples += sound.samples.len() / 2;
                    }
                }
                Read::Eof => break,
            }
        }
        video.drain().unwrap();
        while let Some(picture) = video.picture().unwrap() {
            pictures.push((picture.width, picture.height, picture.pts_ns, picture.luma.len(), picture.chroma.len()));
        }
        let _ = std::fs::remove_file(&path);
        assert_eq!(pictures.len(), 25);
        assert!(pictures.iter().all(|&(w, h, _, luma, chroma)| (w, h, luma, chroma) == (320, 240, 320 * 240, 320 * 240 / 2)));
        assert!(pictures.windows(2).all(|p| p[1].2 > p[0].2), "pictures in presentation order");
        // One second, and the AAC encoder's priming and padding: about 1,024 samples each way.
        assert!((47_000..=51_500).contains(&samples), "{samples} stereo frames at 48 kHz");
        assert!(first_pts.is_some());
    }
}
