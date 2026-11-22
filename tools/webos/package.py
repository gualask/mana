#!/usr/bin/env python3
"""Build the webOS .ipk for the native app from the cross-compiled binary.

Usage: python3 tools/webos/package.py [--binary PATH] [--out DIR] [--ffmpeg PREFIX]

zig and cargo-zigbuild must be the versions the build expects
(tools/webos/target.py). The binary must use VFP for its maths (tools/webos/floats.py, which reads
its symbols); it goes into the package stripped. Every library it and the
bundled ones need must be in the package or in the firmware, and none may
need a glibc newer than webOS 6's (tools/webos/elf.py reads both). What the
app opens at run time must be in every firmware from webOS 6 on, the oldest
supported, which the package's description names (tools/webos/firmware.py).

The binary links the FFmpeg bundled with the app (built by
tools/webos/ffmpeg.py into PREFIX, private/ffmpeg/prefix by default): its
libraries go into the app's lib/ folder under their sonames, with the LGPL
text and the build notice in licenses/ffmpeg/, and the corresponding source
archive is copied beside the package.

The layout mirrors packages produced by ares-package: a GNU ar archive with
debian-binary, control.tar.gz and data.tar.gz, where data holds
usr/palm/applications/<id>/ and usr/palm/packages/<id>/packageinfo.json.
"""
import argparse
import io
import json
import os
import re
import shutil
import struct
import subprocess
import tarfile
import tempfile
import time
import tomllib
import zlib
from pathlib import Path

import elf
import firmware
import floats
from target import APP_BINARY, APP_NAME, GLIBC, check_tools

ROOT = Path(__file__).resolve().parents[2]
APP_ID = 'dev.mana.app'
APP_DIR = f'usr/palm/applications/{APP_ID}'
TITLE = 'Mana'
# webOS 6 is the oldest supported: tools/webos/firmware.py checks it and
# what came after.
DESCRIPTION = f'{TITLE}, per webOS 6 o successivo'
VENDOR = 'Mana'
# Libraries of the firmware the app and FFmpeg may link: glibc's, in every
# webOS. What the app opens at run time is checked by tools/webos/firmware.py.
FIRMWARE_LIBRARIES = {'libc.so.6', 'libm.so.6', 'libpthread.so.0', 'libdl.so.2', 'librt.so.1', 'libgcc_s.so.1'}
ICON_BACKGROUND = (0x10, 0x13, 0x1a)
ICON_ACCENT = (0x2d, 0x6c, 0xdf)


def png(size):
    """A play symbol on a dark square, generated so no artwork is bundled."""
    rows = []
    for y in range(size):
        row = bytearray([0])
        for x in range(size):
            # Triangle pointing right, centred, with height half the icon.
            u, v = (x + 0.5) / size, (y + 0.5) / size
            inside = 0.36 <= u <= 0.70 and abs(v - 0.5) <= (0.70 - u) * 0.75
            row += bytes(ICON_ACCENT if inside else ICON_BACKGROUND)
        rows.append(bytes(row))

    def chunk(kind, data):
        body = kind + data
        return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body))

    header = struct.pack('>IIBBBBB', size, size, 8, 2, 0, 0, 0)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', header)
            + chunk(b'IDAT', zlib.compress(b''.join(rows), 9)) + chunk(b'IEND', b''))


