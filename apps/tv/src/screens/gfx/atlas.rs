//! The alpha atlas of shapes and text: its size, the solid texel that
//! rectangles sample, and the shelves its glyphs are packed in.

pub const ATLAS_SIZE: i32 = 1024;
/// Atlas texel that is always fully opaque; rectangles sample it.
pub(super) const SOLID_UV: [f32; 2] = [0.5 / ATLAS_SIZE as f32, 0.5 / ATLAS_SIZE as f32];

/// A region of the atlas in texels.
#[derive(Debug, Clone, Copy)]
pub struct AtlasRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Shelf allocator for the atlas. The first row starts below the solid texel.
pub(super) struct Shelf {
    x: i32,
    y: i32,
    row_height: i32,
}

impl Shelf {
    pub(super) fn new() -> Self {
        Self { x: 0, y: 3, row_height: 0 }
    }

    pub(super) fn alloc(&mut self, w: i32, h: i32) -> Option<AtlasRect> {
        const GAP: i32 = 1;
        if self.x + w + GAP > ATLAS_SIZE {
            self.x = 0;
            self.y += self.row_height + GAP;
            self.row_height = 0;
        }
        if w > ATLAS_SIZE || self.y + h > ATLAS_SIZE {
            return None;
        }
        let rect = AtlasRect { x: self.x, y: self.y, w, h };
        self.x += w + GAP;
        self.row_height = self.row_height.max(h);
        Some(rect)
    }
}
