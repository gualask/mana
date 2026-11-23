#!/usr/bin/env python3
"""Profile the app on the TV by sampling: where its threads spend the CPU.

Builds the sampler in dev/tv/profile for the TV, runs it on the app's
process, then resolves the samples: kernel or library for code outside the
app, the function for the app's own code, read from the binary that still
has its symbols (the one tools/webos/package.py checks and strips). Prints
the share of samples by library, by thread and by function, the software
float and division routines, and the functions of the busiest threads.

The app must be running the build of that binary. Scenario of the
measurements recorded in docs/sources/cpu-arm.md: the lab's live channel at
1080p (`quality = 1080` in /tmp/mana.conf, `lab-live.stream` in
/tmp/mana-autoplay) and, to scroll the 5,000 test channels, `ui_test = true`
without /tmp/mana-channels.m3u.

Usage: python3 dev/tv/profile.py [--seconds 30] [--save FILE] [--top 25]
       python3 dev/tv/profile.py --samples FILE   # samples saved earlier
TV access is configured as described in tools/webos/tv.py.
"""
import argparse
import bisect
import collections
import dataclasses
import re
import struct
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# The TV access and the target shared with the build tools (tools/webos).
sys.path.insert(0, str(ROOT / 'tools' / 'webos'))

from floats import SOFT_FLOAT, SOFT_ROUNDING
from target import APP_BINARY, APP_NAME, zigbuild
from tv import Tv

SAMPLER = ROOT / 'dev' / 'tv' / 'profile' / 'Cargo.toml'
TOOLS_TARGET = ROOT / 'target' / 'dev-tools'
REMOTE_SAMPLER = '/tmp/mana-sampler'
SOFT_DIVISION = re.compile(r'__(?:u?divsi3|u?divmod[sd]i4|aeabi_u?idiv\w*|aeabi_u?ldivmod)')


def sample(seconds):
    tv = Tv()
    found = tv.run(f'pidof {APP_NAME}')
    if found.returncode not in (0, 1):  # pidof gives 1 when nothing matches
        raise SystemExit('cannot reach the TV: ' + found.stderr.decode(errors='replace').strip())
    pid = found.stdout.decode().split()
    if len(pid) != 1:
        raise SystemExit(f'the app is not running on the TV (pidof {APP_NAME}: {pid})')
    sampler = zigbuild(SAMPLER, TOOLS_TARGET).read_bytes()
    upload = tv.run(f'cat > {REMOTE_SAMPLER} && chmod +x {REMOTE_SAMPLER}', stdin=sampler)
    if upload.returncode:
        raise SystemExit('upload failed: ' + upload.stderr.decode(errors='replace'))
    print(f'sampling process {pid[0]} for {seconds} s…', file=sys.stderr)
    result = tv.run(f'{REMOTE_SAMPLER} {pid[0]} {seconds}; rm -f {REMOTE_SAMPLER}')
    if result.returncode:
        raise SystemExit('sampler failed: ' + result.stderr.decode(errors='replace'))
    return result.stdout.decode()


class Symbols:
    """The functions of an ELF binary with its symbol table, by address."""

    def __init__(self, path):
        data = path.read_bytes()
        offset, = struct.unpack_from('<I', data, 28)
        entry_size, count = struct.unpack_from('<HH', data, 42)
        headers = (struct.unpack_from('<8I', data, offset + i * entry_size) for i in range(count))
        self.loads = [(h[1], h[2], h[4]) for h in headers if h[0] == 1]  # PT_LOAD: offset, vaddr, filesz
        table = subprocess.run(['objdump', '-t', '-C', str(path)], capture_output=True, text=True).stdout
        functions = []
        for line in table.splitlines():
            match = re.match(r'^([0-9a-f]{8}) (.{7}) (\S+)\s+([0-9a-f]{8}) (.+)$', line)
            if match and 'F' in match[2]:
                functions.append((int(match[1], 16), int(match[4], 16), match[5].replace('.hidden ', '').strip()))
        if not functions:
            raise SystemExit(f'{path} has no symbols: profile the build cargo leaves in target/')
        functions.sort()
        self.functions, self.starts = functions, [f[0] for f in functions]

    def name(self, file_offset):
        vaddr = next((file_offset - o + v for o, v, size in self.loads if o <= file_offset < o + size), None)
        i = bisect.bisect_right(self.starts, vaddr) - 1 if vaddr is not None else -1
        if i < 0 or vaddr >= self.starts[i] + max(self.functions[i][1], 1):
            return f'[{APP_NAME} ?]'
        return self.functions[i][2]


@dataclasses.dataclass
class Run:
    """What the sampler prints: samples as (TID, MODE, IP), thread names,
    mappings as (start, end, file offset, file name) and its counters."""
    samples: list = dataclasses.field(default_factory=list)
    threads: dict = dataclasses.field(default_factory=dict)
    maps: list = dataclasses.field(default_factory=list)
    lost: int = 0
    frequency: int = 0
    seconds: int = 0