def tar_gz(files, mtime):
    """files: list of (path, bytes, mode); directories are added for each parent."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode='w:gz', format=tarfile.GNU_FORMAT) as tar:
        seen = set()
        for path, data, mode in files:
            add_directories(tar, path, seen, mtime)
            info = tarfile.TarInfo(path)
            info.size, info.mode, info.mtime = len(data), mode, mtime
            tar.addfile(info, io.BytesIO(data))
    return buffer.getvalue()


def add_directories(tar, path, seen, mtime):
    """The parent directories of `path` not in `seen` yet, added to `tar`."""
    parts = path.split('/')
    for depth in range(1, len(parts)):
        directory = '/'.join(parts[:depth]) + '/'
        if directory in seen:
            continue
        seen.add(directory)
        info = tarfile.TarInfo(directory)
        info.type, info.mode, info.mtime = tarfile.DIRTYPE, 0o755, mtime
        tar.addfile(info)


def ar(members, mtime):
    """GNU-style ar archive; member names are shorter than 16 characters."""
    out = bytearray(b'!<arch>\n')
    for name, data in members:
        out += f'{name:<16}{mtime:<12}{0:<6}{0:<6}{0o100644:<8o}{len(data):<10}`\n'.encode()
        out += data
        if len(data) % 2:
            out += b'\n'
    return bytes(out)


def main():
    version = tomllib.loads((ROOT / 'apps' / 'tv' / 'Cargo.toml').read_text())['package']['version']
    args = arguments()
    print(check_tools())
    check_arm(args.binary)
    print(floats.check(args.binary))
    binary = stripped(args.binary)
    prefix = ROOT / args.ffmpeg
    source = ffmpeg_source(binary, prefix)
    libraries = ffmpeg_libraries(prefix)
    print(check_linking(binary, libraries))
    print(firmware.check(binary))
    data_files = app_files(binary, version) + ffmpeg_files(prefix, libraries)
    args.out.mkdir(parents=True, exist_ok=True)
    target = args.out / f'{APP_ID}_{version}_arm.ipk'
    target.write_bytes(ipk(data_files, version))
    print(target)
    shutil.copyfile(source, args.out / source.name)
    print(args.out / source.name)


def arguments():
    parser = argparse.ArgumentParser(description='Package the webOS native app as .ipk.')
    parser.add_argument('--binary', type=Path, default=APP_BINARY)
    parser.add_argument('--out', type=Path, default=ROOT / 'dist')
    parser.add_argument('--ffmpeg', type=Path, default=ROOT / 'private' / 'ffmpeg' / 'prefix')
    return parser.parse_args()


def check_arm(binary):
    """Raises SystemExit unless `binary` is a 32-bit ARM ELF."""
    header = binary.read_bytes()[:20]
    if header[:4] != b'\x7fELF' or header[4] != 1 or struct.unpack_from('<H', header, 18)[0] != 40:
        raise SystemExit(f'{binary} is not a 32-bit ARM ELF')


def app_files(binary, version):
    """The app's folder and package info, as (path, bytes, mode)."""
    appinfo = {
        'id': APP_ID, 'version': version, 'vendor': VENDOR, 'type': 'native',
        'main': APP_NAME, 'title': TITLE, 'icon': 'icon.png', 'largeIcon': 'largeIcon.png',
        'iconColor': '#10131a',
    }
    packageinfo = {'app': APP_ID, 'id': APP_ID, 'loc_name': TITLE, 'package_format_version': 2,
                   'vendor': VENDOR, 'version': version}
    return [
        (f'{APP_DIR}/appinfo.json', json.dumps(appinfo, indent=2).encode(), 0o644),
        (f'{APP_DIR}/{APP_NAME}', binary, 0o755),
        (f'{APP_DIR}/icon.png', png(80), 0o644),
        (f'{APP_DIR}/largeIcon.png', png(130), 0o644),
        (f'usr/palm/packages/{APP_ID}/packageinfo.json', json.dumps(packageinfo, indent=2).encode(), 0o644),
    ]


def ffmpeg_libraries(prefix):
    """The FFmpeg libraries in `prefix`, under their sonames."""
    return [p for p in sorted((prefix / 'lib').glob('lib*.so.*')) if re.fullmatch(r'lib\w+\.so\.\d+', p.name)]


def ffmpeg_files(prefix, libraries):
    """The bundled FFmpeg `libraries`, and its notices."""
    return ([(f'{APP_DIR}/lib/{p.name}', p.read_bytes(), 0o644) for p in libraries]
            + [(f'{APP_DIR}/licenses/ffmpeg/{name}', (prefix / name).read_bytes(), 0o644)
               for name in ('NOTICE', 'COPYING.LGPLv2.1')])


