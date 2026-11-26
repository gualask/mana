# Mana development — preparation of 15 November 2026, scope live streams on a rooted TV since 16 November

## Next steps

- [x] Define the executable stream contract and update schema and examples together — done when catalogue, detail and resolution have typed inputs/outputs and validatable references, with no versions; unknown capabilities, missing references and cycles are rejected ([Configuration and extensions](configuration.md#source-files)).
- [x] Run HTTP/JSON recipes and composable resolvers — done when nested detail, dispatch and resolver chains work with finite budgets and without provider logic in the UI ([resolution](architecture.md#step-by-step-resolution)).
- [x] Connect the resolvers of the encrypted and binary fixtures through the registry — done when the runtime resolves both cases and rejects tampered wrappers or wrong sessions ([scenarios](local-lab.md#three-origins-and-available-scenarios)).
- [x] Connect the e2e tests to the runtime and the application session — done when the player performs fallback, renewal after expiry and cancellation on channel change, discarding late results; the suite also verifies empty events and denials ([e2e](local-lab.md#player-e2e-tests)).
- [x] Choose an event's signal from a list — done when OK on a channel or event whose source gives several candidates shows them with their labels, choosing one plays it, Mana no longer passes to the next one by itself, and the list can be opened again while watching to change signal ([playing a channel with a source](channel-list.md#playing-a-channel-with-a-source)).
- [x] Show a source's catalogue as a folder of the list — done when `[[include]] source = …` is a row with the source's name, no catalogue is read at start or with the list, OK opens the folder on its copy at once and on the source's events once they come (with signals only, by time, in the TV's local time), the source is asked again when the copy is from an earlier start or older than 2 minutes, a failure keeps the copy and says why, Back on the home screen returns to the list, and a source's nested event names reach the catalogue through `record` ([channel list](channel-list.md#format)).
- [x] Search channels and events by name — done when «Search» on the home screen opens an on-screen keyboard usable with the arrows and the pointer, the results narrow as the text grows among the list's channels and the events of the folders' saved copies, regardless of case and accents and without network requests, OK plays a result and Back closes the search ([TV app](tv-app.md)).
- [ ] Build the desktop app — done when `apps/desktop` on the Mac plays a lab live stream and a clip with synchronized audio, measured with `dev/tv/avsync.py`, the channel list scrolls over the video at 50 fps, and `.local/rune-ui/` is deleted ([desktop client](architecture.md#desktop-client)).

## Open questions

- Offer the automatic passing to the next signal as a setting?
  - Evidence (23 November): the user wants to choose the signal from a list rather than have Mana pass to the next one by itself, which keeps a signal that plays badly as long as it does not stop.
  - To decide: whether the setting is worth its complexity, once the list of signals exists.
- Stop resolving a candidate again when the new URL is the one just refused?
  - Evidence (23 November): on the TV, a signal of an unofficial sports source answered 403 to its media; each of the 6 new resolutions allowed gave the same URL, refused again, and the menu of signals opened again after about 6 s.
  - To decide: whether an identical URL ends the attempts at once, or a source may give the same URL with a token renewed on its server ([reopen.rs](../crates/core/src/session/reopen.rs)).
- Keep the channel logos in a disk cache?
  - Evidence (17 November): in the test with 600 channels, Wikimedia answered 429 to about sixty logos requested in a burst, even with a User-Agent. Stremio Origin and PlxNative keep logos on disk (40 and 128 MB), and the app's in-memory cache loses them at every start.
  - Evidence (19 November): the user wants the cache without clogging the TV with useless or heavy files, and wants to try the logos with Chrome's User-Agent only together with the cache, so as not to risk blocks from Wikimedia. The logos are drawn in 128×72 cells ([gfx.rs](../apps/tv/src/screens/gfx.rs)), while a downloaded file can reach 2 MB: the cache would keep the recompressed thumbnail, a few KB, with an overall cap and eviction of the least used, in a folder that survives power-off.
  - To decide: a choice to make with the real interface, together with the cache size; reference: [logos.rs](../apps/tv/src/screens/logos.rs).
- Hand the first segment to FFmpeg while it downloads, instead of whole?
  - Evidence (21 November): with the TV's network slow (about 5 Mbit/s against the Mac's 45), Rai 1 starts in 7.4 s even from the 576p variant, against 1.8 s in the morning: the segments last 10 s and weigh 2.6 MB, which the TV downloads in 6–9 s, and opening waits for the whole first one ([downloader/](../crates/core/src/live/hls/downloader/mod.rs)); FFmpeg needs about 0.5 MB of it. Media3 and FFmpeg `hls.c` read the segment as it arrives; hls.js downloads it whole unless `progressive` is set.
  - Evidence: the least risky route is the first segment only, with block-wise AES decryption and an interruption halfway treated as a failed open; estimate: from about 7 s to about 3 s on that network, little on a normal network.
  - To decide: the user (21 November) prefers not to complicate the code for now; to revisit if slow starts also show with the network in order.
- Make the quality switch-up faster?
  - Evidence (20 November): on the TV, with the three-variant lab live stream, the down-switch arrives about 9 s after the slowdown (1080p → 720p at 2.5 Mbit/s, 720p → 360p at 1.2 Mbit/s) without stalls or jumps in the timeline (sync not measured with `avsync.py`). Once the network is back, the switch up to 720p arrives after 66 s and the one to 1080p after 216 s, with an estimate of 6022 kbit/s just above the 6 Mbit/s that the 4.2 Mbit/s variant needs with the 70% share.
  - Evidence: the hls.js average weights segments by download time, so those downloaded quickly count little; Media3 uses a sliding median of the last segments and needs 10 s of buffer to switch up ([variant selection](hls-cases.md#variant-selection)).
  - Evidence (20 November): the wait before switching up, added against variants alternating when the bandwidth is at the limit, deliberately slows switch-ups after a down-switch; a faster switch-up must be balanced against that wait ([HLS live edge cases](hls-cases.md#variant-selection)).
  - To decide: how long the switch-up may take on a TV; reference: [ladder/](../crates/core/src/live/hls/ladder/).
- Reduce the CPU of TLS, of the panel during the live stream and of the handoff to Starfish?
  - Evidence (21 November): segments are decrypted with the AES instructions when the CPU has them, about 50 times faster than the `aes` crate on the B1; TLS still encrypts with AES on NEON, because ring does not use the AES instructions on 32-bit ARM ([performance](sources/performance.md#tls-and-aes-decryption)).
  - Evidence (22 November): on Nove, whose CDN picks AES-GCM, TLS is 14% of the app's samples, 0.019 cores ([memory](sources/performance.md#memory)).
  - Evidence: during the live stream the panel is redrawn 4 times per second even when hidden, and the compositor costs 2% of a core; PlxNative fears that stopping presenting would disturb the video plane ([performance](sources/performance.md#compositor-and-ui-redraw)).
  - Evidence: the thread that hands frames to Starfish weighs 11–17% of the app, almost all inside Starfish; all the apps examined hand over one frame per call ([performance](sources/performance.md#feeding-data-to-starfish)).
  - To decide: TLS with AWS-LC, which uses the AES instructions on 32-bit ARM; an A/B test of the panel with screenshots; an experiment with several audio frames per call.
- Bound the segment queue in MB?
  - Evidence (22 November): three segments of at most 32 MB each (`segment::MAX_BYTES`) allow about 100 MB queued on a 4K channel; on Rai 1 and Nove the queue costs at most 8 MB ([memory](sources/performance.md#memory)).
  - To decide: when a 4K or high-bitrate source is played; reference: [download.rs](../crates/core/src/live/hls/download.rs).
- Keep cookies between playlists and segments?
  - Evidence (19 November): webos-native-app-kit reports providers that set a cookie on the playlist and require it on the segments; the app's client (ureq without the cookies feature) does not keep them; no source tested so far requires them.
  - To decide: a source that answers 403 to segments without the cookie received with the playlist; reference: [net.rs](../crates/core/src/net.rs).
