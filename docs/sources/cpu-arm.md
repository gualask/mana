# ARM CPU

Consulted on 20 November 2026. Verification grades as in the [index](README.md).

## Core extensions in AArch32

| Core | Integer division, VRINT, CRC32 | Cryptography (AES, PMULL, SHA) | Source |
|---|---|---|---|
| Cortex-A73 | Yes: `ID_ISAR0` "SDIV and UDIV in the A32 instruction set", `MVFR2.FPMisc` 0x4 (VRINT, conversions with directed rounding), `ID_ISAR5` CRC32 0x1 | Optional: "Can be included or not included" | [TRM 100048](https://documentation-service.arm.com/static/5e7b6c837158f500bd5c03fc), official |
| Cortex-A53 | Yes, CRC32 fixed at 0x1; SIMD and FP are also a licensing option | Optional: "not included in the base product" | [TRM Cryptography DDI 0501F](https://documentation-service.arm.com/static/5e9077ee8259fe2368e2acf7), official |
| Cortex-A55 | `FPMisc` 0x4; SIMD and FP optional | Optional | [TRM SIMD and FP](https://documentation-service.arm.com/static/649ac6f938511951cb799083), official |
| Cortex-A76 | Like the other v8 cores (inference) | Optional: "not included in the base product" | [TRM Cryptographic Extension](https://documentation-service.arm.com/static/64a2964c38511951cb79a375), official |

Integer division, VRINT and CRC32 can be assumed present on these cores.
Cryptography cannot, even though every TV seen so far has it.

## Linux HWCAP for 32-bit processes

- [`arch/arm/include/uapi/asm/hwcap.h`](https://github.com/torvalds/linux/blob/master/arch/arm/include/uapi/asm/hwcap.h),
  official: `HWCAP_NEON` 1<<12, `HWCAP_VFPv4` 1<<16, `HWCAP_IDIVA` 1<<17;
  `HWCAP2_AES` 1<<0, `HWCAP2_PMULL` 1<<1, `HWCAP2_SHA1` 1<<2, `HWCAP2_SHA2`
  1<<3, `HWCAP2_CRC32` 1<<4.
- A 32-bit arm kernel derives `HWCAP2_CRC32` from `ID_ISAR5` and `HWCAP_IDIVA`
  from `ID_ISAR0` (`arch/arm/kernel/setup.c`); an arm64 kernel passes the same
  bits to 32-bit processes (`compat_elf_hwcaps` in
  `arch/arm64/kernel/cpufeature.c`), official.
- Recognizing an ARMv8 core takes `HWCAP2_CRC32`: `IDIVA` and `VFPv4` are
  also present on Cortex-A7, A15 and A17, and `AT_PLATFORM` and `uname -m` say
  `v7l` even on the B1's A73. No bit indicates VRINT (inference).

## LLVM and rustc

- In LLVM (`ARMProcessors.td`) `cortex-a53` enables ARMv8-A, ARM and Thumb
  integer division, CRC and cryptography; `cortex-a73` the same. `armv8-a`
  alone does not enable integer division, and `hwdiv-arm` is not among the
  features rustc accepts, official.
- rustc's `armv7-unknown-linux-gnueabi` target sets
  `+v7,+thumb2,+soft-float,-neon`; removing `soft-float` on non-`hf` targets
  enables VFP while keeping the soft-float calling convention, official.
- An ARMv8 baseline without cryptography would require
  `-C target-cpu=cortex-a53 -C target-feature=-soft-float,+neon,-aes,-sha2`,
  to be verified with `rustc --print cfg` (inference).
- `-Z tune-cpu` is unstable, so nightly only, and the documentation says it
  is effective only on x86, official.

## Checks on the B1

- `/proc/cpuinfo`: MediaTek MT5889, `CPU part 0xd09` (Cortex-A73), Features
  `half thumb fastmult vfp edsp neon vfpv3 tls vfpv4 idiva idivt vfpd32 lpae
  aes pmull sha1 sha2 crc32`; `uname -m` `armv7l`, kernel 4.4.84,
  `LONG_BIT` 32, no aarch64 loader.
- A `cortex-a73` binary runs without SIGILL. Compared with the `cortex-a9`
  binary it divides with `sdiv` and `udiv` and rounds with `vrint` instead of
  `roundf`. But over two alternating runs, on the lab's 1080p live stream
  with the 5,000-channel list, CPU stays the same: 16–19% of a core while
  scrolling, 10–11% with the live stream alone.
- Cost per operation on the B1, with a benchmark built both ways
  ([opbench.py](../../dev/tv/opbench.py)) (two
  runs each; each measurement includes about 35 ns of overhead, the
  conversion to `u64` included): `f32::round` 69–73 ns with `cortex-a9` (call
  to `roundf` in software) and 32–33 ns with `cortex-a73` (`vrint`); `u32`
  division with a variable divisor 119 ns against 12 ns; `f64::from_str` of
  an `EXTINF` duration about 260 ns in both, because it comes from the
  precompiled standard library. In the `cortex-a9` binary the app's functions
  call 32-bit software division at 2 sites; another 192 sites are in the
  standard library and the dependencies. 64-bit divisions
  (`__aeabi_ldivmod`, `__aeabi_uldivmod`) stay in software even on ARMv8.
- Sampling profile ([profile.py](../../dev/tv/profile.py): `cpu-clock`
  event at ~1 kHz on every thread of the process; one run per binary, the
  lab's 1080p live stream encrypted with AES-128 over HTTPS): while scrolling the list the app uses 0.21 cores with
  `cortex-a9` and 0.18 with `cortex-a73`, with the live stream alone 0.12 in
  both. The software float routines (`roundf` and its `__addsf3`) account for
  0.49% of samples while scrolling and 0.06% with the live stream alone, that
  is about 0.1% of a core; software divisions less than 0.1% of samples, in
  both binaries. The rest goes to the kernel, the pipeline's GStreamer and
  GLib (in the app's process), the Mali driver, segment decryption and TLS.
- Profile of Rai Scuola at 1080p (5.6 Mbit/s over HTTPS, unencrypted
  segments), 20 s: 0.18 cores; the `hls` thread has 37% of samples, half of
  them in the kernel and a third in TLS (`_bsaes_encrypt8`, `gcm_ghash_neon`),
  about 13% of the app's CPU; software floats and divisions 0.1% each.
- ring 0.17 on 32-bit ARM detects only NEON: it uses hardware AES, PMULL and
  SHA only on aarch64 ([`src/cpu/arm.rs`](https://github.com/briansmith/ring/blob/0.17.14/src/cpu/arm.rs),
  [`src/cpu/arm/linux.rs`](https://github.com/briansmith/ring/blob/0.17.14/src/cpu/arm/linux.rs)),
  official; on the B1 TLS therefore encrypts with AES on NEON
  (`_bsaes_encrypt8`, `gcm_ghash_neon`).
- The package check ([floats.py](../../tools/webos/floats.py)) shows that
  the precompiled standard library does `f64::from_str`, `roundf` and `round`
  in software; with `cortex-a73` only the `f64::from_str` calls remain.
