#!/usr/bin/env python3
"""Check TOML/JSON data equivalence; no flow engine or network execution.

Python 3.11+; run with uv run --no-project --with jsonschema python dev/checks/check-config-formats.py.
"""
from datetime import date, datetime, time
import json
import math
from pathlib import Path
import sys
import tomllib
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[2]
EXAMPLES = ROOT / 'config/design-examples'


def common_values(value):
    """Experimental profile value policy: no native dates, NaN or infinity."""
    if isinstance(value, (datetime, date, time)):
        raise ValueError('Dates and timestamps must be explicit strings')
    if isinstance(value, float) and not math.isfinite(value):
        raise ValueError('Numbers must be finite')
    if isinstance(value, dict):
        for key, child in value.items():
            if not isinstance(key, str):
                raise ValueError('Object keys must be strings')
            common_values(child)
    elif isinstance(value, list):
        for child in value:
            common_values(child)
    return value


def same_data(left, right):
    return json.dumps(left, sort_keys=True, allow_nan=False) == json.dumps(right, sort_keys=True, allow_nan=False)


def read_toml(name):
    return common_values(tomllib.loads((EXAMPLES / name).read_text()))


def main():
    expected = json.loads((ROOT / 'config/examples/resolver-demo.json').read_text())
    actual = read_toml('resolver-v1.toml')
    assert same_data(actual, expected), 'Current TOML and JSON profiles differ'
    schema = json.loads((ROOT / 'schemas/source.schema.json').read_text())
    validator = Draft202012Validator(schema)
    validator.validate(actual)
    invalid = json.loads(json.dumps(actual))
    invalid['resolution']['max_depth'] = 100
    assert not validator.is_valid(invalid), 'Same schema must reject invalid TOML-derived data'

    flow = read_toml('fixture-flow.toml')
    assert same_data(flow, json.loads((EXAMPLES / 'fixture-flow.json').read_text())), 'Flow representations differ'
    assert not validator.is_valid(flow), 'Flow proposal must not masquerade as a v1 profile'
    expected_steps = {'catalogue': ['catalogue', 'events'], 'sources': ['detail', 'signals'], 'resolve': ['stream']}
    for name, ids in expected_steps.items():
        assert [step['id'] for step in flow['flows'][name]['steps']] == ids, 'Step order changed'
    routes = flow['resolver_sets']['fixture_players']['routes']
    assert [route['use'] for route in routes] == ['direct-hls@1', 'synthetic-envelope@1']
    assert flow['flows']['sources']['steps'][1]['items_pointer'] == '/signals'
    assert 'http_profile' not in flow['flows']['sources']['steps'][1], 'Table scope leaked to next step'

    try:
        tomllib.loads('id = "a"\nid = "b"\n')
    except tomllib.TOMLDecodeError:
        pass
    else:
        raise AssertionError('Duplicate TOML key accepted')
    for text in ['expires_at = 2026-11-15T09:00:00Z', 'timeout = nan', 'timeout = inf']:
        try:
            common_values(tomllib.loads(text))
        except ValueError:
            pass
        else:
            raise AssertionError('Out-of-contract value accepted')
    assert common_values(tomllib.loads('expires_at = "2026-11-15T09:00:00Z"'))['expires_at'].endswith('Z')
    print('PASS: current TOML/JSON profile equality and shared schema/rejection; proposed flow equality; ordered steps/routes; table scope; duplicate/date/nonfinite rejection.')
    print('No Rust parser, execution plan, live requests or flow semantics tested.')


if __name__ == '__main__':
    try:
        main()
    except (AssertionError, ValueError) as error:
        print(f'FAIL: {error}', file=sys.stderr)
        sys.exit(1)
