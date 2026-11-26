//! A decoder for the tests: it takes every packet and presents each video
//! frame as soon as it is fed, recording what it received.

use std::sync::{Arc, Mutex};

use mana_core::playback::{Backend, BackendError, Formats, OpenBackend};

/// What the decoders opened by one `open` received, in order.
#[derive(Default)]
pub struct Fed {
    pub loads: Vec<Formats>,
    /// Presentation times, in ns.
    pub video: Vec<i64>,
    pub audio: Vec<i64>,
}

struct Recorder {
    fed: Arc<Mutex<Fed>>,
    presented_ns: Option<i64>,
}

/// Opens decoders that record into the returned `Fed`.
pub fn open() -> (OpenBackend, Arc<Mutex<Fed>>) {
    let fed = Arc::new(Mutex::new(Fed::default()));
    let shared = fed.clone();
    let open: OpenBackend = Arc::new(move || Ok(Box::new(Recorder { fed: shared.clone(), presented_ns: None }) as Box<dyn Backend>));
    (open, fed)
}

impl Backend for Recorder {
    fn load(&mut self, formats: &Formats) -> Result<(), BackendError> {
        self.fed.lock().unwrap().loads.push(*formats);
        Ok(())
    }

    fn feed_video(&mut self, _data: &[u8], pts_ns: i64) -> Result<bool, BackendError> {
        self.fed.lock().unwrap().video.push(pts_ns);
        self.presented_ns = Some(self.presented_ns.map_or(pts_ns, |p| p.max(pts_ns)));
        Ok(true)
    }

    fn feed_audio(&mut self, _data: &[u8], pts_ns: i64) -> Result<bool, BackendError> {
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
        true
    }

    fn resume(&mut self) -> bool {
        true
    }
}
