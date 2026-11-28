//! The files of the data folder that the next starts read again.

use std::io;
use std::path::Path;

/// Writes `bytes` at `path` whole, through a temporary file beside it, and
/// creates its folder if needed: a power-off meanwhile leaves the previous
/// file, never a part of the new one.
pub(crate) fn write_whole(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".new");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_replaced_whole_and_its_folder_created() {
        let dir = std::env::temp_dir().join(format!("mana-files-{}", std::process::id()));
        let path = dir.join("inner").join("kept.txt");
        write_whole(&path, b"first").unwrap();
        write_whole(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        assert!(!dir.join("inner").join("kept.txt.new").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
