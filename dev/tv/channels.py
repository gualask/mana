#!/usr/bin/env python3
"""Copy real channel names and logos to the TV, for the interface test.

Takes the public iptv-org lists of the given countries and writes an M3U
list where the app reads its channel list (apps/tv/src/screens/mod.rs,
CHANNELS_FILE): the channels with a `tvg-logo`, one per logo, up to COUNT,
without their streams. The logos are fetched by the app from their own
hosts, in their own formats.

Usage: python3 dev/tv/channels.py [--countries it,fr,de,es,uk] [--count 600]
       python3 dev/tv/channels.py --remove
"""
import argparse
import re
import sys
import urllib.request
from pathlib import Path

# The TV access shared with the build tools (tools/webos/tv.py).
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'tools' / 'webos'))

from tv import Tv, app_path

LIST_URL = 'https://iptv-org.github.io/iptv/countries/{}.m3u'
TV_FILE = app_path('CHANNELS_FILE')
EXTINF = re.compile(r'#EXTINF:[^\n]*tvg-logo="(?P<logo>[^"]+)"[^\n]*,(?P<name>[^\n]+)')


def download(country):
    """The iptv-org list of `country`."""
    with urllib.request.urlopen(LIST_URL.format(country), timeout=30) as response:
        return response.read().decode(errors='replace')


def with_logos(texts, count):
    """One `#EXTINF` line per distinct logo, up to `count`."""
    channels, logos = [], set()
    for match in (match for text in texts for match in EXTINF.finditer(text)):
        logo, name = match['logo'], match['name'].strip()
        if logo in logos:
            continue
        logos.add(logo)
        channels.append(f'#EXTINF:-1 tvg-logo="{logo}",{name}')
        if len(channels) == count:
            break
    return channels


def main():
    parser = argparse.ArgumentParser(description='Copy real channel names and logos to the TV.')
    parser.add_argument('--countries', default='it,fr,de,es,uk')
    parser.add_argument('--count', type=int, default=600)
    parser.add_argument('--remove', action='store_true', help='remove the list: the app goes back to its synthetic one')
    args = parser.parse_args()
    tv = Tv()
    if args.remove:
        tv.run(f'rm -f {TV_FILE}')
        return
    texts = [download(country.strip()) for country in args.countries.split(',')]
    lines = with_logos(texts, args.count)
    data = ('#EXTM3U\n' + '\n'.join(lines) + '\n').encode()
    result = tv.run(f'cat > {TV_FILE}', stdin=data)
    if result.returncode != 0:
        raise SystemExit(result.stderr.decode(errors='replace'))
    print(f'{len(lines)} canali con logo diverso copiati in {TV_FILE}')


if __name__ == '__main__':
    main()
