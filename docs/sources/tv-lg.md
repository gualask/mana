# LG TVs

Consulted on 20 November 2026. Verification grades as in the [index](README.md).

## SoCs and cores of TVs with webOS 6 or later

No source links a pre-ARMv8 core (Cortex-A7, A9, A15, A17) to webOS 6 or
later: the Cortex-A9 appears for the 2011–2015 NetCast TVs
([NetCast specifications](https://webostv.developer.lge.com/more/netcast/platform-specifications),
official). However, the `/proc/cpuinfo` of the budget LCD families K7LP,
K8LP, K8AP, K8HP, LM21A and LM21AN are missing.

| Year and model | SoC | Core | Source |
|---|---|---|---|
| 2021 B1 | MediaTek MT5889 (LM21U) | A73 | the B1's `/proc/cpuinfo`, official; family name from [beanjar](https://github.com/webosbrew/beanjar), secondary |
| 2021 C1, G1 | O20 or K7LP | A55 or A73, conflicting | [Glasshouse table](https://github.com/rorygallagher2024/glasshouse/blob/main/docs/development/tv-specs/index.md), secondary |
| 2021 LCD | Realtek K7LP (RTD2875), LM21A | unknown, A53 according to a summary row | beanjar and Glasshouse, secondary |
| 2022 C2, 2023 C3, 2025 C5 | O22, O22N, O22N3 | 4× A76 | Glasshouse, secondary |
| 2022 LCD | K8LP, K8HP, K8AP, LM21AN | unknown | beanjar, secondary |
| 2024 OLED | O24 | 4× A78 | [Arm newsroom](https://newsroom.arm.com/?p=11672), read only in the search preview |
| 2025 UA73, 2026 NU80 | Realtek K25LP, K25LPN | 4× A73, with cryptography | Glasshouse, secondary |

- The webOS Re:New program updates models from 2022 onwards
  ([LG, 4 January 2024](https://www.lg.com/global/newsroom/news/media-entertainment-solution/more-lg-smart-tv-owners-set-to-enjoy-the-latest-webos-upgrade-making-their-tvs-feel-brand-new/),
  official): no TV older than 2021 reaches webOS 6 (inference).
- From the C3 the kernel can be aarch64 with a 32-bit userland
  ([LG forum](https://forum.webostv.developer.lge.com/t/time-to-switch-your-toolchains-to-64-bit/27101),
  secondary); the B1 has a 32-bit kernel ([ARM CPU](cpu-arm.md#checks-on-the-b1)).

## How other native apps are built

None detects the CPU at startup or chooses among several binaries; only
libraries such as FFmpeg do, reading `getauxval(AT_HWCAP)`
([`libavutil/arm/cpu.c`](https://github.com/FFmpeg/FFmpeg/blob/7ddf7bc8a482cf28b4fab2e63b7093ba35cb6a80/libavutil/arm/cpu.c#L72)).

| Project | CPU | Source |
|---|---|---|
| webosbrew toolchain | GCC `--with-cpu=cortex-a9 --with-fpu=neon-fp16 --with-float=softfp` | [CMakeLists.txt](https://github.com/webosbrew/native-toolchain/blob/730dbd680cc319935655b1b04c2d80fcb62fdb22/package/gcc/CMakeLists.txt#L34-L35) |
| moonlight-tv | toolchain default, a single IPK | [action.yml](https://github.com/mariotaku/moonlight-tv/blob/72500d920b19e399ff1c99ad27a888fd76b0d183/.github/actions/build-webos/action.yml#L16-L34) |
| Kodi | `-mcpu=cortex-a53`, after requiring webOS 4 or later | [configure.ac](https://github.com/xbmc/xbmc/blob/d32d97ccf4fe0607ef9adb3f2e3df98a7f6c8ff6/tools/depends/configure.ac#L420-L428), [commit 4d2206e](https://github.com/xbmc/xbmc/commit/4d2206ecdc) |
| RetroArch | `-mcpu=cortex-a9 -mtune=cortex-a53` with NEON; separate arm, arm legacy and aarch64 IPKs, chosen by the user | [Makefile.webos](https://github.com/libretro/RetroArch/blob/850ecca1f611d0378a975efe34c22dd4d7d0b8cc/Makefile.webos#L167-L174) |
| Stremio Origin | zig `-mcpu=cortex_a9 -mfloat-abi=softfp` | [build.sh](https://github.com/stremio-origin/webos/blob/911e61a00276685b81392bf9541dcf5b542f8b62/tools/build.sh#L27-L29) |
| PlxNative, ytnative | `cortex-a9` without NEON, to avoid crates with unstable NEON intrinsics | [PlxNative Makefile](https://github.com/plxnative/plx-native/blob/d6125c847d2261640be95eb52af79ca7424ea809/Makefile#L940-L956) |

No source points to webOS TVs without NEON.

## System libraries from webOS 6 onwards

Consulted on 22 November 2026 in the webosbrew firmware database
([dev-toolbox-cli, `common/data`](https://github.com/webosbrew/dev-toolbox-cli/tree/2d5a7b6881850d825aa7fdc6c0b5e53c91dc40d3/common/data),
commit of 17 October 2026), secondary: for each firmware it lists the
libraries, with dependencies and exported symbols; the glibc is the one in
the file name or the highest version among its symbols. The package check
([firmware.py](../../tools/webos/firmware.py)) uses it at the same commit.

| webOS release | Firmware | glibc | SDL2 | GLES 2 | TV's FFmpeg |
|---|---|---|---|---|---|
| 6.4.0 | 03.40.82.01, W21P | 2.28 | 2.0.10 | Mali | libavformat 58.12 |
| 7.4.0 | 04.40.90.01, W22O | 2.31 | 2.0.14 | Mali | 58.29 |
| 8.3.0 | 03.30.60.01, W23O | 2.31 | 2.0.14 | Mali | 58.29 |
| 9.2.0 | 23.20.50.01, W23O | 2.35 | 2.0.14 | Mali | 58.76 |
| 10.2.0 | 33.20.40.01, W22H | 2.35 | 2.0.14 | PowerVR with Mesa | 59 |
| 11.2.0 | 43.21.70.01, W23O | 2.39 | 2.0.14 | Mali | 60 |

- The B1 has release 6.5.3 (`/etc/starfish-release`), official: it is newer
  than the database's webOS 6, which serves as the reference.
- The 19 SDL functions the app looks up by name, including the three webOS
  ones for the exported window, and the 7 of `StarfishMediaAPIs` plus
  libstdc++'s `_ZdlPv` are present in all six firmwares, with the same
  symbol name.
- The 26 GLES 2.0 functions the app calls through glow are present in all
  six. Firmware 10.2.0 lacks extensions (`OES`, `EXT`, `KHR`) and desktop GL
  functions (`glPointSize`, `glLogicOp`) that glow loads without the app
  calling them.
- Each generation has its own FFmpeg: libavformat 58 is FFmpeg 4 (4.0 on the
  B1, 4.2 and 4.4 later), 59 and 60 are FFmpeg 5 and 6, with a different
  ABI. A binary linked against the B1's FFmpeg would not start from webOS 10
  onwards: this is why the app uses only the one bundled in the package.
- System fonts are not in the database: not verified.

## Remote and pointer

Consulted and measured on the B1 on 21 November 2026.

- With the Magic Remote pointer on screen, OK arrives as a mouse click
  (`SDL_MOUSEBUTTONDOWN`, `0x401`) and not as Enter; with the pointer hidden
  it arrives as Enter (scancode 40). The remote's arrow keys always arrive as
  keys. Measured on the B1 from the app's log; the same on a C1
  ([REMOTE-INPUT.md](https://github.com/Native-Webos/webos-native-app-kit/blob/main/docs/research/REMOTE-INPUT.md)),
  secondary.
- `SDL_WEBOS_CURSOR_SLEEP_TIME`, set before `SDL_Init`, is the time after
  which the idle pointer disappears; without it, it stays for 5 minutes.
  moonlight-tv, RetroArch and Stremio Origin set it to 5000 ms
  ([moonlight-tv's app.c](https://github.com/mariotaku/moonlight-tv/blob/master/src/app/app.c),
  [Stremio Origin's plat_webos.c](https://github.com/stremio-origin/webos/blob/main/src/platform/webos/plat_webos.c)),
  official. On the B1, with 5000, the pointer disappears after 5 s.
- The TV announces the pointer shown and hidden with the keys
  `SDL_SCANCODE_WEBOS_CURSOR_SHOW` 484 and `_HIDE` 485
  ([SDL-webOS's SDL_scancode.h](https://github.com/webosbrew/SDL-webOS/blob/main/include/SDL_scancode.h)),
  official; on the B1 they arrive, on the C1 cited above they do not.
- The wheel arrives as `SDL_MOUSEWHEEL` with `y` at offset 20, as in SDL 2
  ([PlxNative's inject.rs](https://github.com/plxnative/plx-native/blob/main/rust-modules/shell-sdl/src/inject.rs)),
  official; motion and click coordinates at offsets 20 and 24, in
  1920×1080 pixels on the B1.
- On the B1, after `SDL_ShowCursor(SDL_DISABLE)` the TV sends no more motion
  until the app calls `SDL_ShowCursor(SDL_ENABLE)`: only the app can bring
  the pointer back, and in about ten tries once it did not come back even
  that way. This is why Mana does not hide the pointer over the live stream
  and lets the TV put it to sleep after 5 s.
- On the B1, hiding the pointer between the press and release of a click
  loses the release: SDL keeps the button pressed and discards the press of
  the next click, of which only the release arrives. Stremio Origin never
  hides the pointer while a click is pressed
  ([app.c](https://github.com/stremio-origin/webos/blob/main/src/core/app.c)),
  official.
