//! One segment as the demuxer wants it: decrypted with AES-128, its key
//! downloaded once per URL, and packed audio rewrapped in MPEG-TS.

use std::collections::HashMap;

use url::Url;

use super::decrypt::{self, Key};
use super::downloader::Failure;
use super::packed_audio::{TsRewrapper, is_packed_audio};
use super::playlist::Segment;
use crate::live::Cause;
use crate::net::Client;

/// What a segment, or an initialization section, may unpack to: 10 s at
/// 25 Mbit/s, a 4K HEVC channel, are 31 MB. With the segments queued for
/// the demuxer (download.rs), this bounds a stream's memory.
pub(super) const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// What the segments of one media playlist share: the AES-128 keys already
/// downloaded, and the MPEG-TS packing of its packed audio, continuous
/// across segments.
#[derive(Default)]
pub(super) struct Segments {
    keys: HashMap<Url, Key>,
    packed: TsRewrapper,
}

impl Segments {
    /// The segment's bytes, decrypted, packed audio rewrapped in MPEG-TS.
    pub(super) fn fetch(&mut self, client: &Client, segment: &Segment) -> Result<Vec<u8>, Failure> {
        let mut data = client.get(&segment.url, MAX_BYTES)?;
        if let Some(key) = &segment.key {
            let key_bytes = self.key(client, &key.url)?;
            // Without an explicit IV, HLS uses the media sequence number.
            let iv = key.iv.unwrap_or_else(|| (segment.sequence as u128).to_be_bytes());
            decrypt::decrypt(&key_bytes, &iv, &mut data)
                .map_err(|e| Failure::Transient(Cause::Decryption(e)))?;
        }
        if is_packed_audio(&data) {
            data = self.packed.rewrap(&data).map_err(|e| Failure::Unsupported(Cause::PackedAudio(e)))?;
        }
        Ok(data)
    }

    /// The AES-128 key at `url`, downloaded once.
    fn key(&mut self, client: &Client, url: &Url) -> Result<Key, Failure> {
        if let Some(key) = self.keys.get(url) {
            return Ok(*key);
        }
        let key: Key = client
            .get(url, size_of::<Key>() as u64)?
            .try_into()
            .map_err(|_| Failure::Transient(Cause::KeyLength))?;
        self.keys.insert(url.clone(), key);
        Ok(key)
    }
}
