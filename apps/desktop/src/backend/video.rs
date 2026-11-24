//! The video: access units queued by the feeder, decoded on a thread of
//! its own and shown in Rune's surface when the clock reaches each
//! picture's time. A picture already late is dropped, as a TV drops it.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mana_core::log::log;
use mana_core::playback::VideoCodec;
use mana_core::playback::ffmpeg::decoder::{Decoder, Picture};
use rune::NativeSurface;
use rune::native_surface::{ByteFrame, ColorInfo, Format, Matrix, Transfer};

use super::clock::Clock;

/// Access units queued before the feeder must retry: some seconds at 50 fps,
/// beyond the feeder's lead of 1.5 s.
const QUEUE: usize = 256;
/// A picture this late is dropped.
const LATE: Duration = Duration::from_millis(100);
/// Waits are cut in slices this long, to notice a stop.
const SLICE: Duration = Duration::from_millis(10);

/// What the feeder hands the decoding thread.
enum Unit {
    /// An access unit and the time to show it at.
    Access(Vec<u8>, i64),
    /// No more units: the pictures held back come out.
    End,
}

pub struct Video {
    units: Option<SyncSender<Unit>>,
    stop: Arc<AtomicBool>,
    /// Why decoding stopped, once it did.
    failure: Arc<Mutex<Option<String>>>,
    /// Pictures shown so far.
    shown: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Video {
    pub fn start(codec: VideoCodec, surface: NativeSurface, clock: Arc<Clock>) -> Result<Self, String> {
        let decoder = Decoder::video(codec)?;
        let (units, queue) = sync_channel(QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let failure = Arc::new(Mutex::new(None));
        let shown = Arc::new(AtomicU64::new(0));
        let mut presenter = Presenter { decoder, surface, clock, stop: stop.clone(), shown: shown.clone() };
        let failed = failure.clone();
        let thread = std::thread::Builder::new()
            .name("video".into())
            .spawn(move || {
                if let Err(error) = presenter.run(&queue) {
                    *failed.lock().unwrap() = Some(error);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { units: Some(units), stop, failure, shown, thread: Some(thread) })
    }

    /// Queues one access unit; false when the queue is full.
    pub fn feed(&self, data: &[u8], pts_ns: i64) -> Result<bool, String> {
        if let Some(error) = self.failure.lock().unwrap().clone() {
            return Err(error);
        }
        match self.units.as_ref().map(|units| units.try_send(Unit::Access(data.to_vec(), pts_ns))) {
            Some(Ok(())) => Ok(true),
            Some(Err(TrySendError::Full(_))) => Ok(false),
            _ => Err("video decoding stopped".into()),
        }
    }
}

impl Video {
    /// Pictures shown so far.
    #[cfg(test)]
    pub fn shown(&self) -> u64 {
        self.shown.load(Ordering::Relaxed)
    }

    /// No more units will come.
    pub fn finish(&self) {
        if let Some(units) = &self.units {
            let _ = units.send(Unit::End);
        }
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.units = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        log!("video: {} pictures shown", self.shown.load(Ordering::Relaxed));
    }
}

struct Presenter {
    decoder: Decoder,
    surface: NativeSurface,
    clock: Arc<Clock>,
    stop: Arc<AtomicBool>,
    shown: Arc<AtomicU64>,
}

impl Presenter {
    /// Decodes and shows until the queue closes or the backend stops.
    fn run(&mut self, queue: &Receiver<Unit>) -> Result<(), String> {
        while !self.stop.load(Ordering::Relaxed) {
            match queue.recv_timeout(SLICE) {
                Ok(Unit::Access(unit, pts_ns)) => self.decode(&unit, pts_ns)?,
                Ok(Unit::End) => {
                    self.decoder.drain()?;
                    self.show_decoded()?;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        Ok(())
    }

    fn decode(&mut self, unit: &[u8], pts_ns: i64) -> Result<(), String> {
        while !self.decoder.send(unit, pts_ns)? {
            self.show_decoded()?;
        }
        self.show_decoded()
    }

    fn show_decoded(&mut self) -> Result<(), String> {
        while let Some(picture) = self.decoder.picture()? {
            self.show_when_due(picture)?;
        }
        Ok(())
    }

    /// Waits for the clock to reach the picture, then shows it, unless it
    /// is late already.
    fn show_when_due(&mut self, picture: Picture) -> Result<(), String> {
        if self.wait_for(picture.pts_ns) { self.show(picture) } else { Ok(()) }
    }

    /// Waits until `pts_ns` is due: true to show the picture, false when it
    /// is late or the backend stops.
    fn wait_for(&self, pts_ns: i64) -> bool {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return false;
            }
            // Without audio, the first picture starts the clock.
            if !self.clock.has_audio() && self.clock.now_ns().is_none() {
                self.clock.shown(pts_ns);
            }
            match due(self.clock.now_ns(), self.clock.is_paused(), pts_ns) {
                Due::Now => return true,
                Due::Late => return false,
                Due::In(wait) => std::thread::sleep(wait),
            }
        }
    }

    fn show(&mut self, picture: Picture) -> Result<(), String> {
        let frame = ByteFrame {
            width: picture.width,
            height: picture.height,
            format: Format::Nv12Video,
            // Broadcast video: limited range, BT.709 above standard definition.
            color: ColorInfo { matrix: if picture.height <= 576 { Matrix::Bt601 } else { Matrix::Bt709 }, transfer: Transfer::Bt709 },
            planes: [picture.luma, picture.chroma],
        };
        push(&self.surface, frame, self.shown.load(Ordering::Relaxed) + 1)?;
        self.shown.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

/// When a picture at `pts_ns` is due, by a clock at `now` (none before it
/// starts).
#[derive(Debug, PartialEq)]
enum Due {
    Now,
    /// Too late to show.
    Late,
    /// Look again after this long: at most a slice, to notice a stop.
    In(Duration),
}

fn due(now: Option<i64>, paused: bool, pts_ns: i64) -> Due {
    match (now, paused) {
        (None, _) | (_, true) => Due::In(SLICE),
        (Some(now), false) if pts_ns + LATE.as_nanos() as i64 <= now => Due::Late,
        (Some(now), false) if pts_ns > now => Due::In(Duration::from_nanos((pts_ns - now) as u64).min(SLICE)),
        (Some(_), false) => Due::Now,
    }
}

/// Hands one picture to the surface: into an IOSurface on macOS, as bytes
/// elsewhere.
pub fn push(surface: &NativeSurface, frame: ByteFrame, id: u64) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return surface.push(rune::native_surface::macos::PixelBuffer::from_bytes(&frame)?, id);
    #[cfg(not(target_os = "macos"))]
    surface.push_bytes(frame, id)
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::{Duration, Instant};

    use mana_core::playback::ffmpeg::{Media, Read};

    use super::*;

    #[test]
    fn a_picture_is_due_once_the_clock_reaches_it_and_skipped_once_late() {
        let ms = 1_000_000;
        assert_eq!(due(None, false, 0), Due::In(SLICE), "before the clock starts");
        assert_eq!(due(Some(0), true, 0), Due::In(SLICE), "paused");
        assert_eq!(due(Some(100 * ms), false, 103 * ms), Due::In(Duration::from_millis(3)));
        assert_eq!(due(Some(100 * ms), false, 500 * ms), Due::In(SLICE));
        assert_eq!(due(Some(100 * ms), false, 100 * ms), Due::Now);
        assert_eq!(due(Some(100 * ms), false, 20 * ms), Due::Now, "80 ms late, within LATE");
        assert_eq!(due(Some(100 * ms), false, 0), Due::Late, "100 ms late");
    }

    #[test]
    fn at_the_end_the_pictures_the_decoder_held_back_are_shown_too() {
        let path = std::env::temp_dir().join(format!("mana-video-{}.ts", std::process::id()));
        let made = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=25:duration=1"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .status()
            .expect("the ffmpeg command");
        assert!(made.success());
        let clock = Arc::new(Clock::new(false));
        let video = Video::start(VideoCodec::H264, NativeSurface::new(), clock).unwrap();
        let mut media = Media::open(&path).unwrap();
        while let Read::Video { data, pts_us, .. } = media.read().unwrap() {
            while !video.feed(data, pts_us.unwrap() * 1000).unwrap() {
                std::thread::sleep(SLICE);
            }
        }
        video.finish();
        let started = Instant::now();
        while video.shown() < 25 && started.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(SLICE);
        }
        let _ = std::fs::remove_file(path);
        assert_eq!(video.shown(), 25);
    }
}
