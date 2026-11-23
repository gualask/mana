//! AES-128-CBC decryption of HLS segments (RFC 8216 §4.3.2.4), with PKCS#7
//! padding: with the ARMv8 AES instructions once the app declares that a
//! 32-bit ARM CPU has them, as the B1's Cortex-A73 does in AArch32, which
//! neither ring nor the aes crate use there (docs/sources/performance.md);
//! else with the aes crate, which detects AES-NI on x86 and uses ARMv8's on
//! aarch64 only when built with `--cfg aes_armv8`. Telling whether the CPU
//! has them is the platform's: the core has no code of one OS.

#[cfg(target_arch = "arm")]
use aes::cipher::block_padding::RawPadding;
use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockDecryptMut, KeyIvInit};

const BLOCK: usize = 16;

/// An AES-128 key.
pub(super) type Key = [u8; 16];

/// From now on, decrypts with the ARMv8 AES instructions.
///
/// # Safety
///
/// The CPU must have them (Linux reports them as `HWCAP2_AES`): without,
/// the process ends on an illegal instruction.
#[cfg(target_arch = "arm")]
pub unsafe fn assume_aes_instructions() {
    armv8::ASSUMED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Why a segment does not decrypt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Its length is not a multiple of the 16-byte block.
    Length,
    /// Its PKCS#7 padding is invalid: a wrong key or IV, or a damaged segment.
    Padding,
}

/// Decrypts `data` in place and removes its padding.
pub(super) fn decrypt(key: &Key, iv: &[u8; BLOCK], data: &mut Vec<u8>) -> Result<(), Error> {
    #[cfg(target_arch = "arm")]
    if armv8::assumed() {
        if data.is_empty() || !data.len().is_multiple_of(BLOCK) {
            return Err(Error::Length);
        }
        armv8::decrypt(key, iv, data);
        return unpad(data);
    }
    software(key, iv, data)
}

/// As `decrypt`, with the aes crate.
pub(super) fn software(key: &Key, iv: &[u8; BLOCK], data: &mut Vec<u8>) -> Result<(), Error> {
    let length = cbc::Decryptor::<aes::Aes128>::new(key.into(), iv.into())
        .decrypt_padded_mut::<Pkcs7>(data)
        .map_err(|_| Error::Padding)?
        .len();
    data.truncate(length);
    Ok(())
}

/// Removes the PKCS#7 padding of decrypted `data`, whole blocks, as the
/// aes crate does in `software`.
#[cfg(target_arch = "arm")]
fn unpad(data: &mut Vec<u8>) -> Result<(), Error> {
    let last = data.len() - BLOCK;
    let kept = Pkcs7::raw_unpad(&data[last..]).map_err(|_| Error::Padding)?.len();
    data.truncate(last + kept);
    Ok(())
}

/// The AES instructions of ARMv8 in AArch32 (AESD, AESIMC, AESE), written
/// with their own `.fpu` since the app builds for a Cortex-A9; used only
/// once the app assumes them. The rounds follow the equivalent inverse
/// cipher (FIPS-197 §5.3.5), as Linux's aes-ce-core.S.
#[cfg(target_arch = "arm")]
mod armv8 {
    use std::arch::asm;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// Set by `assume_aes_instructions`.
    pub(super) static ASSUMED: AtomicBool = AtomicBool::new(false);

    pub(super) fn assumed() -> bool {
        ASSUMED.load(Ordering::Relaxed)
    }

