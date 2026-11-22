"""A live channel as FFmpeg's HLS muxer packages it, beside the lab's own
live channels (live.py): fMP4 segments with the audio in a separate
rendition (EXT-X-MEDIA), sliding window, no encryption and no fault
injection."""
from pathlib import Path
import re
import shutil
import subprocess
import time

from .live import READY_TIMEOUT, SEGMENT_SECONDS, WINDOW_SEGMENTS
from .testcard import RATE, VIDEO_CODECS, sources

# Served channels: path prefix -> (size, video codec, bitrate).
PACKAGED_CHANNELS = {'live3': ('1280x720', 'h264', '2M')}
PACKAGED_FILE = re.compile(r'(master|stream_\d)\.m3u8|init_\d\.mp4|stream_\d_\d{6}\.m4s')


class PackagedLive:
    """A live channel served as FFmpeg's HLS muxer writes it: fMP4 segments
    with the audio in a separate rendition (EXT-X-MEDIA), sliding window,
    no encryption and no fault injection."""

    def __init__(self, directory, size, codec, bitrate, segment_seconds=SEGMENT_SECONDS):
        if not shutil.which('ffmpeg'):
            raise RuntimeError('FFmpeg is required to generate the synthetic live stream')
        self.directory = Path(directory)
        self.directory.mkdir()
        video, audio = sources(size)
        command = ['ffmpeg', '-hide_banner', '-loglevel', 'error', '-nostdin',
                   '-re', '-f', 'lavfi', '-i', f'{video}[out0];{audio}[out1]', '-map', '0:v', '-map', '0:a',
                   *VIDEO_CODECS[codec], '-pix_fmt', 'yuv420p', '-g', str(RATE * segment_seconds),
                   '-sc_threshold', '0', '-b:v', bitrate, '-maxrate', bitrate, '-bufsize', bitrate,
                   '-c:a', 'aac', '-b:a', '128k',
                   '-f', 'hls', '-hls_time', str(segment_seconds), '-hls_list_size', str(WINDOW_SEGMENTS),
                   '-hls_flags', 'delete_segments+temp_file+independent_segments', '-hls_segment_type', 'fmp4',
                   '-hls_fmp4_init_filename', 'init_%v.mp4', '-var_stream_map', 'v:0,agroup:aud a:0,agroup:aud,default:yes',
                   '-master_pl_name', 'master.m3u8',
                   '-hls_segment_filename', str(self.directory / 'stream_%v_%06d.m4s'),
                   str(self.directory / 'stream_%v.m3u8')]
        self.process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                        stderr=subprocess.PIPE)

    def in_outage(self):
        return False

    def rate(self):
        return None

    def file(self, name):
        if not PACKAGED_FILE.fullmatch(name):
            return None
        try:
            data = (self.directory / name).read_bytes()
        except FileNotFoundError:
            return None
        kind = 'application/vnd.apple.mpegurl' if name.endswith('.m3u8') else 'video/mp4'
        return kind, data

    def wait_ready(self, segments=2):
        deadline = time.monotonic() + READY_TIMEOUT
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError('Live encoder stopped: ' + self.process.stderr.read().decode()[-1000:])
            playlist = self.directory / 'stream_0.m3u8'
            if (self.directory / 'master.m3u8').is_file() and playlist.is_file() \
                    and playlist.read_text().count('#EXTINF') >= segments:
                return
            time.sleep(.2)
        raise RuntimeError('Live playlist not ready')

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
        self.process.stderr.close()
