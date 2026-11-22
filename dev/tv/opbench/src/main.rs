//! Cost per operation of what the app's CPU flags change: rounding (a
//! `roundf` call without ARMv8's VRINT), parsing a decimal (the standard
//! library, built for the target's defaults), integer division (a library
//! call without hardware divide). The control line holds the loop and the
//! conversion that every other line also pays. Then the decryption of an
//! AES-128 segment, with the aes crate and as the app does it (the ARMv8
//! AES instructions, when the CPU has them).

use std::hint::black_box;
use std::time::Instant;

#[path = "../../../../crates/core/src/live/hls/decrypt.rs"]
mod decrypt;

/// A segment of Rai 1's 1080p variant: about 6 s at 5 Mbit/s.
const SEGMENT_BYTES: usize = 4 * 1024 * 1024;

fn bench(name: &str, count: u32, mut operation: impl FnMut(u32) -> u64) {
    let mut sum = 0u64;
    for i in 0..count / 10 {
        sum = sum.wrapping_add(operation(i));
    }
    let started = Instant::now();
    for i in 0..count {
        sum = sum.wrapping_add(operation(i));
    }
    let nanoseconds = started.elapsed().as_nanos() as f64 / f64::from(count);
    println!("{name:<28} {nanoseconds:7.1} ns  (somma {})", sum % 10);
}

fn main() {
    let floats: Vec<f32> = (0..1024).map(|i| i as f32 * 1.37 + 0.25).collect();
    let durations: Vec<String> = (0..1024).map(|i| format!("{}.{:03}", 2 + i % 7, i % 1000)).collect();
    let divisors: Vec<u32> = (0..1024).map(|i| 3 + i % 61).collect();
    let at = |i: u32| (i & 1023) as usize;
    bench("controllo (f32 mul+add)", 20_000_000, |i| (black_box(floats[at(i)]) * 1.5 + 0.5) as u64);
    bench("f32::round", 20_000_000, |i| black_box(floats[at(i)]).round() as u64);
    bench("f64::from_str (EXTINF)", 2_000_000, |i| black_box(&durations[at(i)]).parse::<f64>().unwrap() as u64);
    bench("u32 / u32", 20_000_000, |i| u64::from(black_box(i.wrapping_mul(2_654_435_761)) / black_box(divisors[at(i)])));
    println!("istruzioni AES: {}", if use_aes_instructions() { "sì" } else { "no" });
    decryption("AES-128-CBC, crate aes", decrypt::software);
    decryption("AES-128-CBC, come l'app", decrypt::decrypt);
}

/// As the app's platform layer: the AES instructions when the kernel
/// reports them (`HWCAP2_AES`).
fn use_aes_instructions() -> bool {
    #[cfg(target_arch = "arm")]
    {
        unsafe extern "C" {
            fn getauxval(kind: std::ffi::c_ulong) -> std::ffi::c_ulong;
        }
        // SAFETY: getauxval only reads the auxiliary vector (AT_HWCAP2).
        if unsafe { getauxval(26) } & 1 != 0 {
            // SAFETY: the kernel reports the instructions.
            unsafe { decrypt::assume_aes_instructions() };
            return true;
        }
    }
    false
}

/// Nanoseconds per byte and MB/s of `decrypt` on a segment, over 2 s.
fn decryption(name: &str, decrypt: fn(&[u8; 16], &[u8; 16], &mut Vec<u8>) -> Result<(), &'static str>) {
    let (key, iv) = ([0x2b; 16], [0x0f; 16]);
    let mut segment = vec![0x5a; SEGMENT_BYTES];
    // Random data has no valid padding: the error comes after the whole
    // segment is decrypted, and the segment stays the same length.
    let _ = decrypt(&key, &iv, black_box(&mut segment));
    segment.resize(SEGMENT_BYTES, 0x5a);
    let (started, mut bytes) = (Instant::now(), 0usize);
    while started.elapsed().as_secs_f64() < 2.0 {
        let _ = decrypt(&key, &iv, black_box(&mut segment));
        segment.resize(SEGMENT_BYTES, 0x5a);
        bytes += SEGMENT_BYTES;
    }
    let seconds = started.elapsed().as_secs_f64();
    println!("{name:<28} {:7.2} ns/byte  {:7.1} MB/s", seconds * 1e9 / bytes as f64, bytes as f64 / seconds / 1e6);
}
