# TV app

The `mana` app ([apps/tv/](../apps/tv/)) for LG webOS 6 or later: how it is
built and installed, what the package checks, where it keeps files and
settings on the TV, how it plays, and what is verified on an LG OLED55B16LA
(B1, webOS 6). The channel list has its own document:
[Channel list](channel-list.md).

## Building

The window uses the system SDL2, loaded at runtime, with an OpenGL ES 2
renderer and text rasterized by `ab_glyph` with a system font.
On desktop SDL2 and FFmpeg are needed (with Homebrew: `sdl2-compat` and
`ffmpeg`; on Linux FFmpeg is found with `pkg-config`; `MANA_FFMPEG` points to
another prefix). For now playback exists only on the TV:

```sh
cargo run -p mana-tv
```

The tools needed are listed in the [requirements](development.md#requirements):
`package.py` and `ffmpeg.py` check the versions of zig and `cargo-zigbuild`,
and LLVM's `objdump` is needed for the package's float check. The `.2.28`
suffix limits symbols to the TV's glibc; flags and ABI are in
[.cargo/config.toml](../.cargo/config.toml). The app uses an LGPL FFmpeg
9.0.2 included in the package, built with zig as shared libraries with only
the components in use: the first build prepares it in `private/ffmpeg/prefix`
([tools/webos/ffmpeg.py](../tools/webos/ffmpeg.py), about one minute).

```sh
python3 tools/webos/ffmpeg.py
cargo zigbuild --release --target armv7-unknown-linux-gnueabi.2.28
python3 tools/webos/package.py
python3 tools/webos/deploy.py dist/dev.mana.app_0.1.0_arm.ipk --log 6
python3 dev/tv/screenshot.py /tmp/tv.jpg
```

Installation, launch and screenshots need a TV with root and SSH. Access is
configured in `private/tv.env`, as described in
[tools/webos/tv.py](../tools/webos/tv.py).

## Package checks

Before assembling the package, `package.py` checks that the app's functions
do floating-point arithmetic with the VFP, without software float calls such
as `__aeabi_dadd`, and stops otherwise
([floats.py](../tools/webos/floats.py)): this is why the release build keeps
its symbols, which the package then strips with rustc's `rust-objcopy`.

The package is for webOS 6 or later, as its description says. `package.py`
also stops when the app or an included library asks for a glibc newer than
webOS 6's 2.28, or for a library that is neither in the package nor in the
firmware ([elf.py](../tools/webos/elf.py)), and when a function that the app
looks up by name in SDL, in Starfish or in `libhelpers` is missing from one of the webOS 6 or
later firmwares in the webosbrew firmware database, downloaded the first
time into `private/firmware` ([firmware.py](../tools/webos/firmware.py)); the
results are in the
[sources on LG TVs](sources/tv-lg.md#system-libraries-from-webos-6-onwards).
The package carries the libraries in `lib/`, and the LGPL text and the build
note in `licenses/ffmpeg/`; next to the IPK, in `dist/`, goes the
corresponding source `ffmpeg-9.0.2.tar.xz`. The TV's FFmpeg is not used: its
ABI changes from one webOS generation to the next.

## Files and settings on the TV

The TV empties `/tmp` at every power-off; clips and `.stream` files saved in
`/media/developer/mana/` remain. The folder is created by the app, which runs
as its own user and saves there the list, the device identifier and the
language chosen: if it is created
via SSH as root, it must be made writable (`mkdir -m 777`). If
`/tmp/mana-autoplay` contains the name of one of these files, or of a
channel from the [channel list](channel-list.md), the app plays it at start.

A `.stream` file contains a line `url = …`, or the steps of a
[recipe](channel-list.md#recipe-steps) written as `resolve = …` and run on
the TV at every opening, one line `header = Name: value` for each header
and, optionally, `ca = file.pem`. A header that a request cannot carry, here
or in an M3U list (invalid name, control or non-ASCII characters, value over
4096 bytes), is dropped with a log warning that gives its name
([playable.rs](../crates/core/src/live/playable.rs)). For example, a channel
whose website describes the live stream in a JSON opens like this:

```text
resolve = get https://tv.invalid/live/channel1.json
resolve = json video.content_url
resolve = query format=hls
resolve = get
resolve = json video.0
```

Settings live in `/media/developer/mana/mana.conf`, which persists, and in
`/tmp/mana.conf`, for tests, which overrides it
([settings.rs](../apps/tv/src/settings.rs)):

- `list`: the URL of the channel list, which takes precedence over the one
  chosen from the phone;
- `lead_ms` (1500), `audio_offset_ms` (0), `loop` (`true`): how far data
  runs ahead of the displayed frame, the audio offset and whether local
  clips loop;
- `quality`: the starting quality of live streams with several variants,
  `auto` or the lines, such as `720`;
- `debug = true`: the full log;
- `zap_count`, `zap_interval_ms`: the zapping test, automatic changes between
  `.stream` files;
- `ui_test`: the interface test, automatic scrolling of the channel list,
  then 10 s with the list open and no keys pressed, with frame times, CPU and
  memory in the log;
- `freeze_test_s`: the stalled pipeline test, paused after that many seconds
  of each opening;
- `locale`, only in `/tmp/mana.conf`: the TV's locale code (`it-IT`) without
  asking the TV; empty, a TV whose language cannot be read.

The home screen shows the channels of the list, then clips and `.stream`
files; during playback OK opens the same channels over the video. A channel
that cannot be played is dimmed, and OK shows the reason. The M3U list
copied to `/tmp/mana-channels.m3u` by `python3 dev/tv/channels.py` (one
channel per logo, with no stream) replaces the list in both; the 5,000 test
channels appear only with `ui_test` and no other lists. OK on a channel from
an M3U list plays it with the headers that its entry declares
([m3u.rs](../crates/core/src/catalog/m3u.rs)).

The app's log is `/tmp/mana.log`. The app's stderr and any fatal signals end
up there, where URLs appear without query and without the path segments that
carry tokens. The log writes only what happens rarely: starts, errors,
variant changes and the first segment of each opening with size and time.
With `debug = true` it also writes what repeats during viewing, that is the
position every second, the sections, the decoder events and the keys
([log.rs](../crates/core/src/log.rs)); it is kept in RAM, reset at every
start and emptied beyond 4 MB.

## Language

The screens and the phone page are in English or Italian
([language](../crates/core/src/language/mod.rs)). The texts are in one
Fluent catalog per language, `en.ftl` and `it.ftl` in the same folder,
shared with the core's messages and built into the binary, so the desktop
app will read the same ones. On webOS a native app can instead install
`cstrings.json` files that the system's ResBundle reads, as moonlight-tv
does, converting them from its gettext catalogs for the TV only; that
ties the texts to webOS. At start the
app takes the choice kept in `/media/developer/mana/language`; with
"Automatic", the
default, the TV's UI language, `settings.localeInfo.locales.UI` from
`luna://com.webos.settingsservice/getSystemSettings`, asked once within
600 ms, as plx-native and webos-native-app-kit ask it, through
`HLunaServiceCall` of `libhelpers.so.2`, loaded with dlopen, as
webos-native-app-kit and moonlight-tv call luna (plx-native uses
`libluna-service2`; both are on every webOS 6–11 firmware)
([locale.rs](../apps/tv/src/platform/locale.rs)). LG's SDL 2.0.10 has no
`SDL_GetPreferredLocales`, and `LANG` is not read on webOS; the desktop build
reads `LC_ALL`, `LC_MESSAGES` or `LANG` instead. A code whose language is
`it` gives Italian, any other code English, and so does a call that fails,
times out or answers without the code. The first frame waits for the answer,
together with the window's focus. "Language" in Settings cycles Automatic,
Italiano and English with OK, left and right, applies at once and is kept;
a change of the TV's language while the app runs is not followed. An error
shows untranslated only what comes from outside the app, as FFmpeg's,
Starfish's or the system's words and a server's answer; the errors carry it
as a field of their own, and everything Mana words is in the catalogs. The
log names the language and where it came from, and is always in English.

## Tests on the TV

Synthetic sync clips, which flash with a beep every second; the measurement
uses a 60 fps recording from the phone, with a resolution of about ±8 ms:

```sh
python3 dev/tv/clips.py            # generates the clips in private/clips (sync and formats)
python3 dev/tv/clips.py --upload   # copies them to the TV
python3 dev/tv/avsync.py VIDEO.mp4 # measures the offset from a recording of the TV
```

CPU measurements with the app playing; scenario and results are in the
[sources](sources/cpu-arm.md#checks-on-the-b1):

```sh
python3 dev/tv/profile.py --seconds 30   # where the app's threads spend CPU, by sampling
python3 dev/tv/opbench.py                # cost per operation with the build flags and for ARMv8
```

Live stream from the [synthetic lab](local-lab.md), played on the TV:

```sh
python3 -m dev.lab --host IP_DEL_MAC --tls --live  # HTTPS live stream on the local network
python3 dev/tv/live.py                    # copies the channels' .stream files and the CA to the TV
```

## How playback works

The app reads a local TS file or an HLS live stream downloaded by its own
client ([crates/core/src/live/hls/](../crates/core/src/live/hls/)):
playlists reloaded after the duration of the last segment, start at least 3
target durations from the live edge, headers applied to every request but
credentials (`Authorization`, `Proxy-Authorization`, `Cookie`) only towards
the origins that the source names ([net.rs](../crates/core/src/net.rs)),
gzip responses accepted and decompressed, TLS verified with the public roots
plus an optional CA, AES-128 segments decrypted, TS or fMP4 segments
(`EXT-X-MAP`) and audio in a separate track (`EXT-X-MEDIA`), merged with the
video in time order. If the URL answers with a continuous MPEG-TS stream
instead of a playlist, recognized from the first bytes, the response is read
as it arrives, from the first PAT table onwards as Media3 does, and, when it
breaks off, requested again
([continuous.rs](../crates/core/src/live/hls/continuous.rs)). The first 3 s
of each response, at most 8 MiB, which is what the queue holds, are held back
before being passed to the demuxer: this is the margin against the network
slowdowns that an HLS live stream has in the segments behind the live edge,
at the cost of 3 s more at every start. At start the track that begins
earlier is cut to the start of the other, the video up to a keyframe. AAC
packed audio, whose timestamp is only in the ID3 tag of each segment, is
repackaged into MPEG-TS
([packed_audio.rs](../crates/core/src/live/hls/packed_audio.rs)). At every
discontinuity and at every new initialization section the stream of a
playlist breaks off and the demuxer is reopened, because PIDs, formats and
the presence of audio can change there; two separate tracks are compared by
time only within the same discontinuity (`EXT-X-DISCONTINUITY-SEQUENCE`).

It demuxes with the included FFmpeg
([shim.c](../crates/core/src/playback/ffmpeg/shim.c)), and hands video and
audio, still compressed, to LG's Starfish pipeline (`libplayerAPIs`, loaded
at runtime) in `BUFFERSTREAM` mode
([starfish.rs](../apps/tv/src/platform/starfish.rs)): decoding, sync and
presentation belong to the TV. The codec names for Starfish come from its
internal table, not from the capabilities table: for example E-AC-3 is
called `AC3 PLUS`. The NDL DirectMedia path is discarded because on the B1
H.264 appears there as a green screen; PCM audio is discarded because
Starfish plays it continuously without respecting its timestamps. Supported
are H.264 and HEVC video and AAC (including HE-AAC), AC-3, E-AC-3 and MPEG
(converted to AAC) audio. LG's FFmpeg 4.0 aborted when it resized a stream's
buffer: the shim uses a buffer larger than the maximum analysis size.
FFmpeg 9 analyzes streams, unlike LG's: analysis is limited to 0.5 s,
otherwise opening would wait for seconds of live stream that have not
arrived yet. At every channel change the Starfish object is destroyed after
unloading: one left alive holds on to the decoder, and after two channels
the B1 refuses loading (error 601).

Data goes to the TV at most 1.5 s ahead of the frame it is showing,
regulated on the presented position and not on a clock of its own. The
demuxer reads on its own thread: when data is missing the pipeline pauses
with less than 0.5 s ahead and restarts when it again has the full lead. Left
running, Starfish advances its clock even with nothing to play and then runs
at about 2× to catch up, out of sync. A jump in the timestamps or the move to
a new discontinuity realigns the timeline: the first stream that crosses it
creates the new reference and the other adopts it, so packets still queued
from before do not end up far in the future. The wraparound of the 33-bit
MPEG-TS clock is handled for each stream. If audio is missing while the
pipeline has an audio track, the app sends AAC silence frames: without them,
the TV emits a continuous beep. If instead a section needs a decoder that the
pipeline does not have (audio that appears after a start in a silent clip,
another codec), the pipeline is reloaded as for a channel change, in less
than 200 ms.

When the master has several variants, the client follows the best one and
switches to a lighter one if the bandwidth measured on the segments is not
enough, then goes back up when it returns
([variant selection](hls-cases.md#variant-selection)).

On a live stream the client retries the playlist as long as playback stays
active. The live stream is reopened from the live edge with a new pipeline,
like a TV that retunes: when data returns after more than 10 s of gap, for
example after a network interruption (resuming from where it left off would
keep viewing behind the edge by as long as the gap lasted), when the
playlist does not advance for 3.5 target durations (including because the
numbering restarted lower), and when the pipeline fails, for example shows
no frames for 15 s. Reopening reuses the variant playlists already read: on
RaiPlay the master's token expires after 150 s, that of the playlists and
segments after 24 h. If a playlist or a segment answers 401, 403 or 410
(expired token), the source is instead resolved again. Pipeline failures and
new resolutions are allowed at most 6 times in 5 minutes, then playback
closes with the error.

During playback the up and down arrows show and hide the information panel,
which is closed at first and open when viewing starts, until the TV shows
the first frame and for 5 s after, while the loading indicator spins in the
center ([loader.rs](../apps/tv/src/screens/loader.rs)): channel, status and
viewing time, video and audio of the section on air and, on live streams
with several variants, the chosen quality and the heights offered
([info.rs](../apps/tv/src/screens/info.rs)). On a live stream with several
variants the right arrow opens the quality menu: "Auto" or one of the
heights offered, applied from the next segment and kept when changing
channel ([quality_menu.rs](../apps/tv/src/screens/quality_menu.rs)).

## Verification on the B1

The feasibility phase closed on 17 November 2026. On an LG OLED55B16LA with
webOS 6 the following are verified:

- building from the Mac, IPK package, installation via SSH, OpenGL ES 2,
  system font and remote control keys;
- TS 1080p H.264 (24, 25 and 50 fps) and HEVC with AAC audio, with an
  audio/video offset of about +80 ms, no greater than that of the system
  player measured the same way;
- an HTTPS live stream from the lab, with mandatory headers and AES-128
  segments, for 10 minutes without errors;
- the same live stream after an encoder restart, the wraparound of the
  33-bit timestamps and 10 s without network: it resumes about 1 s after the
  network returns;
- 100 channel changes between two live streams (1080p H.264 and 720p HEVC):
  first frame in about 0.8 s, memory stable around 98 MB;
- real public live streams: Deutsche Welle (1080p50); RaiPlay (URL signed by
  the relinker, AES-128 segments, audio in a separate TS track); Pluto TV
  (session with JWT, AES-128, clips stitched with discontinuities); Canale 5
  (separate audio as packed audio, raw AAC with ID3 timestamps); Nove (CMAF
  fMP4, segments shorter than the declared maximum duration). The TV's
  FFmpeg 4.0 loses the audio of fMP4 fragments with more than one `trun` box
  (for example the ireplay.tv test stream), the FFmpeg 9.0.2 included in the
  app does not: of the 251 Italian live streams reachable from the iptv-org
  list, 241 are TS and the 10 fMP4 ones have a single `trun`. The official
  live streams of Mediaset and La7 use DRM and remain out of reach;
- Pluto TV ad breaks: the clips change PID, resolution (1216×684 and
  1280×720) and presence of audio; the live stream continues without
  stopping, with silence where the clip has no audio and the episode that
  resumes in sync;
- performance on public live streams: app at 13–15% of one core (about 6
  points from the app's code, the rest from LG's pipeline in the process),
  101–111 MB of RSS of which about 70 private, 350–440 MB free on the TV
  (78–95 MB since 22 November, with text rasterized on demand:
  [memory](sources/performance.md#memory));
  the same with the TV's FFmpeg, used in the feasibility phase, and with the
  included one, which adds 2.6 MB of libraries;
- live stream formats: H.264 1080i, AC-3, E-AC-3 5.1 and HE-AAC audio; MPEG
  audio (Layer II, MP3) is converted to AAC by the app because the pipeline
  does not accept it; MPEG-2 video, by now a rare format, is not supported;
- M3U lists with per-channel headers (`#EXTVLCOPT:http-user-agent=`,
  `#EXTVLCOPT:http-referrer=`, `url|User-Agent=…&Referer=…`), sent with every
  request of the channel: of the 19 channels in the Italian iptv-org list
  that declare them (19 November 2026), Italiana, Mediaset Italia, Odeon 24,
  Rai 2 and Rai Sudtirol play, and none rejects the playlist without the
  header; Twentyseven HD is a DASH manifest, which the app reports as not
  supported; the other 13 do not come from the broadcasters and are excluded
  from the tests, like every unofficial source;
- interface over the live stream: a list of 5,000 channels scrolls with the
  remote at the panel's refresh rate (50 frames per second, about 20 ms per
  frame) with the whole app at 15–16% of one core and the live stream
  playing smoothly underneath; with 600 real channels with logos (iptv-org
  lists, logos downloaded from their servers, decoded in threads and kept in
  an 8 MB texture) it stays at 50 frames per second with at most one frame
  dropped at the start, the app at 35–77% of one core and about 58 MB more
  RSS;
- life cycle: with Home or a source change the TV mutes the app in the
  background and on return the live stream resumes; with the screen off it
  continues; in standby the system revokes the decoder (error 600) and the
  app closes, so at power-on it starts from scratch; while Starfish is
  playing, the TV does not start the screensaver, which starts 1 to 2
  minutes after the video stops.
