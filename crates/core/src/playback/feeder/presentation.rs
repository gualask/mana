//! What the decoder displays: its position, stalls and freezes, and the
//! pause while data is missing.

use std::time::{Duration, Instant};

use super::Feeder;
use crate::log::log;

/// While no data arrives, the decoder is paused once it has less than this
/// ahead of the displayed frame (µs); it resumes with the full lead.
const PAUSE_BELOW_US: i64 = 500_000;
/// Displayed position unchanged this long counts as a stall.
const STALL_AFTER: Duration = Duration::from_secs(1);

impl Feeder<'_> {
    /// Called while no data arrives: pauses the decoder before it runs dry.
    pub(super) fn pause_if_running_dry(&mut self) {
        if self.paused.is_some() || self.ran_dry {
            return;
        }
        let Some(position) = self.displayed() else { return };
        let ahead = self.timeline.reached() - position;
        if ahead >= PAUSE_BELOW_US {
            return;
        }
        if self.backend.pause() {
            log!("dati in esaurimento: pipeline in pausa a {} ms con {} ms davanti", position / 1000, ahead / 1000);
            self.paused = Some(Instant::now());
        } else {
            log!("dati in esaurimento: pausa rifiutata");
            self.ran_dry = true;
        }
    }

    /// Resumes a decoder paused for lack of data.
    pub(super) fn resume(&mut self) -> Result<(), String> {
        let Some(paused) = self.paused.take() else { return Ok(()) };
        if !self.backend.resume() {
            return Err("il decoder rifiuta di riprendere".into());
        }
        log!("pipeline ripresa dopo {} ms in pausa", paused.elapsed().as_millis());
        self.restart_frozen_timer();
        Ok(())
    }

    /// The decoder stood still for lack of data, not stuck: its freeze
    /// counts from now.
    pub(super) fn restart_frozen_timer(&mut self) {
        if let Some((position, _)) = self.presented {
            self.presented = Some((position, Instant::now()));
        }
    }

    /// Displayed position on the timeline (µs), tracking stalls.
    pub(super) fn displayed(&mut self) -> Option<i64> {
        let now = self.backend.presented_ns()? / 1000;
        match self.presented {
            Some((last, changed)) if last == now => {
                if !self.stalled && changed.elapsed() > STALL_AFTER {
                    self.stalled = true;
                    log!("presentazione ferma a {} ms", now / 1000);
                }
            }
            _ => {
                if self.stalled {
                    let stopped = self.presented.map_or(0, |(_, changed)| changed.elapsed().as_millis());
                    log!("presentazione ripresa dopo {stopped} ms");
                    self.stalled = false;
                }
                self.presented = Some((now, Instant::now()));
            }
        }
        Some(now)
    }
}
