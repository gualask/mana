# Starfish

Consulted on 25 November 2026. Verification grades as in the [index](README.md).
How Mana feeds Starfish is in [How playback works](../tv-app.md#how-playback-works);
the cost of feeding it is in [Performance](performance.md#feeding-data-to-starfish).

## Presented position

- Mana takes the presented position from Starfish's frame-ready event, the
  PTS of the frame on screen
  ([events.rs](../../apps/tv/src/platform/starfish/events.rs)).
- vplayer (libVLC on webOS) measured both clocks on an OLED65C1 with
  webOS 6.5.3. webos-native-app-kit's estimated clock, which starts at the
  first packet fed, ran 0.7–0.9 s ahead of the picture. The frame-ready
  position stayed within one frame of it, measured with a timer burned into
  the video and a subtitle cue every 200 ms
  ([RESEARCH.md §12](https://github.com/Native-Webos/vplayer/blob/9db4f7f39b4164f97790357dfd6502e55413b113/docs/RESEARCH.md#12-subtitles-on-the-hardware-path-oled65c1ptb-webos-653-2026-11-24-dev)),
  secondary.

## HDR10, HLG and Dolby Vision

Mana does not declare HDR or Dolby Vision to Starfish
([starfish.rs](../../apps/tv/src/platform/starfish.rs)).

- webos-native-app-kit calls `StarfishMediaAPIs::setHdrInfo(const char*)`
  after Load, as Kodi does. The payload is `hdrType` `HDR10` for transfer
  16 (PQ) or `HLG` for 18, a `sei` object (mastering display primaries in
  G, B, R order in units of 1/50000, white point, luminance in
  1/10000 cd/m², MaxCLL and MaxFALL) and a `vui` object (transfer,
  primaries, matrix, full range). With no mastering metadata the call is
  skipped, because some TVs crash on an HDR message without SEI (Kodi)
  ([starfish_backend.cpp](https://github.com/Native-Webos/webos-native-app-kit/blob/10d4b7a907f24f18649e8c8cd8e36cc7bc71d7f7/media/src/starfish_backend.cpp#L420-L440),
  [test_starfish.cpp](https://github.com/Native-Webos/webos-native-app-kit/blob/10d4b7a907f24f18649e8c8cd8e36cc7bc71d7f7/tests/unit/test_starfish.cpp#L165-L189)),
  official.
- On the C1, 10-bit PQ VP9 and AV1 switch the panel to HDR only with
  `setHdrInfo`: without the call it stays SDR
  ([MILESTONES.md](https://github.com/Native-Webos/webos-native-app-kit/blob/10d4b7a907f24f18649e8c8cd8e36cc7bc71d7f7/docs/MILESTONES.md#L330-L333)),
  secondary. The kit's comment says the decoder still reads the HDR SEI
  carried in the bitstream; whether HEVC PQ without the call switches the
  B1 to HDR is not verified.
- `_ZN17StarfishMediaAPIs10setHdrInfoEPKc` is exported by
  `libplayerAPIs.so.1` in all six firmwares of the
  [system libraries table](tv-lg.md#system-libraries-from-webos-6-onwards),
  webOS 6.4.0 to 11.2.0, secondary.
- Dolby Vision goes in the Load payload, next to codec `H265`:
  `"DolbyHdrInfo":{"trackType":"single","encryptionType":"clear","profileId":8}`.
  `profileId` must be a JSON integer, because a string reads as -1
  ([starfish_backend.cpp](https://github.com/Native-Webos/webos-native-app-kit/blob/10d4b7a907f24f18649e8c8cd8e36cc7bc71d7f7/media/src/starfish_backend.cpp#L367-L373)),
  official.
- LG's pipeline takes single-layer Dolby Vision only. The kit converts
  Profile 7 to 8.1 the way `dovi_tool -m 2` does: it drops the enhancement
  layer's NAL units (type 63, or `nuh_layer_id` above 0) and rewrites each
  RPU (type 62) with libdovi. It converts only records whose base layer is
  HDR10-compatible (`dv_bl_signal_compatibility_id` 6). On the C1, a 4K
  Profile 7 file played as Dolby Vision with 0 dropped frames out of 1066
  ([NATIVE-MEDIA.md](https://github.com/Native-Webos/webos-native-app-kit/blob/10d4b7a907f24f18649e8c8cd8e36cc7bc71d7f7/docs/research/NATIVE-MEDIA.md#dolby-vision-profile-7-hw)),
  secondary.
- When the configuration record (`dvcC`, `dvvC`) is missing, vplayer
  recognises Dolby Vision from the first keyframe, by its RPUs or its
  enhancement layer. It takes the profile from the base layer's transfer:
  PQ gives 8.1, HLG 8.4 and anything else 5
  ([RESEARCH.md §14](https://github.com/Native-Webos/vplayer/blob/9db4f7f39b4164f97790357dfd6502e55413b113/docs/RESEARCH.md#14-dolby-vision-on-the-hardware-path-oled65c1ptb-webos-653-2026-11-24-dev)),
  secondary.
