# AGENTS.md

## Verifying a change

- Everywhere: `cargo test`, plus `python3 -B -m unittest discover -s dev/lab/tests` when `dev/lab/` changes. The other checks are listed in [docs/development.md](docs/development.md#checks).
- TV code, build flags or `tools/webos/`: run `cargo zigbuild --release --target armv7-unknown-linux-gnueabi.2.28`, then `python3 tools/webos/package.py`. A desktop build proves nothing for the TV. The packager is also the test: it fails on the wrong zig or cargo-zigbuild version, software float calls, glibc above 2.28, or a library or function missing from a webOS 6–11 firmware ([docs/tv-app.md](docs/tv-app.md#package-checks)). The first build needs `python3 tools/webos/ffmpeg.py`.
- Rust, zig and cargo-zigbuild are pinned (`rust-toolchain.toml`, `tools/webos/target.py`). Bump them only on purpose, after the package checks pass and the app plays on the TV.

## Conventions

- Docs, code, comments and commit messages are in English. Every Markdown file goes under `docs/` (`docs/todo.md` too), except `README.md`, which is for end users only (what the app does, formats, requirements, use), and this file. Text the app shows, on screen and on the phone page, is in English and Italian: it lives in the Fluent catalogs `crates/core/src/language/en.ftl` and `it.ftl`, read through `text!`, and a new text goes in both (`cargo test` checks the catalogs against the sources). The log, and the keys of the channel-list format, are in English.
- One branch, `main`.
- Experiments and spikes go in `.local/` (gitignored), never in the sources.
- `private/` (gitignored) holds TV access (`tv.env`), real stream sources and anything that may not be redistributed.
- Log only what happens rarely. Anything that repeats while watching goes through `debug!`, which shows only with `debug = true` in the settings.

## Rules from outside the code

- Never commit real channels, stream URLs, lists or recipes aimed at a real service. Examples and tests use loopback, `.invalid` or `.example`.
- Before deciding an HLS or HTTP behaviour, compare it with hls.js, Media3 and FFmpeg's `libavformat/hls.c`. Before a webOS build or compatibility choice, compare it with how other native webOS apps do it ([docs/sources/tv-lg.md](docs/sources/tv-lg.md)). Say what you compared.
- webOS 6 is the minimum supported version.
- The TV is the user's own, rooted, reached over SSH (`tools/webos/tv.py`). Ask before installing or launching anything on it: some actions need a confirmation on the remote.
- On the TV, never run a recursive read (`grep -r`, `find … -exec cat`) over `/`, `/var`, `/mnt/lg` or `/var/palm/jail`. The jails contain device nodes that hang the reading process in the kernel and freeze the TV until a forced reboot. Search regular files in named folders only.
