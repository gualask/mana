#!/usr/bin/env python3
"""Measure on the TV the cost per operation of the app's CPU flags.

Builds dev/tv/opbench twice, with the flags of .cargo/config.toml and for
ARMv8 (cortex-a73, the B1's core), runs both on the TV in turn and prints
nanoseconds per operation. Results recorded in docs/sources/cpu-arm.md.

Usage: python3 dev/tv/opbench.py [--rounds 2]
TV access is configured as described in tools/webos/tv.py.
"""
import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# The TV access and the target shared with the build tools (tools/webos).
sys.path.insert(0, str(ROOT / 'tools' / 'webos'))

from target import zigbuild
from tv import Tv

BENCH = ROOT / 'dev' / 'tv' / 'opbench' / 'Cargo.toml'
TOOLS_TARGET = ROOT / 'target' / 'dev-tools'
# None keeps the flags of .cargo/config.toml.
BUILDS = {
    'base': None,
    'armv8': '-C target-cpu=cortex-a73 -C target-feature=+neon,-soft-float',
}


def main():
    parser = argparse.ArgumentParser(description='Measure the cost per operation of the CPU flags on the TV.')
    parser.add_argument('--rounds', type=int, default=2)
    args = parser.parse_args()
    tv = Tv()
    for name, rustflags in BUILDS.items():
        upload = tv.run(f'cat > /tmp/mana-opbench-{name} && chmod +x /tmp/mana-opbench-{name}',
                        stdin=zigbuild(BENCH, TOOLS_TARGET / f'opbench-{name}', rustflags).read_bytes())
        if upload.returncode:
            raise SystemExit('upload failed: ' + upload.stderr.decode(errors='replace'))
    for round_number in range(1, args.rounds + 1):
        for name in BUILDS:
            print(f'== {name}, giro {round_number}')
            print(tv.run(f'/tmp/mana-opbench-{name}').stdout.decode(), end='')
    tv.run('rm -f ' + ' '.join(f'/tmp/mana-opbench-{name}' for name in BUILDS))


if __name__ == '__main__':
    main()
