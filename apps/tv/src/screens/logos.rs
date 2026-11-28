//! Channel logos on the TV: the core's thumbnails (`catalog::logos`), kept
//! as RGBA in fixed slots of one texture, the least recently drawn given up
//! first. Uploads are spread over frames: the GL driver converts texture
//! data on the CPU.

use std::collections::HashMap;
use std::path::Path;

use mana_core::catalog::logos::{Loader, Logo};

use super::gfx::Renderer;

/// Thumbnails turned into textures per frame.
const UPLOADS_PER_FRAME: usize = 3;
/// A slot drawn within this many frames (about 2 s) is not given up.
const KEEP_FRAMES: u64 = 120;

enum State {
    Ready { slot: usize, w: u32, h: u32, drawn: u64 },
    Failed,
}

pub struct Logos {
    states: HashMap<String, State>,
    pub loader: Loader,
    free: Vec<usize>,
    frame: u64,
}

impl Logos {
    /// The logos of `slots` texture slots, their thumbnails kept in
    /// `data_dir`.
    pub fn new(slots: usize, data_dir: &Path) -> Self {
        let free = (0..slots).rev().collect();
        Self { states: HashMap::new(), loader: Loader::start(data_dir), free, frame: 0 }
    }

    /// The logos to have, most wanted first: those not loaded yet replace
    /// what was asked for before.
    pub fn want<'a>(&mut self, urls: impl Iterator<Item = &'a str>) {
        self.loader.want(urls.filter(|url| !self.states.contains_key(*url)));
    }

    /// Turns a few thumbnails into textures; call once per frame.
    pub fn upload(&mut self, r: &mut Renderer) {
        self.frame += 1;
        for (slot, logo) in self.receive() {
            r.upload_image(slot, logo.w as i32, logo.h as i32, &logo.rgba);
        }
    }

    /// Up to UPLOADS_PER_FRAME thumbnails received, with the slot each now
    /// has; failures are recorded.
    fn receive(&mut self) -> Vec<(usize, Logo)> {
        let mut ready = Vec::new();
        for _ in 0..UPLOADS_PER_FRAME {
            let Some((url, result)) = self.loader.take() else { break };
            if let Some(placed) = self.place(url, result) {
                ready.push(placed);
            }
        }
        ready
    }

    /// Records the logo of `url` as `result` gives it: in a slot, which it
    /// returns with the image to upload, or failed. Without a slot free, it
    /// is asked for again when drawn.
    fn place(&mut self, url: String, result: Result<Logo, String>) -> Option<(usize, Logo)> {
        let Ok(logo) = result else {
            self.states.insert(url, State::Failed);
            return None;
        };
        let slot = self.slot()?;
        self.states.insert(url, State::Ready { slot, w: logo.w, h: logo.h, drawn: self.frame });
        Some((slot, logo))
    }

    /// Draws the logo of `url` fitted and centred in the box, if loaded.
    pub fn draw(&mut self, r: &mut Renderer, url: &str, x: f32, y: f32, w: f32, h: f32) -> bool {
        let Some(State::Ready { slot, w: iw, h: ih, drawn }) = self.states.get_mut(url) else { return false };
        *drawn = self.frame;
        let scale = (w / *iw as f32).min(h / *ih as f32);
        let (dw, dh) = (*iw as f32 * scale, *ih as f32 * scale);
        r.image(*slot, *iw as i32, *ih as i32, x + (w - dw) / 2.0, y + (h - dh) / 2.0, dw, dh);
        true
    }

    /// Logos ready in texture slots.
    pub fn resident(&self) -> usize {
        self.states.values().filter(|s| matches!(s, State::Ready { .. })).count()
    }

    /// A free slot, or the one drawn least recently if not within KEEP_FRAMES.
    fn slot(&mut self) -> Option<usize> {
        if let Some(slot) = self.free.pop() {
            return Some(slot);
        }
        let (url, slot) = self
            .states
            .iter()
            .filter_map(|(url, state)| match state {
                State::Ready { slot, drawn, .. } if drawn + KEEP_FRAMES < self.frame => Some((url, *slot, *drawn)),
                _ => None,
            })
            .min_by_key(|(_, _, drawn)| *drawn)
            .map(|(url, slot, _)| (url.clone(), slot))?;
        self.states.remove(&url);
        Some(slot)
    }
}

#[cfg(test)]
mod tests;
