# Performance on the TV

Consulted on 20 November 2026. Verification grades as in the [index](README.md).
The B1 measurements of our process are in [ARM CPU](cpu-arm.md#checks-on-the-b1).

Lowering the CPU further is left aside (decided on 28 November 2026): during
a live stream the whole app takes 0.10–0.13 of the B1's four cores, TLS 0.019
cores on Nove, the compositor 2% of a core and the thread feeding Starfish
11–17% of the app, almost all inside Starfish. TLS with AWS-LC would add a
large C dependency to the ARM build, stopping the panel's redraw may disturb
the video plane (PlxNative), and several audio frames per call would change
the most delicate step of playback, each for about a hundredth of a core.
The sections below hold the measurements, should a weaker TV need them.

## Compositor and UI redraw

- On the B1 the `surface-manager` compositor uses 0% of a core with the app
  closed, 22% with the channel list open, which Mana redraws every frame,
  and 2% during a live stream with the panel redrawn 4 times per second (CPU
  from `/proc/PID/stat`, 15–20 s per state).
- After Mana redraws the list only for keys and incoming logos (100 ms
  while they arrive, then 4 times per second), with the list open and idle
  the compositor drops to 2% and Mana from 29% to 16% of a core, at 4 frames
  per second instead of 49 (no-key phase of the interface test).
- PlxNative measures, on a TV with a Mali-T820, about 24% of a core of
  `surface-manager` at 60 presents per second, whatever the content, even
  with a transparent surface. By stopping presenting when the screen is
  idle, with a keep-alive present every 2 s, it drops from 39.6% to 1.67%.
  During playback it still presents every frame, for the unverified concern
  that the video plane depends on the surface; a commit with a null buffer
  disturbs the video plane
  ([perf-view-buffers-and-thermal.md](https://github.com/GLinnik21/plx-native/blob/HEAD/docs/perf-view-buffers-and-thermal.md),
  [present.rs](https://github.com/GLinnik21/plx-native/blob/HEAD/rust-modules/machine/src/present.rs)),
  secondary.
- moonlight-tv presents only when LVGL has areas to redraw
  ([lv_disp_drv_app.c](https://github.com/mariotaku/moonlight-tv/blob/72500d920b19e399ff1c99ad27a888fd76b0d183/src/app/lvgl/lv_disp_drv_app.c#L69-L101));
  punktfunk presents only when something changes
  ([NOTES.md](https://github.com/punktfunk/client-webos/blob/HEAD/docs/NOTES.md)),
  official.

## Feeding data to Starfish

- All the apps seen call `Feed` once per frame, audio included (ss4s of
  moonlight-tv, PlxNative, ytnative, jellyfin-webos-native); the C API of
  LG's ESPlayer asks for "1 audio or video frame data exactly"
  ([esplayer-api.h](https://github.com/webosbrew/webos-userland/blob/00a5f87/include/ndl-directmedia2/esplayer-api.h#L99-L103)),
  official. None documents the cost of a call.
- On the B1 the thread that calls `Feed` spends almost all its time in the
  kernel, in GLib, `libstdc++` and `libplayerAPIs`, not in the app's code
  ([profile](cpu-arm.md#checks-on-the-b1)).

## TLS and AES decryption

- ring removed hardware AES and GHASH on 32-bit ARM
  ([PR #2044](https://github.com/briansmith/ring/pull/2044)), official; the
  `aes` 0.8.4 crate has the hardware path on x86 (AES-NI, detected) and on
  aarch64 only with `--cfg aes_armv8`
  ([lib.rs](https://docs.rs/aes/0.8.4/src/aes/lib.rs.html)),
  official: on the Mac, without that option, it decrypts at 5.3 ns per byte,
  as in software.
- AWS-LC, BoringSSL and OpenSSL read `AT_HWCAP2` on 32-bit ARM and use
  hardware AES and PMULL
  ([AWS-LC's cpu_arm_linux.c](https://github.com/aws/aws-lc/blob/main/crypto/fipsmodule/cpucap/cpu_arm_linux.c),
  [OpenSSL's armcap.c](https://github.com/openssl/openssl/blob/master/crypto/armcap.c)),
  official; aws-lc-rs is tested on `armv7-unknown-linux-gnueabihf`, not on
  our `gnueabi`
  ([platforms](https://aws.github.io/aws-lc-rs/platform_support.html)),
  official.
- `asm!` is stable on 32-bit ARM, the AES intrinsics of `core::arch::arm`
  are not ([vaesdq_u8](https://doc.rust-lang.org/core/arch/arm/fn.vaesdq_u8.html)),
  official; the Linux kernel uses the AES instructions in AArch32
  ([aes-ce-core.S](https://github.com/torvalds/linux/blob/master/arch/arm/crypto/aes-ce-core.S)),
  official.
- Costs of the CRYPTOGAMS code used by ring on Cortex-A15, in cycles per
  byte: bit-sliced NEON AES 14.2 and NEON GHASH 7.63, against NEON ChaCha20
  5.0 and NEON Poly1305 1.25, that is ChaCha20-Poly1305 about 3.5 times
  faster
  ([bsaes-armv7.pl](https://github.com/openssl/openssl/blob/master/crypto/aes/asm/bsaes-armv7.pl),
  [chacha-armv4.pl](https://github.com/openssl/openssl/blob/master/crypto/chacha/asm/chacha-armv4.pl)),
  official for the numbers, inference for the comparison.
- rustls takes the suites in the provider's order of preference, and ring's
  puts AES-256-GCM first and ChaCha20 last
  ([rustls's ring/mod.rs](https://docs.rs/rustls/latest/src/rustls/crypto/ring/mod.rs.html)),
  official. In TLS 1.3 BoringSSL (Google, Cloudflare) puts AES and ChaCha20
  in a group of equal preference and lets the client choose
  ([ssl_cipher.cc](https://boringssl.googlesource.com/boringssl/+/refs/heads/main/ssl/ssl_cipher.cc)),
  official; CloudFront follows its own order, AES first
  ([CloudFront](https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/secure-connections-supported-viewer-protocols-ciphers.html)),
  official.
- Check with `openssl s_client` on 20 November: offering ChaCha20 first,
  Rai 1's Akamai CDN picks it in both TLS 1.3 and TLS 1.2, while a
  CloudFront CDN always picks AES-128-GCM. On the B1, on Rai 1, TLS drops
  from 22% to 5% of the `hls` thread's samples after putting ChaCha20 first.
- Rai 1's segments are also AES-128 encrypted: on the B1 software
  decryption (`aes::soft::fixslice`) is about half of the `hls` thread's
  samples, about 15% of the app's CPU.
- Mana now decrypts segments with AESD and AESIMC in `asm!` when the kernel
  reports `HWCAP2_AES`, read by the app's platform layer
  ([platform](../../apps/tv/src/platform/mod.rs)), otherwise with the `aes`
  crate ([decrypt.rs](../../crates/core/src/live/hls/decrypt.rs)). Under QEMU
  (`arm32v5/debian`, with the AES instructions emulated) the crate decrypts
  at 49.6 ns per byte and the version with the instructions at 5.2. On the B1
  (`dev/tv/opbench.py`, 21 November) the crate decrypts at 66–69 ns per
  byte, the instructions at 1.31–1.36, about 50 times less. On Rai 1 (that
  day 720p at 2.8 Mbit/s with separate audio, AES-128 encrypted), same stream
  and 20 s of samples: the previous build uses 0.13 cores, with
  `aes::soft::fixslice` at 18.5% of samples and the `hls` thread at 29.4%;
  the new one 0.11 cores, with no AES samples and `hls` at 13.9%.
- ureq 2 keeps the connection pool in the `Agent`, and rustls resumes TLS
  1.3 sessions by default
  ([ureq](https://docs.rs/ureq/2.12.1/src/ureq/lib.rs.html),
  [rustls](https://docs.rs/rustls/0.23.46/src/rustls/client/builder.rs.html)),
  official. Mana creates one `Agent` per source
  ([net.rs](../../crates/core/src/net.rs)), and in the Rai Scuola profile
  certificate verification does not show up.

## Memory

Measured on the B1 on 22 November 2026, release build, 30 s from
`/proc/PID/smaps` after 45 s of viewing; between two launches of the same
build the heap varies by about 3 MB. Rai 1 at 720p (2.8 Mbit/s, 10 s
segments, separate audio), Nove at 1080p (7.1 Mbit/s, fMP4 with separate
audio).

- RSS with fontdue: 122–125 MB on Rai 1, 138–140 MB on Nove, 78 MB on the
  home screen without playback. CPU 0.10 and 0.13 cores.
- GPU memory is not in RSS: the `/dev/mali0` mappings have no resident pages.
  The kernel reports it per process in `/proc/gpu/PID`, in 4 KB pages, and in
  total as `GPUTotal` in `/proc/meminfo`: 12.5 MB for Mana during a live
  stream, 10.8 MB on the home screen.
- The main heap (`[heap]`) held 47–54 MB, already 51 MB on the home screen:
  fontdue 0.9.5 builds the outlines of every glyph when loading ("Fonts are
  fully parsed on creation", a non-goal of its
  [README](https://github.com/mooman219/fontdue#non-goals), official), and
  `LG_Smart_UI-Regular.ttf` (2.7 MB, 14,254 glyphs) takes 47 MB after
  `Font::from_bytes` (measured on the Mac with a counting allocator).
  ab_glyph 0.2 reads an outline when the glyph is drawn: the file plus 3 KB
  after 40 glyphs. With ab_glyph
  ([text.rs](../../apps/tv/src/screens/text.rs)) the app takes 78 MB on Rai 1,
  95 MB on Nove and 30 MB on the home screen, same CPU and same text on
  screen (screenshots compared).
- Frames swapped before the compositor shows the window cost a third
  1920×1080 buffer, 8 MB of GPU memory for the whole life of the process:
  with fontdue loading the font delayed the first frame; without it the
  loader of an autoplayed channel redrew at once and `/proc/gpu/PID` went
  from 12.5 to 20.7 MB, back to 12.1 MB when the first frame waits for
  `SDL_WINDOWEVENT_FOCUS_GAINED`, which arrives about 200 ms after the window
  opens ([main.rs](../../apps/tv/src/main.rs)). None of plx-native,
  moonlight-tv, stremio-origin, webos-native-app-kit and punktfunk waits
  before its first frame or limits the EGL buffers; SDL-webOS waits for the
  Wayland frame callback at most 50 ms at 60 Hz, then swaps anyway
  ([SDL_waylandopengles.c](https://github.com/webosbrew/SDL-webOS/blob/df872985380ba0b5ae0fa97cca7cedf2d95bd959/src/video/wayland/SDL_waylandopengles.c)),
  official; that the callbacks do not arrive before the window is shown is
  inference.
- Fonts in the other native apps: stremio-origin and webos-native-app-kit
  map the system fonts with `mmap` and read only the pages of the glyphs used
  ([text.c](https://github.com/stremio-origin/webos/blob/911e61a00276685b81392bf9541dcf5b542f8b62/src/ui/text.c),
  [font.cpp](https://github.com/Native-Webos/webos-native-app-kit/blob/c13695e09a1319ecc6bfca9d72f4cf3d44647a45/render/src/font.cpp)),
  plx-native opens SDL_ttf per size and measures 2.1 MB per size of a CJK
  face
  ([text.rs](https://github.com/plxnative/plx-native/blob/e4c9ac7b3ad82a0e60dfcafcdec8404f3a973d43/rust-modules/gfx/src/text.rs)),
  RetroArch uses FreeType with `LG_Smart_UI-Regular.ttf` in its webOS list
  ([font_driver.c](https://github.com/libretro/RetroArch/blob/76d55c7ec36e3f5819a6540130365d6ad0e7eb67/gfx/font_driver.c)),
  official. None uses fontdue or ab_glyph.
- The segments waiting for the demuxer are the largest anonymous mappings:
  on Nove five of 8 MB with 5–6 MB resident each (three queued, the one being
  read and the one downloading). With one queued segment instead of three
  (with fontdue), Nove drops to 130 MB, Rai 1 does not change (it never holds more than one
  10 s segment queued).
- `MALLOC_ARENA_MAX=2` (`mallopt` at start, with fontdue): 124 MB on Rai 1,
  139 MB on Nove, no gain.
- Copies of the data in the app's code (`memcpy`, `memmove` in the binary):
  2% of the samples on Nove, about 0.003 cores; the demux thread, which copies
  into FFmpeg's buffer and out of its packets, is 4% of the samples. On Nove
  TLS uses AES-GCM (`_bsaes_encrypt8`, `gcm_ghash_neon`): 14% of the samples,
  0.019 cores.
