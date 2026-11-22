use super::*;
use crate::playback::fake::{self, Fed};
use crate::playback::{AudioCodec, VideoCodec};

const FORMATS: Formats = Formats { video: VideoCodec::H264, width: 1920, height: 1080, audio: Some(AudioCodec::Aac) };
/// An ADTS frame header: AAC-LC, 48 kHz, stereo, 16 bytes long.
const ADTS: [u8; 16] = [0xFF, 0xF1, 0x4C, 0x80, 0x02, 0x1F, 0xFC, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// Runs `test` on a feeder loaded on the fake backend; returns what the
/// backend received.
fn run(test: impl FnOnce(&mut Feeder, &Mutex<Fed>) -> Result<(), String>) -> Fed {
    let (open, fed) = fake::open();
    let (settings, stop, stats) = (Settings::default(), AtomicBool::new(false), Mutex::new(Stats::default()));
    let mut feeder = Feeder::start(&open, &settings, &stop, &stats, FORMATS).unwrap();
    test(&mut feeder, &fed).unwrap();
    drop(feeder);
    fed.lock().unwrap().clone()
}

#[test]
fn no_backend_to_open_is_not_worth_reopening() {
    let (open, fed) = fake::open();
    fed.lock().unwrap().open_error = Some("riproduzione disponibile solo sulla TV".into());
    let (settings, stop, stats) = (Settings::default(), AtomicBool::new(false), Mutex::new(Stats::default()));
    let started = Feeder::start(&open, &settings, &stop, &stats, FORMATS);
    assert!(matches!(started, Err(Error::Unsupported(_))));
}

#[test]
fn video_is_stamped_on_the_timeline() {
    let fed = run(|feeder, _| {
        // Decode order: an I frame, then a P frame presented after a B frame.
        for (dts, pts) in [(900_000, 980_000), (940_000, 1_060_000), (980_000, 1_020_000)] {
            assert!(feeder.feed_video(b"au", 0, Some(dts), Some(pts), None)?);
        }
        Ok(())
    });
    assert_eq!(fed.loads, [FORMATS]);
    assert_eq!(fed.video, [80_000_000, 160_000_000, 120_000_000]);
}

#[test]
fn a_gap_in_the_audio_is_filled_with_silence() {
    let fed = run(|feeder, _| {
        assert!(feeder.feed_audio(&ADTS, 0, 0, 0)?);
        // 1024 samples at 48 kHz: frames of 21.333 ms; the next one 200 ms later.
        assert!(feeder.feed_audio(&ADTS, 0, 200_000, 0)?);
        Ok(())
    });
    let silent = fed.audio.len() - 2;
    assert_eq!(silent, 8, "silenzio tra 21 e 192 ms");
    assert_eq!(fed.audio.first(), Some(&0));
    assert_eq!(fed.audio[1], 21_333_000);
    assert_eq!(fed.audio.last(), Some(&200_000_000));
}

#[test]
fn a_decoder_short_of_data_pauses_until_it_has_the_lead_again() {
    const FRAME_US: i64 = 40_000;
    let fed = run(|feeder, fed| {
        assert!(feeder.feed_video(b"au", 0, Some(0), Some(0), None)?);
        // Nothing more arrives, with nothing ahead of the displayed frame.
        feeder.pause_if_running_dry();
        feeder.pause_if_running_dry();
        assert_eq!(fed.lock().unwrap().pauses, [true]);
        // Data again: paused until the frames fill the 1.5 s lead.
        let lead_frames = Settings::default().lead_ms * 1000 / FRAME_US;
        for frame in 1..=lead_frames {
            assert!(feeder.feed_video(b"au", 0, Some(frame * FRAME_US), Some(frame * FRAME_US), None)?);
        }
        assert_eq!(fed.lock().unwrap().pauses, [true]);
        let next = (lead_frames + 1) * FRAME_US;
        assert!(feeder.feed_video(b"au", 0, Some(next), Some(next), None)?);
        Ok(())
    });
    assert_eq!(fed.pauses, [true, false]);
    assert_eq!(fed.video.len(), 39);
}

#[test]
fn a_full_decoder_is_retried() {
    let fed = run(|feeder, fed| {
        fed.lock().unwrap().refuse = 2;
        assert!(feeder.feed_video(b"au", 0, Some(0), Some(0), None)?);
        Ok(())
    });
    // Both refusals were met, then the packet went in.
    assert_eq!((fed.refuse, fed.video.as_slice()), (0, &[0][..]));
}
