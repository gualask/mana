//! Unattended tests, driving the app as the remote does: a clip played at
//! startup, channel changes, scrolling the channel list over the video. They
//! log first-frame times, frame times, CPU and memory (Linux /proc).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use mana_core::log::log;
use mana_core::session::STREAM_EXTENSION;

use crate::platform::keys::Action;
use crate::screens::App;
use crate::settings;

/// Holds the file name of a clip, or the name of a channel of the list, to
/// play at startup, so tests can run unattended.
const AUTOPLAY_FILE: &str = "/tmp/mana-autoplay";
/// Frame statistics are logged this often while the channel list is open.
const FRAME_REPORT: Duration = Duration::from_secs(5);
/// Interface test: holding Down (remote key repeat), paging with Right,
/// then the list left open without keys, as (end of phase in s, key,
/// presses per second).
const UI_TEST_PHASES: [(f32, Option<Action>, f32); 3] =
    [(16.0, Some(Action::Down), 30.0), (26.0, Some(Action::Right), 10.0), (36.0, None, 0.0)];
const UI_TEST_START: f32 = 1.0;

/// The tests the settings file asks for.
#[derive(Clone, Debug)]
pub struct Options {
    /// Channel changes between the `.stream` files (0: off) and time on
    /// each channel.
    pub zap_count: u32,
    pub zap_interval_ms: u64,
    /// Scrolls the channel list over the video and logs frame times, CPU
    /// and memory.
    pub ui_test: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { zap_count: 0, zap_interval_ms: 8000, ui_test: false }
    }
}

#[derive(Default)]
pub struct Autotest {
    ui_test: Option<UiTest>,
    zapping: Option<Zapping>,
    /// While the channel list is open.
    frames: Option<FrameStats>,
}

/// Scripted remote presses on the channel list.
struct UiTest {
    started: Instant,
    presses: u32,
    phase: usize,
}

impl UiTest {
    /// Sends the keys the current phase, one of `UI_TEST_PHASES`, is due by
    /// `elapsed`, then moves to the next phase once it ends.
    fn press(&mut self, app: &mut App, elapsed: f32, (end, action, rate): (f32, Option<Action>, f32)) {
        let phase_start = if self.phase == 0 { UI_TEST_START } else { UI_TEST_PHASES[self.phase - 1].0 };
        let due = ((elapsed.min(end) - phase_start) * rate) as u32;
        while let Some(action) = action
            && self.presses < due
        {
            self.presses += 1;
            app.handle(action);
        }
        if elapsed >= end {
            log!("prova interfaccia: fase {} finita, {} passi", self.phase + 1, self.presses);
            self.phase += 1;
            self.presses = 0;
        }
    }
}

/// Moves to the next `.stream` file every interval, logging the time to
/// the first displayed frame and the process memory.
struct Zapping {
    remaining: u32,
    interval: Duration,
    number: u32,
    /// When the change was asked for, as the remote's key.
    switched_at: Instant,
    first_frame_logged: bool,
    /// Until the viewing changed from has ended its threads.
    stopping: bool,
}

/// Draw times and intervals between frames, with CPU and memory.
struct FrameStats {
    draw_us: Vec<u64>,
    gap_us: u64,
    last_frame: Option<Instant>,
    since: Instant,
    cpu_ticks: u64,
}

impl Autotest {
    /// Plays the clip or channel named in AUTOPLAY_FILE, if any, and starts
    /// the tests the settings ask for.
    pub fn start(app: &mut App) -> Self {
        let mut autotest = Self::default();
        let Ok(name) = std::fs::read_to_string(AUTOPLAY_FILE) else { return autotest };
        let name = name.trim();
        if !play_named(app, name) {
            return autotest;
        }
        log!("avvio automatico di {name}");
        let options = settings::load().tests;
        if options.ui_test {
            let channels = app.channels();
            log!("prova interfaccia: {} canali, {} con logo, memoria {}", channels.len(), channels.with_logos(), memory());
            autotest.ui_test = Some(UiTest { started: Instant::now(), presses: 0, phase: 0 });
        }
        if options.zap_count > 0 {
            log!("zapping: {} cambi ogni {} ms, memoria {}", options.zap_count, options.zap_interval_ms, memory());
            autotest.zapping = Some(Zapping::new(&options));
        }
        autotest
    }

    /// Whether a test is pressing keys, which needs the screen redrawn
    /// every frame.
    pub fn animating(&self) -> bool {
        self.ui_test.as_ref().is_some_and(|test| UI_TEST_PHASES.get(test.phase).is_some_and(|phase| phase.1.is_some()))
    }

    /// Advances the tests that are running.
    pub fn tick(&mut self, app: &mut App) {
        self.tick_ui_test(app);
        self.tick_zapping(app);
    }

    /// Records one drawn frame; logs the statistics while the list is open.
    pub fn frame_drawn(&mut self, app: &App, draw: Duration) {
        if !app.channels_open() {
            self.frames = None;
            return;
        }
        let logos = app.logos();
        self.frames.get_or_insert_with(FrameStats::new).record(draw, || {
            format!("loghi caricati {}, in memoria {}, falliti {}", logos.loaded, logos.resident(), logos.failed)
        });
    }