def ipk(data_files, version):
    """The package: control file and data, with the installed size."""
    mtime = int(time.time())
    installed_size = sum(len(data) for _, data, _ in data_files)
    control = (f'Package: {APP_ID}\nVersion: {version}\nSection: misc\nPriority: optional\n'
               f'Architecture: arm\nInstalled-Size: {installed_size}\nMaintainer: N/A <nobody@example.com>\n'
               f'Description: {DESCRIPTION}\nwebOS-Package-Format-Version: 2\nwebOS-Packager-Version: x.y.x\n')
    return ar([
        ('debian-binary', b'2.0\n'),
        ('control.tar.gz', tar_gz([('control', control.encode(), 0o644)], mtime)),
        ('data.tar.gz', tar_gz(data_files, mtime)),
    ], mtime)


def stripped(binary):
    """The bytes of `binary` without its symbol table, which cargo leaves
    for the float check. Uses the llvm-objcopy that comes with rustc, which
    finds its LLVM library in the sysroot (zig objcopy cannot strip it)."""
    sysroot = Path(rustc('--print', 'sysroot'))
    host = next(line.split()[1] for line in rustc('-vV').splitlines() if line.startswith('host:'))
    objcopy = sysroot / 'lib' / 'rustlib' / host / 'bin' / 'rust-objcopy'
    environment = {**os.environ, 'DYLD_FALLBACK_LIBRARY_PATH': str(sysroot / 'lib'),
                   'LD_LIBRARY_PATH': str(sysroot / 'lib')}
    with tempfile.TemporaryDirectory() as folder:
        output = Path(folder) / binary.name
        result = subprocess.run([str(objcopy), '--strip-all', str(binary), str(output)],
                                capture_output=True, text=True, env=environment)
        if result.returncode:
            raise SystemExit(f'rust-objcopy failed: {result.stderr.strip()}')
        return output.read_bytes()


def rustc(*args):
    return subprocess.run(['rustc', *args], cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()


def check_linking(binary, libraries):
    """Raises SystemExit unless every library the binary and the bundled
    `libraries` need is among them or FIRMWARE_LIBRARIES, and none needs a
    glibc newer than webOS 6's (target.GLIBC); returns a one-line summary."""
    elves = {APP_NAME: elf.read(binary, APP_NAME)} | {p.name: elf.read(p.read_bytes(), p.name) for p in libraries}
    bundled = {dynamic.soname for name, dynamic in elves.items() if name != APP_NAME}
    problems, newest = [], {}
    for name, dynamic in elves.items():
        problems += [f'{name} needs {library}, neither bundled nor in the firmware'
                     for library in dynamic.needed if library not in bundled | FIRMWARE_LIBRARIES]
        newest[name] = elf.glibc_needed(dynamic)
        if newest[name] > GLIBC:
            problems.append(f'{name} needs GLIBC_{".".join(map(str, newest[name]))}, '
                            f'webOS 6 has {".".join(map(str, GLIBC))}')
    if problems:
        raise SystemExit('linking: ' + '; '.join(problems))
    return 'linking: ' + ', '.join(f'{name} GLIBC_{".".join(map(str, glibc)) or "-"}' for name, glibc in newest.items())


def ffmpeg_source(binary, prefix):
    """The source archive of the FFmpeg in `prefix`, which the binary must
    link."""
    needed = [n for n in elf.read(binary, APP_NAME).needed if re.fullmatch(r'libavformat\.so\.\d+', n)]
    if not needed:
        raise SystemExit('the binary does not link libavformat')
    soname = needed[0]
    if not (prefix / 'lib' / soname).exists():
        raise SystemExit(f'{soname}: not in {prefix}/lib; run python3 tools/webos/ffmpeg.py')
    version = (prefix / 'NOTICE').read_text().split()[1]
    archive = ROOT / 'private' / 'ffmpeg' / 'download' / f'ffmpeg-{version}.tar.xz'
    if not archive.is_file():
        raise SystemExit(f'{archive}: missing source archive; run python3 tools/webos/ffmpeg.py')
    return archive


if __name__ == '__main__':
    main()
