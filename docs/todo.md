# Mana development — preparation of 15 November 2026, scope live streams on a rooted TV since 16 November

## Next steps

- [x] Define the executable stream contract and update schema and examples together — done when catalogue, detail and resolution have typed inputs/outputs and validatable references, with no versions; unknown capabilities, missing references and cycles are rejected ([Configuration and extensions](configuration.md#source-files)).
- [ ] Add the local CLI on the core — done when a direct URL and the synthetic JSON catalogue go through the same application models and reach local decoding; the core and the apps do not import the lab ([Modular architecture](architecture.md#target-structure)).
- [ ] Implement TOML loading and JSON import into the same plan — done when equivalent profiles produce the same plan, errors point to the file and field, and an invalid update keeps the last valid profile ([pipeline](configuration.md#profiles-state-and-versions)).
- [ ] Run HTTP/JSON recipes and composable resolvers — done when nested detail, dispatch and resolver chains work with finite budgets and without provider logic in the UI ([resolution](architecture.md#step-by-step-resolution)).
- [ ] Connect the resolvers of the encrypted and binary fixtures through the registry — done when the runtime resolves both cases and rejects tampered wrappers or wrong sessions ([scenarios](local-lab.md#three-origins-and-available-scenarios)).
- [ ] Connect the e2e tests to the runtime and the application session — done when the player performs fallback, renewal after expiry and cancellation on channel change, discarding late results; the suite also verifies empty events and denials ([e2e](local-lab.md#moving-to-player-e2e-tests)).
- [ ] Build the desktop app — done when `apps/desktop` on the Mac plays a lab live stream and a clip with synchronized audio, measured with `dev/tv/avsync.py`, the channel list scrolls over the video at 50 fps, and `.local/rune-ui/` is deleted ([desktop client](architecture.md#desktop-client)).

## Open questions

- Skip the Pluto TV ads?
  - Evidence: the break opens with a discontinuity and a bumper from the path `…/Pluto_TV_OandO/clip/…_ad_bumper_…` or `…/pluto_tv_o_o/clip/…_ad_bumper_…`; the following clips come from `/mp4/v1/…/p(…/video_*.mp4)/id3(p=clik,…)`, converted on the fly from the video track alone, without audio. The break points match across different sessions (segment 269 on the TV and in the probe), but the content of the break changes between sessions. It is not yet known which following clips are ads and where the break ends.
  - Evidence (17 November): the playlist has no ad markers (`CUE-OUT`, `DATERANGE`) and holds 5 segments, about 25 s, so in a live stream a break can be covered but not skipped. Each episode is a complete HLS asset on the CDN (`…/clip/<id>_int/…/hls/<inizio>-end/…`), playable from any point.
  - To decide: the user's choice is postponed. The option that interests them, for later, is playing the episode from the complete asset instead of the stitched live stream, but that falls outside the live-only scope; Pluto's terms probably do not allow it.
- How does the interface show a source's catalogue and an event's several signals?
  - Evidence (23 November): a live event of an unofficial sports source, resolved by the throwaway interpreter and played on the TV from a `.stream` file, ran for over 40 minutes without interruptions; its detail held three signals: the direct HLS blocked by the home DNS, one player that plays, one answering 403. The Rai source's catalogue lists 15 channels, Pluto's guide 129.
  - Proposal: the list names a source as it includes an M3U (`[[include]] source = …`); its catalogue is a group in the home screen and over the video; events by time, those without signals dimmed with the reason; OK tries the signals in order and passes to the next when one fails, with a key to move to the next signal by hand; the catalogue read again when the list opens if the copy is a few minutes old.
  - To decide: automatic signals or a list to choose from, when to read the catalogue again, how future and ended events show; with steps 3–4 ([source files](configuration.md#not-built-yet)).
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
