//! The installation's device identifier: a UUID v4 that some providers ask
//! for (Pluto's `clientID`), the same for every channel and across starts.
//! Created the first time a channel with a source is read, kept in the
//! data directory the app gives.

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::log::log;

/// The file in the data directory that keeps it.
const FILE: &str = "device";

pub struct Device {
    dir: PathBuf,
    id: OnceLock<Option<String>>,
}

impl Device {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, id: OnceLock::new() }
    }

    /// The identifier: the one kept, else a new one, kept for the next
    /// starts if the directory takes it and for this one anyway. None only
    /// when the system gives no random bytes.
    pub fn id(&self) -> Option<&str> {
        self.id.get_or_init(|| self.kept().or_else(|| self.create())).as_deref()
    }

    fn kept(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.dir.join(FILE)).ok()?;
        let id = text.trim();
        is_uuid(id).then(|| id.to_string())
    }

    fn create(&self) -> Option<String> {
        let mut bytes = [0; 16];
        if let Err(error) = getrandom::getrandom(&mut bytes) {
            log!("device identifier not created: {error}");
            return None;
        }
        let id = uuid_v4(bytes);
        let kept = crate::files::write_whole(&self.dir.join(FILE), id.as_bytes());
        match kept {
            Ok(()) => log!("new device identifier"),
            Err(error) => log!("device identifier for this session only: {error}"),
        }
        Some(id)
    }
}

/// `bytes` as a version 4, variant 1 UUID (RFC 9562), lowercase.
fn uuid_v4(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// Whether `text` is a UUID as `uuid_v4` writes it.
fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uuid_has_its_version_and_variant() {
        let id = uuid_v4([0xff; 16]);
        assert_eq!(id, "ffffffff-ffff-4fff-bfff-ffffffffffff");
        assert!(is_uuid(&id) && !is_uuid("ffffffff-ffff-4fff-bfff-fffffffffff") && !is_uuid(&id.replace('-', "_")));
    }

    #[test]
    fn the_identifier_is_kept_across_starts() {
        let dir = std::env::temp_dir().join(format!("mana-device-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = Device::new(dir.clone()).id().map(str::to_string).unwrap();
        assert!(is_uuid(&first));
        assert_eq!(Device::new(dir.clone()).id(), Some(first.as_str()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn without_a_directory_it_lasts_the_session() {
        let device = Device::new(PathBuf::from("/dev/null/mana"));
        let id = device.id().map(str::to_string);
        assert!(id.as_deref().is_some_and(is_uuid));
        assert_eq!(device.id().map(str::to_string), id);
    }
}
