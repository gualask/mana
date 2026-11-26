import argparse
import json
from pathlib import Path
import signal
import sys
import threading
from .live import SEGMENT_SECONDS
from .profiles import live_endpoints
from .server import LIVE_CHANNELS, Lab


def main():
    args = arguments()
    live = False if args.live is None else args.live or True
    with Lab(api_port=args.api_port, resolver_port=args.resolver_port, media_port=args.media_port,
             host=args.host, tls=args.tls, live=live, segment_seconds=args.segment_seconds) as lab:
        config = lab.write_config(args.write_config)
        summary = {'origins': lab.state.origins, 'fixture_config': str(config),
                   'public_hls': lab.state.public_url()}
        summary.update(live_endpoints(lab.state))
        print(json.dumps(summary), flush=True)
        wait_for_stop(args.until_stdin_closes)


def arguments():
    parser = argparse.ArgumentParser(description='Synthetic streaming lab; no upstream traffic.')
    parser.add_argument('--api-port', type=int, default=0)
    parser.add_argument('--resolver-port', type=int, default=0)
    parser.add_argument('--media-port', type=int, default=0)
    parser.add_argument('--write-config', type=Path, default=Path('private/lab'))
    parser.add_argument('--host', default='127.0.0.1',
                        help='IP address to listen on; a local network address makes the lab reachable by the TV')
    parser.add_argument('--tls', action='store_true', help='serve HTTPS with a generated certificate')
    parser.add_argument('--live', nargs='*', choices=LIVE_CHANNELS,
                        help='also serve real-time live HLS channels and a continuous MPEG-TS one; '
                             'with names, only those channels')
    parser.add_argument('--until-stdin-closes', action='store_true',
                        help='stop when standard input closes, so that a test harness ending takes the lab with it')
    parser.add_argument('--segment-seconds', type=int, default=SEGMENT_SECONDS,
                        help='live segment duration; real channels use 4 to 10 s')
    return parser.parse_args()


def wait_for_stop(until_stdin_closes):
    """Returns on Ctrl-C or SIGTERM, or once standard input closes when
    `until_stdin_closes`: leaving the lab's `with` then stops the encoders,
    which would otherwise outlive it."""
    signal.signal(signal.SIGTERM, signal.default_int_handler)
    closed = threading.Event()

    def watch_stdin():
        sys.stdin.buffer.read()
        closed.set()

    if until_stdin_closes:
        threading.Thread(target=watch_stdin, daemon=True).start()
    try:
        closed.wait()
    except KeyboardInterrupt:
        pass


if __name__ == '__main__':
    main()
