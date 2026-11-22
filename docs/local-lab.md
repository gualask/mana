# Local streaming lab

A synthetic lab to develop the universal player and the resolvers without
depending on external services. It reproduces protocol behaviours with
fictitious events, locally generated keys and video/audio created on the fly.
It contains no recordings of broadcasts and no responses copied from
providers. The scenarios are defined by protocol and behaviour.

**Status:** lab implemented, with fifteen
contract/integration tests. Mana does not run profiles yet: the core
(`crates/core`) has no profile runtime, so these tests do not prove that it
runs them. The media server can already be
attached to an HLS player to verify transport and decoding.

## Start and playback

Requirements: Python 3.11+, Node with `node:crypto`, FFmpeg with the `libx264`
and `aac` encoders, and OpenSSL for HTTPS. The lab and the suite need no
additional Python dependencies. FFplay is needed only to open a playback
window.

From the Mana root:

```sh
python3 -m dev.lab --api-port 8781 --resolver-port 8782 --media-port 8783
```

The process prints the actual addresses and keeps running until
Ctrl-C. If a port is in use, choose another port or omit the three
options to get free ports assigned by the system. By default the servers
listen only on `127.0.0.1`. Options:

- `--host IP`: listen on a local network address, for example for the TV;
- `--tls`: serve HTTPS with a CA generated at startup and a server certificate
  signed by it; clients must add the CA as a trusted root;
- `--live`: add three live HLS channels generated in real time;
- `--segment-seconds N`: segment duration of the live streams, 2 s if omitted;
  real live streams use 4 to 10, and tests of the margin from the live edge
  need segments of that duration.

With the ports of the example, in a second terminal:

```sh
ffplay http://127.0.0.1:8783/public/master.m3u8
```

The content is a four-second HLS VOD clip, with a moving 320×180 video pattern
at 25 fps and a 440 Hz audio tone. FFmpeg generates it in a temporary
directory, which is removed when the lab closes. The `--live` stream is instead
encoded in real time while the lab is running: every second the picture
flashes white with a 1 kHz beep, so sync can be measured with
`dev/tv/avsync.py` as for the clips.

The `private/lab` directory, which `--write-config` can change, contains:

- `endpoints.toml`: fixture manifest with three origins, synthetic endpoints
  and headers. It is for the test clients; it is not a Mana source profile.
- `direct-v1.toml`: complete, disabled source profile, conforming to the
  schema v1 data, that points to the public HLS. The future Mana loader will
  have to read it; today the URL can be used directly with FFplay.

The files are updated at every start. The addresses are valid while the
process is running. `private/` is excluded from version control.

## Three origins and available scenarios

The origins are real HTTP servers on separate ports, in the same process:

```text
catalogue/API → resolver → media server → real decoder
```

