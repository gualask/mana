#!/usr/bin/env python3
"""Generate A/V sync test clips for judging playback on the TV.

Every second the frame flashes white while a 1 kHz beep plays: the two must
coincide. A white bar sweeps across the frame to make judder visible.
Content is synthetic (FFmpeg test sources), so the clips carry no licence.

Usage: python3 dev/tv/clips.py [OUTPUT_DIR] [--seconds N]
       python3 dev/tv/clips.py [OUTPUT_DIR] --upload
Generating requires FFmpeg with libx264, libx265 and, for HE-AAC, the
AudioToolbox AAC encoder (macOS) on the build machine.
--upload copies the clips already in OUTPUT_DIR to the folder the app lists
on the TV (tmpfs: emptied when the TV powers off); TV access is configured as
described in tools/webos/tv.py.
"""
import argparse
import io
import shutil
import subprocess
import sys
import tarfile
from pathlib import Path

# The TV access shared with the build tools (tools/webos/tv.py).
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'tools' / 'webos'))

from tv import ROOT, Tv, app_path

TV_CLIPS_DIR = app_path('CLIPS_DIR')

H264 = ['-c:v', 'libx264', '-preset', 'veryfast', '-profile:v', 'high', '-level', '4.1',
        '-b:v', '5M', '-maxrate', '6M', '-bufsize', '6M']
H264_INTERLACED = H264 + ['-flags', '+ildct+ilme', '-x264-params', 'interlaced=1:tff=1']
HEVC = ['-c:v', 'libx265', '-preset', 'veryfast', '-tag:v', 'hvc1', '-x265-params', 'log-level=error',
        '-b:v', '5M', '-maxrate', '6M', '-bufsize', '6M']
MPEG2_SD = ['-c:v', 'mpeg2video', '-b:v', '5M', '-maxrate', '8M', '-bufsize', '1835k',
            '-flags', '+ilme+ildct', '-field_order', 'tt', '-aspect', '16:9']
AAC = ['-c:a', 'aac', '-b:a', '128k']
# The AAC encoder delays audio by 1024 priming samples that MPEG-TS keeps:
# the beep is generated that much early so it plays with the flash. The other
# encoders' delays are not compensated: those clips test decoding, not sync.
AAC_PRIMING_SECONDS = 1024 / 48000

# name, frame rate, size, video codec, audio codec, audio priming (s)
VARIANTS = [
    ('avsync-1080p25-h264', 25, '1920x1080', H264, AAC, AAC_PRIMING_SECONDS),
    ('avsync-1080p50-h264', 50, '1920x1080', H264, AAC, AAC_PRIMING_SECONDS),
    ('avsync-1080p24-h264', 24, '1920x1080', H264, AAC, AAC_PRIMING_SECONDS),
    ('avsync-1080p25-hevc', 25, '1920x1080', HEVC, AAC, AAC_PRIMING_SECONDS),
    # Formats common in live TV.
    ('format-1080i25-h264-aac', 25, '1920x1080', H264_INTERLACED, AAC, AAC_PRIMING_SECONDS),
    ('format-1080p25-h264-mp2', 25, '1920x1080', H264, ['-c:a', 'mp2', '-b:a', '192k'], 0),
    ('format-1080p25-h264-ac3', 25, '1920x1080', H264, ['-c:a', 'ac3', '-b:a', '192k'], 0),
    ('format-1080p25-h264-eac3-51', 25, '1920x1080', H264, ['-c:a', 'eac3', '-b:a', '384k', '-ac', '6'], 0),
    ('format-1080p25-h264-heaac', 25, '1920x1080', H264,
     ['-c:a', 'aac_at', '-profile:a', '4', '-b:a', '64k'], 0),
    ('format-576i25-mpeg2-mp2', 25, '720x576', MPEG2_SD, ['-c:a', 'mp2', '-b:a', '192k'], 0),
]
FLASH_FRAMES = 2
BEEP_SECONDS = 0.08
BAR_PIXELS_PER_SECOND = 480


def generate(variant, seconds, output):
    name, rate, size, video_codec, audio_codec, priming = variant
    flash = FLASH_FRAMES / rate
    video = (f'testsrc=size={size}:rate={rate},'
             f"drawbox=x='mod(t*{BAR_PIXELS_PER_SECOND},iw)':y=0:w=40:h=ih:color=white:t=fill,"
             f"drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='lt(mod(t,1),{flash})'")
    beep_time = f'(t+{priming:.6f})'
    beep = f'if(lt(mod({beep_time},1),{BEEP_SECONDS}),0.7*sin(2*PI*1000*t),0)'
    audio = f"aevalsrc=exprs='{beep}|{beep}':sample_rate=48000:channel_layout=stereo"
    target = output / f'{name}.ts'
    command = ['ffmpeg', '-hide_banner', '-loglevel', 'error', '-y', '-nostdin',
               '-f', 'lavfi', '-i', video, '-f', 'lavfi', '-i', audio, '-t', str(seconds),
               *video_codec, '-pix_fmt', 'yuv420p', '-g', str(rate * 2), *audio_codec,
               '-f', 'mpegts', str(target)]
    subprocess.run(command, check=True)
    print(f'{target}: {target.stat().st_size // 1024} KiB')


def upload(output):
    clips = sorted(output.glob('*.ts'))
    if not clips:
        raise SystemExit(f'no clips in {output}')
    archive = io.BytesIO()
    with tarfile.open(fileobj=archive, mode='w') as tar:
        for clip in clips:
            tar.add(clip, arcname=clip.name)
    result = Tv().run(f'mkdir -p {TV_CLIPS_DIR} && tar xf - -C {TV_CLIPS_DIR}', stdin=archive.getvalue())
    if result.returncode != 0:
        raise SystemExit(result.stderr.decode(errors='replace'))
    print(f'{len(clips)} clip copiati in {TV_CLIPS_DIR}')


def main():
    parser = argparse.ArgumentParser(description='Generate A/V sync test clips.')
    parser.add_argument('output', type=Path, nargs='?', default=ROOT / 'private' / 'clips')
    parser.add_argument('--seconds', type=int, default=30)
    parser.add_argument('--upload', action='store_true', help='copy existing clips to the TV instead')
    args = parser.parse_args()
    if args.upload:
        upload(args.output)
        return
    if not shutil.which('ffmpeg'):
        raise SystemExit('ffmpeg not found')
    args.output.mkdir(parents=True, exist_ok=True)
    for variant in VARIANTS:
        generate(variant, args.seconds, args.output)


if __name__ == '__main__':
    main()
