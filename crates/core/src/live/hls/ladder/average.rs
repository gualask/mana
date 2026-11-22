//! The moving average of the download rate the ladder estimates the
//! bandwidth with.

/// An exponentially weighted moving average whose samples weigh their
/// download time (hls.js `EWMA`).
pub(super) struct Average {
    /// What is left of the average after one second.
    alpha: f64,
    value: f64,
    weight: f64,
}

impl Average {
    pub(super) fn new(half_life: f64) -> Self {
        Self { alpha: 0.5f64.powf(1.0 / half_life), value: 0.0, weight: 0.0 }
    }

    pub(super) fn add(&mut self, weight: f64, value: f64) {
        let kept = self.alpha.powf(weight);
        self.value = value * (1.0 - kept) + self.value * kept;
        self.weight += weight;
    }

    /// The average, corrected for starting from zero; None before any sample.
    pub(super) fn value(&self) -> Option<f64> {
        let filled = 1.0 - self.alpha.powf(self.weight);
        (self.weight > 0.0).then(|| self.value / filled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_start_from_the_first_sample() {
        let mut average = Average::new(3.0);
        assert_eq!(average.value(), None);
        average.add(0.5, 1000.0);
        assert!((average.value().unwrap() - 1000.0).abs() < 1e-6);
    }
}