| Origin / path | Behaviour |
|---|---|
| API `/catalogue.json` | JSON catalogue; requires `User-Agent: StreamLab/1` |
| API `/events/demo.json` | Two signals: one denied and one resolvable; the catalogue exposes only one of them |
| API `/events/empty.json` | Valid event without signals |
| API `/channels.m3u` | M3U playlist with one public synthetic channel |
| API `/embedded` | Synthetic data embedded in a Next Flight-style serialization |
| Resolver `/frame` → `/embed/demo` → `/api/links/demo` | Iframe, API discovery and JSON response with a media link |
| Resolver `/wrapper` → `/player?liveUrl=…` | AES-256-GCM wrapper with destination and expiry, followed by a URL parameter |
| API `/session.pb` | Synthetic Protobuf message, ROT47 URL and session header; AES-256-CTR token inserted in the media path |
| Media `/public/master.m3u8` | Public HLS master → relative media playlist → relative MPEG-TS segments |
| Media `/protected/master.m3u8` | Same clip with HMAC signature, expiry and mandatory headers on manifests and segments |
| Media `/denied.m3u8` | Intentional HTTP 403 to test the alternative signal |
| API `/faults/rate-limited` | First request 429 with `Retry-After: 0`, then success |
| API `/faults/server-error` | HTTP 503 |
| API `/faults/malformed.json` | HTTP 200 with invalid JSON |
| API `/faults/redirect-loop` | HTTP redirect loop |
| Media `/live/master.m3u8` and `/live2/master.m3u8` | With `--live`: two sliding-window live channels (6 segments of 2 s, or of the `--segment-seconds` duration) with AES-128 segments. The second is 720p25 HEVC. The first is H.264 with three variants encoded together, so with aligned segments: 1080p at 4 Mbit/s (`index.m3u8`), 720p at 2 Mbit/s and 360p at 800 kbit/s. Playlists, key and segments require the headers of the protected media |
| Media `/live3/master.m3u8` | With `--live`: 720p25 H.264 as FFmpeg packages it, fMP4 segments and audio in a separate track (`EXT-X-MEDIA`), without encryption or injectable faults |
| Media `/live/stream.ts` | With `--live`: the first channel as a single MPEG-TS response without a length, as IPTV servers send it: its decrypted segments one after the other, starting from the middle of the most recent one. Restarts, outages and slowdowns of the first channel apply here too, and an outage closes the response when it ends |
| API `/control/live-stats` | Live stream requests accepted and rejected because of the headers |
| API `POST /control/live-restart` | Restarts the encoder of the first channel. `{"ts_offset": s}` starts the timestamps from `s` seconds (near 95443 the 33-bit MPEG-TS clock wraps to zero shortly after); `{"ts_shift": s}` starts them `s` seconds after the point reached, earlier if negative (on the TV the jump turned out about 2 s further back: `-1` gave −3.5 s, `2` gave +0.4 s). The playlist marks `EXT-X-DISCONTINUITY`, or with `{"reset_sequence": true}` it restarts from segment 0 with only the new encoder |
| API `POST /control/live-outage` | `{"seconds": n}`: for n seconds the requests of the first channel are closed without a response, like a missing network |
| API `POST /control/live-throttle` | `{"seconds": n, "kbps": k}`: for n seconds the responses of the first channel arrive at k kbit/s, like a slow network |
| API `POST /control/live-stall` | `{"seconds": n}`: for n seconds the playlist of the first channel stays the same, without new segments |
| API `GET /live/resolve.json` | With `--live`: JSON in the RaiPlay shape (`video.content_url`) with the URL of the first channel and the current `?t=` token. Playlists requested with a token need the current one, otherwise 403 |
| API `POST /control/live-token` | Changes the token of the first channel: URLs resolved earlier receive 403, like an expired token |

The headers required by the protected media are in the TOML manifest: User-Agent,
Referer of the resolver origin and `X-Lab-Media: synthetic`. The playlist
token does not propagate implicitly to the segments: the fixture also emits
signed links in the media playlist.

The AES-GCM wrapper uses the synthetic format defined in
[crypto.cjs](../dev/lab/crypto.cjs), with cryptographic material created in the lab. The binary session uses
its own scheme: field 1 obfuscated URL, field 2 demonstration key and field
3 IV, with the `X-Lab-Session` header. The message and keys are for demonstration; they do not implement an external
service or a commercial authentication protocol.

## Automated tests and expiry

```sh
python3 -B -m unittest discover -s dev/lab/tests -v
```

The suite starts three servers on free ports, generates the media, runs the
checks and closes the servers and temporary files. The Python client uses lab
addresses and rejects redirects to external hosts. FFmpeg generation and
decoding use only synthetic sources and local URLs; no step requires external
services.

The tests verify:

1. Catalogue headers, partial initial list, alternatives in the detail and empty event.
2. M3U, HLS master, media playlist and actual bytes of a relative segment.
3. Extraction of embedded data and the iframe → API → HLS chain.
4. AES-GCM wrapper, required headers and signature kept down to the segments.
5. Expiry of the old URL and successful new resolution.
6. First source denied, alternative resolved and two seconds of video/audio decoding with FFmpeg.
7. Protobuf/ROT47 decoding, valid AES-CTR session and rejection of a wrong session.
8. Rejection of a tampered AES-GCM wrapper.
9. Rate limit, subsequent success, 503 error, malformed JSON and redirect loop.
10. Local TOML manifest and disabled direct profile with a consistent URL.
11. Live stream over HTTPS: three variants with the same segment numbering, mandatory headers, AES-128 key, decoding with FFmpeg and counters.
12. Live stream: encoder restart with `EXT-X-DISCONTINUITY` and network outage, then recovery; stuck playlist, timestamps shifted slightly and numbering restarting from 0.
13. Live stream with token: URL from `/live/resolve.json`, 403 after the token change, access without a token as before.
14. A client stuck halfway through the TLS handshake does not block the other connections.
15. Live stream as a continuous TS stream: response without a length, decoding with FFmpeg and closing during an outage.