    /// Decrypts `data`, whole blocks and at least one, in place.
    pub(super) fn decrypt(key: &super::Key, iv: &[u8; super::BLOCK], data: &mut [u8]) {
        let keys = decryption_keys(key);
        let blocks = data.len() / super::BLOCK;
        assert!(blocks > 0 && data.len().is_multiple_of(super::BLOCK));
        // SAFETY: the CPU has the instructions (`assumed`); the keys are
        // 11 blocks, `iv` one, `data` `blocks` blocks, all read and written
        // within their bounds; the registers used are declared clobbered.
        unsafe {
            asm!(
                ".arch armv8-a",
                ".fpu crypto-neon-fp-armv8",
                // Round keys in q5-q15, the previous ciphertext in q4.
                "vld1.8 {{d10-d13}}, [{keys}]!",
                "vld1.8 {{d14-d17}}, [{keys}]!",
                "vld1.8 {{d18-d21}}, [{keys}]!",
                "vld1.8 {{d22-d25}}, [{keys}]!",
                "vld1.8 {{d26-d29}}, [{keys}]!",
                "vld1.8 {{d30-d31}}, [{keys}]",
                "vld1.8 {{d8-d9}}, [{iv}]",
                "1:",
                "vld1.8 {{d0-d1}}, [{data}]",
                "vmov q1, q0",
                "aesd.8 q0, q5",
                "aesimc.8 q0, q0",
                "aesd.8 q0, q6",
                "aesimc.8 q0, q0",
                "aesd.8 q0, q7",
                "aesimc.8 q0, q0",
                "aesd.8 q0, q8",
                "aesimc.8 q0, q0",
                "aesd.8 q0, q9",
                "aesimc.8 q0, q0",
                "aesd.8 q0, q10",
                "aesimc.8 q0, q0",
                "aesd.8 q0, q11",
                "aesimc.8 q0, q0",
                "aesd.8 q0, q12",
                "aesimc.8 q0, q0",
                "aesd.8 q0, q13",
                "aesimc.8 q0, q0",
                "aesd.8 q0, q14",
                "veor q0, q0, q15",
                // CBC: the plaintext is the block's output XOR the previous
                // ciphertext.
                "veor q0, q0, q4",
                "vmov q4, q1",
                "vst1.8 {{d0-d1}}, [{data}]!",
                "subs {blocks}, {blocks}, #1",
                "bne 1b",
                keys = inout(reg) keys.as_ptr() => _,
                iv = in(reg) iv.as_ptr(),
                data = inout(reg) data.as_mut_ptr() => _,
                blocks = inout(reg) blocks => _,
                out("q0") _, out("q1") _, out("q4") _, out("q5") _, out("q6") _, out("q7") _,
                out("q8") _, out("q9") _, out("q10") _, out("q11") _, out("q12") _, out("q13") _,
                out("q14") _, out("q15") _,
                options(nostack),
            );
        }
    }

    /// The round keys of the equivalent inverse cipher: the encryption ones
    /// in reverse order, the inner ones through InvMixColumns.
    fn decryption_keys(key: &super::Key) -> [[u8; 16]; 11] {
        let words = expand_key(key);
        let round = |r: usize| {
            let mut bytes = [0u8; 16];
            for (chunk, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(&words[4 * r..4 * r + 4]) {
                *chunk = word.to_le_bytes();
            }
            bytes
        };
        let mut keys = [[0u8; 16]; 11];
        keys[0] = round(10);
        for (i, key) in keys.iter_mut().enumerate().take(10).skip(1) {
            *key = inv_mix_columns(round(10 - i));
        }
        keys[10] = round(0);
        keys
    }

    /// The 44 words of the encryption round keys (FIPS-197 §5.2), little
    /// endian: a key's first byte is the low byte of its first word.
    fn expand_key(key: &super::Key) -> [u32; 44] {
        let mut words = [0u32; 44];
        for (i, word) in key.as_chunks::<4>().0.iter().enumerate() {
            words[i] = u32::from_le_bytes(*word);
        }
        let mut rcon = 1u32;
        for i in 4..44 {
            let mut word = words[i - 1];
            if i % 4 == 0 {
                // SubWord(RotWord(word)) ^ Rcon: RotWord moves the first
                // byte last, the low byte high.
                word = sub_word(word).rotate_right(8) ^ rcon;
                rcon = xtime(rcon);
            }
            words[i] = words[i - 4] ^ word;
        }
        words
    }

    /// Multiplication by x in GF(2^8), the next Rcon.
    fn xtime(byte: u32) -> u32 {
        let shifted = byte << 1;
        if byte & 0x80 != 0 { shifted ^ 0x11b } else { shifted }
    }

    /// SubWord: AESE with a zero key on the word in every column, where
    /// ShiftRows moves nothing (as Linux's ce_aes_sub).
    fn sub_word(word: u32) -> u32 {
        let out: u32;
        // SAFETY: as in `decrypt`; registers only.
        unsafe {
            asm!(
                ".arch armv8-a",
                ".fpu crypto-neon-fp-armv8",
                "vdup.32 q0, {word}",
                "vmov.i8 q1, #0",
                "aese.8 q0, q1",
                "vmov.32 {out}, d0[0]",
                word = in(reg) word,
                out = lateout(reg) out,
                out("q0") _, out("q1") _,
                options(pure, nomem, nostack),
            );
        }
        out
    }

    fn inv_mix_columns(mut block: [u8; 16]) -> [u8; 16] {
        // SAFETY: as in `decrypt`; one block read and written in place.
        unsafe {
            asm!(
                ".arch armv8-a",
                ".fpu crypto-neon-fp-armv8",
                "vld1.8 {{d0-d1}}, [{block}]",
                "aesimc.8 q0, q0",
                "vst1.8 {{d0-d1}}, [{block}]",
                block = in(reg) block.as_mut_ptr(),
                out("q0") _,
                options(nostack),
            );
        }
        block
    }
}

#[cfg(test)]
mod tests;
