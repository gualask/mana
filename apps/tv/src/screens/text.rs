//! Text drawing with glyphs rasterized on demand into the renderer's atlas.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use fontdue::{Font, FontSettings};

use super::gfx::{AtlasRect, Color, Renderer};

#[derive(Clone, Copy)]
struct Glyph {
    rect: Option<AtlasRect>,
    left: f32,
    top: f32,
    advance: f32,
}

pub struct Text {
    font: Font,
    glyphs: HashMap<(char, u16), Glyph, BuildHasherDefault<KeyHasher>>,
}

/// The multiply-and-rotate hash of rustc's FxHash, for the glyph keys: they
/// are few and come from the app, so SipHash's resistance to chosen keys
/// buys nothing, and it showed in the TV's profile of the channel list.
#[derive(Default)]
struct KeyHasher(u64);

impl KeyHasher {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        bytes.iter().for_each(|&byte| self.add(u64::from(byte)));
    }

    fn write_u16(&mut self, word: u16) {
        self.add(u64::from(word));
    }

    fn write_u32(&mut self, word: u32) {
        self.add(u64::from(word));
    }
}

impl Text {
    pub fn load(candidates: &[&str]) -> Result<Self, String> {
        let mut errors = Vec::new();
        for path in candidates {
            match std::fs::read(path) {
                Ok(data) => {
                    let font = Font::from_bytes(data, FontSettings::default())
                        .map_err(|e| format!("font {path}: {e}"))?;
                    return Ok(Self { font, glyphs: HashMap::default() });
                }
                Err(e) => errors.push(format!("{path}: {e}")),
            }
        }
        Err(format!("nessun font disponibile ({})", errors.join("; ")))
    }

    /// Draws `text` with its top-left corner at (x, y) and returns its width.
    pub fn draw(&mut self, r: &mut Renderer, x: f32, y: f32, px: f32, color: Color, text: &str) -> f32 {
        let size = glyph_size(px);
        let ascent = self.font.horizontal_line_metrics(size as f32).map_or(size as f32 * 0.8, |m| m.ascent);
        let baseline = (y + ascent).round();
        let mut pen = x;
        for ch in text.chars() {
            let glyph = self.glyph(r, ch, size);
            if let Some(rect) = glyph.rect {
                r.glyph((pen + glyph.left).round(), baseline - glyph.top, rect, color);
            }
            pen += glyph.advance;
        }
        pen - x
    }

    /// The width `text` takes when drawn at `px`.
    pub fn width(&mut self, r: &mut Renderer, px: f32, text: &str) -> f32 {
        let size = glyph_size(px);
        text.chars().map(|ch| self.glyph(r, ch, size).advance).sum()
    }

    fn glyph(&mut self, r: &mut Renderer, ch: char, size: u16) -> Glyph {
        if let Some(glyph) = self.glyphs.get(&(ch, size)) {
            return *glyph;
        }
        let (m, bitmap) = self.font.rasterize(ch, size as f32);
        let glyph = Glyph {
            rect: if m.width > 0 { r.upload_alpha(m.width as i32, m.height as i32, &bitmap) } else { None },
            left: m.xmin as f32,
            top: (m.height as i32 + m.ymin) as f32,
            advance: m.advance_width,
        };
        self.glyphs.insert((ch, size), glyph);
        glyph
    }
}

/// The size, in whole pixels, glyphs are rasterized at for `px`.
fn glyph_size(px: f32) -> u16 {
    px.round().max(1.0) as u16
}
