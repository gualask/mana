"""Three origins for repeatable streaming integration tests: loopback by
default, optionally on a local network address over HTTPS with live streams."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import shutil
import ssl
import tempfile
import threading
import time
from urllib.parse import parse_qs, urlsplit
from .live import CHANNELS, SEGMENT_SECONDS, Live
from .packaged import PACKAGED_CHANNELS, PackagedLive
from .media import generate
from .scenarios import ROUTES, document
from .state import Clock, State
from .profiles import write_config
from .tls import server_context


# Every live channel the lab can serve, by name.
LIVE_CHANNELS = (*CHANNELS, *PACKAGED_CHANNELS)
# A client that stalls in the TLS handshake must only hold its own thread.
HANDSHAKE_TIMEOUT = 10


# A paced body is sent in slices this often.
PACE_SLICE = .1
# A continuous answer waits this long for a slow client to take more.
CONTINUOUS_WRITE_TIMEOUT = 30


def send_paced(stream, body, rate):
    """Writes `body` at the bytes per second `rate()` gives, asked again for
    each slice: the rest goes at full speed as soon as it gives None."""
    offset = 0
    while offset < len(body):
        current = rate()
        if current is None:
            stream.write(body[offset:])
            return
        step = max(1, int(current * PACE_SLICE))
        stream.write(body[offset:offset + step])
        stream.flush()
        offset += step
        time.sleep(PACE_SLICE)


class Handler(BaseHTTPRequestHandler):
    def setup(self):
        self.handshake_failed = False
        if isinstance(self.request, ssl.SSLSocket):
            self.request.settimeout(HANDSHAKE_TIMEOUT)
            try:
                self.request.do_handshake()
            except (ssl.SSLError, OSError):
                self.handshake_failed = True
        super().setup()

    def handle(self):
        if self.handshake_failed:
            return
        try:
            super().handle()
        except (ssl.SSLError, OSError):
            pass  # connection dropped by the client

    def log_message(self, *args):
        pass  # Do not print signed URLs, sessions or test keys.

    def do_GET(self):
        self.respond()

    def do_POST(self):
        self.respond()

    def respond(self):
        response = self.answer()
        if response is None:
            self.close_connection = True  # simulated outage: no response at all
            return
        self.send(response)

    def answer(self):
        """The route's answer to the request, its body read first."""
        parts = urlsplit(self.path)
        try:
            length = int(self.headers.get('Content-Length', '0'))
            if not 0 <= length <= 4096:
                return document(413, {'code': 'request_too_large'})
            self.connection.settimeout(3)
            body = self.rfile.read(length) if length else b''
            return ROUTES[self.server.role](self.server.state, parts.path, parse_qs(parts.query),
                                            self.headers, self.command, body)
        except (ValueError, TimeoutError):
            return document(400, {'code': 'bad_request'})
        except Exception:
            return document(500, {'code': 'fixture_error'})

    def send(self, response):
        """Sends `response`. A fifth item, if any, gives the rate in bytes
        per second to send the body at, or None for full speed (a network
        that slows down). A body that is not bytes yields the pieces of an
        answer with no length, which ends when the connection closes."""
        status, content_type, body, headers, *rate = response
        pieces = [body] if isinstance(body, bytes) else body
        pace = rate[0] if rate else lambda: None
        try:
            self.send_response(status)
            self.send_header('Content-Type', content_type)
            if isinstance(body, bytes):
                self.send_header('Content-Length', str(len(body)))
            else:
                self.close_connection = True
                self.connection.settimeout(CONTINUOUS_WRITE_TIMEOUT)
            self.send_header('Cache-Control', 'no-store')
            for key, value in headers.items():
                self.send_header(key, value)
            self.end_headers()
            for piece in pieces:
                send_paced(self.wfile, piece, pace)
                self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError, TimeoutError):
            pass


def live_channels(live):
    """The names of the live channels `live` asks for: none for False,
    every one for True, else those it names."""
    if live is True:
        return set(LIVE_CHANNELS)
    names = set(live or ())
    unknown = names - set(LIVE_CHANNELS)
    if unknown:
        raise ValueError('unknown live channels: ' + ', '.join(sorted(unknown)))
    return names


class Lab:
    """`live` is False, True for every live channel, or the names of those
    to serve: each one encodes in real time."""

    def __init__(self, manual_clock=False, api_port=0, resolver_port=0, media_port=0,
                 host='127.0.0.1', tls=False, live=False, segment_seconds=SEGMENT_SECONDS):
        self.ports = {'api': api_port, 'resolver': resolver_port, 'media': media_port}
        self.manual_clock = manual_clock
        self.host, self.tls = host, tls
        self.live = live_channels(live)
        self.segment_seconds = segment_seconds
        self.channels = {}
        self.certificate = None
        self.servers = []
        self.threads = []
        self.temporary = None
        self.state = None

    def __enter__(self):
        if not shutil.which('node'):
            raise RuntimeError('Node is required for synthetic crypto fixtures')
        self.temporary = tempfile.TemporaryDirectory(prefix='stream-streaming-lab-')
        try:
            media_dir = Path(self.temporary.name)
            generate(media_dir)
            self.state = State(media_dir, Clock(manual=self.manual_clock))
            self.start_channels(media_dir)
            self.start_servers(media_dir)
            for channel in self.channels.values():
                channel.wait_ready()
            return self
        except BaseException:
            self.close()
            raise

    def start_channels(self, media_dir):
        """The live channels asked for, each with its encoder running."""
        for name, (codec, variants) in CHANNELS.items():
            if name in self.live:
                self.channels[name] = Live(media_dir / name, codec, variants, self.segment_seconds)
        for name, (size, codec, bitrate) in PACKAGED_CHANNELS.items():
            if name in self.live:
                self.channels[name] = PackagedLive(media_dir / name, size, codec, bitrate, self.segment_seconds)
        self.state.channels = self.channels

    def start_servers(self, media_dir):
        """The three origins, each serving on a thread of its own."""
        context = None
        if self.tls:
            context, self.certificate = server_context(media_dir, self.host)
        scheme = 'https' if self.tls else 'http'
        for role, port in self.ports.items():
            server = ThreadingHTTPServer((self.host, port), Handler)
            server.daemon_threads = True
            if context:
                # The handshake runs in each connection's thread (Handler.setup),
                # not in the accepting thread.
                server.socket = context.wrap_socket(server.socket, server_side=True,
                                                    do_handshake_on_connect=False)
            server.role, server.state = role, self.state
            self.servers.append(server)
            self.state.origins[role] = f'{scheme}://{self.host}:{server.server_port}'
        for server in self.servers:
            thread = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': .05}, daemon=True)
            thread.start()
            self.threads.append(thread)

    def close(self):
        for channel in self.channels.values():
            channel.close()
        for server, thread in zip(self.servers, self.threads):
            server.shutdown()
            thread.join(timeout=2)
        for server in self.servers:
            server.server_close()
        if self.temporary:
            self.temporary.cleanup()
        self.servers.clear()
        self.threads.clear()

    def __exit__(self, *args):
        self.close()

    def write_config(self, directory):
        return write_config(self.state, directory, self.certificate)
