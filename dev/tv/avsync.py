#!/usr/bin/env python3
"""Measure A/V sync from a recording of a clip made by clips.py.

Film the TV playing an avsync clip (phone, ideally slow motion, 10-20 s with
the whole screen in frame) and pass the file. Each flash onset is paired with
the strongest beep within half a second; the offset is beep minus flash, so a positive value
means the sound comes late. The source clips themselves should measure ~0.

Usage: python3 dev/tv/avsync.py RECORDING [--threshold 0.5]
Requires FFmpeg on the build machine.
"""
import argparse
import array
import re
import statistics
import subprocess

AUDIO_RATE = 48000
WINDOW = 48  # samples per envelope step: 1 ms
BEEP_BAND = 'bandpass=f=1000:width_type=q:w=8,bandpass=f=1000:width_type=q:w=8'
MIN_GAP = 0.5  # seconds between two flashes
MAX_PAIR = 0.5  # seconds between a flash and its beep
SMOOTH = 20  # envelope steps averaged to tell a beep from a click
OUTLIER_MS = 100  # farther than this from the median: a noise, not a beep


def flash_onsets(path, threshold):
    """Times where mean luma rises above threshold between its min and max."""
    command = ['ffmpeg', '-hide_banner', '-nostdin', '-i', path, '-an',
               '-vf', 'scale=160:-2,signalstats,metadata=print:key=lavfi.signalstats.YAVG:file=-',
               '-f', 'null', '-']
    text = subprocess.run(command, capture_output=True, text=True, check=True).stdout
    times = [float(t) for t in re.findall(r'pts_time:([\d.]+)', text)]
    lumas = [float(v) for v in re.findall(r'YAVG=([\d.]+)', text)]
    level = min(lumas) + threshold * (max(lumas) - min(lumas))
    onsets, last, previous = [], -1.0, lumas[0]
    for time, luma in zip(times, lumas):
        if luma >= level > previous and time - last > MIN_GAP:
            onsets.append(time)
            last = time
        previous = luma
    return onsets, 1 / statistics.median(b - a for a, b in zip(times, times[1:]))


def beep_envelope(path):
    """1 ms peak envelope of the 1 kHz band, as (values, step seconds)."""
    command = ['ffmpeg', '-hide_banner', '-nostdin', '-loglevel', 'error', '-i', path, '-vn',
               '-af', BEEP_BAND, '-ac', '1', '-ar', str(AUDIO_RATE), '-f', 's16le', '-']
    samples = array.array('h', subprocess.run(command, capture_output=True, check=True).stdout)
    envelope = [max(map(abs, samples[i:i + WINDOW])) for i in range(0, len(samples) - WINDOW, WINDOW)]
    return envelope, WINDOW / AUDIO_RATE


def beep_near(envelope, step, flash, threshold):
    """Onset of the strongest beep within MAX_PAIR of a flash: the first point,
    walking back from the local peak, still above threshold of that peak."""
    first = max(0, int((flash - MAX_PAIR) / step))
    last = min(len(envelope), int((flash + MAX_PAIR) / step))
    if last - first < 2:
        return None
    # Pick the peak on a 20 ms average: a beep lasts 80 ms, a click a few.
    def smoothed(i):
        return sum(envelope[max(0, i - SMOOTH):i + 1])
    peak = max(range(first, last), key=smoothed)
    peak = max(range(max(first, peak - SMOOTH), peak + 1), key=envelope.__getitem__)
    level = threshold * envelope[peak]
    onset = peak
    while onset > first and envelope[onset - 1] >= level:
        onset -= 1
    # A beep stands well above the floor of its window; otherwise it is noise.
    floor = statistics.median(envelope[first:last])
    return onset * step if envelope[peak] > 4 * floor else None


def main():
    parser = argparse.ArgumentParser(description='Measure A/V sync of a filmed avsync clip.')
    parser.add_argument('recording')
    parser.add_argument('--threshold', type=float, default=0.5)
    args = parser.parse_args()

    flashes, fps = flash_onsets(args.recording, args.threshold)
    envelope, step = beep_envelope(args.recording)
    offsets = []
    for flash in flashes:
        beep = beep_near(envelope, step, flash, args.threshold)
        if beep is not None:
            offsets.append((beep - flash) * 1000)
    print(f'video {fps:.0f} fps (risoluzione ±{500 / fps:.0f} ms) · lampi {len(flashes)} · con bip {len(offsets)}')
    if not offsets:
        raise SystemExit('nessuna coppia lampo/bip trovata')
    print('scarti (ms, positivo = audio in ritardo): ' + ' '.join(f'{o:+.0f}' for o in offsets))
    median = statistics.median(offsets)
    kept = [o for o in offsets if abs(o - median) <= OUTLIER_MS]
    spread = statistics.stdev(kept) if len(kept) > 1 else 0
    print(f'mediana {median:+.0f} ms · media {statistics.mean(kept):+.0f} ms · deviazione {spread:.0f} ms '
          f'· scartati {len(offsets) - len(kept)} (oltre {OUTLIER_MS} ms dalla mediana: rumori)')

if __name__ == '__main__':
    main()
