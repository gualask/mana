"""Fixture clock, signatures and generated protocol data; no upstream requests."""
from dataclasses import dataclass, field
import hashlib
import hmac
import json
from pathlib import Path
import secrets
import subprocess
import threading
import time
from urllib.parse import urlencode

UA = 'StreamLab/1'
TTL = 60


class Clock:
    def __init__(self, manual=False):
        self.manual = manual
        self.epoch = int(time.time())
        self.started = time.monotonic()
        self.offset = 0
        self.lock = threading.Lock()

    def now(self):
        with self.lock:
            elapsed = 0 if self.manual else int(time.monotonic() - self.started)
            return self.epoch + self.offset + elapsed

    def advance(self, seconds):
        with self.lock:
            self.offset += seconds
        return self.now()


def crypto(action, **values):
    result = subprocess.run(['node', str(Path(__file__).with_name('crypto.cjs'))],
                            input=json.dumps({'action': action, **values}),
                            capture_output=True, text=True, timeout=5)
    if result.returncode:
        raise RuntimeError('Local fixture crypto failed')
    return json.loads(result.stdout)


def varint(value):
    output = bytearray()
    while value > 127:
        output.append((value & 127) | 128)
        value >>= 7
    output.append(value)
    return bytes(output)


def binary_field(number, value):
    return varint(number << 3 | 2) + varint(len(value)) + value


def rot47(text):
    return ''.join(chr(33 + (ord(char) - 33 + 47) % 94) if 33 <= ord(char) <= 126 else char for char in text)


@dataclass
class State:
    media_dir: Path
    clock: Clock
    origins: dict = field(default_factory=dict)
    signing_key: bytes = field(default_factory=lambda: secrets.token_bytes(32))
    ctr_key: bytes = field(default_factory=lambda: secrets.token_bytes(32))
    ctr_iv: bytes = field(default_factory=lambda: secrets.token_bytes(16))
    sessions: dict = field(default_factory=dict)
    # Path prefix -> lab.live.Live when serving live streams; faults are
    # injected into the first one.
    channels: dict = field(default_factory=dict)
    # Token the first live channel's playlists accept when one is given
    # (`?t=`), changed by /control/live-token as an expiring CDN token.
    live_token: str = field(default_factory=lambda: secrets.token_hex(8))

    @property
    def live(self):
        return self.channels.get('live')
    counts: dict = field(default_factory=dict)
    lock: threading.Lock = field(default_factory=threading.Lock)

    def count(self, name):
        with self.lock:
            self.counts[name] = self.counts.get(name, 0) + 1
            return self.counts[name]

    def signed_query(self):
        expires = str(self.clock.now() + TTL)
        signature = hmac.new(self.signing_key, ('synthetic:' + expires).encode(), hashlib.sha256).hexdigest()
        return urlencode({'expires': expires, 'token': signature})

    def media_url(self):
        return self.origins['media'] + '/protected/master.m3u8?' + self.signed_query()

    def check_signature(self, query):
        expires = query.get('expires', [''])[0]
        token = query.get('token', [''])[0]
        if not expires.isdecimal():
            return 'invalid_token'
        expected = hmac.new(self.signing_key, ('synthetic:' + expires).encode(), hashlib.sha256).hexdigest()
        if not hmac.compare_digest(token, expected):
            return 'invalid_token'
        return 'token_expired' if int(expires) <= self.clock.now() else None

    def headers(self):
        return {'User-Agent': UA, 'Referer': self.origins['resolver'] + '/', 'X-Lab-Media': 'synthetic'}

    def envelope(self):
        target = self.origins['resolver'] + '/player?' + urlencode({'liveUrl': self.media_url()})
        pool = crypto('seal', url=target, expires=self.clock.now() + TTL)
        return '<!doctype html><title>Mana synthetic envelope</title><script>const _0xp=' + json.dumps(pool) + ',_0xi=' + json.dumps(list(range(11))) + ';</script>'

    def binary_session(self):
        session = secrets.token_hex(16)
        token = crypto('ctr', key=self.ctr_key.hex(), iv=self.ctr_iv.hex(), session=session)
        with self.lock:
            self.sessions[token] = self.clock.now() + TTL
        raw_url = rot47('lab-v1::' + self.origins['media'] + '/session/master.m3u8').encode()
        # Synthetic schema: 1 = ROT47 URL, 2 = demo AES key, 3 = demo IV.
        body = binary_field(1, raw_url) + binary_field(2, self.ctr_key) + binary_field(3, self.ctr_iv)
        return body, {'X-Lab-Session': session}
