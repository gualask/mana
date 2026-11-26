"""HTTP scenarios organized by origin; all links point at this lab."""
import json
import re
import secrets
import time
from urllib.parse import urlencode
from . import binary
from .state import UA

# The longest wait /faults/slow grants, in seconds.
SLOW_LIMIT = 10


def document(status, value, headers=None):
    return status, 'application/json', json.dumps(value).encode(), headers or {}


def text(status, value, content_type='text/html', headers=None):
    return status, content_type, value.encode(), headers or {}


def signals(state):
    return [{'id': 'denied', 'label': 'Deliberate access denial', 'play_type': 'm3u8',
             'url': state.origins['media'] + '/denied.m3u8'},
            {'id': 'encrypted', 'label': 'Synthetic encrypted link', 'play_type': 'iframe',
             'url': state.origins['resolver'] + '/wrapper'}]


def api(state, path, query, headers, method, body):
    if path == '/health':
        return document(200, {'service': 'stream-synthetic-lab', 'role': 'api'})
    if path == '/control/live-stats' and method == 'GET':
        return live_stats(state)
    if path in ('/control/live-restart', '/control/live-outage', '/control/live-stall',
                '/control/live-throttle') and method == 'POST':
        return live_control(state, path, body)
    if path == '/control/live-token' and method == 'POST':
        return new_live_token(state)
    if path == '/control/advance' and method == 'POST':
        return advance_clock(state, body)
    if method != 'GET':
        return document(405, {'code': 'method_not_allowed'})
    signed = binary.handle(state, path, query, headers)
    if signed is not None:
        return signed
    if path == '/channels.m3u':
        return text(200, '#EXTM3U\n#EXTINF:-1,Synthetic bars and tone\n' + state.public_url() + '\n', 'audio/x-mpegurl')
    if path == '/embedded':
        payload = json.dumps({'streams': [dict(signal, streaming_link=signal['url']) for signal in signals(state)]})
        return text(200, '<!doctype html><script>self.__next_f = [];self.__next_f.push(' + json.dumps([1, payload]) + ');</script>')
    if path == '/faults/rate-limited':
        if state.count('rate-limited') == 1:
            return document(429, {'code': 'rate_limited'}, {'Retry-After': '0'})
        return document(200, {'recovered': True})
    if path == '/faults/server-error':
        return document(503, {'code': 'temporarily_unavailable'})
    if path == '/faults/malformed.json':
        return text(200, '{not valid JSON', 'application/json')
    if path == '/faults/redirect-loop':
        return text(302, '', headers={'Location': state.origins['api'] + path})
    if path == '/faults/slow':
        return slow(state, query)
    if headers.get('User-Agent') != UA:
        return document(403, {'code': 'wrong_user_agent'})
    if path == '/live/resolve.json' and state.live is not None:
        return live_resolve(state)
    if path == '/catalogue.json':
        return document(200, [{'id': 'demo', 'title': 'Synthetic bars and tone', 'signals': signals(state)[:1]},
                              {'id': 'empty', 'title': 'Empty event', 'signals': []}])
    if path in ('/events/demo.json', '/events/empty.json'):
        return document(200, {'id': path.split('/')[2].split('.')[0],
                              'signals': signals(state) if path == '/events/demo.json' else []})
    if path == '/session.pb':
        data, extra = state.binary_session()
        return 200, 'application/x-protobuf', data, extra
    return document(404, {'code': 'not_found'})


def live_stats(state):
    with state.lock:
        return document(200, {'accepted': state.counts.get('live-accepted', 0),
                              'rejected': state.counts.get('live-rejected', 0),
                              'resolved': state.counts.get('live-resolved', 0)})


def new_live_token(state):
    """Changes the first channel's token, as an expiring CDN token."""
    if state.live is None:
        return document(404, {'code': 'not_found'})
    with state.lock:
        state.live_token = secrets.token_hex(8)
        return document(200, {'token': state.live_token})


def advance_clock(state, body):
    try:
        seconds = integer(json.loads(body)['seconds'], 0, 3600)
    except (ValueError, KeyError, TypeError):
        return document(400, {'code': 'invalid_clock_advance'})
    return document(200, {'now': state.clock.advance(seconds)})


def slow(state, query):
    """A resolution that comes late: the public clip, after the seconds
    asked, at most SLOW_LIMIT."""
    try:
        seconds = float(query.get('seconds', ['1'])[0])
    except ValueError:
        return document(400, {'code': 'invalid_delay'})
    if not 0 <= seconds <= SLOW_LIMIT:
        return document(400, {'code': 'invalid_delay'})
    time.sleep(seconds)
    return document(200, {'url': state.public_url()})


def live_resolve(state):
    """Shaped as RaiPlay's live channel JSON: the URL to play, with the
    current token, under video.content_url; counted for /control/live-stats."""
    state.count('live-resolved')
    with state.lock:
        url = f"{state.origins['media']}/live/master.m3u8?t={state.live_token}"
    return document(200, {'video': {'content_url': url}})


def resolver(state, path, query, headers, method, body):
    if method != 'GET':
        return document(405, {'code': 'method_not_allowed'})
    if headers.get('User-Agent') != UA:
        return document(403, {'code': 'wrong_user_agent'})
    if path == '/wrapper':
        return text(200, state.envelope())
    if path == '/player':
        return text(200, '<!doctype html><title>Mana synthetic player target</title>')
    if path == '/frame':
        return text(200, '<!doctype html><iframe src="' + state.origins['resolver'] + '/embed/demo"></iframe>')
    if path == '/embed/demo':
        return text(200, '<!doctype html><video data-api="' + state.origins['resolver'] + '/api/links/demo"></video>')
    if path == '/api/links/demo':
        return document(200, {'links': [{'id': 'synthetic', 'playUrl': state.media_url()}]})
    return document(404, {'code': 'not_found'})


