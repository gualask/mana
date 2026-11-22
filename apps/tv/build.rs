//! Decides the webOS build, the only 32-bit ARM Linux target: the code asks
//! `cfg(webos)`. There it compiles the crash handler and tells the binary
//! where the FFmpeg bundled with it is (arguments from the core's build
//! script, which links it, do not reach the binary).

fn main() {
    println!("cargo::rustc-check-cfg=cfg(webos)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if os != "linux" || arch != "arm" {
        return;
    }
    println!("cargo::rustc-cfg=webos");

    println!("cargo::rerun-if-changed=src/crash.c");
    cc::Build::new().file("src/crash.c").compile("mana_crash");
    // In the package's lib/. DT_RPATH rather than RUNPATH: it also applies
    // to the libraries' own dependencies on each other.
    println!("cargo::rustc-link-arg=-Wl,--disable-new-dtags,-rpath,$ORIGIN/lib");
}
