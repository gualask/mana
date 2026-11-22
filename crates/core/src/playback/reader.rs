//! The demuxing thread: reads packets ahead of the feeder, so that the
//! feeder keeps control of the decoder while data is missing instead of
//! waiting inside a read.

use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use super::Error;
use super::demux::{Demux, Formats, Packet};

/// Packets read ahead of the feeder: a fraction of a second.
const READ_AHEAD: usize = 16;

/// What the demuxing thread hands the feeder, in order.
pub enum Demuxed {
    Packet(Packet),
    /// The packets that follow belong to a new section: its formats, when
    /// known, and whether it lacks the audio the stream had.
    Section { formats: Option<(Formats, String)>, without_audio: bool },
    /// A local file started again from its beginning.
    Looped,
    Failed(String),
}

pub struct Reader {
    /// Formats of the first section, and their description.
    pub formats: (Formats, String),
    /// Whether the first section lacks the audio the stream has.
    pub without_audio: bool,
    demuxed: Receiver<Demuxed>,
    thread: JoinHandle<()>,
}

impl Reader {
    /// Starts reading `demux` on its own thread; a local file is read again
    /// from the start at its end when `loop_file`.
    pub fn start(demux: Demux, loop_file: bool) -> Result<Self, Error> {
        let formats = Formats::of(&demux)?;
        let without_audio = demux.section_without_audio();
        let (sender, demuxed) = sync_channel(READ_AHEAD);
        let thread = std::thread::Builder::new()
            .name("demux".into())
            .spawn(move || read_ahead(demux, loop_file, &sender))
            .map_err(|e| Error::Failed(e.to_string()))?;
        Ok(Self { formats, without_audio, demuxed, thread })
    }

    /// The next item, or None when none arrived within `wait`.
    pub fn next(&self, wait: Duration) -> Result<Option<Demuxed>, Error> {
        match self.demuxed.recv_timeout(wait) {
            Ok(next) => Ok(Some(next)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(Error::Failed("demuxer interrotto".into())),
        }
    }

    /// Stops reading and waits for the thread. A live input must be stopped
    /// first: the thread may be waiting on it for data.
    pub fn finish(self) {
        let Self { demuxed, thread, .. } = self;
        drop(demuxed);
        let _ = thread.join();
    }
}

/// Reads packets until the end, a failure or the reader gone.
fn read_ahead(mut demux: Demux, loop_file: bool, demuxed: &SyncSender<Demuxed>) {
    loop {
        let read = demux.read();
        // The packet just read starts the section, if a new one began.
        let section = demux.take_new_section().then(|| Demuxed::Section {
            formats: Formats::of(&demux).ok(),
            without_audio: demux.section_without_audio(),
        });
        let next = match read {
            Ok(Packet::Eof) if loop_file => demux.rewind().map_or_else(Demuxed::Failed, |()| Demuxed::Looped),
            Ok(packet) => Demuxed::Packet(packet),
            Err(error) => Demuxed::Failed(error),
        };
        let last = matches!(next, Demuxed::Packet(Packet::Eof) | Demuxed::Failed(_));
        if section.is_some_and(|section| demuxed.send(section).is_err()) || demuxed.send(next).is_err() || last {
            return;
        }
    }
}
