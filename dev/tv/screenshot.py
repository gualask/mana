#!/usr/bin/env python3
"""Capture the TV screen (UI layer) to a local JPEG.

Usage: python3 dev/tv/screenshot.py OUTPUT.jpg
TV access is configured as described in tools/webos/tv.py.
"""
import argparse
import json
import shlex
import sys
from pathlib import Path

# The TV access shared with the build tools (tools/webos/tv.py).
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'tools' / 'webos'))

from tv import Tv

REMOTE_SHOT = '/tmp/mana-shot.jpg'


def main():
    parser = argparse.ArgumentParser(description='Capture the TV screen.')
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    tv = Tv()
    payload = json.dumps({'path': REMOTE_SHOT, 'method': 'DISPLAY', 'width': 1920, 'height': 1080,
                          'format': 'JPEG'})
    reply = tv.run(f'luna-send -n 1 luna://com.webos.service.capture/executeOneShot {shlex.quote(payload)}',
                   tty=True)
    if b'"returnValue":true' not in reply.stdout.replace(b' ', b''):
        raise SystemExit('capture failed: ' + reply.stdout.decode(errors='replace'))
    image = tv.read(REMOTE_SHOT)
    tv.run(f'rm -f {REMOTE_SHOT}')
    args.output.write_bytes(image)
    print(args.output)


if __name__ == '__main__':
    main()
