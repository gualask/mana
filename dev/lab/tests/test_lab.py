"""Synthetic server integration tests, not tests of the future Mana engine."""
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import re
import socket
import ssl
import subprocess
import tempfile
import time
import tomllib
import unittest
import urllib.error
import urllib.request
from urllib.parse import parse_qs, urljoin, urlsplit, urlunsplit
from dev.lab.server import Lab
from dev.lab.state import TTL, UA, crypto

from dev.lab.codecs import decode_envelope, extract_embedded_streams, decode_binary, first_field, rot47


class Elements(HTMLParser):
    def __init__(self):
        super().__init__()
        self.targets = {}

    def handle_starttag(self, tag, attrs):
        self.targets[tag] = dict(attrs)


class LocalRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, url):
        if urlsplit(url).hostname != '127.0.0.1':
            raise AssertionError('External redirect attempted')
        return super().redirect_request(request, fp, code, message, headers, url)


class LabTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.lab = Lab(manual_clock=True).__enter__()
        cls.addClassCleanup(cls.lab.close)
        cls.origins = cls.lab.state.origins
        cls.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), LocalRedirects())

    def fetch(self, url, headers=None, data=None):
        self.assertEqual(urlsplit(url).scheme, 'http')
        self.assertIn(urlunsplit((*urlsplit(url)[:2], '', '', '')), self.origins.values())
        request = urllib.request.Request(url, data=data, headers={'User-Agent': UA, **(headers or {})})
        with self.opener.open(request, timeout=4) as response:
            return response.read(2_000_000), dict(response.headers)

    def api(self, path, **kwargs):
        return self.fetch(self.origins['api'] + path, **kwargs)

    def assert_http_error(self, url, status, code=None, headers=None):
        with self.assertRaises(urllib.error.HTTPError) as context:
            self.fetch(url, headers=headers)
        error = context.exception
        self.assertEqual(error.code, status)
        if code:
            self.assertEqual(json.loads(error.read())['code'], code)
        error.close()

    def resolved_envelope(self):
        raw, _ = self.fetch(self.origins['resolver'] + '/wrapper')
        envelope = decode_envelope(raw.decode())
        self.assertGreater(envelope['e'], self.lab.state.clock.now() * 1000)
        self.assertEqual(urlsplit(envelope['u']).path, '/player')
        self.fetch(envelope['u'])
        return parse_qs(urlsplit(envelope['u']).query)['liveUrl'][0]

    def assert_hls_chain(self, master, headers=None):
        raw, _ = self.fetch(master, headers)
        self.assertTrue(raw.startswith(b'#EXTM3U'))
        self.assertIn(b'#EXT-X-STREAM-INF:', raw)
        relative = next(line for line in raw.decode().splitlines() if line and not line.startswith('#'))
        playlist_url = urljoin(master, relative)
        raw, _ = self.fetch(playlist_url, headers)
        self.assertIn(b'#EXT-X-ENDLIST', raw)
        relative = next(line for line in raw.decode().splitlines() if line and not line.startswith('#'))
        segment_url = urljoin(playlist_url, relative)
        data, _ = self.fetch(segment_url, headers)
        self.assertGreater(len(data), 188)
        self.assertEqual(data[0], 0x47, 'MPEG-TS sync byte missing')
        return segment_url

    def test_01_catalogue_and_detail(self):
        self.assert_http_error(self.origins['api'] + '/catalogue.json', 403, 'wrong_user_agent', {'User-Agent': 'plain-client'})
        raw, _ = self.api('/catalogue.json'); events = json.loads(raw)
        self.assertEqual([event['id'] for event in events], ['demo', 'empty'])
        self.assertEqual(len(events[0]['signals']), 1)
        raw, _ = self.api('/events/demo.json')
        self.assertEqual([s['id'] for s in json.loads(raw)['signals']], ['denied', 'encrypted'])
        raw, _ = self.api('/events/empty.json')
        self.assertEqual(json.loads(raw)['signals'], [])

    def test_02_m3u_and_relative_hls(self):
        raw, _ = self.api('/channels.m3u')
        self.assertTrue(raw.startswith(b'#EXTM3U'))
        url = next(line for line in raw.decode().splitlines() if line and not line.startswith('#'))
        self.assert_hls_chain(url)

    def test_03_embedded_and_iframe_api(self):
        raw, _ = self.api('/embedded')
        rows = extract_embedded_streams(raw.decode())
        self.assertEqual(len(rows), 2)
        self.assertTrue(all(urlsplit(row['streaming_link']).hostname == '127.0.0.1' for row in rows))
        raw, _ = self.fetch(self.origins['resolver'] + '/frame')
        tags = Elements(); tags.feed(raw.decode())
        raw, _ = self.fetch(tags.targets['iframe']['src'])
        tags = Elements(); tags.feed(raw.decode())
        raw, _ = self.fetch(tags.targets['video']['data-api'])
        self.assert_hls_chain(json.loads(raw)['links'][0]['playUrl'], self.lab.state.headers())

    def test_04_envelope_headers_segment_signature(self):
        url = self.resolved_envelope()
        self.assert_http_error(url, 403, 'missing_media_headers')
        headers = self.lab.state.headers()
        segment = self.assert_hls_chain(url, headers)
        self.assertEqual(parse_qs(urlsplit(url).query), parse_qs(urlsplit(segment).query))
        self.assert_http_error(urlunsplit(urlsplit(segment)._replace(query='')), 403, 'invalid_token', headers)

    def test_05_expiration_and_reresolution(self):
        old = self.resolved_envelope(); headers = self.lab.state.headers()
        self.fetch(old, headers)
        self.api('/control/advance', data=json.dumps({'seconds': TTL + 1}).encode())
        self.assert_http_error(old, 403, 'token_expired', headers)
        fresh = self.resolved_envelope()
        self.assertNotEqual(old, fresh)
        self.assert_hls_chain(fresh, headers)

    def test_06_fallback_and_av_decode(self):
        raw, _ = self.api('/events/demo.json')
        attempted, playable = [], None
        for candidate in json.loads(raw)['signals']:
            attempted.append(candidate['id'])
            if candidate['play_type'] == 'm3u8':
                self.assert_http_error(candidate['url'], 403, 'synthetic_denial')
                continue
            playable = self.resolved_envelope()
            break
        self.assertEqual(attempted, ['denied', 'encrypted'])
        self.assertIsNotNone(playable)
        block = ''.join(f'{key}: {value}\r\n' for key, value in self.lab.state.headers().items())
        result = subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'info', '-nostdin',
            '-protocol_whitelist', 'http,tcp', '-rw_timeout', '5000000', '-headers', block,
            '-i', playable, '-t', '2', '-map', '0:v:0', '-map', '0:a:0',
            '-progress', 'pipe:1', '-f', 'null', '-'], capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr[-1500:])
        progress = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line)
        self.assertGreaterEqual(int(progress['frame']), 40)
        pcm = re.search(r'audio:\s*([0-9.]+)[KMGT]?i?B', result.stderr)
        self.assertIsNotNone(pcm)
        self.assertGreater(float(pcm[1]), 0)

    def test_07_binary_rot47_ctr_session(self):
        raw, headers = self.api('/session.pb'); message = decode_binary(raw)
        original = rot47(first_field(message, 1).decode())[8:]
        token = crypto('ctr', key=first_field(message, 2).hex(), iv=first_field(message, 3).hex(),
                       session=headers['X-Lab-Session'])
        parts = urlsplit(original)
        url = urlunsplit(parts._replace(path='/token-' + token + parts.path))
        self.assert_hls_chain(url, self.lab.state.headers())
        self.assert_http_error(url.replace(token, 'bad-token'), 403, 'invalid_session', self.lab.state.headers())

    def test_08_authenticated_envelope_tampering(self):
        raw, _ = self.fetch(self.origins['resolver'] + '/wrapper'); html = raw.decode()
        match = re.search(r'const _0xp=(\[.*?\]),_0xi=', html)
        pool = json.loads(match[1]); cipher = pool[3]
        pool[3] = ('A' if cipher[0] != 'A' else 'B') + cipher[1:]
        changed = html[:match.start(1)] + json.dumps(pool) + html[match.end(1):]
        with self.assertRaises(ValueError):
            decode_envelope(changed)

    def test_09_controlled_http_failures(self):
        with self.assertRaises(urllib.error.HTTPError) as context:
            self.api('/faults/rate-limited')
        self.assertEqual(context.exception.code, 429)
        self.assertEqual(context.exception.headers['Retry-After'], '0'); context.exception.close()
        raw, _ = self.api('/faults/rate-limited')
        self.assertEqual(json.loads(raw), {'recovered': True})
        self.assert_http_error(self.origins['api'] + '/faults/server-error', 503, 'temporarily_unavailable')
        raw, _ = self.api('/faults/malformed.json')
        with self.assertRaises(json.JSONDecodeError):
            json.loads(raw)
        with self.assertRaises(urllib.error.HTTPError) as redirect:
            self.api('/faults/redirect-loop')
        redirect.exception.close()

    def test_16_signed_binary_catalogue(self):
        from dev.lab import binary
        import base64, hashlib
        from urllib.parse import quote, urlencode
        def message(raw):
            return decode_binary(first_field(decode_binary(raw), 10, b''))
        raw, _ = self.api('/bin/api/common/params')
        config = json.loads(rot47(raw.decode()))
        domain = json.loads(config['common:web:client'])['trd']['iframePlayerDomains'][0]
        origin = 'http://' + domain
        headers = {'Origin': origin, 'Referer': origin + '/'}
        self.assert_http_error(self.origins['api'] + '/bin/api/common/bs', 403, 'missing_player_origin')
        def signed(kind, params):
            raw, _ = self.api('/bin/api/common/bs?code=' + str(binary.CODES[kind]), headers=headers)
            entries = [decode_binary(entry) for entry in message(raw)[1]]
            version = next(first_field(e, 2).decode() for e in entries if first_field(e, 1) == binary.CODES[kind])
            query = urlencode(params)
            prefix = hashlib.md5(query.encode()).hexdigest()[:6]
            self.assert_http_error(self.origins['api'] + f'/bin/v000000{version}/api/match/{kind}?{query}', 403, 'bad_signature', headers)
            raw, _ = self.api(f'/bin/v{prefix}{version}/api/match/{kind}?{query}', headers=headers)
            return message(raw)
        matches = [decode_binary(m) for m in signed('live', {'language': 8, 'sportType': 1, 'stream': 'true'})[1]]
        self.assertEqual([first_field(m, 1) for m in matches], [7001, 7002])
        streams = signed('detail', {'matchId': 7001, 'language': 8, 'sportType': 1, 'stream': 'true'}).get(2, [])
        stream_id = first_field(decode_binary(streams[0]), 1)
        self.assertEqual(signed('detail', {'matchId': 7002, 'language': 8, 'sportType': 1, 'stream': 'true'}).get(2, []), [])
        raw, _ = self.api('/bin/api/user/info', headers=headers)
        user = message(raw)
        query = urlencode({'matchId': 7001, 'streamId': stream_id, 'siteType': 2001,
                           'continent': first_field(user, 3).decode(), 'country': first_field(user, 2).decode()})
        raw, answer = self.api('/bin/api/stream/detail?' + query, headers=headers)
        stream = decode_binary(first_field(message(raw), 2))
        url = rot47(first_field(stream, 4).decode())[8:]
        encoded = crypto('ctr', key=binary.KEY.encode().hex(), iv=binary.IV.encode().hex(), session=answer['X-Lab-Session'])
        sealed = base64.urlsafe_b64decode(encoded + '=' * (-len(encoded) % 4))
        token = quote(base64.b64encode(sealed).decode(), safe='') + 'a'
        parts = urlsplit(url)
        signed_url = urlunsplit(parts._replace(path='/token-' + token + parts.path))
        self.assert_hls_chain(signed_url, self.lab.state.headers())
        self.assert_http_error(signed_url.replace(token, token[:-1] + 'b'), 403, 'invalid_session', self.lab.state.headers())

    def test_10_generated_configs(self):
        with tempfile.TemporaryDirectory(prefix='stream-lab-config-') as directory:
            path = self.lab.write_config(directory)
            data = tomllib.loads(path.read_text())
            self.assertEqual(data['origins'], self.origins)
            self.assertEqual(data['fixture_manifest_version'], 1)
            source = tomllib.loads((Path(directory) / 'direct.toml').read_text())
            self.assertEqual(source['catalogue']['items'][0]['url'], data['endpoints']['public_hls'])
            for url in data['endpoints'].values():
                self.assertEqual(urlsplit(url).hostname, '127.0.0.1')


