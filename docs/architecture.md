# Mana's modular architecture

Project direction of 15 November 2026. The agreed requirement is a player
that can be extended through configuration: everything that can be expressed
by composing capabilities already available must stay in the profiles. New
inputs, resolvers and protocols must have distinct extension points.

This document defines the structure, the contracts and the responsibilities.
The workspace contains the shared library `mana-core` and the [TV
app](tv-app.md). **The recipe contract described here is not implemented
yet**: live streams use the provisional recipe of the `.stream` files and of
the channel list. The syntax currently validated is defined in the
[v1 schema](../schemas/source.schema.json); the [Configuration and
extensions](configuration.md) rules separate that draft from the capabilities
still to be built.

## Requirements and evidence

| Need | Evidence and consequence |
|---|---|
| New sources mostly through profiles | Endpoints, mappings and flows must compose registered capabilities |
| Different inputs and chains of resolvers | [Synthetic scenarios](../dev/lab/scenarios.py): JSON, embedded data, iframe, encrypted envelope and binary session |
| Algorithms without implicit network logic | [Lab decoders](../dev/lab/codecs.py): decoders without network access, usable by the tests |
| TV and desktop apps, automated tests | Same session service; each client has its own presentation and video backend |
| Alternatives and temporary links | [Integration tests](../dev/lab/tests/test_lab.py): denial, fallback, expiry and new resolution |
| Lightweight app | One process; one shared library and one app per client |

## Chosen shape and alternatives

| Shape | Advantage | Cost or limit |
|---|---|---|
| Complete clients for each protocol | Fast early trials | Repeats transport, errors and lifecycle |
| Dynamic plugins with a general script runtime | Each extension can supply a program | One more execution and distribution environment to maintain |
| Modular monolith with recipes and ports/adapters | Shared engine and isolated variations | A missing capability needs code and a compatible version of the app |

The third shape is the chosen one. The code is a modular monolith organised by
capability (`live`, `playback`, `catalog`), with an application service,
`session`, that depends on the capabilities and not the other way round. The
apps are separate crates because TV and desktop do not build for the same
target: gpui and Rune do not run on 32-bit ARM with OpenGL ES 2, and Starfish
exists only on webOS. The language stays Rust; zig serves only as C compiler
and as linker for the TV's glibc 2.28.
Plugins loadable at
runtime may be evaluated when a concrete requirement for independent
distribution emerges. External profiles may be added without recompiling
if all the capabilities they require are already in the binary.

## Common contract

Clients use three asynchronous operations, all cancellable:

```text
catalogue(source, cursor?) → Page<CatalogItem>
sources(item_ref)          → List<StreamCandidate>
resolve(candidate_ref)     → PlayableStream
```

The application uses the engine to resolve the intermediate steps, and
`playback` to check the resource and the capabilities before returning
`PlayableStream`. The engine can therefore produce a resolved resource without
importing the backend. References keep a stable source and identity; they do
not use the signed URL as ID. The types below are the proposed model, not
existing Rust signatures.

| Type | Responsibility and data |
|---|---|
| `CatalogItem` | ID with source namespace, event/channel type, title, group/category, time and status when available, reference to the detail |
| `StreamCandidate` | Stable ID within the event's context, label, declared language/quality, reference to the resolver and input to resolve |
| `ResolutionOutcome` | Internal outcome of a resolver: resolved media resource or next candidate to resolve; failure uses `Failure` |
| `PlayableStream` | Concrete media resource, detected protocol, HTTP policy for manifests/segments/keys, known or unknown expiry, opaque reference for renewal |
| `MediaInfo` | Container, codecs, tracks and quality actually detected; they can be unknown before the probe |
| `Page<T>` | Items and optional cursor, independent of the site's pagination |
| `Failure` | Structured code, phase, source/stream, optional HTTP status, diagnostic context without tokens |

An iframe is an input to resolve; it is not handed to the decoder as an HLS
stream. The format declared by the provider stays a hint until the response
is checked. Missing metadata stays absent: language, quality, availability or
expiry are not invented.

A provider's internal structure stays in its opaque context. The UI knows
only these models and the outcomes of the operations.

### Step-by-step resolution

A resolver receives a candidate and returns one of two internal outcomes:

```text
ResolutionOutcome = MediaResource(resource) | NextCandidate(candidate)
```

With `NextCandidate`, the engine selects the resolver again from the registry
and continues the chain. With `MediaResource`, the application passes the
resource to `playback` to check it before handing `PlayableStream` to the
client. The resolver can be a declarative recipe or a registered adapter:
the contract is the same and the UI does not know the intermediate steps.