    fn tick_ui_test(&mut self, app: &mut App) {
        let Some(test) = &mut self.ui_test else { return };
        let elapsed = test.started.elapsed().as_secs_f32();
        if elapsed < UI_TEST_START {
            return;
        }
        // Without playback the same keys act on the clip list instead.
        if !app.is_playing() {
            log!("prova interfaccia interrotta: nessuna riproduzione");
            self.ui_test = None;
            return;
        }
        if !app.channels_open() {
            app.handle(Action::Select);
        }
        let Some(&phase) = UI_TEST_PHASES.get(test.phase) else {
            log!("prova interfaccia finita, memoria {}", memory());
            app.handle(Action::Back);
            self.ui_test = None;
            return;
        };
        test.press(app, elapsed, phase);
    }

    fn tick_zapping(&mut self, app: &mut App) {
        let Some(zapping) = &mut self.zapping else { return };
        zapping.observe(app);
        if zapping.switched_at.elapsed() < zapping.interval {
            return;
        }
        if zapping.remaining == 0 {
            log!("zapping finito, memoria {}", memory());
            self.zapping = None;
            return;
        }
        match next_stream(app.clips(), app.focused_clip()) {
            Some(next) => zapping.switch_to(app, next),
            None => self.zapping = None,
        }
    }
}

impl Zapping {
    fn new(options: &Options) -> Self {
        Self {
            remaining: options.zap_count,
            interval: Duration::from_millis(options.zap_interval_ms),
            number: 0,
            switched_at: Instant::now(),
            first_frame_logged: false,
            stopping: false,
        }
    }

    /// Logs the first frame of the channel changed to, and when the one
    /// changed from has stopped.
    fn observe(&mut self, app: &App) {
        if !self.first_frame_logged && app.stats().is_some_and(|s| s.playback.presented_ms.is_some()) {
            self.first_frame_logged = true;
            log!("cambio {}: primo fotogramma dopo {} ms", self.number, self.switched_at.elapsed().as_millis());
        }
        if self.stopping && !app.is_stopping() {
            self.stopping = false;
            log!("cambio {}: canale precedente fermato dopo {} ms, memoria {}", self.number, self.switched_at.elapsed().as_millis(), memory());
        }
    }

    /// Changes to the clip at `next`, timed from the key, as the viewer
    /// waits: the change returns at once, the channel changed from stops
    /// meanwhile.
    fn switch_to(&mut self, app: &mut App, next: usize) {
        self.switched_at = Instant::now();
        self.stopping = app.is_playing();
        self.remaining -= 1;
        self.number += 1;
        self.first_frame_logged = false;
        app.play_clip(next);
        log!("cambio {}: tasto gestito in {} ms", self.number, self.switched_at.elapsed().as_millis());
    }
}

/// The `.stream` file after the clip `focus` in `clips`, going round, from
/// the first without one; None when there is none.
fn next_stream(clips: &[PathBuf], focus: Option<usize>) -> Option<usize> {
    let last = clips.len().checked_sub(1)?;
    (1..=clips.len())
        .map(|step| (focus.unwrap_or(last) + step) % clips.len())
        .find(|&i| clips[i].extension().is_some_and(|e| e == STREAM_EXTENSION))
}

/// Plays the clip with the file name `name`, else the channel of the list
/// with that name; false when there is neither, or it cannot be played.
fn play_named(app: &mut App, name: &str) -> bool {
    if let Some(index) = app.clips().iter().position(|c| c.file_name().is_some_and(|n| n == name)) {
        app.play_clip(index);
        true
    } else if let Some(index) = app.channels().position(name) {
        app.play_channel(index)
    } else {
        false
    }
}

impl FrameStats {
    fn new() -> Self {
        Self { draw_us: Vec::new(), gap_us: 0, last_frame: None, since: Instant::now(), cpu_ticks: cpu_ticks() }
    }

    /// `extra` adds to the periodic report.
    fn record(&mut self, draw: Duration, extra: impl FnOnce() -> String) {
        let now = Instant::now();
        if let Some(last) = self.last_frame {
            self.gap_us = self.gap_us.max(now.duration_since(last).as_micros() as u64);
        }
        self.last_frame = Some(now);
        self.draw_us.push(draw.as_micros() as u64);
        if self.since.elapsed() >= FRAME_REPORT {
            self.report(extra());
        }
    }

    /// Logs the frames since the last report, with CPU and memory, and
    /// starts counting again.
    fn report(&mut self, extra: String) {
        let seconds = self.since.elapsed().as_secs_f64();
        let ticks = cpu_ticks();
        // Linux counts CPU time in ticks of 1/100 s.
        let cpu = (ticks - self.cpu_ticks) as f64 / 100.0 / seconds * 100.0;
        self.draw_us.sort_unstable();
        let percentile = |p: usize| self.draw_us[(self.draw_us.len() - 1) * p / 100] as f64 / 1000.0;
        log!(
            "fotogrammi: {:.0}/s, disegno mediano {:.1} ms, p95 {:.1} ms, max {:.1} ms, pausa max {:.0} ms, CPU {:.0}% di un core, memoria {}, {}",
            self.draw_us.len() as f64 / seconds,
            percentile(50),
            percentile(95),
            percentile(100),
            self.gap_us as f64 / 1000.0,
            cpu,
            memory(),
            extra
        );
        *self = Self { cpu_ticks: ticks, ..Self::new() };
    }
}

/// User plus system CPU time of this process, in clock ticks.
fn cpu_ticks() -> u64 {
    std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|stat| {
            // Fields after the parenthesised command name; utime and stime
            // are the 12th and 13th of them.
            let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
            Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
        })
        .unwrap_or(0)
}

/// Resident memory of this process, from /proc on Linux.
fn memory() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| status.lines().find(|l| l.starts_with("VmRSS:")).map(|l| l[6..].trim().to_string()))
        .unwrap_or_else(|| "?".into())
}
