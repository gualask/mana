# HLS live edge cases

Reference for the cases an HLS client for live streams must handle, compared
with [hls.js](https://github.com/video-dev/hls.js) (commit `127c6ad`, 12
November 2026) and with the HLS module of [Media3](https://github.com/androidx/media)
(commit `8c6678b`, 8 September 2026). Both are Apache 2.0: their logic and tests
can be reused with attribution. For each case: how they handle it, its status
in our client ([live/hls/](../crates/core/src/live/hls/),
[playback/](../crates/core/src/playback/)) and, if missing, its priority for
live streams on a single TV. Scope: live streams only, without seeking,
subtitles or DRM. The comparison comes from reading the code, not from tests on
the TV.

Abbreviations: **H** = hls.js `src/`, **M** = Media3
`libraries/exoplayer_hls/src/main/java/androidx/media3/exoplayer/hls/`.

## Playlists and reloading

| Case | hls.js / Media3 | Our status |
|---|---|---|
| Reload after the last segment, half the target if unchanged | H `utils/level-helper.ts` `computeReloadInterval`; M `playlist/DefaultHlsPlaylistTracker.processLoadedPlaylist` | Handled (`Downloader::run`), never less than half the target. Negative, non-finite or out-of-range durations make the playlist invalid, and the next reload retries. A target duration below 1 s counts as 1 s, as in hls.js |
| Stuck playlist: it responds but does not advance | M `PlaylistStuckException` after 3.5 targets; H `liveMaxUnchangedPlaylistRefresh` | Handled: after 3.5 target durations the stream reopens from the edge |
| `MEDIA-SEQUENCE` that goes backwards (encoder restarted) | M `PlaylistResetException`, `HlsMediaPlaylist.isNewerThan`; H `adjustSliding` | Handled like a stuck playlist: no new segment for 3.5 target durations, then a reopen from the edge |
| Older playlist from another CDN node | M `isNewerThan` | Handled: only segments after the last one are taken |
| Empty live playlist | M `HlsChunkSource.getNextChunk`; H `LEVEL_EMPTY_ERROR` | Handled |
| BOM, `EXT-X-DEFINE`, `EXT-X-START`, `HOLD-BACK` | M `HlsPlaylistParser.checkPlaylistHeader`, `HlsMediaSource.getTargetLiveOffsetUs`; H `latency-controller.ts` | BOM handled (`playlist::is_playlist`); to add, low: `EXT-X-DEFINE`; `EXT-X-START` and `HOLD-BACK` are ignored |
| Base URL after a redirect | M `ParsingLoadable.load`; H `loader/playlist-loader.ts` `getResponseUrl` | Handled: the master and media playlists use the final URL as their base (`Client::open` for the master, `Client::get_with_final_url` for the media playlists). RaiPlay on netrw.it needs this, because its 307 puts the token in the path: verified on the TV with Rai Premium and Rai Yoyo |
| Compressed playlists | — | Handled: `Accept-Encoding: gzip`; the Rai 1 playlist goes from 242 KB to 7 KB |

## Segments and network

| Case | hls.js / Media3 | Our status |
|---|---|---|
| Segment retries | M `DefaultLoadErrorHandlingPolicy` (increasing wait, then error); H `fragLoadPolicy` (4xx responses are not retried, `utils/error-helper.ts`) | Handled: 3 attempts, then the segment is skipped, which suits a TV better |
| Slow download | H `maxTimeToFirstByteMs`, `maxLoadTimeMs` | To add, low: there is only the read timeout |
| Bytes downloaded ahead | H `maxBufferSize` (60 MB); M `DefaultLoadControl.DEFAULT_VIDEO_BUFFER_SIZE` (125 MiB of video); FFmpeg `hls.c` reads each segment as it goes | Handled: at most 3 segments wait for the demuxer, and past the first at most 48 MB ([download.rs](../crates/core/src/live/hls/download.rs)). A 4K channel's 10 s segments, 31 MB at 25 Mbit/s, wait one at a time: with the one being read and the one downloading, about 93 MB instead of 155; those of Rai 1 and Nove (3–6 MB) never reach the bound |
| Key cache | M LRU of 4 (`HlsChunkSource`) | To add, low: the cache grows without limit |
| Encrypted `EXT-X-MAP`, several `EXT-X-KEY` per segment | M `HlsMediaPlaylistParserTest` | To add, low |
| `EXT-X-GAP` | M `HlsMediaChunk.load`; H `FRAG_GAP` | To add, low |
| `BYTERANGE` | — | Out of scope for now: explicitly rejected |
| Segment read as it arrives | M and FFmpeg `hls.c` read it as it arrives; H downloads it whole unless `progressive` is set | Out of scope (decided on 28 November): the segment is downloaded whole, so a download broken halfway is retried in silence instead of failing the opening. On the B1 with a slow network (about 5 Mbit/s, 21 November) Rai 1 starts in 7.4 s, against 1.8 s on a normal network: its 10 s segments weigh 2.6 MB, of which FFmpeg needs about 0.5 MB to open |

## Discontinuities and timestamps

| Case | hls.js / Media3 | Our status |
|---|---|---|
| One time reference for each discontinuity | M `TimestampAdjusterProvider.getAdjuster`; H `initPTS[frag.cc]` in `controller/stream-controller.ts` | Handled: a change of `DISCONTINUITY-SEQUENCE` realigns the timeline of each stream, whatever the jump |
| `DISCONTINUITY-SEQUENCE` missing when the discontinuity leaves the window | H `mergeDetails`; M `getLoadedPlaylistDiscontinuitySequence` | To add, medium: the number would go down |
| Wraparound of the 33-bit clock | M `TimestampAdjuster.adjustTsTimestamp`; H `normalizePts` | Handled (`Unwrap`) |
| Video that does not start on a keyframe | H `forceKeyFrameOnDiscontinuity` | To add, medium: done only at startup (`align`) |
| Changing PIDs and formats | M `HlsSampleStreamWrapper.getMappedTrackOutput`; H `needsProbing` | Handled: one demuxer per section |

## Separate tracks

| Case | hls.js / Media3 | Our status |
|---|---|---|
| Separate audio that waits for the video in the same discontinuity | H `audio-stream-controller.ts` (`WAITING_INIT_PTS`, `syncWithAnchor`); M `sharedInitializeOrWait` | Handled for equal numbers; to add, medium: a fallback if the numbers of the two playlists diverge |
| Alignment at startup | H `utils/discontinuities.ts` (`alignStream`, `alignMediaPlaylistByPDT`) | Handled (`align`) |
| Audio-only variants or variants with codecs that cannot be decoded | H `level-controller.ts` `filterAndSortMediaOptions`; M `HlsMediaPeriod` | Partly handled: variants declared audio-only are discarded, and switching happens only between variants with the audio group and the video codec family of the best one; to add, medium: the start does not check whether the TV decodes those `CODECS` |

## Formats and audio

| Case | hls.js / Media3 | Our status |
|---|---|---|
| Codec change | — | Handled: the pipeline reloads; to add, medium: audio sample rate and channels are not compared |
| Packed audio with ID3 timestamps | M `HlsMediaChunk.peekId3PrivTimestamp` | Handled ([packed_audio.rs](../crates/core/src/live/hls/packed_audio.rs)) |
| Gaps in the audio | H `MP4Remuxer.remuxAudio` (silent frames, overlaps discarded) | Handled for gaps; to add, medium: overlaps |
| Whole stretches without audio | no equivalent found | Handled: AAC silence |

## Live edge, stalls and recovery

| Case | hls.js / Media3 | Our status |
|---|---|---|
| Start from the live edge | H `liveSyncDurationCount` (3); M `HlsMediaSource.getTargetLiveOffsetUs` (3 targets without `HOLD-BACK`) | Handled: at least 3 target durations (RFC 8216 §6.3.3), the margin against a slow network. In the lab with 6 s segments, 11 s at 100 kbit/s stall a start at 2 segments, not one at 3 |
| Outside the window | M `BehindLiveWindowException`; H `synchronizeToLiveEdge` | To add, medium: playback jumps to the first segment of the window, not to the edge |
| Growing delay from the live edge | H `latency-controller.ts`; M `DefaultLivePlaybackSpeedControl` (not read) | To add, medium: reopen from the edge beyond a threshold |
| Stalled playback | H `gap-controller.ts` | Handled: after 15 s the stream reopens, at most 6 times in 5 minutes |
| Network outage or slow network | — | Handled: the pipeline pauses before it runs out of data, and after more than 10 s without data the stream reopens from the edge ([playback](tv-app.md#how-playback-works)) |

## Variant selection

| Case | hls.js / Media3 | Our status |
|---|---|---|
| Bandwidth estimate | H `utils/ewma-bandwidth-estimator.ts` (two averages over 3 and 9 s of download, the lower one); M `DefaultBandwidthMeter` (sliding median) | Handled as in hls.js, on the video segments ([ladder/](../crates/core/src/live/hls/ladder/)). Segments downloaded quickly carry little weight, so switching up takes longer the closer the network is to what the heavier variant needs |
| Switching down and up | H `abr-controller.ts` (`abrBandWidthFactor` 0.95, `abrBandWidthUpFactor` 0.7); M `AdaptiveTrackSelection` (`bandwidthFraction` 0.7, 10 s of buffer to switch up); Shaka `abr.switchInterval` (8 s between two switches) | Handled: it switches down at once when the variant needs more than 95% of the estimate, and switches up to one that fits in 70%. Before any measurement, it starts from the best variant within 576 lines (or within 2.5 Mbit/s if the variants do not declare their resolution), as hls.js starts from an estimate of 500 kbit/s (`abrEwmaDefaultEstimate`) and Media3 from an initial estimate. A reopen resumes from the last choice |
| Variants that alternate when the bandwidth is at the limit | dash.js `SwitchHistoryRule` (beyond a share of recent down-switches it stays one step lower); Shaka `switchInterval` | Handled: after a down-switch, it switches up no sooner than 10 s later. An up-switch reversed within 60 s raises the wait to 60 s, then doubles it up to 240 s; an up-switch that holds for 60 s brings it back to 10 s. Below 540 lines, a picture that is not yet good on a TV, the wait stays at most 60 s. In 10 simulated minutes with the bandwidth between 4 and 8 Mbit/s every minute, around what 1080p needs, the switches drop from 20 to 7; between 2 and 8 Mbit/s every 20 s, from 33 to 15, because from 360p it retries every minute |
| Quality chosen by the viewer | Player selectors (YouTube); H `currentLevel`; M `TrackSelectionOverride` | Handled in the core: automatic or a height, with the best variant not taller than it kept whatever the bandwidth, for the whole session (`Session::set_quality`). On the TV from the menu opened with the right arrow, starting from `quality` in `/tmp/mana.conf` |
| Variant switch mid-stream | H `adjustSliding` (alignment by sequence number without `PROGRAM-DATE-TIME`); M `HlsChunkSource.getNextMediaSequenceAndPartIndex` (by time); RFC 8216 §4.3.3.2 forbids relying on equal numbers | Handled: the new variant continues from the segment that starts closest to the same instant, by `PROGRAM-DATE-TIME` if both have it, otherwise by distance from the live edge (from the start for ended playlists), in a new section with its own discontinuity. On the B1 (21 November) the lab stream with limited bandwidth goes down from 1080p to 720p and 360p and back up without low buffer, with the presented position advancing one second per second across the sections |
| Download aborted because it is too slow | H `_abandonRulesCheck` | To add, low: it switches down only after the slow segment |

Switching up is kept as it is (decided on 28 November): steady quality matters more than a quick return to the best one, and the viewer can still fix the quality from the menu. Simulated with the estimate alone (no hold) after 60 s of slow network, with the lab's variants (1.0, 2.2 and 4.2 Mbit/s declared), the ladder goes back to 720p after 28–50 s and to 1080p after 48 s at 45 Mbit/s, 82 s at 10, 202 s at 7, and never at 6.5. The 216 s measured on the B1 on 20 November (estimate 6022 kbit/s, against the 6 Mbit/s 1080p needs) fall in that last case: a network at the threshold, where going up sooner would bring back the alternation the hold is there to prevent.

## Errors and fallback

| Case | hls.js / Media3 | Our status |
|---|---|---|
| 403 or expired token | H `retryForHttpStatus` never retries a 4xx; M `DefaultLoadErrorHandlingPolicy.isEligibleForFallback` excludes the track for 60 s, else retries the same URL 3 times at most 5 s apart; F gives up; none renews tokens. Nuvio, asking its addons again before playback, drops a URL equal to the failed one | Handled, also on the first playlist or on the recipe at open ([renewal.rs](../crates/core/src/session/reopen/renewal.rs)): on 401, 403 and 410 a stream of the list, which cannot change, is tried again after 1, 3 and 5 s (a server may still count the connection of the channel just left); one of a source is resolved again, at most 6 times in 5 minutes, an event's signals read again first since its stream may come from them. The very stream just refused, given again, is tried once more (a server may authorize the device when asked), then viewing ends; a stream that played starts a new series. Viewing ends with the HTTP status |
| Redundant variants, content steering | M `HlsRedundantGroup`, `excludePlaylist`; H `error-controller.ts` | To add, medium (steering low) |

## Metadata and low latency

| Case | hls.js / Media3 | Our status |
|---|---|---|
| `PROGRAM-DATE-TIME` | M `HlsMediaSource.getLiveEdgeOffsetUs` | Read to align variant switches; for the delay from the live edge, to add, low |
| ID3, `DATERANGE`, interstitials | H `id3-track-controller.ts`, `loader/date-range.ts` | Out of scope: no current source uses them |
| LL-HLS (`PART`, `PRELOAD-HINT`, blocking reload) | both | Out of scope: the tags are ignored and the stream plays with normal latency |
| Updated playlist without `EXT-X-MAP` | M `DefaultHlsPlaylistTrackerTest` | To add, low |

## Test bench

- hls.js, `tests/test-streams.js`: among others
  `test-streams.mux.dev/dai-discontinuity-deltatre/manifest.m3u8` (ads
  with discontinuities) and `test-streams.mux.dev/pts_shift/master.m3u8`. Almost
  all are VOD: they must be served with a sliding window by the
  [Synthetic lab](local-lab.md). The reachability of the URLs is not verified.
- hls.js, unit tests: `tests/unit/controller/` (`level-helper`,
  `error-controller`, `audio-stream-controller`, `gap-controller`,
  `latency-controller`), `tests/unit/loader/m3u8-parser.ts`,
  `tests/unit/remux/mp4-remuxer.ts`.
- Media3: `libraries/test_data/src/test/assets/media/m3u8/` and the tests
  `DefaultHlsPlaylistTrackerTest`, `HlsMediaSourceTest`, `HlsChunkSourceTest`,
  `HlsMediaPlaylistParserTest`.
