//! A backend for tests: it takes every packet, unless told to refuse some,
//! and presents each video frame as soon as it is fed, unless paused.

use std::sync::{Arc, Mutex};

use super::{Backend, Formats, OpenBackend};

/// What the fake backends opened by one `open` received, in order.
#[derive(Clone, Default, Debug)]
pub struct Fed {
    pub loads: Vec<Formats>,
    /// Presentation times, in ns.
    pub video: Vec<i64>,
    pub audio: Vec<i64>,
    /// Feeds still to refuse as if the buffer were full.
    pub refuse: u32,
    /// Opening a backend fails with this, when set.
    pub open_error: Option<String>,
    /// Feeding fails with this, when set: a decoder gone wrong.
    pub feed_error: Option<String>,
    /// Calls to pause (true) and resume (false), in order.
    pub pauses: Vec<bool>,
}

struct Fake {
    fed: Arc<Mutex<Fed>>,
    presented_ns: Option<i64>,
    paused: bool,
    /// Latest video presentation time fed.
    fed_ns: Option<i64>,
}

/// Opens fake backends that record into the returned `Fed`.
pub fn open() -> (OpenBackend, Arc<Mutex<Fed>>) {
    let fed = Arc::new(Mutex::new(Fed::default()));
    let shared = fed.clone();
    let open: OpenBackend = Arc::new(move || {
        if let Some(error) = shared.lock().unwrap().open_error.clone() {
            return Err(error);
        }
        Ok(Box::new(Fake { fed: shared.clone(), presented_ns: None, paused: false, fed_ns: None }) as Box<dyn Backend>)
    });
    (open, fed)
}

impl Fake {
    /// Whether to take the packet, after the refusals and failures asked for.
    fn take(&self) -> Result<bool, String> {
        let mut fed = self.fed.lock().unwrap();
        if let Some(error) = fed.feed_error.clone() {
            return Err(error);
        }
        if fed.refuse > 0 {
            fed.refuse -= 1;
            return Ok(false);
        }
        Ok(true)
    }
}

impl Backend for Fake {
    fn load(&mut self, formats: &Formats) -> Result<(), String> {
        self.fed.lock().unwrap().loads.push(*formats);
        Ok(())
    }

    fn feed_video(&mut self, _data: &[u8], pts_ns: i64) -> Result<bool, String> {
        if !self.take()? {
            return Ok(false);
        }
        self.fed.lock().unwrap().video.push(pts_ns);
        self.fed_ns = Some(self.fed_ns.map_or(pts_ns, |p| p.max(pts_ns)));
        if !self.paused {
            self.presented_ns = self.fed_ns;
        }
        Ok(true)
    }

    fn feed_audio(&mut self, _data: &[u8], pts_ns: i64) -> Result<bool, String> {
        if !self.take()? {
            return Ok(false);
        }
        self.fed.lock().unwrap().audio.push(pts_ns);
        Ok(true)
    }

    fn presented_ns(&self) -> Option<i64> {
        self.presented_ns
    }

    fn buffer_low(&self) -> u64 {
        0
    }

    fn pause(&mut self) -> bool {
        self.paused = true;
        self.fed.lock().unwrap().pauses.push(true);
        true
    }

    /// Presents at once everything fed while paused.
    fn resume(&mut self) -> bool {
        self.paused = false;
        self.presented_ns = self.fed_ns;
        self.fed.lock().unwrap().pauses.push(false);
        true
    }
}
