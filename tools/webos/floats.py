#!/usr/bin/env python3
"""Check that the TV binary does its floating point maths with VFP.

The webOS userland uses the soft-float calling convention; .cargo/config.toml
asks for VFP and NEON instructions within it. Should those flags be lost, the
app's maths would go through software routines (__aeabi_dadd and the like)
and still run, only slower. This check fails when a function of the app
calls such a routine, or when the app has no VFP instruction at all.

Only functions whose names mention the app's crates count: they, generic
code of the standard library among them, are compiled with the app's flags.
The precompiled standard library is built for the target's defaults, so its
float parsing and the rounding functions (roundf, round) stay in software
until it can be rebuilt (-Zbuild-std, nightly only).

It needs the binary with its symbol table, as cargo leaves it in target/
(target.APP_BINARY): the package strips it after the check. Disassembly
uses an LLVM objdump (macOS's own, or llvm-objdump).

Usage: python3 tools/webos/floats.py [BINARY]
"""
import re
import shutil
import subprocess
import sys
from pathlib import Path

from target import APP_BINARY, APP_NAME

# Functions of the app's crates, by their demangled names: the binary's
# (apps/tv) and the library's, `mana_core`.
APP_FUNCTION = re.compile(rf'(?<![\w:]){APP_NAME}(?:_core)?::')
# Software floating point, in the ARM EABI names and in the libgcc ones.
# Conversions between floats and 64-bit integers are left out: VFPv3 has
# no instruction for them, so VFP code calls them too.
SOFT_FLOAT = re.compile(
    r'__aeabi_(?:c?[fd](?:add|sub|rsub|mul|div|neg|cmp\w*)|[fd]2[fd]|[fd]2u?iz|u?i2[fd])'
    r'|__(?:add|sub|mul|div|neg)[sd]f3|__(?:eq|ne|lt|le|gt|ge|un)[sd]f2'
    r'|__extendsfdf2|__truncdfsf2|__fix(?:uns)?[sd]fsi|__float(?:un)?si[sd]f')
# The libm rounding functions: without VRINT (ARMv7) they work on the
# float's bits in software. Not counted by the check: the precompiled
# standard library calls them for f32::round and f64::round.
SOFT_ROUNDING = re.compile(r'roundf?')
# The line that starts a function, as `0002804e4 <name>:`.
FUNCTION_START = re.compile(r'^[0-9a-f]+ <(.+)>:$', re.MULTILINE)
# A direct call or tail call, as `bl 0x2804e4 <name>`.
CALL = re.compile(r'\s(?:bl|blx|b)(?:eq|ne|cs|cc|hs|lo|mi|pl|vs|vc|hi|ls|ge|lt|gt|le|al)?\s+(?:0x)?[0-9a-f]+ <([^>+]+)>')
FLOAT_INSTRUCTION = re.compile(r'\sv\w+\.f(?:32|64)\s')


def objdump():
    for name in ('llvm-objdump', 'objdump'):
        if path := shutil.which(name):
            return path
    raise SystemExit('floats: no objdump found (macOS has one; elsewhere install llvm-objdump)')


def tally(disassembly):
    """In `disassembly`, the app's functions, their VFP instructions and
    their calls to software float routines, and the routines called outside
    the app."""
    app_functions, float_instructions, soft_calls, runtime_soft = 0, 0, [], set()
    # re.split with a group gives the text before the first function, then
    # name and body of each.
    parts = FUNCTION_START.split(disassembly)
    for function, body in zip(parts[1::2], parts[2::2]):
        calls = [name for name in CALL.findall(body) if SOFT_FLOAT.fullmatch(name)]
        if not APP_FUNCTION.search(function):
            runtime_soft.update(calls)
            continue
        app_functions += 1
        soft_calls += [(function, name) for name in calls]
        float_instructions += len(FLOAT_INSTRUCTION.findall(body))
    return app_functions, float_instructions, soft_calls, runtime_soft


def check(binary):
    """Raises SystemExit unless the app's functions in `binary` use VFP and
    call no software float routine; returns a one-line summary."""
    result = subprocess.run([objdump(), '-d', '-C', '--no-show-raw-insn', str(binary)],
                            capture_output=True, text=True)
    if result.returncode or 'elf32-littlearm' not in result.stdout[:1000]:
        raise SystemExit(f'floats: cannot disassemble {binary} as 32-bit ARM: {result.stderr.strip()[:300]}')
    app_functions, float_instructions, soft_calls, runtime_soft = tally(result.stdout)
    if not app_functions:
        raise SystemExit(f'floats: no function of the app in {binary}: is it stripped? '
                         'The release profile must keep the symbols (strip = false)')
    if soft_calls:
        listed = '\n'.join(f'  {f} -> {r}' for f, r in list(dict.fromkeys(soft_calls))[:10])
        raise SystemExit(f'floats: {len(soft_calls)} software float calls in the app; '
                         f'are the VFP flags of .cargo/config.toml applied?\n{listed}')
    if not float_instructions:
        raise SystemExit(f'floats: no VFP instruction in the {app_functions} functions of the app; '
                         'are the VFP flags of .cargo/config.toml applied?')
    runtime = ', '.join(sorted(runtime_soft)) or 'none'
    return (f'floats: {float_instructions} VFP instructions in {app_functions} app functions, '
            f'no software float call; outside the app (standard library, dependencies): {runtime}')


if __name__ == '__main__':
    print(check(Path(sys.argv[1]) if len(sys.argv) > 1 else APP_BINARY))
