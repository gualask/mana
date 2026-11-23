//! Samples where the threads of a process run: a `cpu-clock` perf event at
//! about 1 kHz on each thread (new threads are picked up every 250 ms),
//! read from the kernel's ring buffers. The TV has the perf events but no
//! `perf` tool.
//!
//! Usage: mana-sampler PID SECONDS. Prints `TID MODE IP` per sample (MODE
//! 1 kernel, 2 user), then `thread TID NAME`, `lost N`, `frequency HZ`,
//! `seconds N` and the process maps as `map LINE`, for dev/tv/profile.py to
//! resolve.

use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{Ordering, fence};
use std::time::{Duration, Instant};

const SYS_PERF_EVENT_OPEN: libc::c_long = 364; // ARM EABI
const PERF_EVENT_IOC_ENABLE: libc::c_ulong = 0x2400;
const PERF_TYPE_SOFTWARE: u32 = 1;
const PERF_COUNT_SW_CPU_CLOCK: u64 = 0;
const PERF_SAMPLE_IP: u64 = 1;
const PERF_SAMPLE_TID: u64 = 2;
const PERF_RECORD_LOST: u32 = 2;
const PERF_RECORD_SAMPLE: u32 = 9;
const FLAG_DISABLED: u64 = 1 << 0;
const FLAG_FREQ: u64 = 1 << 10;
const FREQUENCY: u64 = 997;
const DATA_PAGES: usize = 16;
const RESCAN: Duration = Duration::from_millis(250);
const DRAIN: Duration = Duration::from_millis(20);

/// `perf_event_attr` up to PERF_ATTR_SIZE_VER5.
#[repr(C)]
#[derive(Default)]
struct Attr {
    kind: u32,
    size: u32,
    config: u64,
    sample_freq: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events: u32,
    bp_type: u32,
    config1: u64,
    config2: u64,
    branch_sample_type: u64,
    sample_regs_user: u64,
    sample_stack_user: u32,
    clockid: i32,
    sample_regs_intr: u64,
    aux_watermark: u32,
    reserved: u32,
}

/// The mapped ring buffer of one thread's event.
struct Ring {
    base: *mut u8,
    page: usize,
}

impl Attr {
    /// Samples of a thread's instruction pointer and id, `FREQUENCY` times
    /// a second of CPU time; created disabled.
    fn cpu_clock() -> Self {
        Self {
            kind: PERF_TYPE_SOFTWARE,
            size: size_of::<Self>() as u32,
            config: PERF_COUNT_SW_CPU_CLOCK,
            sample_freq: FREQUENCY,
            sample_type: PERF_SAMPLE_IP | PERF_SAMPLE_TID,
            flags: FLAG_DISABLED | FLAG_FREQ,
            ..Default::default()
        }
    }
}

impl Ring {
    /// Opens and starts the event of thread `tid`, if the kernel allows it.
    fn open(tid: i32) -> Option<Self> {
        let attr = Attr::cpu_clock();
        // SAFETY: plain syscalls on a valid attribute; the mapping is
        // checked and lives for the rest of the process.
        unsafe {
            let fd = libc::syscall(SYS_PERF_EVENT_OPEN, &attr as *const Attr, tid, -1i32, -1i32, 0u32) as i32;
            if fd < 0 {
                return None;
            }
            let page = libc::sysconf(libc::_SC_PAGESIZE) as usize;
            let protection = libc::PROT_READ | libc::PROT_WRITE;
            let base = libc::mmap(std::ptr::null_mut(), (1 + DATA_PAGES) * page, protection, libc::MAP_SHARED, fd, 0);
            if base == libc::MAP_FAILED {
                libc::close(fd);
                return None;
            }
            libc::ioctl(fd, PERF_EVENT_IOC_ENABLE as _, 0);
            Some(Self { base: base as *mut u8, page })
        }
    }

    /// Writes the samples gathered since the last call; counts lost ones.
    fn drain(&self, out: &mut impl Write, lost: &mut u64) {
        // SAFETY: `data_head` and `data_tail` of `perf_event_mmap_page`, at
        // the start of the mapping.
        let (head, tail_ptr) = unsafe { (std::ptr::read_volatile(self.base.add(1024) as *const u64), self.base.add(1032) as *mut u64) };
        fence(Ordering::Acquire);
        // SAFETY: as above.
        let mut tail = unsafe { std::ptr::read_volatile(tail_ptr) };
        while tail < head {
            let Some(length) = self.record(tail, out, lost) else { break };
            tail += length;
        }
        fence(Ordering::Release);
        // SAFETY: as above.
        unsafe { std::ptr::write_volatile(tail_ptr, tail) };
    }

