//! Text drawing with glyphs rasterized on demand into the renderer's atlas.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};

use super::gfx::{AtlasRect, Color, Renderer};

#[derive(Clone, Copy)]
struct Glyph {
    rect: Option<AtlasRect>,
    left: f32,
    top: f32,
    advance: f32,
}

/// The font is parsed on demand: a glyph's outline is read from the file
/// when it is first rasterized. A rasterizer that builds every outline at
/// load (fontdue) holds 47 MB for the TV's 14,254-glyph system font.
pub struct Text {
    font: FontVec,
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
                    let font = FontVec::try_from_vec(data).map_err(|e| format!("font {path}: {e}"))?;
                    return Ok(Self { font, glyphs: HashMap::default() });
                }
                Err(e) => errors.push(format!("{path}: {e}")),
            }
        }
        Err(format!("no font available ({})", errors.join("; ")))
    }

    /// Draws `text` with its top-left corner at (x, y) and returns its width.
    pub fn draw(&mut self, r: &mut Renderer, x: f32, y: f32, px: f32, color: Color, text: &str) -> f32 {
        let size = glyph_size(px);
        let ascent = self.font.as_scaled(self.scale(size)).ascent();
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
        let font = self.font.as_scaled(self.scale(size));
        let id = font.glyph_id(ch);
        let mut glyph = Glyph { rect: None, left: 0.0, top: 0.0, advance: font.h_advance(id) };
        if let Some(outline) = self.font.outline_glyph(id.with_scale_and_position(font.scale(), point(0.0, 0.0))) {
            let bounds = outline.px_bounds();
            let (width, height) = (bounds.width() as usize, bounds.height() as usize);
            let mut bitmap = vec![0u8; width * height];
            outline.draw(|x, y, coverage| {
                bitmap[y as usize * width + x as usize] = (coverage.clamp(0.0, 1.0) * 255.0).round() as u8;
            });
            if width > 0 {
                glyph.rect = r.upload_alpha(width as i32, height as i32, &bitmap);
            }
            (glyph.left, glyph.top) = (bounds.min.x, -bounds.min.y);
        }
        self.glyphs.insert((ch, size), glyph);
        glyph
    }

    /// The scale at which the em square is `size` pixels, as CSS sizes text.
    /// ab_glyph scales the height from descent to ascent instead, and its
    /// `pt_to_px_scale` assumes 96 dpi.
    fn scale(&self, size: u16) -> PxScale {
        let em = self.font.units_per_em().unwrap_or(1.0);
        PxScale::from(f32::from(size) * self.font.height_unscaled() / em)
    }
}

/// The size, in whole pixels, glyphs are rasterized at for `px`.
fn glyph_size(px: f32) -> u16 {
    px.round().max(1.0) as u16
}
