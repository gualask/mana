"""Decoders used by synthetic fixture clients; no network access. AES-GCM uses Node."""
import json
import re
import shutil
import struct
import subprocess

DECRYPT = """
const crypto = require('node:crypto'), fs = require('node:fs');
const p = JSON.parse(fs.readFileSync(0, 'utf8'));
const b = n => Buffer.from(p.pool[p.order[n]], 'base64url');
const key = Buffer.from(b(0).map((v, i) => v ^ b(1)[i]));
const data = b(3), d = crypto.createDecipheriv('aes-256-gcm', key, b(2));
d.setAAD(b(4)); d.setAuthTag(data.subarray(-16));
process.stdout.write(Buffer.concat([d.update(data.subarray(0, -16)), d.final()]));
"""

def decode_envelope(html):
    match = re.search(r'const _0xp=(\[.*?\]),_0xi=(\[.*?\]);', html)
    if not match:
        raise ValueError('Unknown player envelope shape')
    pool, order = json.loads(match[1]), json.loads(match[2])
    if (len(pool) != 11 or len(order) != 11 or sorted(order) != list(range(11))
            or not all(isinstance(value, str) and len(value) < 10000 for value in pool)):
        raise ValueError('Unexpected player envelope fields')
    if not shutil.which('node'):
        raise ValueError('Node required for observed AES-GCM envelope')
    result = subprocess.run(['node', '-e', DECRYPT],
                            input=json.dumps({'pool': pool, 'order': order}),
                            capture_output=True, text=True, timeout=5)
    if result.returncode:
        raise ValueError('Player envelope authentication failed')
    value = json.loads(result.stdout)
    if not isinstance(value.get('u'), str):
        raise ValueError('Missing target in player envelope')
    return value

def extract_embedded_streams(html):
    decoder = json.JSONDecoder()
    result = {}
    # Decode serialized data only; no eval or execution of JavaScript.
    for match in re.finditer(r'self\.__next_f\.push\(', html):
        try:
            argument, _ = decoder.raw_decode(html[match.end():])
        except ValueError:
            continue
        if not isinstance(argument, list) or len(argument) < 2 or not isinstance(argument[1], str):
            continue
        for marker in re.finditer(r'"streams"\s*:\s*', argument[1]):
            try:
                rows, _ = decoder.raw_decode(argument[1][marker.end():])
            except ValueError:
                continue
            if isinstance(rows, list):
                for row in rows:
                    if isinstance(row, dict) and 'streaming_link' in row:
                        key = row.get('_id') or row['streaming_link']
                        result[key] = row
    return list(result.values())

def varint(data, pos):
    value = 0
    for shift in range(0, 70, 7):
        if pos >= len(data):
            raise ValueError('Truncated Protobuf varint')
        byte = data[pos]
        pos += 1
        value |= (byte & 127) << shift
        if byte < 128:
            return value, pos
    raise ValueError('Oversized Protobuf varint')

def decode_binary(data):
    result = {}
    pos = 0
    while pos < len(data):
        tag, pos = varint(data, pos)
        field, wire = tag >> 3, tag & 7
        if not field:
            raise ValueError('Invalid Protobuf field')
        if wire == 0:
            value, pos = varint(data, pos)
        elif wire == 2:
            size, pos = varint(data, pos)
            end = pos + size
            if end > len(data):
                raise ValueError('Truncated Protobuf field')
            value, pos = data[pos:end], end
        elif wire in (1, 5):
            size = 8 if wire == 1 else 4
            if pos + size > len(data):
                raise ValueError('Truncated Protobuf number')
            value = struct.unpack('<d' if wire == 1 else '<f', data[pos:pos + size])[0]
            pos += size
        else:
            raise ValueError(f'Unsupported Protobuf wire type {wire}')
        result.setdefault(field, []).append(value)
    return result

def first_field(message, field, default=None):
    return message.get(field, [default])[0]

def rot47(value):
    return ''.join(chr((ord(c) - 33 + 47) % 94 + 33) if 33 <= ord(c) <= 126 else c for c in value)
