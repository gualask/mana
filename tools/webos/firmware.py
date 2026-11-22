#!/usr/bin/env python3
"""Check that what the app takes from the TV's system exists on every webOS
from 6 on, the oldest supported.

The app opens SDL and Starfish with dlopen and takes their functions by
name (apps/tv/src/platform), so neither the linker nor tools/webos/elf.py
sees them. webosbrew's firmware database (dev-toolbox-cli, common/data)
lists for each firmware the symbols its libraries export. For every
library the binary names as a soname (`libSDL2-2.0.so.0`,
`libplayerAPIs.so.1`), the check takes the C strings of the binary that the
webOS 6 firmware exports from that library or from its direct dependencies,
which dlsym searches too (`_ZdlPv` comes from libstdc++), and fails when a
newer firmware lacks one of them. Strings that only happen to be symbols
(`open`, `main`) count as well: harmless while every firmware keeps them.

GL functions come through SDL_GL_GetProcAddress and glow keeps the names of
all of them, called or not: they are left to the notes in
docs/sources/tv-lg.md. The libraries the binary links (DT_NEEDED) are
package.py's: glibc's and the bundled FFmpeg.

The database files are downloaded at a pinned commit into private/firmware
on first use.

Usage: python3 tools/webos/firmware.py [BINARY]
"""
import json
import re
import sys
import urllib.request
from pathlib import Path

import elf
from target import APP_BINARY, ROOT

COMMIT = '2d5a7b6881850d825aa7fdc6c0b5e53c91dc40d3'
URL = f'https://raw.githubusercontent.com/webosbrew/dev-toolbox-cli/{COMMIT}/common/data'
CACHE = ROOT / 'private' / 'firmware' / COMMIT[:12]
# The database's firmwares with webOS 6 or later, by folder and webOS
# release: 6 is webOS 6.0 (2021), 7 webOS 22, and so on to 11, webOS 26.
# The first is the reference: the B1 has release 6.5.3.
FIRMWARES = {
    '03.40.82.01-HE_DTV_W21P_AFADATAA': '6.4.0',
    '04.40.90.01-HE_DTV_W22O_AFABATAA': '7.4.0',
    '03.30.60.01-HE_DTV_W23O_AFABATAA': '8.3.0',
    '23.20.50.01-HE_DTV_W23O_AFABATAA': '9.2.0',
    '33.20.40.01-HE_DTV_W22H_AFABATPU': '10.2.0',
    '43.21.70.01-HE_DTV_W23O_AFABJAAA': '11.2.0',
}
SONAME = re.compile(rb'lib[\w+-][\w.+-]*\.so\.\d+')
# A C string that may be a symbol: the names passed to dlsym end in NUL.
C_NAME = re.compile(rb'[A-Za-z_]\w+(?=\0)')


class Firmware:
    def __init__(self, folder, release):
        self.folder, self.release = folder, release
        # Every name of a library, soname included, to its file.
        self.index = json.loads(fetch(folder, 'index.json'))

    def has(self, soname):
        return soname in self.index

    def exports(self, soname):
        """The symbols of `soname` and of the libraries it needs, without
        their versions (`glAttachShader@libmali.so.0`)."""
        library = self.library(soname)
        symbols = set(library['symbols'])
        for needed in library['needed']:
            if self.has(needed):
                symbols.update(self.library(needed)['symbols'])
        return {symbol.split('@')[0] for symbol in symbols}

    def library(self, soname):
        return json.loads(fetch(self.folder, self.index[soname]))


def fetch(folder, name):
    """A file of the database, from the cache or downloaded into it."""
    path = CACHE / folder / name
    if not path.is_file():
        with urllib.request.urlopen(f'{URL}/{folder}/{name}') as response:
            data = response.read()
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    return path.read_bytes()


def check(binary):
    """Raises SystemExit when a webOS 6 or later firmware lacks a library the
    ELF in `binary` opens or a name it takes from one; returns a one-line
    summary."""
    rodata = elf.section(binary, '.rodata')
    names = {match.decode() for match in C_NAME.findall(rodata)}
    firmwares = [Firmware(folder, release) for folder, release in FIRMWARES.items()]
    reference = firmwares[0]
    problems, found = [], []
    for soname in sorted({match.decode() for match in SONAME.findall(rodata)}):
        if not reference.has(soname):
            problems.append(f'{soname} is not in webOS {reference.release}')
            continue
        wanted = names & reference.exports(soname)
        found.append(f'{soname} {len(wanted)} names')
        for firmware in firmwares[1:]:
            missing = wanted - firmware.exports(soname) if firmware.has(soname) else {'the library'}
            if missing:
                problems.append(f'webOS {firmware.release} lacks {", ".join(sorted(missing))} of {soname}')
    if problems:
        raise SystemExit('firmware: ' + '; '.join(problems))
    return f'firmware: {", ".join(found)} in webOS {reference.release}-{firmwares[-1].release}'


if __name__ == '__main__':
    print(check(Path(sys.argv[1] if len(sys.argv) > 1 else APP_BINARY).read_bytes()))
