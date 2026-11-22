"""Generate a short, original HLS asset from mathematical video/audio sources."""
from pathlib import Path
import shutil
import subprocess


def generate(directory):
    if not shutil.which('ffmpeg'):
        raise RuntimeError('FFmpeg is required to generate synthetic media')
    command = ['ffmpeg', '-hide_banner', '-loglevel', 'error', '-nostdin',
               '-f', 'lavfi', '-i', 'testsrc2=size=320x180:rate=25',
               '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000',
               '-t', '4', '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p',
               '-g', '25', '-sc_threshold', '0', '-c:a', 'aac', '-b:a', '64k',
               '-f', 'hls', '-hls_time', '1', '-hls_list_size', '0', '-hls_playlist_type', 'vod',
               '-hls_segment_filename', str(Path(directory) / 'segment-%03d.ts'),
               str(Path(directory) / 'index.m3u8')]
    result = subprocess.run(command, capture_output=True, text=True, timeout=30)
    if result.returncode:
        raise RuntimeError('Synthetic media generation failed: ' + result.stderr[-1000:])
