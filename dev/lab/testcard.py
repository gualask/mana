"""The lab's test card and how FFmpeg encodes it. The picture flashes white
while a 1 kHz beep plays once a second, so A/V sync can be measured on the
TV with dev/tv/avsync.py."""

RATE = 25
FLASH_FRAMES = 2
BEEP_SECONDS = 0.08
# The AAC encoder delays audio by 1024 priming samples that MPEG-TS keeps:
# the beep is generated that much early so it plays with the flash.
AAC_PRIMING_SECONDS = 1024 / 48000
VIDEO_CODECS = {
    'h264': ['-c:v', 'libx264', '-preset', 'veryfast', '-profile:v', 'high', '-level', '4.1'],
    'hevc': ['-c:v', 'libx265', '-preset', 'ultrafast', '-x265-params', 'log-level=error'],
}


def sources(size='1920x1080'):
    """FFmpeg lavfi sources: a test card with a sweeping bar and a white flash
    on each second, and a beep on each second."""
    flash = FLASH_FRAMES / RATE
    video = (f'testsrc=size={size}:rate={RATE},'
             "drawbox=x='mod(t*480,iw)':y=0:w=40:h=ih:color=white:t=fill,"
             f"drawbox=x=0:y=0:w=iw:h=ih:color=white:t=fill:enable='lt(mod(t,1),{flash})'")
    beep_time = f'(t+{AAC_PRIMING_SECONDS:.6f})'
    beep = f'if(lt(mod({beep_time},1),{BEEP_SECONDS}),0.7*sin(2*PI*1000*t),0)'
    audio = f"aevalsrc=exprs='{beep}|{beep}':sample_rate=48000:channel_layout=stereo"
    return video, audio


def scaled_graph(sizes):
    """One lavfi graph for every stream, as two real-time inputs would start a
    few tens of milliseconds apart, a variable A/V offset: the test card at
    the first size scaled to each of `sizes`, as outputs 0 to n - 1, and the
    beep as output n."""
    video, audio = sources(sizes[0])
    count = len(sizes)
    scaled = ';'.join(f'[v{i}]scale={size.replace("x", ":")}[out{i}]' for i, size in enumerate(sizes))
    return f'{video},split={count}{"".join(f"[v{i}]" for i in range(count))};{scaled};{audio}[out{count}]'


def variant_streams(bitrates):
    """FFmpeg options for the outputs of `scaled_graph`: one video stream at
    each of `bitrates` with the beep, each pair an HLS variant."""
    options = []
    for i, bitrate in enumerate(bitrates):
        options += ['-map', f'0:v:{i}', '-map', '0:a',
                    f'-b:v:{i}', bitrate, f'-maxrate:v:{i}', bitrate, f'-bufsize:v:{i}', bitrate]
    return options + ['-var_stream_map', ' '.join(f'v:{i},a:{i}' for i in range(len(bitrates)))]


def bits(bitrate):
    """Bits per second of an FFmpeg bitrate such as `4M` or `800k`."""
    scale = {'k': 1_000, 'M': 1_000_000}.get(bitrate[-1], 1)
    return int(float(bitrate.rstrip('kM')) * scale)