The engine checks dynamic cycles and depth within the context of the single
resolution, and applies a total budget of steps/requests even when the
intermediate URLs change. Each step keeps the candidate's identity and the
references to the context, and respects the HTTP scopes of the new host.
The split of the recipes and a synthetic example are described in the
[Configuration and extensions](configuration.md#composable-resolvers) rules.

## Modules and decision ownership

| Module | Owns | Change it isolates |
|---|---|---|
| `session` | The current viewing, one at a time; reopening and new resolution with their limit; the viewing state (`State`) and what the screens show of it, merged for the clients | Viewing rules shared between clients |
| `live` | `.stream` files, provisional recipe, HLS client; reports interruptions without deciding | Source protocols and formats |
| `playback` | Demux with FFmpeg, timeline, pacing on the presented frame, silence; `Backend` and `Input` ports | Media formats and each client's decoders |
| `catalog` | Channel lists as models for the UI, each channel with the source to play or the reason why it cannot be played; saved copy and update of the list from a URL, with queued downloads and the URL chosen by the user, accepted only if the list is valid TOML; device identifier | List formats and their lifecycle |
| `net` | TLS and roots, timeouts, User-Agent, responses rejected beyond the limit set by the reader (playlists, keys and segments in `live::hls`), URLs without tokens in the log; headers and CA of the single source, with credentials sent only to the origins the source names; maximum length of a value in a request line (`MAX_LINE_VALUE`), for URLs and headers | HTTP transport |
| `language` | English and Italian: the language in use, from the viewer's choice kept in the data folder, else the TV's locale code, else English; the Fluent catalogs with every text the apps show (`en.ftl`, `it.ftl`, built into the binary) and `text!`, which reads them; `Localized`, which every error shown on a screen implements, each in the module that owns it, its `Display` the English text for the log | Languages of the interface and their texts |
| `apps/tv` | Composition, screens (their texts in the core's catalogs), SDL, Starfish backend, reading the TV's locale (luna on webOS, `LC_ALL`, `LC_MESSAGES` or `LANG` on the desktop build), settings file, page for the phone, automated trials | Remote control, screens and webOS integration |

Decisions on reopening and new resolution belong to `session`
([reopen.rs](../crates/core/src/session/reopen.rs)): `live` reports typed
interruptions (token rejected, stalled playlist, unsupported features, failed
source), and `playback` reports how an opening ended (end, too long without
data) or typed errors (unsupported codec, decoder fault). The backend owns
buffers and decoders. Channel change and stop return at once: the threads of
the previous viewing end on their own at their next check, and the new viewing
opens its own backend only after the previous one has released its own; a
viewing stopped in the meantime opens none. This way a request stuck on the
network does not block the remote control; only exiting the app waits for the
threads, within a limit. When recipes arrive, the profile will parametrise
the policy.

Protocol-specific context belongs to the provider or resolver instance, with
source/session scope. Cookies and connections belong to the transport, with
explicit scopes. Temporary parameters and tokens stay in memory. There is
currently no DB, nor any operation that must coordinate several persistent
stores.

## Allowed dependencies

```text
apps → session → live → playback (Input port only)
             → playback
apps → playback (Backend port), catalog, net
catalog → live (source, recipe), net
apps, catalog, live, net, playback, session → language
apps, catalog, language, live, playback, session → log
live → net
live, playback → adts
```

`net` and `language` are below the other modules: `net` depends on
`language` only because its errors reach the screens, and `log` depends on
nothing.

No core module imports an app. The core has no `cfg(webos)` and no Unix-only
code: the platform is decided by `apps/tv/build.rs`, while the core's
`build.rs` only chooses which FFmpeg to link. The CPU's AES instructions are
also detected by the app (`platform::use_aes_instructions`, from
`HWCAP2_AES`): the core uses them only after `hls::assume_aes_instructions`.
The backend is chosen in one place, in each app's composition
(`platform::open_backend` on the TV).
The folder for persistent data is also decided by the app
(`platform::DATA_DIR`), which passes it to `catalog::update`.
The contract between `live` and `playback`, `playback::Input`, is defined by
its consumer.

## Target structure

Tree of the crates; the other folders of the repository are described in
[Development](development.md#structure).

```text
Cargo.toml                  # workspace
crates/core/                # mana-core: no UI and no platform dependency
  build.rs                  # shim and FFmpeg: bundled for the TV, MANA_FFMPEG or Homebrew on the desktop
  src/
    session/                # viewing, reopening (reopen.rs), viewing state and merged statistics (stats.rs)
    live/                   # playable.rs (a stream to play), recipe.rs, hls/ (playlist, openings, downloader, variant choice, progress, stream to the demuxer, segments, AES-128 decryption, packed audio, continuous TS stream, MPEG-TS packets)
    playback/               # ports, demux, reader (demux thread), feeder/ (one opening, feeding, presentation), timeline, silence, ffmpeg/ (wrapper, and the C shim: shim.c with probe.c and mp4.c, transcoder.c for MPEG audio to AAC), fake.rs for the tests
    catalog/                # channels, M3U lists, TOML list (list.rs), update and chosen URL (update.rs), copies on disk (update/copies.rs), device identifier
    language/               # Fluent catalogs (en.ftl, it.ftl) and text! (mod.rs), the viewer's choice and the language in use (choice.rs); tests.rs checks the catalogs against every text! in the workspace
    net.rs, log.rs, adts.rs
apps/
  tv/                       # mana-tv, binary `mana`
    build.rs                # cfg(webos), crash.c, linker arguments of the binary
    src/
      main.rs               # composition and event loop
      screens/              # home screen (home.rs), Settings (setup.rs), language in use (language.rs), arrival of the list (list_updates.rs), over the video (video.rs, with the information panel in info.rs and the loading indicator in loader.rs), renderer, text, QR code, channel list, logos, pointer zones
      phone/                # page served to the phone while Settings is open: server, minimal HTTP, HTML
      palette.rs            # colours of the screens and of the page
      platform/             # SDL, keys, Magic Remote pointer, TV locale (locale.rs), CPU AES instructions, Starfish backend, data folder
      settings.rs           # mana.conf in the data folder, then /tmp/mana.conf
      autotest.rs           # automatic start, zapping, interface trial; frame statistics in autotest/frames.rs
  desktop/                  # mana-desktop, to be built: Rune UI and desktop backend
```

`session` exists because two clients, the TV and the automated trials, and
soon the desktop, start the same viewings. The live policy, once split between
the HLS client and the player and changed in both across eight commits, lives
only in `session`: `live` and `playback` report and do not decide. The fake
backend of `playback` runs feeder, timeline and reopening in `cargo test`
on the Mac. Domain modules, profile loader, recipe engine, CLI and contract
tests will arrive with the first executable contract for recipes
([Next steps and open questions](todo.md)).

### Desktop client

The desktop app will be distributed for macOS, Windows and possibly Linux:

- **UI** in [Rune](https://github.com/anomalyco/rune), pinned to a
  revision; gpui is the fallback if Rune fails on Windows. In the trial, the
  list of 5,000 channels holds 1080p50 even when frames pass as bytes, as on
  Windows and Linux.
- **Own backend** on the `playback::Backend` port, so the desktop uses the
  TV's demux, timeline and feeder. FFmpeg decodes video in software, the same
  way on all three systems, into a texture and the audio output. libmpv is
  the fallback if synchronisation costs too much.
- **FFmpeg:** for development, the one from Homebrew or `pkg-config`
  ([build.rs](../crates/core/build.rs)). For distribution, an LGPL build per
  platform, given with `MANA_FFMPEG`, with the components of the
  [TV build](../tools/webos/ffmpeg.py) plus the video decoders.
- **Interface fixtures** (M3U lists, `.stream` files pointing to the lab,
  local logos) in `dev/fixtures/`, shared by desktop and TV.

Constraints found in the trial with Rune:

- frames enter `NativeSurface` as bytes; on macOS also as IOSurface;
- while a source plays, the window asks for the next frame on every update.
  If it waits for the wake-up of each send, every frame arrives one update
  late: on a 60 Hz screen, 400 out of 500 frames of a 50 fps clip were drawn
  ([anomalyco/rune#1141](https://github.com/anomalyco/rune/issues/1141));
- the channel list declares the version of its rows
  (`virtual_rows_version`), so the layout does not ask for the key of each row
  on every commit;
- Rune does not provide the presentation time: the presented position of the
  desktop backend comes from the audio clock;
- Rune's built-in `Player` is of no use: it uses AVFoundation and Media
  Foundation, does not exist on Linux and does not accept Mana's byte stream.

## Extensions and compatibility

The three extension points have distinct contracts:

- Recipe operation: typed input and output, parameter schema and declared
  capability/version. It can be reused by several providers.
- Provider/resolver: produces common models or the next resolution step;
  receives transport and context through the engine's ports.
- Media backend: declares the supported formats, codecs and HTTP policies,
  and produces playback events common to the application.

A new API protocol may need a data provider/decoder. A new transport or video
format may need support in `playback` and in the backend. Adding a value to
the profile does not enable a codec on the TV.

Loading checks versions and capabilities before activating a profile.
An update replaces the profile only after full validation; a wrong profile
does not delete the last valid one. The current session keeps the revision it
started with. A new session uses the new plan.
Binary plugins and automatic distribution of profiles stay out of the first
step: local import must already respect this contract. The provisional
recipes arrive with the channel list, from the URL the user chooses on the
phone ([Channel list](channel-list.md)).

Outcomes include `upcoming`, `ended`, `no_sources`, `unsupported_host`,
`auth_required`, `challenge_required`, `source_changed`, `network_error`,
`rate_limited`, `media_unsupported`, `access_denied`, `invalid_profile`,
`unsupported_capability` and `cancelled`. HTTP 403 is kept as a denial whose
cause may be unknown; `auth_required` is used when there is explicit evidence
that authentication is required. The failure of a stream allows trying
alternatives according to policy, without losing catalogue and
configuration.

## Verification and implementation order

The [Synthetic lab](local-lab.md) is already available: it generates original
media and serves catalogue, resolution, session and failure cases.
The current tests check contracts and FFmpeg decoders; the application e2e
tests will use the same servers through the future Mana runtime.

The operational sequence and the completion criteria are kept in
[Next steps and open questions](todo.md).

The proof of modularity is a concrete change: adding a source with existing
operations changes the profile and fixtures; a new resolver changes its
module, registration and contract; a new parser changes the loader.
None of these cases requires site logic in the screens or in the decoder.

The runtime will be separate from the lab's test servers and clients.
Checking the actual dependencies will start with the core modules.
