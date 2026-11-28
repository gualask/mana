//! The desktop's decoder on the `playback::Backend` port: FFmpeg decodes in
//! software, the pictures go to Rune's surface at the clock's time, the
//! audio to the system's output, whose samples move the clock
//! (docs/architecture.md, desktop client).

mod audio;
mod clock;
mod video;

use std::sync::Arc;

use mana_core::log::log;
use mana_core::playback::{Backend, BackendError, Formats, OpenBackend};
use rune::NativeSurface;

use audio::Audio;
use clock::Clock;
use video::Video;

pub use audio::Volume;
pub use video::push;

/// A new backend showing on `surface` for each decoder the feeder loads,
/// its audio at `volume`.
pub fn open(surface: NativeSurface, volume: Volume) -> OpenBackend {
    Arc::new(move || Ok(Box::new(Desktop::new(surface.clone(), volume.clone())) as Box<dyn Backend>))
}

struct Desktop {
    surface: NativeSurface,
    volume: Volume,
    clock: Arc<Clock>,
    video: Option<Video>,
    audio: Option<Audio>,
}

impl Desktop {
    fn new(surface: NativeSurface, volume: Volume) -> Self {
        Self { surface, volume, clock: Arc::new(Clock::default()), video: None, audio: None }
    }
}

impl Backend for Desktop {
    fn load(&mut self, formats: &Formats) -> Result<(), BackendError> {
        self.clock = Arc::new(Clock::new(formats.audio.is_some()));
        self.audio = match formats.audio.map(|codec| Audio::open(codec, self.clock.clone(), self.volume.clone())) {
            Some(Ok(audio)) => Some(audio),
            Some(Err(error)) => {
                log!("audio not played: {error}");
                self.clock.without_audio();
                None
            }
            None => None,
        };
        self.video = Some(Video::start(formats.video, self.surface.clone(), self.clock.clone()).map_err(BackendError::Open)?);
        Ok(())
    }

    fn feed_video(&mut self, data: &[u8], pts_ns: i64) -> Result<bool, BackendError> {
        let video = self.video.as_ref().ok_or(BackendError::Unavailable)?;
        video.feed(data, pts_ns).map_err(BackendError::Reported)
    }

    fn feed_audio(&mut self, data: &[u8], pts_ns: i64) -> Result<bool, BackendError> {
        match &mut self.audio {
            Some(audio) => audio.feed(data, pts_ns).map_err(BackendError::Reported),
            // Audio the output cannot play is dropped.
            None => Ok(true),
        }
    }

    fn presented_ns(&self) -> Option<i64> {
        self.clock.now_ns()
    }

    fn buffer_low(&self) -> u64 {
        0
    }

    fn pause(&mut self) -> bool {
        self.clock.pause();
        if let Some(audio) = &self.audio {
            audio.set_paused(true);
        }
        true
    }

    fn resume(&mut self) -> bool {
        if let Some(audio) = &self.audio {
            audio.set_paused(false);
        }
        self.clock.resume();
        true
    }

    fn finish(&mut self) {
        if let Some(video) = &self.video {
            video.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::{Duration, Instant};

    use mana_core::playback::Settings;
    use mana_core::session::{Program, Session, State};

    use super::*;

    /// `seconds` of 25 fps H.264 without audio, so that the test makes no
    /// sound: the wall clock presents it.
    fn clip(seconds: u32) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("mana-desktop-{}-{seconds}.ts", std::process::id()));
        let source = format!("testsrc=size=320x240:rate=25:duration={seconds}");
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", &source])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .expect("the ffmpeg command");
        assert!(made.success());
        path
    }

    #[test]
    fn a_file_plays_through_the_session_at_the_pace_of_its_timestamps() {
        let path = clip(3);
        let mut session = Session::new(open(NativeSurface::new(), Volume::default()));
        session.play("clip".into(), Program::Path(path.clone()), Settings::default()).unwrap();
        let presented = |session: &Session| session.stats().and_then(|s| s.playback.presented_ms);
        std::thread::sleep(Duration::from_millis(700));
        let (first, from) = (presented(&session).expect("a position presented"), Instant::now());
        std::thread::sleep(Duration::from_millis(1000));
        let stats = session.stats().unwrap();
        assert_eq!(stats.state, State::Playing, "{:?}", stats.state);
        let (moved, elapsed) = (stats.playback.presented_ms.unwrap() - first, from.elapsed().as_millis() as i64);
        // Real time, not as fast as decoding goes: within a frame or two.
        assert!((elapsed - 100..=elapsed + 100).contains(&moved), "{moved} ms presented in {elapsed} ms");
        session.stop();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_file_ends_once_its_last_pictures_are_shown_not_at_the_drain_timeout() {
        let path = clip(1);
        let mut session = Session::new(open(NativeSurface::new(), Volume::default()));
        let settings = Settings { loop_playback: false, ..Settings::default() };
        let started = Instant::now();
        session.play("clip".into(), Program::Path(path.clone()), settings).unwrap();
        let ended = |session: &Session| session.stats().is_some_and(|stats| stats.state.ended());
        while !ended(&session) && started.elapsed() < Duration::from_secs(8) {
            std::thread::sleep(Duration::from_millis(20));
        }
        // One second of pictures and the opening: shown whole, not cut before
        // its first picture, and not waiting 5 s for pictures the decoder
        // holds back.
        let took = started.elapsed();
        assert!((Duration::from_millis(900)..Duration::from_secs(3)).contains(&took), "ended after {took:?}");
        assert_eq!(session.stats().unwrap().state, State::Stopped);
        let _ = std::fs::remove_file(path);
    }
}
