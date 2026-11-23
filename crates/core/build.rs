//! Compiles the FFmpeg demuxer shim (src/playback/ffmpeg/*.c) and links FFmpeg. Paths are relative to
//! the workspace root.
//!
//! For the TV (32-bit ARM Linux): the LGPL build bundled with the app
//! (`tools/webos/ffmpeg.py`, found at run time in the app's lib/ folder), or
//! another bundled build whose prefix `MANA_FFMPEG` names. The TV's own
//! FFmpeg changes ABI from one webOS generation to the next.
//!
//! On the desktop: the prefix `MANA_FFMPEG` names, by default Homebrew's or
//! the one `pkg-config` knows (Linux distributions).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where Homebrew installs FFmpeg, on Apple silicon and on Intel.
const HOMEBREW_FFMPEG: [&str; 2] = ["/opt/homebrew/opt/ffmpeg", "/usr/local/opt/ffmpeg"];

fn main() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let tv = os == "linux" && arch == "arm";

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.ancestors().nth(2).expect("crates/core sits two levels below the workspace root");
    println!("cargo::rerun-if-env-changed=MANA_FFMPEG");
    println!("cargo::rerun-if-env-changed=PKG_CONFIG_PATH");
    let ffmpeg = match std::env::var("MANA_FFMPEG").ok() {
        Some(choice) => Ffmpeg::at(&root.join(choice)),
        None if tv => Ffmpeg::at(&root.join("private/ffmpeg/prefix")),
        None => HOMEBREW_FFMPEG
            .iter()
            .map(|prefix| Ffmpeg::at(Path::new(prefix)))
            .find(Ffmpeg::has_headers)
            .or_else(Ffmpeg::from_pkg_config)
            .unwrap_or_else(|| {
                panic!(
                    "FFmpeg non trovato: installarlo (Homebrew `ffmpeg`, su Linux i pacchetti di sviluppo di \
                     libavformat) o indicarne il prefisso con MANA_FFMPEG"
                )
            }),
    };
    assert!(
        ffmpeg.has_headers(),
        "missing {}: {}",
        ffmpeg.include.display(),
        if tv { "run python3 tools/webos/ffmpeg.py" } else { "MANA_FFMPEG must name a prefix with include/ and lib/" }
    );
    println!("cargo::rerun-if-changed={}", ffmpeg.include.join("libavformat/version.h").display());
    println!("cargo::rerun-if-changed=src/playback/ffmpeg");
    cc::Build::new()
        .file("src/playback/ffmpeg/shim.c")
        .file("src/playback/ffmpeg/probe.c")
        .file("src/playback/ffmpeg/mp4.c")
        .file("src/playback/ffmpeg/transcoder.c")
        .include(&ffmpeg.include)
        .flag("-Wno-deprecated-declarations")
        .compile("mana_ffmpeg");
    println!("cargo::rustc-link-search=native={}", ffmpeg.lib.display());
    for lib in ["avformat", "avcodec", "avutil"] {
        println!("cargo::rustc-link-lib=dylib={lib}");
    }
}

/// Where FFmpeg's headers and libraries are.
struct Ffmpeg {
    include: PathBuf,
    lib: PathBuf,
}

impl Ffmpeg {
    fn at(prefix: &Path) -> Self {
        Self { include: prefix.join("include"), lib: prefix.join("lib") }
    }

    /// As `pkg-config` reports libavformat, when it is installed.
    fn from_pkg_config() -> Option<Self> {
        let variable = |name: &str| {
            let output = Command::new("pkg-config").args([&format!("--variable={name}"), "libavformat"]).output().ok()?;
            let value = String::from_utf8(output.stdout).ok()?;
            (output.status.success() && !value.trim().is_empty()).then(|| PathBuf::from(value.trim()))
        };
        let ffmpeg = Self { include: variable("includedir")?, lib: variable("libdir")? };
        ffmpeg.has_headers().then_some(ffmpeg)
    }

    fn has_headers(&self) -> bool {
        self.include.join("libavformat/avformat.h").is_file()
    }
}
