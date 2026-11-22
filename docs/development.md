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
uv run --no-project --with jsonschema python dev/checks/check-source-profiles.py
uv run --no-project --with jsonschema python dev/checks/check-config-formats.py
```

`cargo test` runs session and playback on a fake backend; the 15 lab tests
decode with FFmpeg without remote services; the schema and TOML/JSON
comparison checks do not run the recipes. Examples and tests use only
loopback and `.invalid` or `.example` domains. For the TV, the package is
what counts: `tools/webos/package.py` stops if the build does not pass its
[checks](tv-app.md#package-checks).

## Structure

```text
crates/core/              shared library: viewing session, HLS live streams, playback, catalog; C shim over FFmpeg
apps/tv/                  TV app (`mana` binary): SDL, renderer, screens, Starfish backend
config/examples/          five synthetic profiles of the v1 draft
config/design-examples/   TOML/JSON comparison and proposed flow language
schemas/                  initial structural contract
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