class LiveTests(unittest.TestCase):
    """Live HLS over HTTPS with AES-128 segments and mandatory headers."""

    @classmethod
    def setUpClass(cls):
        cls.lab = Lab(tls=True, live=True).__enter__()
        cls.addClassCleanup(cls.lab.close)
        cls.origins = cls.lab.state.origins
        context = ssl.create_default_context(cafile=str(cls.lab.certificate))
        cls.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                                 urllib.request.HTTPSHandler(context=context))

    def fetch(self, path, headers, channel='live'):
        request = urllib.request.Request(f"{self.origins['media']}/{channel}/{path}", headers=headers)
        with self.opener.open(request, timeout=4) as response:
            return response.read(2_000_000)

    def stats(self):
        request = urllib.request.Request(self.origins['api'] + '/control/live-stats')
        with self.opener.open(request, timeout=4) as response:
            return json.loads(response.read())

    def test_live_requires_headers_and_decodes(self):
        headers = self.lab.state.headers()
        with self.assertRaises(urllib.error.HTTPError) as context:
            self.fetch('index.m3u8', {'User-Agent': UA})
        self.assertEqual(context.exception.code, 403)
        context.exception.close()

        master = self.fetch('master.m3u8', headers).decode()
        self.assertEqual(master.count('#EXT-X-STREAM-INF'), 3)
        self.assertIn('index.m3u8', master)
        # Variants encoded together: the same segments, numbered alike.
        best, lightest = (self.fetch(name, headers).decode() for name in ('index.m3u8', 'index-2.m3u8'))
        numbers = [set(re.findall(r'segment-\d+-(\d{6})\.ts', playlist)) for playlist in (best, lightest)]
        self.assertTrue(numbers[0] & numbers[1])
        self.assertIn('RESOLUTION=640x360', master)
        self.assertIn('RESOLUTION=1280x720', self.fetch('master.m3u8', headers, channel='live2').decode())
        master = self.fetch('master.m3u8', headers, channel='live3').decode()
        self.assertIn('#EXT-X-MEDIA:TYPE=AUDIO', master)
        self.assertIn('#EXT-X-MAP:URI="init_0.mp4"', self.fetch('stream_0.m3u8', headers, channel='live3').decode())
        self.assertEqual(self.fetch('init_0.mp4', headers, channel='live3')[4:8], b'ftyp')
        playlist = self.fetch('index.m3u8', headers).decode()
        self.assertIn('#EXT-X-KEY:METHOD=AES-128,URI="key"', playlist)
        self.assertNotIn('#EXT-X-ENDLIST', playlist)
        self.assertGreaterEqual(playlist.count('#EXTINF'), 2)
        self.assertEqual(len(self.fetch('key', headers)), 16)

        block = ''.join(f'{key}: {value}\r\n' for key, value in headers.items())
        result = subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-nostdin',
            '-rw_timeout', '5000000', '-headers', block, '-i', self.origins['media'] + '/live/master.m3u8',
            '-t', '2', '-map', '0:v:0', '-map', '0:a:0', '-progress', 'pipe:1', '-f', 'null', '-'],
            capture_output=True, text=True, timeout=30,
            env={**os.environ, 'SSL_CERT_FILE': str(self.lab.certificate)})
        self.assertEqual(result.returncode, 0, result.stderr[-1500:])
        progress = dict(line.split('=', 1) for line in result.stdout.splitlines() if '=' in line)
        self.assertGreaterEqual(int(progress['frame']), 40)

        stats = self.stats()
        self.assertEqual(stats['rejected'], 1)
        self.assertGreaterEqual(stats['accepted'], 5)

    def test_live_continuous_ts(self):
        """The first channel as one MPEG-TS answer: no length, from the middle
        of a segment, decodable, and broken off by an outage."""
        request = urllib.request.Request(self.origins['media'] + '/live/stream.ts', headers=self.lab.state.headers())
        data = b''
        with self.opener.open(request, timeout=6) as response:
            self.assertEqual(response.headers['Content-Type'], 'video/mp2t')
            self.assertIsNone(response.headers['Content-Length'])
            started = time.monotonic()
            while time.monotonic() - started < 4:
                data += response.read1(65536)
            self.assertEqual(self.control('live-outage', {'seconds': 1})['outage_seconds'], 1)
            started = time.monotonic()
            while response.read1(65536):
                pass
            self.assertLess(time.monotonic() - started, 3)
        self.assertEqual((data[0], data[188], data[376]), (0x47, 0x47, 0x47))
        result = subprocess.run(['ffmpeg', '-hide_banner', '-loglevel', 'error', '-nostdin', '-f', 'mpegts',
            '-i', 'pipe:0', '-map', '0:v:0', '-map', '0:a:0', '-progress', 'pipe:1', '-f', 'null', '-'],
            input=data, capture_output=True, timeout=30)
        progress = dict(line.split('=', 1) for line in result.stdout.decode().splitlines() if '=' in line)
        # Half a segment and the next ones, less what precedes a keyframe.
        self.assertGreaterEqual(int(progress['frame']), 50)
        time.sleep(1.2)  # the outage ends

    def control(self, path, value):
        request = urllib.request.Request(self.origins['api'] + '/control/' + path, data=json.dumps(value).encode())
        with self.opener.open(request, timeout=4) as response:
            return json.loads(response.read())

    def test_stalled_tls_client_does_not_block_others(self):
        host, port = urlsplit(self.origins['media']).hostname, urlsplit(self.origins['media']).port
        with socket.create_connection((host, port), timeout=4):  # never sends a ClientHello
            started = time.monotonic()
            self.assertIn('#EXTM3U', self.fetch('index.m3u8', self.lab.state.headers()).decode())
            self.assertLess(time.monotonic() - started, 3)

    def test_live_token_expires(self):
        headers = self.lab.state.headers()
        request = urllib.request.Request(self.origins['api'] + '/live/resolve.json', headers=headers)
        with self.opener.open(request, timeout=10) as response:
            url = json.loads(response.read())['video']['content_url']
        query = '?' + url.split('?', 1)[1]
        self.assertIn('index.m3u8' + query, self.fetch('master.m3u8' + query, headers).decode())
        self.assertIn('#EXTM3U', self.fetch('index.m3u8' + query, headers).decode())
        self.assertNotEqual(self.control('live-token', {})['token'], query[3:])
        with self.assertRaises(urllib.error.HTTPError) as context:
            self.fetch('index.m3u8' + query, headers)
        self.assertEqual(context.exception.code, 403)
        context.exception.close()
        self.assertIn('#EXTM3U', self.fetch('index.m3u8', headers).decode())  # without a token, as before

    def test_live_faults(self):
        headers = self.lab.state.headers()
        sequence = int(re.search(r'#EXT-X-MEDIA-SEQUENCE:(\d+)', self.fetch('index.m3u8', headers).decode())[1])
        self.assertTrue(self.control('live-restart', {'ts_offset': 1000})['restarted'])
        self.lab.channels['live'].wait_ready(segments=1)
        playlist = self.fetch('index.m3u8', headers).decode()
        self.assertIn('#EXT-X-DISCONTINUITY\n', playlist)
        self.assertGreaterEqual(int(re.search(r'#EXT-X-MEDIA-SEQUENCE:(\d+)', playlist)[1]), sequence)
        new_segment = re.findall(r'^run-1/segment-0-\d{6}\.ts$', playlist, re.MULTILINE)[0]
        self.assertGreater(len(self.fetch(new_segment, headers)), 0)

        self.assertEqual(self.control('live-outage', {'seconds': 1})['outage_seconds'], 1)
        with self.assertRaises((urllib.error.URLError, ConnectionError)):
            self.fetch('index.m3u8', headers)
        time.sleep(1.2)
        self.assertIn('#EXTM3U', self.fetch('index.m3u8', headers).decode())
        with self.assertRaises(urllib.error.HTTPError) as context:
            self.control('live-outage', {'seconds': 0})
        self.assertEqual(context.exception.code, 400)
        context.exception.close()

        stalled = self.fetch('index.m3u8', headers)
        self.assertEqual(self.control('live-stall', {'seconds': 3})['stall_seconds'], 3)
        time.sleep(2.5)  # a new segment every 2 s
        self.assertEqual(self.fetch('index.m3u8', headers), self.fetch('index.m3u8', headers))
        self.assertEqual(self.fetch('index.m3u8', headers).count(b'#EXTINF'), stalled.count(b'#EXTINF'))
        time.sleep(1)

        segment = re.findall(r'^run-\d+/segment-0-\d{6}\.ts$', self.fetch('index.m3u8', headers).decode(), re.MULTILINE)[-1]
        self.assertGreater(len(self.fetch(segment, headers)), 62_500 * 3)
        # Slow for 1 s, then at full speed: not the 3 s and more of 500 kbit/s.
        self.assertEqual(self.control('live-throttle', {'seconds': 1, 'kbps': 500})['kbps'], 500)
        started = time.monotonic()
        self.fetch(segment, headers)
        self.assertGreater(time.monotonic() - started, .8)
        self.assertLess(time.monotonic() - started, 2)
        with self.assertRaises(urllib.error.HTTPError) as context:
            self.control('live-throttle', {'seconds': 3, 'kbps': 0})
        self.assertEqual(context.exception.code, 400)
        context.exception.close()

        self.assertEqual(self.control('live-restart', {'ts_shift': -1})['ts_shift'], -1)
        self.lab.channels['live'].wait_ready(segments=1)
        self.assertTrue(self.control('live-restart', {'reset_sequence': True})['reset_sequence'])
        # The other tests expect a playlist of at least two segments.
        self.lab.channels['live'].wait_ready(segments=2)
        playlist = self.fetch('index.m3u8', headers).decode()
        self.assertIn('#EXT-X-MEDIA-SEQUENCE:0\n', playlist)
        self.assertNotIn('run-1/', playlist)
        with self.assertRaises(urllib.error.HTTPError) as context:
            self.control('live-restart', {'ts_shift': 60})
        self.assertEqual(context.exception.code, 400)
        context.exception.close()
