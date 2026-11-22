"""Self-signed certificate for serving the lab over HTTPS on the local network."""
import ipaddress
from pathlib import Path
import shutil
import ssl
import subprocess


def server_context(directory, host):
    """Creates a certificate authority and a server certificate for `host` (an
    IP address) in `directory`; returns the server context and the authority
    certificate, which clients add as a trusted root."""
    if not shutil.which('openssl'):
        raise RuntimeError('OpenSSL is required to serve the lab over HTTPS')
    ipaddress.ip_address(host)  # only IP literals: the TV resolves no lab names
    directory = Path(directory)
    ca, ca_key = directory / 'lab-ca.pem', directory / 'lab-ca-key.pem'
    cert, key = directory / 'lab-server.pem', directory / 'lab-server-key.pem'
    common = ['openssl', 'req', '-x509', '-newkey', 'ec', '-pkeyopt', 'ec_paramgen_curve:prime256v1',
              '-nodes', '-days', '2']
    run(common + ['-subj', '/CN=Mana synthetic lab CA',
                  '-addext', 'basicConstraints=critical,CA:TRUE',
                  '-addext', 'keyUsage=critical,keyCertSign,cRLSign',
                  '-keyout', str(ca_key), '-out', str(ca)])
    run(common + ['-CA', str(ca), '-CAkey', str(ca_key), '-subj', '/CN=Mana synthetic lab',
                  '-addext', f'subjectAltName=IP:{host},IP:127.0.0.1',
                  '-addext', 'basicConstraints=critical,CA:FALSE',
                  '-addext', 'extendedKeyUsage=serverAuth',
                  '-keyout', str(key), '-out', str(cert)])
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(cert, key)
    return context, ca


def run(command):
    result = subprocess.run(command, capture_output=True, text=True, timeout=10)
    if result.returncode:
        raise RuntimeError('Certificate generation failed: ' + result.stderr[-500:])
