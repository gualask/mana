# Sources

Official and first-hand documentation consulted for the project's choices,
with the facts we draw from it. Each fact carries its link and a
verification grade:

- **official**: read in the manufacturer's source or in the source code of
  the cited project;
- **secondary**: third-party tables, forums, summaries;
- **inference**: derived by us, to be verified before relying on it.

Full copies of documents that cannot be redistributed are in
`private/fonti/`, outside the repository.

## Notes

- [ARM CPU](cpu-arm.md): extensions of Cortex-A53, A55, A73 and A76 in
  AArch32, Linux HWCAP, ARM features in LLVM and rustc, measurements on the
  B1.
- [LG TVs](tv-lg.md): SoCs and cores of TVs with webOS 6 or later, how other
  native apps are built, system libraries from webOS 6 onwards, remote and
  Magic Remote pointer.
- [Performance](performance.md): compositor and redraw, feeding data to
  Starfish, TLS and AES decryption on 32-bit ARM.

## Platform and format documentation

- [Rust ARM Linux](https://doc.rust-lang.org/stable/rustc/platform-support/arm-linux.html).
- [webOS Brew native toolchain](https://github.com/webosbrew/native-toolchain).
- [LG streaming protocols and DRM](https://webostv.developer.lge.com/develop/specifications/streaming-protocol-drm).
- [LG webOS 6.0 audio/video formats](https://webostv.developer.lge.com/develop/specifications/video-audio-60).
- [HLS, RFC 8216](https://www.rfc-editor.org/rfc/rfc8216).
- [TOML](https://toml.io/en/v1.0.0) and [JSON Schema](https://json-schema.org/overview/what-is-jsonschema).
- [webos-userland](https://github.com/webosbrew/webos-userland): headers of
  `StarfishMediaAPIs` and pipeline events.
- Public notes on NDL and Starfish:
  [jellyfin-webos-native](https://github.com/HookedBehemoth/jellyfin-webos-native/blob/HEAD/docs/ndl.md)
  and [punktfunk client-webos](https://github.com/punktfunk/client-webos/blob/HEAD/docs/NOTES.md).
