"""Signed binary catalogue: the shapes of an app whose API answers Protobuf.

A ROT47 JSON configuration names the player domain, whose origin every
later request must send. Catalogue and detail requests are signed: a first
call gives a version per request code, and the path starts with
`/bin/v` + the first six hex digits of the query's MD5 + that version.
Answers wrap a message in field 10 of an envelope; lists are repeated
messages. A match nests its names: field 10 the competition, field 30 a
title then the two teams, each name in a translation (language, name);
field 3 its start in milliseconds. The stream URL is ROT47 after an eight-character prefix; the
session, in a response header, is encrypted with AES-256-CTR under a key
and IV written as text in the app, then base64 (standard alphabet),
percent-encoded, with an `a` appended, and goes into the media path.
All data is synthetic; nothing reproduces an external service.
"""
import base64
import hashlib
import json
import re
import secrets
from urllib.parse import quote, urlencode

from .state import TTL, UA, binary_field, crypto, rot47, varint

# Written as text in the synthetic app, as the key of the shape studied.
KEY = 'lab-binary-key-0123456789abcdefg'
IV = 'lab-binary-iv-01'
CODES = {'live': 100, 'detail': 102}
MATCHES = [(7001, 'Lab North', 'Lab South', 1795474800000), (7002, 'Lab East', 'Lab West', 1795478400000)]
LEAGUE = 'Lab League'
STREAMS = {7001: [(501, 'Synthetic feed', 2001), (502, 'Second feed', 2001)], 7002: []}


def number_field(number, value):
    return varint(number << 3) + varint(value)


def text_field(number, value):
    return binary_field(number, value.encode())


def translated(name):
    return binary_field(3, number_field(1, 8) + text_field(2, name))


def match(id, home, away, start):
    teams = b''.join(binary_field(30, number_field(1, 1) + binary_field(10, translated(team))) for team in (home, away))
    return (number_field(1, id) + number_field(3, start) + binary_field(10, translated(LEAGUE))
            + binary_field(30, text_field(2, f'{home} vs {away}')) + teams)


def envelope(inner):
    return binary_field(10, inner)


def answer(body, headers=None):
    return 200, 'application/x-protobuf', body, headers or {}


def denied(code):
    return 403, 'application/json', json.dumps({'code': code}).encode(), {}


def handle(state, path, query, headers):
    """The answer to a `/bin/` request of the API origin, or None if the
    path is not one of them."""
    if not path.startswith('/bin/'):
        return None
    if headers.get('User-Agent') != UA:
        return denied('wrong_user_agent')
    player = state.origins['resolver']
    if path == '/bin/api/common/params':
        client = {'trd': {'iframePlayerDomains': [player.split('://', 1)[1]]}}
        config = {'common:web:client': json.dumps(client), 'common:ad:app': json.dumps({'links': []})}
        return 200, 'text/plain', rot47(json.dumps(config)).encode(), {}
    if headers.get('Origin') != player or headers.get('Referer') != player + '/':
        return denied('missing_player_origin')
    values = {key: items[0] for key, items in query.items()}
    if path == '/bin/api/common/bs':
        entries = b''.join(binary_field(1, number_field(1, code) + text_field(2, state.binary_versions[code]))
                           for code in CODES.values())
        return answer(envelope(entries))
    signed = re.fullmatch(r'/bin/v([0-9a-f]{6})(\w+)/api/match/(live|detail)', path)
    if signed:
        prefix, version, kind = signed.groups()
        if prefix != hashlib.md5(urlencode(values).encode()).hexdigest()[:6] or version != state.binary_versions[CODES[kind]]:
            return denied('bad_signature')
        if kind == 'live':
            matches = b''.join(binary_field(1, match(*entry)) for entry in MATCHES)
            return answer(envelope(matches))
        streams = STREAMS.get(int(values.get('matchId', '0')), [])
        body = b''.join(binary_field(2, number_field(1, id) + text_field(3, name) + number_field(9, site))
                        for id, name, site in streams)
        return answer(envelope(body))
    if path == '/bin/api/user/info':
        return answer(envelope(text_field(2, 'IT') + text_field(3, 'EU')))
    if path == '/bin/api/stream/detail':
        if values.get('country') != 'IT' or values.get('continent') != 'EU':
            return denied('missing_location')
        session = secrets.token_hex(16)
        encoded = crypto('ctr', key=KEY.encode().hex(), iv=IV.encode().hex(), session=session)
        sealed = base64.urlsafe_b64decode(encoded + '=' * (-len(encoded) % 4))
        token = quote(base64.b64encode(sealed).decode(), safe='') + 'a'
        with state.lock:
            state.sessions[token] = state.clock.now() + TTL
        url = rot47('lab-v1::' + state.origins['media'] + '/session/master.m3u8')
        stream = number_field(1, int(values.get('streamId', '0'))) + text_field(4, url)
        return answer(envelope(binary_field(2, stream)), {'X-Lab-Session': session})
    return 404, 'application/json', b'{"code": "not_found"}', {}
