//! The bytes of one media playlist, or of a continuous stream, on their way
//! from its downloader thread to the demuxer: a bounded queue cut into
//! sections, and the stop that ends one opening of a source.

use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
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

/// A queue of at most `chunks` chunks and, past the first, `bytes` bytes
/// waiting for the demuxer, ended by `stop`: the downloader's side and the
/// demuxer's.
pub(super) fn queue(chunks: usize, bytes: usize, stop: Stop) -> (Sender, HlsStream) {
    let (sender, receiver) = sync_channel(chunks);
    let queued = Arc::new(AtomicUsize::new(0));
    let stream = HlsStream {
        queue: receiver,
        queued: queued.clone(),
        current: Vec::new(),
        position: 0,
        read_any: false,
        sequence: 0,
        next: None,
        stop: stop.clone(),
    };
    (Sender { chunks: sender, queued, max_bytes: bytes, stop }, stream)
}

/// The downloader's side of the queue.
pub(super) struct Sender {
    chunks: SyncSender<Chunk>,
    /// Bytes sent and not yet taken by the demuxer, shared with it.
    queued: Arc<AtomicUsize>,
    max_bytes: usize,
    stop: Stop,
}

/// The demuxer is gone, or the opening stopped.
#[derive(Debug)]
pub(super) struct Gone;

impl Sender {
    /// Queues `chunk` once the queue has room for it: a chunk heavier than
    /// the bound in bytes goes alone, when nothing else waits.
    pub(super) fn send(&self, chunk: Chunk) -> Result<(), Gone> {
        if let Chunk::Bytes(bytes) = &chunk {
            self.wait_for_room(bytes.len())?;
            self.queued.fetch_add(bytes.len(), Ordering::AcqRel);
        }
        self.chunks.send(chunk).map_err(|_| Gone)
    }

    /// Waits until `len` more bytes fit in the queue, or it holds none.
    /// Only this thread adds: the room seen stays there.
    fn wait_for_room(&self, len: usize) -> Result<(), Gone> {
        loop {
            let queued = self.queued.load(Ordering::Acquire);
            if queued == 0 || queued + len <= self.max_bytes {
                return Ok(());
            }
            // The demuxer's side dropped its count with itself.
            if self.stop.is_set() || Arc::strong_count(&self.queued) == 1 {
                return Err(Gone);
            }
            std::thread::sleep(POLL);
        }
    }
}

/// The demuxer's side of the queue. Reading blocks until a segment arrives;
/// the end of a section, of the stream or of the downloader reads as end of
/// file, and `next_section` tells them apart.
pub struct HlsStream {
    queue: Receiver<Chunk>,
    /// The bytes in `queue`, which the downloader waits on.
    queued: Arc<AtomicUsize>,
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
        if !self.wait_for_bytes()? {
            return Ok(0);
        }
        let count = buf.len().min(self.current.len() - self.position);
        buf[..count].copy_from_slice(&self.current[self.position..self.position + count]);
        self.position += count;
        self.read_any = true;
        Ok(count)
    }
}

impl HlsStream {
    /// Waits until bytes of the section being read are at hand; false at
    /// the end of the section or of the stream.
    fn wait_for_bytes(&mut self) -> std::io::Result<bool> {
        while self.position == self.current.len() {
            if self.next.is_some() {
                return Ok(false);
            }
            if self.stop.is_set() {
                return Err(std::io::Error::other("stopped"));
            }
            match self.queue.recv_timeout(POLL) {
                Ok(Chunk::Bytes(segment)) => {
                    self.queued.fetch_sub(segment.len(), Ordering::AcqRel);
                    (self.current, self.position) = (segment, 0);
                }
                Ok(Chunk::Section(sequence)) if !self.read_any => self.sequence = sequence,
                Ok(Chunk::Section(sequence)) => self.next = Some(sequence),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Ok(false),
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop() -> Stop {
        Stop::new(&Arc::new(AtomicBool::new(false)))
    }

    #[test]
    fn stream_reads_each_section_to_its_end() {
        let (sender, mut stream) = queue(4, 1024, stop());
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

    /// Steps of a downloader sending `chunks`, counted as each is queued.
    fn send_counting(sender: Sender, chunks: Vec<&'static [u8]>) -> (Arc<AtomicUsize>, std::thread::JoinHandle<bool>) {
        let sent = Arc::new(AtomicUsize::new(0));
        let counter = sent.clone();
        let thread = std::thread::spawn(move || {
            chunks.into_iter().all(|chunk| {
                let queued = sender.send(Chunk::Bytes(chunk.to_vec())).is_ok();
                counter.fetch_add(usize::from(queued), Ordering::SeqCst);
                queued
            })
        });
        (sent, thread)
    }

    fn wait_for(sent: &AtomicUsize, count: usize) {
        while sent.load(Ordering::SeqCst) < count {
            std::thread::yield_now();
        }
    }

    #[test]
    fn bytes_past_the_bound_wait_for_the_demuxer() {
        let (sender, mut stream) = queue(8, 4, stop());
        // Heavier than the bound, it goes alone; the next one waits for it to be taken.
        let (sent, thread) = send_counting(sender, vec![b"abcdef", b"gh"]);
        wait_for(&sent, 1);
        std::thread::sleep(3 * POLL);
        assert_eq!(sent.load(Ordering::SeqCst), 1, "the second chunk waits while the first is queued");
        let mut read = Vec::new();
        stream.read_to_end(&mut read).unwrap();
        assert!(thread.join().unwrap());
        assert_eq!(read, b"abcdefgh");
    }

    #[test]
    fn chunks_within_the_bound_queue_together() {
        let (sender, _stream) = queue(8, 4, stop());
        let (sent, thread) = send_counting(sender, vec![b"ab", b"cd"]);
        assert!(thread.join().unwrap());
        assert_eq!(sent.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_waiting_downloader_ends_when_the_demuxer_is_gone() {
        let (sender, stream) = queue(8, 4, stop());
        let (sent, thread) = send_counting(sender, vec![b"abcd", b"ef"]);
        wait_for(&sent, 1);
        drop(stream);
        assert!(!thread.join().unwrap(), "the second chunk is not queued");
        assert_eq!(sent.load(Ordering::SeqCst), 1);
    }
}
