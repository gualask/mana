//! A QR code drawn with the renderer's rectangles: dark modules on white,
//! with the quiet zone of 4 modules the format asks for around them.

use qrcodegen::{QrCode, QrCodeEcc};

use super::gfx::{Color, Renderer};

/// Modules of white around the code.
const QUIET_ZONE: i32 = 4;
const LIGHT: Color = Color::hex(0xffffff);
const DARK: Color = Color::hex(0x000000);

pub struct Qr(QrCode);

impl Qr {
    /// The code of `text`; none if it does not fit in a QR code.
    pub fn new(text: &str) -> Option<Self> {
        QrCode::encode_text(text, QrCodeEcc::Medium).ok().map(Qr)
    }

    /// Draws the code in a square of at most `side` pixels at `(x, y)`,
    /// each module a whole number of pixels so its edges stay sharp.
    /// Returns the side drawn.
    pub fn draw(&self, r: &mut Renderer, x: f32, y: f32, side: f32) -> f32 {
        let size = self.0.size();
        let modules = size + 2 * QUIET_ZONE;
        let module = (side / modules as f32).floor().max(1.0);
        let (x, y) = (x.round(), y.round());
        r.rect(x, y, module * modules as f32, module * modules as f32, LIGHT);
        for row in 0..size {
            let top = y + (row + QUIET_ZONE) as f32 * module;
            for (start, end) in self.dark_runs(row) {
                let left = x + (start + QUIET_ZONE) as f32 * module;
                r.rect(left, top, (end - start) as f32 * module, module, DARK);
            }
        }
        module * modules as f32
    }

    /// The runs of dark modules in `row`, as its first column and the one
    /// past its last: one rectangle each.
    fn dark_runs(&self, row: i32) -> Vec<(i32, i32)> {
        let size = self.0.size();
        let dark = |column| self.0.get_module(column, row);
        let mut runs = Vec::new();
        let mut column = 0;
        while column < size {
            let start = column;
            while column < size && dark(column) == dark(start) {
                column += 1;
            }
            if dark(start) {
                runs.push((start, column));
            }
        }
        runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_address_fits_a_small_code() {
        let qr = Qr::new("http://192.0.2.145:8080/K7QM").unwrap();
        // Version 3 or less: 29 modules a side, large on a TV.
        assert!(qr.0.size() <= 29, "{}", qr.0.size());
    }

    #[test]
    fn the_runs_cover_exactly_the_dark_modules_of_each_row() {
        let qr = Qr::new("http://192.0.2.145:8080/K7QM").unwrap();
        for row in 0..qr.0.size() {
            let mut dark = vec![false; qr.0.size() as usize];
            for (start, end) in qr.dark_runs(row) {
                assert!(start < end);
                (start..end).for_each(|c| dark[c as usize] = true);
            }
            let expected: Vec<bool> = (0..qr.0.size()).map(|c| qr.0.get_module(c, row)).collect();
            assert_eq!(dark, expected, "riga {row}");
        }
    }
}