def parse(text):
    run = Run()
    for line in text.splitlines():
        kind, _, rest = line.partition(' ')
        if kind.isdigit():
            mode, ip = rest.split()
            run.samples.append((kind, mode, int(ip, 16)))
        elif kind == 'thread':
            tid, _, name = rest.partition(' ')
            run.threads[tid] = name
        elif kind == 'map':
            fields = rest.split()
            start, end = (int(x, 16) for x in fields[0].split('-'))
            run.maps.append((start, end, int(fields[2], 16), fields[5].rsplit('/', 1)[-1] if len(fields) > 5 else '[anon]'))
        elif kind in ('lost', 'frequency', 'seconds'):
            setattr(run, kind, int(rest))
    if not run.samples:
        raise SystemExit('no samples')
    return run


def locate(mode, ip, maps, symbols):
    """Where a sample ran: `[kernel]`, `[library]`, or the app's function."""
    if mode == '1':
        return '[kernel]'
    mapping = next((m for m in maps if m[0] <= ip < m[1]), None)
    if not mapping:
        return '[?]'
    start, _, offset, name = mapping
    return symbols.name(ip - start + offset) if name == APP_NAME else f'[{name}]'


@dataclasses.dataclass
class Tally:
    """Samples counted by library, thread and function, and the software
    float and division routines among the functions."""
    by_module: collections.Counter = dataclasses.field(default_factory=collections.Counter)
    by_thread: collections.Counter = dataclasses.field(default_factory=collections.Counter)
    by_function: collections.Counter = dataclasses.field(default_factory=collections.Counter)
    soft_float: collections.Counter = dataclasses.field(default_factory=collections.Counter)
    soft_division: collections.Counter = dataclasses.field(default_factory=collections.Counter)
    per_thread: collections.defaultdict = dataclasses.field(default_factory=lambda: collections.defaultdict(collections.Counter))


def tally(run, symbols):
    counts = Tally()
    for tid, mode, ip in run.samples:
        thread = run.threads.get(tid, tid)
        where = locate(mode, ip, run.maps, symbols)
        leaf = where.rsplit('::', 1)[-1]
        counts.soft_float[where] += bool(SOFT_FLOAT.fullmatch(leaf) or SOFT_ROUNDING.fullmatch(leaf))
        counts.soft_division[where] += bool(SOFT_DIVISION.fullmatch(leaf))
        counts.by_module[where if where.startswith('[') else APP_NAME] += 1
        counts.by_thread[thread] += 1
        counts.by_function[where] += 1
        counts.per_thread[thread][where] += 1
    return counts


def report(run, counts, top):
    total = len(run.samples)
    share = lambda n, of=total: f'{100 * n / of:5.1f}%'
    # Samples saved before the sampler printed its frequency give no cores.
    cores = (f', {total / (run.frequency * run.seconds):.2f} cores on average over {run.seconds} s'
             if run.frequency and run.seconds else '')
    print(f'samples {total}, lost {run.lost}{cores}')
    sections = [('by library', counts.by_module, 12), ('by thread', counts.by_thread, 12),
                ('by function', counts.by_function, top)]
    for title, counter, count in sections:
        print(f'\n# {title}')
        for name, n in counter.most_common(count):
            print(f'{share(n)}  {short(name)}')
    for title, counter in (('float software', counts.soft_float), ('software divisions', counts.soft_division)):
        counter = +counter
        print(f'\n# {title}: {share(sum(counter.values()))}')
        for name, n in counter.most_common(8):
            print(f'{share(n)}  {short(name)}')
    for thread, n in counts.by_thread.most_common(3):
        print(f'\n# thread {thread} ({share(n)} of the samples), broken down')
        for name, m in counts.per_thread[thread].most_common(8):
            print(f'{share(m, n)}  {short(name)}')


def short(name):
    name = re.sub(r'::\{closure#\d+\}', '::{closure}', name)
    return name if len(name) < 100 else name[:97] + '…'


def main():
    parser = argparse.ArgumentParser(description='Profile the app on the TV by sampling.')
    parser.add_argument('--seconds', type=int, default=30)
    parser.add_argument('--samples', type=Path, help='resolve samples saved earlier instead of sampling')
    parser.add_argument('--save', type=Path, help='also write the raw samples here')
    parser.add_argument('--binary', type=Path, default=APP_BINARY, help='the running build, with symbols')
    parser.add_argument('--top', type=int, default=25)
    args = parser.parse_args()
    text = args.samples.read_text() if args.samples else sample(args.seconds)
    if args.save:
        args.save.write_text(text)
    run = parse(text)
    report(run, tally(run, Symbols(args.binary)), args.top)


if __name__ == '__main__':
    main()
