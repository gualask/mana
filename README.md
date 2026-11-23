# Mana

Mana is a live channel player for LG TVs with webOS. It plays the live
streams you choose, from a channel list that you set from your phone, and
leaves decoding and picture to the TV, which does them in hardware: fast
starts, quick channel changes and little load on the TV.

Mana contains no channels and no lists: it only plays the sources you choose.

## What it does

- **English or Italian.** The app follows the TV's language: Italian on a
  TV set to Italian, English otherwise. "Settings" can also fix it to either.
- **Channel list from the phone.** In "Settings" the TV shows
  a QR code: on the phone you enter the address of your list, and the TV
  downloads it and remembers it even after it is switched off. The list is
  updated at every start, and if the network is down the last copy stays.
- **M3U lists and single channels.** The list can include M3U lists and
  channels with their address, logo and group, also with the headers
  (User-Agent, Referer) that some sources require.
- **Live streams that don't stop.** If the network drops or slows down, the
  stream pauses and resumes on its own; when bandwidth drops it switches to a
  lighter quality and goes back up when bandwidth returns; if a link expires,
  it renews it.
- **Over the live stream.** The channel list with logos, a panel with
  channel, quality, formats and viewing time, and a menu to choose the
  quality or leave it automatic.

## Formats

| | Supported |
|---|---|
| Live streams | HLS, including AES-128 encrypted segments, separate audio and format changes mid-stream; continuous MPEG-TS streams |
| Containers | MPEG-TS, fMP4 (CMAF) |
| Video | H.264, including interlaced, and HEVC |
| Audio | AAC and HE-AAC, AC-3, E-AC-3 5.1; MP2 and MP3, converted to AAC |

DRM-protected content (Widevine, PlayReady, FairPlay), DASH live streams and
MPEG-2 video are not supported.

## Requirements

- An LG TV with **webOS 6 or later**. Mana is tested on a 2021 LG OLED B1.
- The TV must be **rooted**, with Homebrew Channel, and have SSH access: Mana
  is a native app that is installed from there.

There is no ready-made package to download yet: for now Mana has to be
built, as described in [TV app](docs/tv-app.md#building).

## Usage

1. Open Mana and, on the home screen, press the right arrow twice to go to
   "Settings" ("Impostazioni" in Italian): the first stops on "Search".
2. Scan the QR code with your phone and enter the address of your channel
   list: [Channel list](docs/channel-list.md) explains how to write it.
3. Choose a channel and press OK. A row marked `›` is a folder with the
   events of a source: OK opens it, Back returns to the list. "Search"
   ("Cerca"), next to "Settings", finds channels and events by name with
   an on-screen keyboard, among the folders already opened.

While watching:

| Key | Action |
|---|---|
| OK | opens the channel list over the live stream |
| Up and down | show and hide the information panel |
| Right | opens the quality menu, on live streams that offer more than one |
| Left | opens the signals of the channel or event, when its source gives several |
| Back | closes the list or menu; otherwise stops playback |

## Status

Mana is a personal project, still in development: it is tested on a single
TV, an LG OLED B1, and not yet on other models. Development continues with
a desktop app.

## Technical documentation

- [TV app](docs/tv-app.md): build, package checks, files and settings, playback, verification.
- [Channel list](docs/channel-list.md): setup from the phone, format and sources.
- [Development](docs/development.md): requirements, lab, checks and repository structure.
- [Next steps and open questions](docs/todo.md): development sequence and completion criteria.
- [Modular architecture](docs/architecture.md): contracts, dependencies, session and backends.
- [Configuration and extensions](docs/configuration.md): source files, their operations and how they run.
- [HLS live edge cases](docs/hls-cases.md): comparison with hls.js and Media3.
- [Synthetic lab](docs/local-lab.md): endpoints, scenarios, expiry and tests.
- [Sources](docs/sources/README.md): documentation consulted, TV CPUs and libraries, performance.

## License

Mana is free software, © 2026 gualask, distributed under the [GNU GPL 3.0 or
later](LICENSE): you can use, study and modify it, and anyone who
distributes a modified version must make its source available under the
same license. The TV package includes FFmpeg, under LGPL 2.1.
