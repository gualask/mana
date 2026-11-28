//! The logos of the rows, as on the TV: the core's thumbnails
//! (`catalog::logos`), each shown through a Rune image once it came.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use mana_core::catalog::logos::Loader;
use rune::Pixels;

use super::levels::Row;

/// Rows each way from the focus whose logos are asked for, the nearest
/// first: the rows shown and about as many around them.
const AROUND: usize = 20;

pub struct Logos {
    loader: Loader,
    shown: HashMap<String, Pixels>,
    failed: HashSet<String>,
}

impl Logos {
    /// The logos whose thumbnails `data_dir` keeps.
    pub fn new(data_dir: &Path) -> Self {
        Self { loader: Loader::start(data_dir), shown: HashMap::new(), failed: HashSet::new() }
    }

    /// Asks for the logos of `rows` around `focus` not come yet.
    pub fn want(&self, rows: &[Row], focus: usize) {
        let mut near: Vec<usize> = (focus.saturating_sub(AROUND)..(focus + AROUND + 1).min(rows.len())).collect();
        near.sort_by_key(|&i| i.abs_diff(focus));
        let urls = near.into_iter().filter_map(|i| rows[i].channel()?.logo.as_deref());
        self.loader.want(urls.filter(|url| !self.shown.contains_key(*url) && !self.failed.contains(*url)));
    }

    /// Takes the thumbnails that came; whether any did.
    pub fn take(&mut self) -> bool {
        let mut came = false;
        while let Some((url, result)) = self.loader.take() {
            match result {
                Ok(logo) => {
                    let pixels = Pixels::new();
                    pixels.set(logo.w, logo.h, logo.rgba);
                    self.shown.insert(url, pixels);
                    came = true;
                }
                Err(_) => {
                    self.failed.insert(url);
                }
            }
        }
        came
    }

    /// The logo of `url`, once it came.
    pub fn get(&self, url: &str) -> Option<&Pixels> {
        self.shown.get(url)
    }
}
