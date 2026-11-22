"""What a 32-bit little-endian ELF asks of the dynamic linker: the libraries
it names (DT_NEEDED), its own soname, and the symbol versions it needs
from each library (.gnu.version_r), as `llvm-readelf --dynamic
--version-info` shows them. Read from the section headers, which
`objcopy --strip-all` keeps.
"""
import functools
import struct
from dataclasses import dataclass, field

SHT_DYNAMIC = 6
SHT_GNU_VERNEED = 0x6ffffffe
DT_NULL, DT_NEEDED, DT_SONAME = 0, 1, 14


@dataclass
class Dynamic:
    soname: str | None = None
    needed: list[str] = field(default_factory=list)
    # Library -> symbol versions needed from it, as GLIBC_2.28.
    versions: dict[str, list[str]] = field(default_factory=dict)


def read(data, name='ELF'):
    """The dynamic linking needs of the ELF in `data`; SystemExit when it is
    not a 32-bit little-endian ELF."""
    if data[:4] != b'\x7fELF' or data[4] != 1 or data[5] != 1:
        raise SystemExit(f'{name}: not a 32-bit little-endian ELF')
    shoff, = struct.unpack_from('<I', data, 0x20)
    shentsize, shnum = struct.unpack_from('<HH', data, 0x2e)
    sections = [struct.unpack_from('<10I', data, shoff + i * shentsize) for i in range(shnum)]

    def string(section, offset):
        start = sections[section][4] + offset
        return data[start:data.index(b'\0', start)].decode()

    dynamic = Dynamic()
    for _, kind, _, _, offset, size, link, info, _, _ in sections:
        strings = functools.partial(string, link)
        if kind == SHT_DYNAMIC:
            read_dynamic(data, offset, size, strings, dynamic)
        elif kind == SHT_GNU_VERNEED:
            read_versions(data, offset, info, strings, dynamic)
    return dynamic


def section(data, name):
    """The bytes of the section called `name` in the ELF in `data`; b'' when
    it has none."""
    shoff, = struct.unpack_from('<I', data, 0x20)
    shentsize, shnum, shstrndx = struct.unpack_from('<HHH', data, 0x2e)
    sections = [struct.unpack_from('<10I', data, shoff + i * shentsize) for i in range(shnum)]
    names = sections[shstrndx][4]
    for name_at, _, _, _, offset, size, *_ in sections:
        if data[names + name_at:data.index(b'\0', names + name_at)].decode() == name:
            return data[offset:offset + size]
    return b''


def read_dynamic(data, offset, size, string, dynamic):
    """DT_NEEDED and DT_SONAME from the dynamic section at `offset`, whose
    string table `string` reads."""
    for at in range(offset, offset + size, 8):
        tag, value = struct.unpack_from('<iI', data, at)
        if tag == DT_NULL:
            break
        if tag == DT_NEEDED:
            dynamic.needed.append(string(value))
        elif tag == DT_SONAME:
            dynamic.soname = string(value)


def read_versions(data, offset, count, string, dynamic):
    """The symbol versions needed from each library, from the `count`
    Verneed entries at `offset`."""
    at = offset
    for _ in range(count):
        _, entries, file, aux, following = struct.unpack_from('<HHIII', data, at)
        dynamic.versions.setdefault(string(file), []).extend(version_names(data, at + aux, entries, string))
        at += following


def version_names(data, entry, count, string):
    """The names of the `count` Vernaux entries from `entry`."""
    names = []
    for _ in range(count):
        _, _, _, version, following = struct.unpack_from('<IHHII', data, entry)
        names.append(string(version))
        entry += following
    return names


def glibc_needed(dynamic):
    """The highest GLIBC_x.y version `dynamic` needs, as a tuple; () when none."""
    versions = [tuple(int(n) for n in v.removeprefix('GLIBC_').split('.'))
                for vs in dynamic.versions.values() for v in vs if v.startswith('GLIBC_2')]
    return max(versions, default=())
