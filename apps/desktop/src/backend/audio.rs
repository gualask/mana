//! The audio: each frame decoded at once, as stereo at the output's rate,
//! queued with its timestamp, and taken by the system's output (cpal) on a
//! thread of its own, which moves the clock to the sample it plays, at the
//! window's volume. Paused, or with nothing queued, the output plays
//! silence and the clock stands.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use mana_core::playback::AudioCodec;
use mana_core::playback::ffmpeg::decoder::Decoder;

use super::clock::Clock;

/// Audio queued beyond this many seconds is refused until the output takes
/// it: the feeder keeps 1.5 s ahead.
const MAX_QUEUED_S: u32 = 3;
/// The volume's steps, in percent.
const VOLUME_STEP: i32 = 10;

/// The output's volume, which the window sets and the audio of every
/// viewing reads: a percentage, and whether it is muted.
#[derive(Clone)]
pub struct Volume(Arc<(AtomicU32, AtomicBool)>);

impl Default for Volume {
    fn default() -> Self {
        Self(Arc::new((AtomicU32::new(100), AtomicBool::new(false))))
    }
}

impl Volume {
    /// Raises it a step, or lowers it with `up` false, and unmutes it.
    pub fn step(&self, up: bool) {
        let by = if up { VOLUME_STEP } else { -VOLUME_STEP };
        let level = (self.level() as i32 + by).clamp(0, 100) as u32;
        self.0.0.store(level, Ordering::Relaxed);
        self.0.1.store(false, Ordering::Relaxed);
    }

    pub fn toggle_mute(&self) {
        self.0.1.fetch_xor(true, Ordering::Relaxed);
    }

    pub fn level(&self) -> u32 {
        self.0.0.load(Ordering::Relaxed)
    }

    pub fn muted(&self) -> bool {
        self.0.1.load(Ordering::Relaxed)
    }

    /// What the samples are multiplied by: the level cubed, as mpv's volume
    /// goes, so that each step sounds about as large.
    fn gain(&self) -> f32 {
        if self.muted() { 0.0 } else { (self.level() as f32 / 100.0).powi(3) }
    }
}

pub struct Audio {
    decoder: Decoder,
    queue: Arc<Mutex<Queue>>,
    rate: u32,
    /// Plays while it lives.
    _stream: cpal::Stream,
}

#[derive(Default)]
struct Queue {
    /// Stereo samples with the timestamp of their first, and how many of
    /// them were played.
    chunks: VecDeque<(i64, Vec<f32>, usize)>,
    /// Stereo frames queued.
    frames: usize,
    paused: bool,
}

impl Audio {
    /// Opens the system's default output for `codec`, at `volume`.
    pub fn open(codec: AudioCodec, clock: Arc<Clock>, volume: Volume) -> Result<Self, String> {
        let decoder = Decoder::audio(codec)?;
        let device = cpal::default_host().default_output_device().ok_or("no audio output")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        if supported.sample_format() != cpal::SampleFormat::F32 {
            return Err(format!("audio output in {}: only f32 is played", supported.sample_format()));
        }
        let config: cpal::StreamConfig = supported.config();
        let (rate, channels) = (config.sample_rate, usize::from(config.channels));
        let queue = Arc::new(Mutex::new(Queue::default()));
        let taken = queue.clone();
        let stream = device
            .build_output_stream(
                config,
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| play(&taken, &clock, out, channels, rate, volume.gain()),
                |error| mana_core::log::log!("audio output: {error}"),
                None,
            )
            .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self { decoder, queue, rate, _stream: stream })
    }

    /// Decodes one frame into the queue; false when the queue is full and
    /// the frame must come again.
    pub fn feed(&mut self, data: &[u8], pts_ns: i64) -> Result<bool, String> {
        if self.queue.lock().unwrap().frames > (MAX_QUEUED_S * self.rate) as usize {
            return Ok(false);
        }
        while !self.decoder.send(data, pts_ns)? {
            self.take()?;
        }
        self.take()?;
        Ok(true)
    }

    fn take(&mut self) -> Result<(), String> {
        while let Some(sound) = self.decoder.sound(self.rate)? {
            let mut queue = self.queue.lock().unwrap();
            queue.frames += sound.samples.len() / 2;
            queue.chunks.push_back((sound.pts_ns, sound.samples, 0));
        }
        Ok(())
    }

    pub fn set_paused(&self, paused: bool) {
        self.queue.lock().unwrap().paused = paused;
    }
}

/// Fills `out`, of `channels` interleaved, from the queue at `gain`: stereo
/// to the first two channels, silence elsewhere and when nothing is queued.
fn play(queue: &Mutex<Queue>, clock: &Clock, out: &mut [f32], channels: usize, rate: u32, gain: f32) {
    out.fill(0.0);
    let mut guard = queue.lock().unwrap();
    let queue = &mut *guard;
    if queue.paused {
        return;
    }
    let mut heard = None;
    for frame in out.chunks_mut(channels) {
        let Some((pts, samples, played)) = queue.chunks.front_mut() else { break };
        let (left, right) = (samples[*played * 2], samples[*played * 2 + 1]);
        frame[0] = left * gain;
        if channels > 1 {
            frame[1] = right * gain;
        }
        heard = Some(*pts + *played as i64 * 1_000_000_000 / i64::from(rate));
        *played += 1;
        if *played * 2 >= samples.len() {
            queue.chunks.pop_front();
        }
        queue.frames = queue.frames.saturating_sub(1);
    }
    if let Some(pts) = heard {
        clock.heard(pts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_output_takes_stereo_into_its_first_channels_and_moves_the_clock_to_what_it_plays() {
        let clock = Clock::new(true);
        let queue = Mutex::new(Queue::default());
        {
            let mut q = queue.lock().unwrap();
            q.chunks.push_back((1_000_000_000, vec![0.1, 0.2, 0.3, 0.4], 0));
            q.chunks.push_back((2_000_000_000, vec![0.5, 0.6], 0));
            q.frames = 3;
        }
        // Four channels, three frames of them: the first two take the stereo.
        let mut out = [9.0; 12];
        play(&queue, &clock, &mut out, 4, 1000, 1.0);
        assert_eq!(out, [0.1, 0.2, 0.0, 0.0, 0.3, 0.4, 0.0, 0.0, 0.5, 0.6, 0.0, 0.0]);
        assert_eq!(clock.now_ns(), Some(2_000_000_000));
        assert_eq!(queue.lock().unwrap().frames, 0);
        // Nothing left, or paused: silence, and the clock stands.
        play(&queue, &clock, &mut out, 4, 1000, 1.0);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(clock.now_ns(), Some(2_000_000_000));
    }

    #[test]
    fn the_volume_scales_the_samples_and_muting_silences_them() {
        let volume = Volume::default();
        volume.step(false);
        volume.step(false);
        assert_eq!(volume.level(), 80);
        let clock = Clock::new(true);
        let queue = Mutex::new(Queue::default());
        queue.lock().unwrap().chunks.push_back((0, vec![1.0, -1.0, 1.0, -1.0], 0));
        let mut out = [0.0; 2];
        play(&queue, &clock, &mut out, 2, 1000, volume.gain());
        assert!((out[0] - 0.512).abs() < 1e-6 && (out[1] + 0.512).abs() < 1e-6, "{out:?}");
        volume.toggle_mute();
        play(&queue, &clock, &mut out, 2, 1000, volume.gain());
        assert_eq!(out, [0.0, 0.0]);
        // A step unmutes, and the level stays within 0 and 100.
        (0..12).for_each(|_| volume.step(true));
        assert_eq!((volume.level(), volume.muted()), (100, false));
    }
}
