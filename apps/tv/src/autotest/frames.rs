//! The frame statistics the interface test logs: draw times, the longest
//! pause between frames, CPU and memory (Linux /proc).

use std::time::{Duration, Instant};

use mana_core::log::log;

/// Frame statistics are logged this often while the channel list is open.
const FRAME_REPORT: Duration = Duration::from_secs(5);

/// Draw times and intervals between frames, with CPU and memory.
pub(super) struct FrameStats {
    draw_us: Vec<u64>,
    gap_us: u64,
    last_frame: Option<Instant>,
    since: Instant,
    cpu_ticks: u64,
}

impl FrameStats {
    pub(super) fn new() -> Self {
        Self { draw_us: Vec::new(), gap_us: 0, last_frame: None, since: Instant::now(), cpu_ticks: cpu_ticks() }
    }

    /// `extra` adds to the periodic report.
    pub(super) fn record(&mut self, draw: Duration, extra: impl FnOnce() -> String) {
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
            "frames: {:.0}/s, median draw {:.1} ms, p95 {:.1} ms, max {:.1} ms, longest pause {:.0} ms, CPU {:.0}% of a core, memory {}, {}",
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
pub(super) fn memory() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| status.lines().find(|l| l.starts_with("VmRSS:")).map(|l| l[6..].trim().to_string()))
        .unwrap_or_else(|| "?".into())
}
