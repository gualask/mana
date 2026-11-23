//! The bytes of one media playlist, or of a continuous stream, on their way
//! from its downloader thread to the demuxer: a bounded queue cut into
//! sections, and the stop that ends one opening of a source.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::live::Interruption;
use crate::playback::Input;

/// How often a thread waiting on the queue or on a delay checks the stop.
pub(super) const POLL: Duration = Duration::from_millis(100);

/// What the downloader queues for the demuxer.
pub(super) enum Chunk {
    Bytes(Vec<u8>),
    /// The bytes that follow start a section, with the discontinuity
    /// sequence number of its first segment. It ends the section being
    /// read, if any.
    Section(u64),
}

/// Stops the threads of one opening of a source: when the viewing stops, or
/// when this opening is interrupted and the session drops it.
#[derive(Clone)]
pub struct Stop {
    viewing: Arc<AtomicBool>,
    source: Arc<AtomicBool>,
    /// Why this opening interrupted itself, if it did.
    interruption: Arc<Mutex<Option<Interruption>>>,
    /// Whether a stuck playlist or a refused token ends this opening; while
    /// resolving the source, a refused token is the resolution's error.
    pub(super) retunes: bool,
}

impl Stop {
    /// For an opening of a source, which a stuck playlist or a refused token ends.
    pub fn new(viewing: &Arc<AtomicBool>) -> Self {
        Self {
            viewing: viewing.clone(),
            source: Arc::new(AtomicBool::new(false)),
            interruption: Arc::new(Mutex::new(None)),
            retunes: true,
        }
    }

    /// For resolving a source, which retries network errors until it
    /// succeeds or is stopped, and gives up on a refused token.
    pub fn resolving(viewing: &Arc<AtomicBool>) -> Self {
        Self { retunes: false, ..Self::new(viewing) }
    }

    pub fn set(&self) {
        self.source.store(true, Ordering::Relaxed);
    }

    /// Why this opening stopped itself, if it did. Stopping right away
    /// also unblocks a demuxer waiting on the other stream.
    pub fn interruption(&self) -> Option<Interruption> {
        self.interruption.lock().unwrap().clone()
    }

    /// Records the first interruption and stops the opening.
    pub(super) fn interrupt(&self, interruption: Interruption) {
        self.interruption.lock().unwrap().get_or_insert(interruption);
        self.set();
    }

    pub(super) fn is_set(&self) -> bool {
        self.viewing.load(Ordering::Relaxed) || self.source.load(Ordering::Relaxed)
    }
}

/// The demuxer's side of the queue. Reading blocks until a segment arrives;
/// the end of a section, of the stream or of the downloader reads as end of
/// file, and `next_section` tells them apart.
pub struct HlsStream {
    queue: Receiver<Chunk>,
    current: Vec<u8>,
    position: usize,
    /// Whether any byte was read: a section before that starts the stream.
    read_any: bool,
    /// Discontinuity sequence number of the section being read.
    sequence: u64,
    /// At the end of a section, the next one's sequence number: reads stay
    /// at end of file until `next_section`, so the old demuxer cannot read
    /// into the new section.
    next: Option<u64>,
    stop: Stop,
}

impl HlsStream {
    pub(super) fn new(queue: Receiver<Chunk>, stop: Stop) -> Self {
        Self { queue, current: Vec::new(), position: 0, read_any: false, sequence: 0, next: None, stop }
    }
}

impl Input for HlsStream {
    /// False at the end of the stream.
    fn next_section(&mut self) -> bool {
        let Some(next) = self.next.take() else { return false };
        self.sequence = next;
        true
    }

    /// The playlists of one variant number their discontinuities alike
    /// (RFC 8216 §6.2.2).
    fn discontinuity_sequence(&self) -> u64 {
        self.sequence
    }
}

impl Read for HlsStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.position == self.current.len() {
            if self.next.is_some() {
                return Ok(0);
            }
            if self.stop.is_set() {
                return Err(std::io::Error::other("stopped"));
            }
            match self.queue.recv_timeout(POLL) {
                Ok(Chunk::Bytes(segment)) => (self.current, self.position) = (segment, 0),
                Ok(Chunk::Section(sequence)) if !self.read_any => self.sequence = sequence,
                Ok(Chunk::Section(sequence)) => self.next = Some(sequence),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
        let count = buf.len().min(self.current.len() - self.position);
        buf[..count].copy_from_slice(&self.current[self.position..self.position + count]);
        self.position += count;
        self.read_any = true;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::sync_channel;

    use super::*;

    #[test]
    fn stream_reads_each_section_to_its_end() {
        let (sender, queue) = sync_channel(4);
        let mut stream = HlsStream::new(queue, Stop::new(&Arc::new(AtomicBool::new(false))));
        for chunk in [Chunk::Section(7), Chunk::Bytes(b"ab".to_vec()), Chunk::Section(8), Chunk::Bytes(b"cd".to_vec())] {
            sender.send(chunk).unwrap();
        }
        drop(sender);
        let mut first = Vec::new();
        stream.read_to_end(&mut first).unwrap();
        assert_eq!(first, b"ab");
        assert_eq!(stream.discontinuity_sequence(), 7);
        assert_eq!(stream.read(&mut [0; 4]).unwrap(), 0); // the old demuxer reads no further
        assert!(stream.next_section());
        assert_eq!(stream.discontinuity_sequence(), 8);
        let mut second = Vec::new();
        stream.read_to_end(&mut second).unwrap();
        assert_eq!(second, b"cd");
        assert!(!stream.next_section()); // the end of the stream
    }
}
