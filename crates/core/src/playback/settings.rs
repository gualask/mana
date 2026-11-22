//! Tunables for playback experiments. The client reads them from where it
//! keeps them (on the TV, a file changed over SSH without reinstalling).

#[derive(Clone, Debug)]
pub struct Settings {
    /// How far ahead of the displayed frame data is fed.
    pub lead_ms: i64,
    /// Added to audio PTS only, to correct lip sync.
    pub audio_offset_ms: i64,
    pub loop_playback: bool,
    /// Frozen-decoder test: pauses the decoder after this many seconds of
    /// each opening (0: off), so a live source is reopened after 15 s.
    pub freeze_test_s: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self { lead_ms: 1500, audio_offset_ms: 0, loop_playback: true, freeze_test_s: 0 }
    }
}