    /// Writes the sample at offset `at`, or counts it when the kernel lost
    /// some; returns the record's length, `None` for an empty one.
    fn record(&self, at: u64, out: &mut impl Write, lost: &mut u64) -> Option<u64> {
        let header: [u8; 8] = self.read(at);
        let kind = u32::from_le_bytes(header[0..4].try_into().unwrap());
        let mode = u16::from_le_bytes(header[4..6].try_into().unwrap()) & 7;
        let length = u16::from_le_bytes(header[6..8].try_into().unwrap()) as u64;
        if length == 0 {
            return None;
        }
        if kind == PERF_RECORD_SAMPLE {
            let body: [u8; 16] = self.read(at + 8);
            let ip = u64::from_le_bytes(body[0..8].try_into().unwrap());
            let tid = u32::from_le_bytes(body[12..16].try_into().unwrap());
            let _ = writeln!(out, "{tid} {mode} {ip:x}");
        } else if kind == PERF_RECORD_LOST {
            *lost += 1;
        }
        Some(length)
    }

    /// The bytes of the data area from offset `at`, which wraps around.
    fn read<const N: usize>(&self, at: u64) -> [u8; N] {
        let size = (DATA_PAGES * self.page) as u64;
        // SAFETY: the data area follows the first page of the mapping, and
        // every offset is taken modulo its size.
        std::array::from_fn(|i| unsafe { *self.base.add(self.page + ((at + i as u64) % size) as usize) })
    }
}

/// The threads of process `pid`, with their names.
fn threads(pid: i32) -> Vec<(i32, String)> {
    let Ok(entries) = std::fs::read_dir(format!("/proc/{pid}/task")) else { return Vec::new() };
    entries
        .filter_map(|entry| {
            let tid: i32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
            let name = std::fs::read_to_string(format!("/proc/{pid}/task/{tid}/comm")).unwrap_or_default();
            Some((tid, name.trim().to_string()))
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(pid), Some(seconds)) = (args.get(1).and_then(|a| a.parse().ok()), args.get(2).and_then(|a| a.parse().ok()))
    else {
        eprintln!("usage: mana-sampler PID SECONDS");
        std::process::exit(2);
    };
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let mut lost = 0;
    let names = sample(pid, Duration::from_secs(seconds), &mut out, &mut lost);
    for (tid, name) in &names {
        let _ = writeln!(out, "thread {tid} {name}");
    }
    let _ = writeln!(out, "lost {lost}");
    let _ = writeln!(out, "frequency {FREQUENCY}");
    let _ = writeln!(out, "seconds {seconds}");
    for line in std::fs::read_to_string(format!("/proc/{pid}/maps")).unwrap_or_default().lines() {
        let _ = writeln!(out, "map {line}");
    }
}

/// Samples the threads of `pid` for `length`, picking up new ones as they
/// start; returns the name of each thread seen.
fn sample(pid: i32, length: Duration, out: &mut impl Write, lost: &mut u64) -> HashMap<i32, String> {
    let (mut rings, mut names) = (HashMap::new(), HashMap::new());
    let end = Instant::now() + length;
    let mut scanned: Option<Instant> = None;
    while Instant::now() < end {
        if scanned.is_none_or(|at| at.elapsed() >= RESCAN) {
            scanned = Some(Instant::now());
            watch(pid, &mut rings, &mut names);
        }
        rings.values().for_each(|ring| ring.drain(out, lost));
        std::thread::sleep(DRAIN);
    }
    rings.values().for_each(|ring| ring.drain(out, lost));
    names
}

/// Opens the event of each thread of `pid` not watched yet.
fn watch(pid: i32, rings: &mut HashMap<i32, Ring>, names: &mut HashMap<i32, String>) {
    for (tid, name) in threads(pid) {
        names.insert(tid, name);
        if !rings.contains_key(&tid)
            && let Some(ring) = Ring::open(tid)
        {
            rings.insert(tid, ring);
        }
    }
}
