"""Synthetic live HLS: a real-time FFmpeg encode of the test card
(testcard.py) with a sliding window and AES-128 segments, in one or more
variants encoded together so that their segments line up.

The lab writes the served playlist itself, so faults can be injected while
the stream runs: an encoder restart (timestamps start again, marked by
EXT-X-DISCONTINUITY), with a timestamp offset that makes the 33-bit MPEG-TS
clock wrap shortly after, or with timestamps going on a little after or before
where they were, or with the segment numbering starting again as a new
playlist; a playlist that stops advancing; a network outage; and a slow
network.

The first channel has three variants, for the client to switch between on a
slow network, and is also served as one continuous MPEG-TS answer, as IPTV
servers send it: the same segments, decrypted and joined, so that the same
faults reach it."""
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import threading
import time

from .testcard import RATE, VIDEO_CODECS, bits, scaled_graph, variant_streams

# Segment duration, unless the lab is asked for another.
SEGMENT_SECONDS = 2
WINDOW_SEGMENTS = 6
# Segment files kept on disk behind the window, for slow clients.
KEPT_SEGMENTS = 20
# How often a continuous answer looks for a new segment.
CONTINUOUS_POLL = .2
# Decrypted segments kept for the continuous answers.
KEPT_PLAIN = 4
TS_PACKET = 188
READY_TIMEOUT = 30
# The 33-bit 90 kHz MPEG-TS clock wraps after this many seconds.
TS_WRAP_SECONDS = 2 ** 33 / 90000
SEGMENT_NAME = re.compile(r'run-(\d+)/segment-\d+-\d{6}\.ts')
# Served channels: path prefix -> (video codec, variants as (size, bitrate),
# best first). The second one differs in codec and size, as two real channels
# may, for channel changes.
CHANNELS = {'live': ('h264', [('1920x1080', '4M'), ('1280x720', '2M'), ('640x360', '800k')]),
            'live2': ('hevc', [('1280x720', '2M')])}
# Declared on each variant beside its video bitrate: the audio and the
# MPEG-TS overhead.
OVERHEAD_BITS = 200_000


