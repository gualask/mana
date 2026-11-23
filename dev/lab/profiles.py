"""Write neutral fixture endpoints and a source file for the public HLS."""
import json
from pathlib import Path
from .state import UA


def live_endpoints(state):
    """Endpoint name -> URL of each live channel: its master playlist, and
    the first channel's continuous MPEG-TS answer."""
    media = state.origins['media']
    endpoints = {f'{name}_hls': f'{media}/{name}/master.m3u8' for name in state.channels}
    if state.live is not None:
        endpoints['live_ts'] = f'{media}/live/stream.ts'
    return endpoints


def write_config(state, directory, certificate=None):
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    origins = state.origins
    lines = ['fixture_manifest_version = 1', 'user_agent = ' + json.dumps(UA), '', '[origins]']
    lines += [role + ' = ' + json.dumps(origin) for role, origin in origins.items()]
    lines += ['', '[media_headers]'] + [json.dumps(key) + ' = ' + json.dumps(value) for key, value in state.headers().items()]
    lines += ['', '[endpoints]', 'catalogue = ' + json.dumps(origins['api'] + '/catalogue.json'),
              'public_hls = ' + json.dumps(origins['media'] + '/public/master.m3u8'),
              'm3u = ' + json.dumps(origins['api'] + '/channels.m3u'),
              'wrapper = ' + json.dumps(origins['resolver'] + '/wrapper'),
              'session = ' + json.dumps(origins['api'] + '/session.pb')]
    lines += [f'{key} = ' + json.dumps(url) for key, url in live_endpoints(state).items()]
    if certificate:
        (directory / 'lab-ca.pem').write_bytes(Path(certificate).read_bytes())
        lines += ['', '[tls]', 'root_certificate = ' + json.dumps(str(directory / 'lab-ca.pem'))]
    (directory / 'endpoints.toml').write_text('\n'.join(lines) + '\n')
    source = '''# A source file (docs/configuration.md) listing the public synthetic HLS,
# which also plays directly with an ordinary HLS player while the lab runs.
id = "lab-direct"
name = "Mana synthetic HLS"

[catalogue]
items = [{ id = "synthetic", title = "Generated bars and tone", url = MEDIA_URL }]

[[resolve.steps]]
id = "stream"
op = "stream"
url = "{input.candidate.url}"
'''.replace('MEDIA_URL', json.dumps(origins['media'] + '/public/master.m3u8'))
    (directory / 'direct.toml').write_text(source)
    return directory / 'endpoints.toml'
