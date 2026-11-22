"""Write neutral fixture endpoints and a disabled TOML source profile."""
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
    source = '''# Disabled source profile conforming to the current v1 data schema.
# Only a future Mana loader will execute this profile; public_hls also works
# directly with an ordinary HLS player while the lab is running.
schema_version = 1
id = "lab-direct"
name = "Mana synthetic HLS"
enabled = false

[catalogue]
kind = "direct"
[[catalogue.entries]]
id = "synthetic"
title = "Generated bars and tone"
url = MEDIA_URL

[resolution]
kind = "direct"

[http]
timeout_seconds = 10
max_redirects = 3
cookie_scope = "per_source_and_domain"
cross_origin_credentials = "never_automatic"
[http.headers_by_role.catalogue]
User-Agent = "StreamLab/1"
[http.headers_by_role.resolver]
User-Agent = "StreamLab/1"

[lifecycle]
resolve_on_play = true
expiry = "not_applicable"
max_network_retries = 1
respect_retry_after = true
max_refresh_attempts = 0

[playback]
allowed_formats = ["hls"]
detect_from_body = true
require_device_capability_check = true
'''.replace('MEDIA_URL', json.dumps(origins['media'] + '/public/master.m3u8'))
    (directory / 'direct-v1.toml').write_text(source)
    return directory / 'endpoints.toml'
