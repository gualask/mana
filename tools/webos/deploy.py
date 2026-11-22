#!/usr/bin/env python3
"""Install an .ipk on the TV over SSH, launch the app and optionally show its log.

Usage: python3 tools/webos/deploy.py IPK [--no-launch] [--log SECONDS]
TV access is configured as described in tools/webos/tv.py.
"""
import argparse
import json
import shlex
import subprocess
import sys
import time
from pathlib import Path

from tv import APP_ID, REMOTE_LOG, Tv

REMOTE_IPK = '/tmp/mana.ipk'
INSTALL_TIMEOUT = 60


def install(tv, ipk):
    result = tv.run(f'cat > {REMOTE_IPK}', stdin=ipk.read_bytes())
    if result.returncode:
        raise SystemExit('upload failed: ' + result.stderr.decode(errors='replace'))
    payload = json.dumps({'id': APP_ID, 'ipkUrl': REMOTE_IPK, 'subscribe': True})
    # -f prints one field per line, so each install state arrives on its own line.
    command = f'luna-send -f -i luna://com.webos.appInstallService/dev/install {shlex.quote(payload)}'
    process = subprocess.Popen(tv.argv(command, tty=True), stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True)
    deadline = time.monotonic() + INSTALL_TIMEOUT
    state = None
    try:
        for line in process.stdout:
            if '"state"' in line:
                state = line.split(':', 1)[1].strip().strip('",')
                print('  install:', state)
            if 'errorCode' in line or 'errorText' in line:
                print('  ' + line.strip())
            if state == 'installed' or (state and 'fail' in state) or time.monotonic() > deadline:
                break
    finally:
        process.kill()
    if state != 'installed':
        raise SystemExit(f'install did not complete (last state: {state})')


def main():
    parser = argparse.ArgumentParser(description='Install and launch the app on the TV.')
    parser.add_argument('ipk', type=Path)
    parser.add_argument('--no-launch', action='store_true')
    parser.add_argument('--log', type=float, default=0, help='seconds to wait before printing the app log')
    args = parser.parse_args()
    tv = Tv()

    install(tv, args.ipk)
    if not args.no_launch:
        payload = json.dumps({'id': APP_ID})
        result = tv.run(f'luna-send -n 1 luna://com.webos.applicationManager/launch {shlex.quote(payload)}', tty=True)
        print('  launch:', result.stdout.decode(errors='replace').strip())
    if args.log:
        time.sleep(args.log)
        sys.stdout.write(tv.read(REMOTE_LOG).decode(errors='replace'))


if __name__ == '__main__':
    main()
