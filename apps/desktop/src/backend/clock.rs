//! The presented position: the audio's, from the samples the output has
//! taken, as Rune gives no presentation time; without audio, the wall clock
//! from the first picture shown. Paused, it stands still.

use std::sync::Mutex;
use std::time::Instant;

#[derive(Default)]
pub struct Clock(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    /// The stream has audio: its samples move the clock.
    audio: bool,
    /// The position, and when it was last set, for the wall clock.
    at: Option<(i64, Instant)>,
    paused: bool,
}

impl Clock {
    pub fn new(audio: bool) -> Self {
        Self(Mutex::new(Inner { audio, ..Inner::default() }))
    }

    /// The position presented, in nanoseconds; None before the first sample
    /// or picture.
    pub fn now_ns(&self) -> Option<i64> {
        position(&self.0.lock().unwrap())
    }

    /// The output played up to `pts_ns`.
    pub fn heard(&self, pts_ns: i64) {
        self.0.lock().unwrap().at = Some((pts_ns, Instant::now()));
    }

    /// A picture at `pts_ns` was shown: it starts the wall clock of a stream
    /// without audio.
    pub fn shown(&self, pts_ns: i64) {
        let mut inner = self.0.lock().unwrap();
        if !inner.audio && inner.at.is_none() {
            inner.at = Some((pts_ns, Instant::now()));
        }
    }

    pub fn has_audio(&self) -> bool {
        self.0.lock().unwrap().audio
    }

    /// The audio could not play: the wall clock takes over.
    pub fn without_audio(&self) {
        self.0.lock().unwrap().audio = false;
    }

    pub fn is_paused(&self) -> bool {
        self.0.lock().unwrap().paused
    }

    pub fn pause(&self) {
        let mut inner = self.0.lock().unwrap();
        if !inner.paused {
            let now = position(&inner);
            inner.at = now.map(|at| (at, Instant::now()));
            inner.paused = true;
        }
    }

    pub fn resume(&self) {
        let mut inner = self.0.lock().unwrap();
        inner.paused = false;
        inner.at = inner.at.map(|(at, _)| (at, Instant::now()));
    }
}

/// Where `inner` is: the audio's last position, or the wall clock run on.
fn position(inner: &Inner) -> Option<i64> {
    let (at, since) = inner.at?;
    if inner.audio || inner.paused {
        return Some(at);
    }
    Some(at + i64::try_from(since.elapsed().as_nanos()).unwrap_or(i64::MAX / 2))
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn without_audio_the_first_picture_starts_a_clock_that_pauses() {
        let clock = Clock::new(false);
        assert_eq!(clock.now_ns(), None);
        clock.shown(5_000_000_000);
        std::thread::sleep(Duration::from_millis(20));
        let running = clock.now_ns().unwrap();
        assert!(running >= 5_020_000_000, "{running}");
        clock.pause();
        let paused = clock.now_ns().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(clock.now_ns(), Some(paused));
        clock.resume();
        std::thread::sleep(Duration::from_millis(10));
        assert!(clock.now_ns().unwrap() >= paused + 10_000_000);
    }

    #[test]
    fn with_audio_only_the_samples_heard_move_it() {
        let clock = Clock::new(true);
        clock.shown(1_000);
        assert_eq!(clock.now_ns(), None, "a picture does not start an audio clock");
        clock.heard(2_000_000);
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(clock.now_ns(), Some(2_000_000));
    }
}
