#!/usr/bin/env python3
"""Validate draft source schema/examples and rejection invariants.
Run with: uv run --no-project --with jsonschema python dev/checks/check-source-profiles.py
This checks configuration structure, not a source engine or playback compatibility.
"""
from copy import deepcopy
import json
from pathlib import Path
import sys
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[2]


def main():
    schema = json.loads((ROOT / 'schemas/source.schema.json').read_text())
    Draft202012Validator.check_schema(schema)
    validator = Draft202012Validator(schema)
    profiles = {}
    for path in sorted((ROOT / 'config/examples').glob('*.json')):
        profile = json.loads(path.read_text())
        validator.validate(profile)
        if profile['enabled']:
            raise AssertionError('Design examples must stay disabled')
        profiles[path.stem] = profile
    invalid = []
    def altered(name, change):
        value = deepcopy(profiles[name])
        change(value)
        invalid.append(value)
    altered('unverified-demo', lambda p: p.update(enabled=True))
    altered('direct-demo', lambda p: p.update(unknown_option=True))
    altered('direct-demo', lambda p: p['catalogue']['entries'][0].update(url='javascript:run()'))
    altered('direct-demo', lambda p: p['http']['headers_by_role'].update(manifest={'Referer': 'a\r\nX-Injected: yes'}))
    altered('direct-demo', lambda p: p['lifecycle'].update(max_network_retries=100))
    altered('direct-demo', lambda p: p['lifecycle'].update(max_refresh_attempts=1))
    altered('resolver-demo', lambda p: p['resolution'].update(rules=[]))
    altered('resolver-demo', lambda p: p['resolution'].update(max_depth=100))
    altered('adapter-demo', lambda p: p['resolution']['adapter'].update(version=0))
    for index, profile in enumerate(invalid):
        if validator.is_valid(profile):
            raise AssertionError(f'Invalid configuration accepted: case {index + 1}')
    # A refresh hook becomes structurally valid when a versioned adapter is supplied.
    refresh = deepcopy(profiles['adapter-demo'])
    refresh['lifecycle'].update(max_refresh_attempts=1, refresh_adapter={'id': 'synthetic-session', 'version': 1})
    validator.validate(refresh)
    print(f'PASS: schema; {len(profiles)} disabled profiles; {len(invalid)} rejection cases; versioned refresh contract.')
    print('Runtime capability/semantic checks and live playback are not covered by this validator.')


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(f'FAIL: {error}', file=sys.stderr)
        sys.exit(1)