def media(state, path, query, headers, method, body):
    if method != 'GET':
        return document(405, {'code': 'method_not_allowed'})
    if path == '/denied.m3u8':
        return document(403, {'code': 'synthetic_denial'})
    channel = re.match(r'/(live\d*)/', path)
    if channel and channel[1] in state.channels:
        return live(state, state.channels[channel[1]], path[channel.end():], headers, query)
    protected = path.startswith('/protected/')
    session = re.fullmatch(r'/token-([A-Za-z0-9_%-]+)/session/(.+)', path)
    if not (protected or session or path.startswith('/public/')):
        return document(404, {'code': 'not_found'})
    if protected or session:
        error = clip_refusal(state, query, headers, None if protected else session[1])
        if error:
            return document(403, {'code': error})
    name = session[2] if session else path.split('/')[-1]
    return clip_file(state, name, query if protected else None)


def clip_refusal(state, query, headers, session):
    """Why a request for the protected clip, or for the clip of `session`,
    is refused; None when it may be served."""
    if any(headers.get(key) != value for key, value in state.headers().items()):
        return 'missing_media_headers'
    if session is None:
        return state.check_signature(query)
    with state.lock:
        expiry = state.sessions.get(session)
    if expiry is None:
        return 'invalid_session'
    return 'token_expired' if expiry <= state.clock.now() else None


def clip_file(state, name, signed):
    """The clip's master, media playlist or segment `name`; with the
    `signed` query, every link of the playlists carries it."""
    suffix = '' if signed is None else '?' + encode_query(signed)
    if name == 'master.m3u8':
        return text(200, '#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=500000,RESOLUTION=320x180\nindex.m3u8' + suffix + '\n', 'application/vnd.apple.mpegurl')
    if name == 'index.m3u8':
        playlist = (state.media_dir / name).read_text()
        if suffix:
            playlist = '\n'.join(line + suffix if line and not line.startswith('#') else line for line in playlist.splitlines()) + '\n'
        return text(200, playlist, 'application/vnd.apple.mpegurl')
    if re.fullmatch(r'segment-\d{3}\.ts', name) and (state.media_dir / name).is_file():
        return 200, 'video/mp2t', (state.media_dir / name).read_bytes(), {}
    return document(404, {'code': 'not_found'})


def number(value, low, high):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not low <= value < high:
        raise ValueError()
    return value


def integer(value, low, high):
    """`value`, an integer from `low` to `high` included; ValueError otherwise."""
    if isinstance(value, bool) or not isinstance(value, int) or not low <= value <= high:
        raise ValueError()
    return value


def live_control(state, path, body):
    """Fault injection on the first channel: restart its encoder, or
    disturb it for a number of seconds."""
    if state.live is None:
        return document(404, {'code': 'not_found'})
    try:
        values = json.loads(body or b'{}')
        if path == '/control/live-restart':
            return restart_live(state.live, values)
        return disturb_live(state.live, path, values)
    except (ValueError, KeyError, TypeError, AttributeError):
        return document(400, {'code': 'invalid_live_control'})


def restart_live(channel, values):
    """Restarts the encoder: timestamps from an offset in seconds or
    shifted from where they were, numbering optionally from 0."""
    offset = number(values.get('ts_offset', 0), 0, 2 ** 33 / 90000)
    shift = values.get('ts_shift')
    shift = None if shift is None else number(shift, -30, 30)
    reset = values.get('reset_sequence', False)
    if not isinstance(reset, bool):
        raise ValueError()
    channel.restart(offset, ts_shift=shift, reset_sequence=reset)
    return document(200, {'restarted': True, 'ts_offset': offset, 'ts_shift': shift, 'reset_sequence': reset})


def disturb_live(channel, path, values):
    """For a number of seconds: serve slowly, freeze the playlist, or drop
    every request."""
    seconds = integer(values['seconds'], 1, 120)
    if path == '/control/live-throttle':
        kbps = integer(values['kbps'], 64, 100_000)
        channel.throttle(seconds, kbps)
        return document(200, {'throttle_seconds': seconds, 'kbps': kbps})
    if path == '/control/live-stall':
        channel.stall(seconds)
        return document(200, {'stall_seconds': seconds})
    channel.outage(seconds)
    return document(200, {'outage_seconds': seconds})


def live(state, channel, name, headers, query):
    """Live HLS whose playlist, key and segments all require the media headers;
    accepted and rejected requests are counted for /control/live-stats. During
    a simulated outage every request is dropped without a response, and on a
    slow network the answer carries what gives the rate to serve it at. On the
    first channel, playlists asked for with a token (`?t=`) need the current
    one, which the master passes on to its media playlists."""
    if channel.in_outage():
        return None
    accepted = all(headers.get(key) == value for key, value in state.headers().items())
    state.count('live-accepted' if accepted else 'live-rejected')
    if not accepted:
        return document(403, {'code': 'missing_media_headers'})
    token = query.get('t', [None])[0] if channel is state.live and name.endswith('.m3u8') else None
    if token is not None:
        with state.lock:
            if token != state.live_token:
                return document(403, {'code': 'token_expired'})
    served = channel.file(name)
    if served is None:
        return document(404, {'code': 'not_found'})
    data = served[1]
    if token is not None and name == 'master.m3u8':
        data = data.replace(b'.m3u8\n', f'.m3u8?t={token}\n'.encode())
    return 200, served[0], data, {}, channel.rate


def encode_query(query):
    return urlencode({key: values[0] for key, values in query.items()})


ROUTES = {'api': api, 'resolver': resolver, 'media': media}
