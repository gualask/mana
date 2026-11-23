#!/usr/bin/env python3
"""Hand the lab's live channels to the app on the TV.

Reads the fixture manifest written by `python3 -m dev.lab --live` (with --host set
to this machine's local network address, usually with --tls), writes one
`.stream` file per channel with its URL and the media headers, and copies
them, with the lab certificate when present, to the folder the app lists.

Usage: python3 dev/tv/live.py [FIXTURE_DIR]
TV access is configured as described in tools/webos/tv.py.
"""
import argparse
import io
import sys
import tarfile
import tomllib
from pathlib import Path

# The TV access shared with the build tools (tools/webos/tv.py).
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'tools' / 'webos'))

from clips import TV_CLIPS_DIR
from tv import ROOT, Tv

CERTIFICATE_NAME = 'lab-ca.pem'


def main():
    parser = argparse.ArgumentParser(description="Copy the lab's live channels to the TV.")
    parser.add_argument('fixtures', type=Path, nargs='?', default=ROOT / 'private' / 'lab')
    args = parser.parse_args()
    manifest = tomllib.loads((args.fixtures / 'endpoints.toml').read_text())
    # live_hls -> lab-live.stream, live_ts -> lab-live-ts.stream
    channels = {key.removesuffix('_hls').replace('_', '-'): url for key, url in manifest['endpoints'].items()
                if key.startswith('live') and key.endswith(('_hls', '_ts'))}
    if not channels:
        raise SystemExit('no live endpoints: start the lab with --live')
    headers = [f'header = {name}: {value}' for name, value in manifest['media_headers'].items()]
    files = {}
    certificate = manifest.get('tls', {}).get('root_certificate')
    if certificate:
        headers.append(f'ca = {CERTIFICATE_NAME}')
        files[CERTIFICATE_NAME] = Path(certificate).read_bytes()
    for name, url in channels.items():
        files[f'lab-{name}.stream'] = ('\n'.join([f'url = {url}', *headers]) + '\n').encode()
    archive = io.BytesIO()
    with tarfile.open(fileobj=archive, mode='w') as tar:
        for name, data in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
    result = Tv().run(f'mkdir -p {TV_CLIPS_DIR} && tar xf - -C {TV_CLIPS_DIR}', stdin=archive.getvalue())
    if result.returncode != 0:
        raise SystemExit(result.stderr.decode(errors='replace'))
    print(f"{', '.join(sorted(f for f in files if f.endswith('.stream')))} copied to {TV_CLIPS_DIR}")


if __name__ == '__main__':
    main()
