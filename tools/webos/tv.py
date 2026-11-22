"""SSH access to the rooted TV, shared by the webOS tools.

Settings come from the environment or from private/tv.env (KEY=VALUE lines,
not versioned):
  TV_SSH   SSH executable or wrapper (default: ssh)
  TV_HOST  user@address of the rooted TV (required)
"""
import os
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
APP_ID = 'dev.mana.app'
APP_SOURCES = ROOT / 'apps' / 'tv' / 'src'


def app_path(name):
    """The TV path the app declares as `const NAME: &str = "…";` in its
    sources: the app owns where it reads and writes on the TV."""
    pattern = re.compile(rf'const {name}: &str = "([^"]+)";')
    found = {m[1] for source in APP_SOURCES.rglob('*.rs') for m in pattern.finditer(source.read_text())}
    if len(found) != 1:
        raise SystemExit(f'{name}: {len(found)} definitions in {APP_SOURCES}, expected one')
    return found.pop()


REMOTE_LOG = app_path('LOG_FILE')


class Tv:
    def __init__(self):
        values = {}
        env_file = ROOT / 'private' / 'tv.env'
        if env_file.is_file():
            for line in env_file.read_text().splitlines():
                if '=' in line and not line.lstrip().startswith('#'):
                    key, value = line.split('=', 1)
                    values[key.strip()] = value.strip()
        values.update({key: os.environ[key] for key in ('TV_SSH', 'TV_HOST') if key in os.environ})
        if 'TV_HOST' not in values:
            raise SystemExit('TV_HOST missing: set it in the environment or in private/tv.env')
        self.ssh, self.host = values.get('TV_SSH', 'ssh'), values['TV_HOST']

    def argv(self, command, tty=False):
        # luna-send buffers its output when stdout is a pipe; a remote
        # terminal (-tt) makes it print each reply as it arrives.
        return [self.ssh, *(['-tt'] if tty else []), '-o', 'BatchMode=yes', self.host, command]

    def run(self, command, stdin=None, tty=False):
        return subprocess.run(self.argv(command, tty), input=stdin, capture_output=True, check=False)

    def read(self, path):
        return self.run(f'cat {path}').stdout