The suite uses the [local decoders](../dev/lab/codecs.py) for the wrapper, the
embedded data and the binary message. It imports no external research tools.
Selection, fallback and new resolution are driven by the test and are not
the future application runtime.
The FFmpeg check requires decoded frames and non-empty PCM audio output,
without saving the video. It does not verify listening or visual rendering.

Tokens last 60 seconds on the fixture clock. In a normal start
the clock follows elapsed time; in the tests it is manual. The request:

```text
POST <api-origin>/control/advance
Content-Type: application/json

{"seconds": 61}
```

moves the time past the expiry without waiting. The old URL returns
403 `token_expired`; reading the resolver again returns a new link.
This verifies recovery between requests. Renewal in the middle of a
live stream is instead tested on the TV with the token of the first live channel
(`/live/resolve.json` and `POST /control/live-token`): the app, which resolves it with
the provisional recipe of its `.stream` file, receives 403 and resolves
the source again.

## Moving to player e2e tests

The lab stays outside the core, as described in [Modular architecture](architecture.md).
Error classes and protocols are described by behaviour, so the
fixtures stay useful for future providers too.

When the runtime exists, the e2e tests will have to use its TOML loader,
its models, resolvers and session handling. The test will have to import
a local profile, get the catalogue, select an event, apply the
fallback and verify playback, then force expiry/renewal or a channel
change. The test client will be replaced by the application, keeping the
servers and the observable expectations. The fixture manifest will make it
possible to map the assigned ports to the profiles during the test.

| Level | Status |
|---|---|
| Fixture/parser contract and HTTP integration | Implemented in the suite |
| Real decoder with synthetic media over HTTP | Implemented with FFmpeg |
| TOML loading and execution of Mana recipes | To implement in the runtime; on the TV only the provisional recipe of the `.stream` files runs |
| E2e of the Mana session and UI | To connect to the runtime |
| Decoder, remote control, HLS with headers and webOS lifecycle | Verified on the TV ([checks](tv-app.md#verification-on-the-b1)) |

Other scenarios to add when the runtime's capabilities require them:
byte ranges, discontinuities with fMP4, DASH/progressive, cookies,
transient 5xx, slow network,
loops between resolvers, and cancellation. DRM and P2P require specific
environments and backends; this first version does not simulate them.

## Components

- [server.py](../dev/lab/server.py): lifecycle of the three origins and HTTP handling.
- [scenarios.py](../dev/lab/scenarios.py): scenario responses by origin.
- [state.py](../dev/lab/state.py): time, signatures, synthetic sessions and live stream token.
- [live.py](../dev/lab/live.py): live streams encoded in real time and injectable faults, also as a continuous TS stream.
- [testcard.py](../dev/lab/testcard.py): test picture with flash and beep for sync, and its encoding in one or more variants.
- [packaged.py](../dev/lab/packaged.py): live stream packaged by FFmpeg, fMP4 with separate audio.
- [tls.py](../dev/lab/tls.py): CA and server certificate for `--tls`.
- [codecs.py](../dev/lab/codecs.py): standalone decoders of the test client.
- [crypto.cjs](../dev/lab/crypto.cjs): local generation of wrappers/tokens through Node.
- [media.py](../dev/lab/media.py): generation of the HLS clip with FFmpeg.
- [profiles.py](../dev/lab/profiles.py): generated manifest with the live stream addresses and TOML profile.
- [test_lab.py](../dev/lab/tests/test_lab.py): contracts and integration with a real decoder.

API references used: [Python HTTP server](https://docs.python.org/3/library/http.server.html),
[Node crypto](https://nodejs.org/api/crypto.html),
[FFmpeg synthetic sources](https://ffmpeg.org/ffmpeg-filters.html#testsrc_002c-testsrc2).
