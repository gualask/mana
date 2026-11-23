//! AES with a key of 16, 24 or 32 bytes: decryption in GCM, CTR and CBC,
//! and CTR, the same both ways.

use aes::{Aes128, Aes192, Aes256};
use aes_gcm::aead::consts::U12;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{AesGcm, Nonce};
use cbc::cipher::{BlockDecryptMut, KeyIvInit, StreamCipher, block_padding::Pkcs7};

use crate::source::Mode;

macro_rules! by_key {
    ($key:expr, $cipher:ident => $body:expr) => {
        match $key.len() {
            16 => {
                type $cipher = Aes128;
                $body
            }
            24 => {
                type $cipher = Aes192;
                $body
            }
            32 => {
                type $cipher = Aes256;
                $body
            }
            n => Err(format!("AES key of {n} bytes")),
        }
    };
}

pub fn decrypt(mode: Mode, key: &[u8], iv: &[u8], data: &[u8], aad: Option<&[u8]>) -> Result<Vec<u8>, String> {
    match mode {
        Mode::Gcm => {
            if iv.len() != 12 {
                return Err(format!("GCM IV of {} bytes, 12 are needed", iv.len()));
            }
            by_key!(key, C => {
                let cipher = AesGcm::<C, U12>::new_from_slice(key).map_err(|e| e.to_string())?;
                cipher
                    .decrypt(Nonce::<U12>::from_slice(iv), Payload { msg: data, aad: aad.unwrap_or_default() })
                    .map_err(|_| "GCM authentication failed".to_string())
            })
        }
        Mode::Ctr => ctr(key, iv, data),
        Mode::Cbc => by_key!(key, C => {
            let decryptor = cbc::Decryptor::<C>::new_from_slices(key, iv).map_err(|e| e.to_string())?;
            let mut buffer = data.to_vec();
            let plain = decryptor.decrypt_padded_mut::<Pkcs7>(&mut buffer).map_err(|_| "CBC padding wrong".to_string())?;
            Ok(plain.to_vec())
        }),
    }
}

/// AES-CTR, the same both ways.
pub fn ctr(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    by_key!(key, C => {
        let mut cipher = ctr::Ctr128BE::<C>::new_from_slices(key, iv).map_err(|e| e.to_string())?;
        let mut out = data.to_vec();
        cipher.apply_keystream(&mut out);
        Ok(out)
    })
}
