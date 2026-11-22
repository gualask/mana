import argparse
import json
from pathlib import Path
import signal
import threading
from .live import SEGMENT_SECONDS
from .profiles import live_endpoints
from .server import Lab


def main():
    parser = argparse.ArgumentParser(description='Synthetic streaming lab; no upstream traffic.')
    parser.add_argument('--api-port', type=int, default=0)
    parser.add_argument('--resolver-port', type=int, default=0)
    parser.add_argument('--media-port', type=int, default=0)
    parser.add_argument('--write-config', type=Path, default=Path('private/lab'))
    parser.add_argument('--host', default='127.0.0.1',
                        help='IP address to listen on; a local network address makes the lab reachable by the TV')
    parser.add_argument('--tls', action='store_true', help='serve HTTPS with a generated certificate')
    parser.add_argument('--live', action='store_true', help='also serve real-time live HLS channels and a continuous MPEG-TS one')
    parser.add_argument('--segment-seconds', type=int, default=SEGMENT_SECONDS,
                        help='live segment duration; real channels use 4 to 10 s')
    args = parser.parse_args()
    with Lab(api_port=args.api_port, resolver_port=args.resolver_port, media_port=args.media_port,
             host=args.host, tls=args.tls, live=args.live, segment_seconds=args.segment_seconds) as lab:
        config = lab.write_config(args.write_config)
        summary = {'origins': lab.state.origins, 'fixture_config': str(config),
                   'public_hls': lab.state.origins['media'] + '/public/master.m3u8'}
        summary.update(live_endpoints(lab.state))
        print(json.dumps(summary), flush=True)
        # Stopped with SIGTERM as with Ctrl-C: leaving the `with` stops the
        # encoders, which would otherwise outlive the lab.
        signal.signal(signal.SIGTERM, signal.default_int_handler)
        try:
            threading.Event().wait()
        except KeyboardInterrupt:
            pass


if __name__ == '__main__':
    main()