class Live:
    """Runs the encoder under `directory`, one subdirectory per encoder run;
    `close` stops it."""

    def __init__(self, directory, codec='h264', variants=(('1920x1080', '4M'),), segment_seconds=SEGMENT_SECONDS):
        if not shutil.which('ffmpeg'):
            raise RuntimeError('FFmpeg is required to generate the synthetic live stream')
        self.directory = Path(directory)
        self.directory.mkdir()
        self.codec, self.variants = codec, list(variants)
        self.segment_seconds = segment_seconds
        self.key = secrets.token_bytes(16)
        key_file = self.directory / 'live.key'
        key_file.write_bytes(self.key)
        # Key URI as written in the encoder's playlist, the key file, and a
        # fixed IV, declared once in the served playlist.
        self.key_info = self.directory / 'live.keyinfo'
        self.key_info.write_text(f'key\n{key_file}\n{"0" * 32}\n')
        self.lock = threading.Lock()
        self.runs = []
        # Timestamp offset of each run, and the first run of the served
        # numbering (a later one after a sequence reset).
        self.offsets = []
        self.first_run = 0
        self.process = None
        self.outage_until = 0.0
        self.stall_until = 0.0
        # Variant -> the playlist served while it stands still.
        self.stalled = {}
        self.throttle_until = 0.0
        self.throttle_rate = None
        self.plain = {}
        self.restart()

    def restart(self, ts_offset=0.0, ts_shift=None, reset_sequence=False):
        """Starts a new encoder run whose timestamps begin at `ts_offset`
        seconds, or `ts_shift` seconds after where the current run reached
        (before, if negative). The served playlist marks the change as a
        discontinuity, or, with `reset_sequence`, starts again from segment 0
        with the new run only, as an encoder restarting its playlist."""
        if ts_shift is not None and self.runs:
            run = len(self.runs) - 1
            reached = sum(float(duration) for index, _, duration in self.segments(0) if index == run)
            ts_offset = max(0.0, self.offsets[run] + reached + ts_shift)
        with self.lock:
            self._stop()
            run = self.directory / f'run-{len(self.runs)}'
            run.mkdir()
            if reset_sequence:
                self.first_run = len(self.runs)
            graph = scaled_graph([size for size, _ in self.variants])
            command = ['ffmpeg', '-hide_banner', '-loglevel', 'error', '-nostdin',
                       '-re', '-f', 'lavfi', '-i', graph, *variant_streams([bitrate for _, bitrate in self.variants]),
                       *VIDEO_CODECS[self.codec],
                       '-pix_fmt', 'yuv420p', '-g', str(RATE * self.segment_seconds), '-sc_threshold', '0',
                       '-c:a', 'aac', '-b:a', '128k',
                       '-output_ts_offset', str(ts_offset),
                       '-f', 'hls', '-hls_time', str(self.segment_seconds), '-hls_list_size', '0',
                       '-hls_flags', 'temp_file', '-hls_key_info_file', str(self.key_info),
                       '-hls_segment_filename', str(run / 'segment-%v-%06d.ts'), str(run / 'index-%v.m3u8')]
            self.process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                            stderr=subprocess.PIPE)
            self.runs.append(run)
            self.offsets.append(ts_offset)

    def master(self):
        lines = ['#EXTM3U']
        for i, (size, bitrate) in enumerate(self.variants):
            bandwidth = bits(bitrate) + OVERHEAD_BITS
            lines += [f'#EXT-X-STREAM-INF:BANDWIDTH={bandwidth},RESOLUTION={size}', playlist_name(i)]
        return '\n'.join(lines) + '\n'

    def file(self, name):
        """(content type, bytes) of a served file, or None."""
        if name == 'master.m3u8':
            return 'application/vnd.apple.mpegurl', self.master().encode()
        playlists = [playlist_name(i) for i in range(len(self.variants))]
        if name in playlists:
            return 'application/vnd.apple.mpegurl', self.playlist(playlists.index(name)).encode()
        if name == 'key':
            return 'application/octet-stream', self.key
        if name == 'stream.ts':
            return 'video/mp2t', self.continuous()
        segment = self.segment(name)
        return None if segment is None else ('video/mp2t', segment)

    def outage(self, seconds):
        """Drops every live request for `seconds`."""
        self.outage_until = time.monotonic() + seconds

    def in_outage(self):
        return time.monotonic() < self.outage_until

    def throttle(self, seconds, kbps):
        """Serves every live response at `kbps` kilobits per second for
        `seconds`, as a slow network."""
        self.throttle_rate = kbps * 1000 // 8
        self.throttle_until = time.monotonic() + seconds

    def rate(self):
        """Bytes per second to serve at, or None for full speed."""
        return self.throttle_rate if time.monotonic() < self.throttle_until else None

    def stall(self, seconds):
        """Serves the playlist unchanged for `seconds`, as a stuck packager."""
        self.stall_until = time.monotonic() + seconds

    def segments(self, variant):
        """Every segment of `variant` produced so far: (run index, path
        relative to the live directory, duration)."""
        with self.lock:
            runs = list(enumerate(self.runs))
        found = []
        for index, run in runs:
            playlist = run / f'index-{variant}.m3u8'
            if playlist.is_file():
                found += [(index, f'{run.name}/{uri}', duration) for uri, duration in listed(playlist.read_text())]
        return found

    def playlist(self, variant):
        if time.monotonic() < self.stall_until and variant in self.stalled:
            return self.stalled[variant]
        self.stalled[variant] = self._playlist(variant)
        return self.stalled[variant]

    def _playlist(self, variant):
        all_segments = self.segments(variant)
        self._prune(all_segments)
        segments = [segment for segment in all_segments if segment[0] >= self.first_run]
        first = max(0, len(segments) - WINDOW_SEGMENTS)
        # A segment that opens a later run follows a discontinuity.
        opens_run = [i > 0 and segments[i][0] != segments[i - 1][0] for i in range(len(segments))]
        lines = ['#EXTM3U', '#EXT-X-VERSION:3', f'#EXT-X-TARGETDURATION:{self.segment_seconds}',
                 f'#EXT-X-MEDIA-SEQUENCE:{first}', f'#EXT-X-DISCONTINUITY-SEQUENCE:{sum(opens_run[:first + 1])}',
                 f'#EXT-X-KEY:METHOD=AES-128,URI="key",IV=0x{"0" * 32}']
        for i in range(first, len(segments)):
            if opens_run[i] and i > first:
                lines.append('#EXT-X-DISCONTINUITY')
            lines += [f'#EXTINF:{segments[i][2]},', segments[i][1]]
        return '\n'.join(lines) + '\n'

    def segment(self, name):
        """Bytes of a served segment, or None."""
        if not SEGMENT_NAME.fullmatch(name):
            return None
        try:
            return (self.directory / name).read_bytes()
        except FileNotFoundError:
            return None

    def continuous(self):
        """The channel as one MPEG-TS answer that goes on while the encoder
        runs, yielded a piece at a time: from the middle of the newest
        segment, as a server sends from wherever the channel is, then each
        new segment as the encoder writes it, across encoder restarts. An
        outage holds the answer and then ends it, as a network gone and back
        leaves the connection broken."""
        names = [name for _, name, _ in self.segments(0)]
        first = self.decrypted(names[-1]) if names else None
        if first is None:
            return
        yield first[len(first) // 2 // TS_PACKET * TS_PACKET:]
        sent = names[-1]
        while not self.in_outage():
            written = self.written_after(sent)
            if not written:
                time.sleep(CONTINUOUS_POLL)
            for sent, data in written:
                yield data
        while self.in_outage():
            time.sleep(CONTINUOUS_POLL)

    def written_after(self, sent):
        """(name, decrypted bytes) of each segment written after `sent`, or
        of the newest once `sent` is gone; those gone meanwhile are left
        out."""
        names = [name for _, name, _ in self.segments(0)]
        new = names[names.index(sent) + 1:] if sent in names else names[-1:]
        return [(name, data) for name in new if (data := self.decrypted(name)) is not None]

    def decrypted(self, name):
        """A segment's bytes without the AES-128 encryption, or None when it
        is gone; the last few are kept, as every answer sends the same."""
        with self.lock:
            if name in self.plain:
                return self.plain[name]
        data = self.segment(name)
        if data is None:
            return None
        plain = subprocess.run(['openssl', 'enc', '-d', '-aes-128-cbc', '-K', self.key.hex(), '-iv', '0' * 32],
                               input=data, capture_output=True, check=True).stdout
        with self.lock:
            self.plain[name] = plain
            for old in list(self.plain)[:-KEPT_PLAIN]:
                del self.plain[old]
        return plain

    def wait_ready(self, segments=2):
        """Waits until the current run has produced `segments` segments of
        each variant."""
        deadline = time.monotonic() + READY_TIMEOUT
        with self.lock:
            run = len(self.runs) - 1
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError('Live encoder stopped: ' + self.process.stderr.read().decode()[-1000:])
            if all(sum(1 for segment in self.segments(variant) if segment[0] == run) >= segments
                   for variant in range(len(self.variants))):
                return
            time.sleep(.2)
        raise RuntimeError('Live playlist not ready')

    def close(self):
        with self.lock:
            self._stop()

    def _prune(self, segments):
        for _, name, _ in segments[:-KEPT_SEGMENTS]:
            (self.directory / name).unlink(missing_ok=True)

    def _stop(self):
        if self.process is None:
            return
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
        self.process.stderr.close()
        self.process = None


def playlist_name(variant):
    """The served name of a variant's playlist: the best one keeps the name
    the lab served before it had variants."""
    return 'index.m3u8' if variant == 0 else f'index-{variant}.m3u8'


def listed(playlist):
    """The segments of the media playlist text `playlist`: (URI, duration
    as written)."""
    found = []
    duration = None
    for line in playlist.splitlines():
        if line.startswith('#EXTINF:'):
            duration = line[len('#EXTINF:'):].rstrip(',')
        elif line and not line.startswith('#') and duration:
            found.append((line, duration))
            duration = None
    return found
