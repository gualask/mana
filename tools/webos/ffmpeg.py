#!/usr/bin/env python3
"""Cross-compile the FFmpeg bundled with the app, for the TV, with zig.

The TV's own FFmpeg 4.0.2 is frozen with the firmware: it loses audio in
fMP4 fragments with several `trun` boxes, aborts when its stream buffer is
resized and does not analyse streams. This builds a pinned LGPL release as
shared libraries with only what the shim uses, for the soft-float ABI and
glibc 2.28 of the TV, into private/ffmpeg/prefix, which the app's build uses
by default. The prefix also gets the LGPL text and a notice of the build,
which tools/webos/package.py ships in the package.

Usage: python3 tools/webos/ffmpeg.py [--jobs N]
"""
import argparse
import hashlib
import os
import shutil
import subprocess
import tarfile
import urllib.request
from pathlib import Path

from target import check_tools
from tv import ROOT

VERSION = '9.0.2'
SHA256 = '8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e'
URL = f'https://ffmpeg.org/releases/ffmpeg-{VERSION}.tar.xz'
WORK = ROOT / 'private' / 'ffmpeg'
PREFIX = WORK / 'prefix'
# As the Rust target: cortex-a9 with NEON, softfp calling convention.
CC_FLAGS = '-target arm-linux-gnueabi.2.28 -mcpu=cortex_a9 -mfpu=neon -mfloat-abi=softfp'
COMPONENTS = [
    # Live TV containers; `file` for local files.
    '--enable-demuxer=mpegts,mov',
    # MPEG-TS identifies some streams by probing their contents with these:
    # without `aac`, opening Canale 5's audio read on until the stream ended.
    '--enable-demuxer=aac,ac3,eac3,mp3,h264,hevc',
    '--enable-protocol=file',
    # Stream analysis fills the codec parameters from these.
    '--enable-parser=h264,hevc,aac,ac3,mpegaudio',
    '--enable-decoder=aac,ac3,eac3',
    # MPEG audio, which the TV refuses, is transcoded to AAC.
    '--enable-decoder=mp2,mp2float,mp3,mp3float',
    '--enable-encoder=aac',
    # Length-prefixed MP4 video to the Annex B the TV takes.
    '--enable-bsf=h264_mp4toannexb,hevc_mp4toannexb',
]


def tarball():
    path = WORK / 'download' / f'ffmpeg-{VERSION}.tar.xz'
    if not path.is_file():
        path.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(URL) as response:
            path.write_bytes(response.read())
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    if digest != SHA256:
        raise SystemExit(f'{path}: sha256 {digest}, expected {SHA256}')
    return path


def wrappers(directory):
    """zig subcommands as single executables, as configure expects."""
    directory.mkdir(parents=True, exist_ok=True)
    tools = {'cc': f'zig cc {CC_FLAGS}', 'ar': 'zig ar', 'ranlib': 'zig ranlib'}
    for name, command in tools.items():
        script = directory / f'zig-{name}'
        script.write_text(f'#!/bin/sh\nexec {command} "$@"\n')
        script.chmod(0o755)
    return {name: directory / f'zig-{name}' for name in tools}


def main():
    parser = argparse.ArgumentParser(description='Cross-compile the bundled FFmpeg for the TV.')
    parser.add_argument('--jobs', type=int, default=os.cpu_count())
    args = parser.parse_args()
    print(check_tools())
    source = WORK / f'ffmpeg-{VERSION}'
    if not source.is_dir():
        with tarfile.open(tarball()) as archive:
            archive.extractall(WORK, filter='data')
    tools = wrappers(WORK / 'bin')
    shutil.rmtree(PREFIX, ignore_errors=True)
    configure = [
        './configure', f'--prefix={PREFIX}',
        # The CPU is in CC_FLAGS: configure's --cpu spells it as zig does not.
        '--enable-cross-compile', '--arch=arm', '--target-os=linux',
        f'--cc={tools["cc"]}', f'--ar={tools["ar"]}', f'--ranlib={tools["ranlib"]}', '--nm=nm',
        # Stripped by the linker (zig objcopy cannot strip 32-bit ARM);
        # exported symbols stay in .dynsym.
        '--disable-stripping', '--extra-ldsoflags=-Wl,--strip-all',
        # LGPL only: no --enable-gpl, no --enable-nonfree.
        '--enable-shared', '--disable-static', '--enable-pic',
        '--disable-programs', '--disable-doc', '--disable-avdevice', '--disable-avfilter',
        '--disable-swscale', '--disable-swresample', '--disable-network', '--disable-debug',
        '--disable-everything', '--disable-autodetect',
        *COMPONENTS,
    ]
    subprocess.run(configure, cwd=source, check=True)
    # glibc 2.28 still exports sysctl(), but zig's headers come from a newer
    # glibc without sys/sysctl.h; FFmpeg has a Linux fallback.
    config = source / 'config.h'
    config.write_text(config.read_text().replace('#define HAVE_SYSCTL 1', '#define HAVE_SYSCTL 0'))
    subprocess.run(['make', f'-j{args.jobs}'], cwd=source, check=True)
    subprocess.run(['make', 'install'], cwd=source, check=True)
    (PREFIX / 'COPYING.LGPLv2.1').write_bytes((source / 'COPYING.LGPLv2.1').read_bytes())
    (PREFIX / 'NOTICE').write_text(notice(configure[1:]))
    for library in sorted((PREFIX / 'lib').glob('lib*.so.*')):
        if not library.is_symlink():
            print(f'{library.name}: {library.stat().st_size // 1024} KB')


def notice(options):
    """What the LGPL asks to accompany the libraries: what they are, where
    their source is and how they were built, so they can be rebuilt."""
    flags = '\n'.join(f'  {o}' for o in options if not o.startswith(('--prefix=', '--cc=', '--ar=', '--ranlib=')))
    return (f'FFmpeg {VERSION} (libavformat, libavcodec, libavutil)\n'
            f'Licence: GNU LGPL 2.1 or later, see COPYING.LGPLv2.1.\n'
            f'Source: {URL}\nSHA-256: {SHA256}\n'
            f'The same source archive is distributed beside the app package.\n\n'
            f'Unmodified; built with tools/webos/ffmpeg.py of Mana, zig cc\n'
            f'({CC_FLAGS}), HAVE_SYSCTL set to 0 in config.h, configured with:\n{flags}\n\n'
            f'The libraries are shared objects in the app\'s lib/ folder and can be\n'
            f'replaced with a compatible build of the same major versions.\n')


if __name__ == '__main__':
    main()
