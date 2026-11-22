"""The TV's Rust target, the tools that build for it and the app binary,
shared by the webOS tools and the measurements in dev/tv."""
import os
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGET = 'armv7-unknown-linux-gnueabi'
# The glibc of webOS 6, the oldest supported: cargo zigbuild links against
# it, and the package refuses a library that needs a newer one.
GLIBC = (2, 28)
ZIGBUILD_TARGET = f'{TARGET}.{GLIBC[0]}.{GLIBC[1]}'
# The linker of the app and the compiler of FFmpeg, besides the Rust that
# rust-toolchain.toml fixes. A new version is adopted by changing it here
# once the package's checks pass with it and the app plays on the TV.
TOOLS = {'zig': '0.17.1', 'cargo-zigbuild': '0.24.1'}


def binary_name(manifest):
    """The name of the one binary the crate at `manifest` declares."""
    package = tomllib.loads(manifest.read_text())
    names = [b['name'] for b in package.get('bin', [])] or [package['package']['name']]
    if len(names) != 1:
        raise SystemExit(f'{manifest}: {len(names)} binaries, expected one')
    return names[0]


# The app's binary: the package's `main`, the process on the TV.
APP_NAME = binary_name(ROOT / 'apps' / 'tv' / 'Cargo.toml')
# As cargo leaves it, with the symbols the float check and the profiler read.
APP_BINARY = ROOT / 'target' / TARGET / 'release' / APP_NAME


def check_tools():
    """Raises SystemExit unless zig and cargo-zigbuild are the versions in
    TOOLS; returns a one-line summary."""
    found = {}
    for tool, flag in (('zig', 'version'), ('cargo-zigbuild', '--version')):
        try:
            output = subprocess.run([tool, flag], capture_output=True, text=True, check=True).stdout
        except (OSError, subprocess.CalledProcessError) as error:
            raise SystemExit(f'{tool}: {error}')
        found[tool] = output.split()[-1] if output.split() else ''
    wrong = [f'{tool} {found[tool]}, expected {version}' for tool, version in TOOLS.items() if found[tool] != version]
    if wrong:
        raise SystemExit('tools: ' + '; '.join(wrong) + ' (tools/webos/target.py)')
    return 'tools: ' + ', '.join(f'{tool} {version}' for tool, version in TOOLS.items())


def zigbuild(manifest, target_dir, rustflags=None):
    """Builds the crate at `manifest` for the TV in release, with
    `rustflags` in place of those of .cargo/config.toml when given, and
    returns its binary."""
    check_tools()
    environment = {**os.environ, 'RUSTFLAGS': rustflags} if rustflags else None
    result = subprocess.run(['cargo', 'zigbuild', '--quiet', '--release', '--target', ZIGBUILD_TARGET,
                             '--manifest-path', str(manifest), '--target-dir', str(target_dir)],
                            cwd=ROOT, env=environment, capture_output=True, text=True)
    if result.returncode:
        raise SystemExit(result.stderr)
    return target_dir / TARGET / 'release' / binary_name(manifest)
