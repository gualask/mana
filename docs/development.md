# Development

What you need to work on Mana, how a change is verified and how the
repository is organized. Building and installing on the TV are covered in
[TV app](tv-app.md#building).

## Requirements

- A Mac, the only system tested.
- Rust, pinned with the TV target in
  [rust-toolchain.toml](../rust-toolchain.toml), which rustup installs on its
  own.
- For the TV: zig and `cargo-zigbuild` at the versions in
  [tools/webos/target.py](../tools/webos/target.py), an LLVM `objdump`
  (the macOS one or `llvm-objdump`), and a TV with root and SSH.
- For the lab and the tools: Python 3.11 or later, Node and FFmpeg in the
  PATH.

## Desktop app

An alpha of the desktop app plays the list of a URL, its folders and
signals through the core's session, decoding in software
([desktop client](architecture.md#desktop-client)); it needs FFmpeg from
Homebrew or `MANA_FFMPEG`, and builds only on the desktop:

```sh
cargo run -p mana-desktop --release -- --list https://lists.example/list.toml
```

The URL is kept in the app's data folder (`~/Library/Application
Support/Mana` on macOS) and used from then on. Keys: Enter opens the list
and chooses; Esc over a video shows the list where it was left (the
signals of the event playing), then goes back a level, then hides it; the
arrows move by a row or a page.

## Lab

Without a TV, the [synthetic lab](local-lab.md) serves synthetic live streams
locally:

```sh
python3 -m dev.lab --api-port 8781 --resolver-port 8782 --media-port 8783
ffplay http://127.0.0.1:8783/public/master.m3u8
```

## Checks

```sh
cargo test
python3 -B -m unittest discover -s dev/lab/tests -v
```

`cargo test` covers the core and the TV app; `cargo test --workspace` adds
the desktop app, which a plain `cargo test` leaves out so that the TV's
build commands do not try to build it for ARM. The tests run session and
playback on a fake backend, and check every
source of `config/examples/`, and every case of `config/examples/rejected/`
against the rule it breaks; the
[player e2e tests](local-lab.md#player-e2e-tests) start the lab, so
`cargo test` needs its [requirements](local-lab.md#start-and-playback). The
16 lab tests decode with FFmpeg without remote services. Examples and tests use only
loopback and `.invalid` or `.example` domains. For the TV, the package is
what counts: `tools/webos/package.py` stops if the build does not pass its
[checks](tv-app.md#package-checks).

## Structure

```text
crates/core/              shared library: viewing session, HLS live streams, playback, catalog; C shim over FFmpeg
apps/tv/                  TV app (`mana` binary): SDL, renderer, screens, Starfish backend
apps/desktop/             desktop app, alpha (`mana-desktop`): Rune window, backend decoding in software
config/examples/          source files of the lab's chains; rejected/: one case per validation rule
schemas/                  JSON Schema of the source files, for editors
tools/webos/              build, checks, package and installation for the TV
dev/lab/                  server, generated media and standalone test clients
dev/tv/                   tests on the TV: clips, sync, channels, live streams, screenshots, CPU profile and costs
dev/checks/               configuration validators
docs/                     documentation and work list
private/                  not versioned: FFmpeg and firmware database for the TV, lab
dist/                     not versioned: IPK package and FFmpeg source
```

The crate structure is described in
[Modular architecture](architecture.md#target-structure).
