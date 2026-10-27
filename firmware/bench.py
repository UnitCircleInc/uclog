#!/usr/bin/env python3
# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0

"""Performance and footprint of ucLog on a board (plan 08 §6), as JSON for the docs.

  python3 firmware/bench.py --board-dir firmware/nucleo-l452re-p --build BUILD \\
      --device serial:/dev/cu.usbmodem1103 [--device hid:SERIAL ...] [-o website/bench/BOARD.json]

The board runs the test firmware built in BUILD (its app.elf and app.map). With the first
--device, the firmware's `bench` command (app/bench.c): the bytes and cycles of each log
statement, beside snprintf. With each --device (one per link): `burst` (throughput) and a round
trip through the hub (port 5 echo). From the link map: the library's flash and RAM; and the
flash snprintf costs, from two small programs built with the board's flags. On this machine:
the decode rate of the Python library and of `uclog decode`.

Needs the Python library (python/src) and the native tools (native/target/release) of this
repository, and the board's Arm GCC.
"""

import argparse
import datetime
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / 'python/src'))
import uclog  # noqa: E402
from uclog import proto  # noqa: E402

NATIVE = ROOT / 'native/target/release'
TERM = 61
ECHO = 5  # the test firmware echoes this port

# The library's objects, as the boards' Makefiles name them (obj/device/...).
# (ucComm's crc32c.o is built with them, but only the test app uses it.)
LIBRARY = ['device/src/log.o', 'device/src/log_plot.o', 'device/src/semihost.o', 'device/src/syscalls.o',
           'device/uccomm/cb.o', 'device/uccomm/cobs.o', 'device/uccomm/cbor.o']


def statements():
  """id → the C of each measured statement, from app/bench.c (the one source of truth)."""
  src = (ROOT / 'firmware/app/bench.c').read_text()
  return {m[1]: f'LOG_INFO({m[2]});' for m in re.finditer(r'MEASURE\(port, "(\w+)", (.*)\);', src)}


def terminal(dev, line, until, timeout=60.0):
  """Send a terminal line; the text that comes back, once it matches the regex `until`."""
  while dev.recv(TERM, timeout=0.2) is not None:  # what an earlier command left (its prompt)
    pass
  dev.send(TERM, line + '\r')
  buf, end = '', time.monotonic() + timeout
  while not re.search(until, buf):
    if time.monotonic() > end:
      raise TimeoutError(f'{line!r}: no {until!r} in {buf[-300:]!r}')
    d = dev.recv(TERM, timeout=1.0)
    if d:
      buf += d.decode('utf-8', 'replace')
  return buf


def bench(dev):
  out = terminal(dev, 'bench', 'bench end')
  start = re.search(r'bench start board=(\S+) hz=(\d+) n=(\d+)', out)
  code = statements()
  rows = []
  for m in re.finditer(r'bench (\w+) ((?:\w+=\d+ ?)+)', out):
    if m[1] in ('start', 'end'):
      continue
    row = {'id': m[1], 'code': code.get(m[1], '')}
    row.update({k: int(v) for k, v in re.findall(r'(\w+)=(\d+)', m[2])})
    rows.append(row)
  return start[1], int(start[2]), int(start[3]), rows


def throughput(dev, n=2000):
  link = re.search(r'link: (\w+)', terminal(dev, 'link', r'link: \w+'))[1]
  out = terminal(dev, f'burst {n}', r'\d+ dropped', timeout=120)
  m = re.search(r'burst of (\d+) logs: (\d+) bytes \((\d+)\.(\d) per log\) in (\d+) ms = (\d+) B/s, (\d+) dropped', out)
  logs, nbytes, ms, dropped = int(m[1]), int(m[2]), int(m[5]), int(m[7])
  return {'link': link, 'logs': logs, 'bytes': nbytes, 'ms': ms, 'dropped': dropped,
          'bytes_per_s': round(nbytes * 1000 / ms), 'logs_per_s': round(logs * 1000 / ms)}


def round_trip(dev, n=50, size=16):
  times = []
  for i in range(n):
    data = bytes([i]) * size
    t0 = time.perf_counter()
    dev.send(ECHO, data)
    while True:
      d = dev.recv(ECHO, timeout=2.0)
      if d is None:
        raise TimeoutError('no echo on port 5')
      if d == data:
        break
    times.append((time.perf_counter() - t0) * 1000)
  return {'bytes': size, 'n': n, 'median_ms': round(statistics.median(times), 2),
          'p90_ms': round(sorted(times)[int(n * 0.9)], 2)}


def map_footprint(map_path):
  """The library's linked sizes from the link map: flash (code, constants, initial data), RAM
  (data, bss), and the format strings (in LOGDATA: not loaded)."""
  text = Path(map_path).read_text(errors='replace')
  text = text[text.index('Linker script and memory map'):]
  objs = {o: {'flash': 0, 'ram': 0, 'logdata': 0} for o in LIBRARY}
  section = None
  for line in text.splitlines():
    m = re.match(r'^ (\.\S+|COMMON)(?:\s+0x([0-9a-f]+)\s+0x([0-9a-f]+)\s+(\S+))?$', line)
    if m:
      section = m[1]
      if not m[2]:
        continue
      size, path = int(m[3], 16), m[4]
    else:
      m = re.match(r'^\s+0x([0-9a-f]+)\s+0x([0-9a-f]+)\s+(\S+\.o)$', line)
      if not m or section is None:
        continue
      size, path = int(m[2], 16), m[3]
    obj = next((o for o in LIBRARY if path.endswith('/obj/' + o)), None)
    if obj is None or size == 0:
      continue
    if section.startswith('.logstr'):
      objs[obj]['logdata'] += size
    elif section.startswith(('.text', '.rodata')):
      objs[obj]['flash'] += size
    elif section.startswith('.data'):
      objs[obj]['flash'] += size
      objs[obj]['ram'] += size
    elif section.startswith(('.bss', 'COMMON', '.noinit')):
      objs[obj]['ram'] += size
  objs = {Path(o).name: v for o, v in objs.items() if any(v.values())}
  strings = re.search(r'^\.logdata\s+0x[0-9a-f]+\s+0x([0-9a-f]+)', text, re.M)
  return {'objects': objs, 'flash': sum(v['flash'] for v in objs.values()),
          'ram': sum(v['ram'] for v in objs.values()),
          # every format string of the firmware: in the ELF's LOGDATA, not in flash
          'format_strings': int(strings[1], 16) if strings else 0}


def printf_footprint(cc, cpu):
  """Flash for snprintf (newlib-nano), integers only and with floats: two small programs, each
  against the same program without it."""
  progs = {
    'none': 'int main(void) { return 0; }',
    'int': '#include <stdio.h>\nchar b[64]; volatile int v = 1;\n'
           'int main(void) { return snprintf(b, sizeof b, "%d %u %x %s", v, v, v, "s"); }',
    'float': '#include <stdio.h>\nchar b[64]; volatile int v = 1; volatile double d = 1.5;\n'
             'int main(void) { return snprintf(b, sizeof b, "%d %u %x %s %f", v, v, v, "s", d); }',
  }
  sizes = {}
  with tempfile.TemporaryDirectory() as t:
    for name, src in progs.items():
      c, elf = Path(t, f'{name}.c'), Path(t, f'{name}.elf')
      c.write_text(src)
      flags = [*cpu, '-Os', '-ffunction-sections', '-fdata-sections', '-Wl,--gc-sections',
               '--specs=nano.specs', '--specs=nosys.specs', '-Wl,--no-warn-rwx-segments']
      if name == 'float':
        flags += ['-u', '_printf_float']
      subprocess.run([cc, *flags, str(c), '-o', str(elf)], check=True, capture_output=True)
      out = subprocess.run([cc.replace('gcc', 'size'), str(elf)], check=True, capture_output=True,
                           text=True).stdout.splitlines()[1].split()
      sizes[name] = int(out[0]) + int(out[1])  # text + data: flash
  return {'snprintf_int': sizes['int'] - sizes['none'], 'snprintf_float': sizes['float'] - sizes['none']}


def board_flags(board_dir):
  """The board Makefile's CPU flags and compiler."""
  mk = (Path(board_dir) / 'Makefile').read_text()
  cpu = re.search(r'^CPU\s*:=\s*(.+)$', mk, re.M)[1].split()
  cc = shutil.which('arm-none-eabi-gcc') or '/opt/ST/STM32CubeCLT_1.22.0/GNU-tools-for-STM32/bin/arm-none-eabi-gcc'
  return cc, cpu


def host_decode(frames, elf):
  """Logs decoded per second on this machine: the Python library, and `uclog decode` (native)."""
  ld = uclog.LogData.load(elf)
  logs = [m for m in map(proto.mux_decode, frames) if m[0] == 'log']
  reps = max(1, 200_000 // max(1, len(logs)))
  t0 = time.perf_counter()
  for _ in range(max(1, reps // 20)):
    for _, _, _, addr, args in logs:
      ld.decode(addr, args)
  py = len(logs) * max(1, reps // 20) / (time.perf_counter() - t0)
  native = None
  exe = NATIVE / ('uclog.exe' if os.name == 'nt' else 'uclog')
  if exe.exists():
    wire = b''.join(proto.wire(f) for f in frames if proto.mux_decode(f)[0] == 'log') * reps
    with tempfile.NamedTemporaryFile(suffix='.bin', delete=False) as f:
      f.write(wire)
    try:
      t0 = time.perf_counter()
      subprocess.run([str(exe), 'decode', '-l', str(elf), f.name], check=True, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL)
      native = len(logs) * reps / (time.perf_counter() - t0)
    finally:
      os.unlink(f.name)
  return {'python_logs_per_s': round(py), 'native_logs_per_s': round(native) if native else None,
          'machine': f'{platform.system()} {platform.machine()}, Python {platform.python_version()}'}


def firmware_commit() -> str:
  """The commit the firmware was built from: this tree's (build it from a clean checkout), with
  -dirty if the firmware's sources (firmware/, device/; not this script) have changed."""
  git = ['git', '-C', str(ROOT)]
  sha = subprocess.run([*git, 'rev-parse', '--short', 'HEAD'], capture_output=True, text=True).stdout.strip()
  changed = subprocess.run([*git, 'status', '--porcelain', '--', 'firmware', 'device', ':!firmware/bench.py'],
                           capture_output=True, text=True).stdout.strip()
  return sha + ('-dirty' if changed else '')


def main():
  ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
  ap.add_argument('--board-dir', required=True, help='firmware/<board> (its Makefile: the CPU flags)')
  ap.add_argument('--build', required=True, help='the build directory (app.elf, app.map)')
  ap.add_argument('--device', action='append', required=True, help='a device URI per link; the first runs bench')
  ap.add_argument('--cpu-name', help='e.g. Cortex-M4F (default: from the Makefile)')
  ap.add_argument('-o', '--output', help='JSON file (default: stdout)')
  a = ap.parse_args()
  os.environ['PATH'] = f'{NATIVE}{os.pathsep}{os.environ["PATH"]}'
  elf = Path(a.build) / 'app.elf'
  cc, cpu = board_flags(a.board_dir)
  result = {'date': datetime.date.today().isoformat(),
            'firmware': firmware_commit(),
            'cpu': a.cpu_name or next((f[6:] for f in cpu if f.startswith('-mcpu=')), '?'),
            'links': [], 'round_trip': {}}
  for i, uri in enumerate(a.device):
    with uclog.connect(device=uri, logdata=str(elf), skip_backlog=True) as dev:
      terminal(dev, '', '> ')  # a prompt: the firmware is listening
      if i == 0:
        result['board'], result['hz'], result['n'], result['statements'] = bench(dev)
      tp = throughput(dev)
      result['links'].append(tp)
      result['round_trip'][tp['link']] = round_trip(dev)
      if i == 0:
        list(dev.frames(timeout=0))
        terminal(dev, 'burst 500', r'\d+ dropped', timeout=60)
        frames = [f for f in dev.frames(timeout=0.5)]
        result['host_decode'] = host_decode(frames, elf)
  result['footprint'] = map_footprint(Path(a.build) / 'app.map')
  result['footprint'].update(printf_footprint(cc, cpu))
  # The log ring (part of `ram`): LOG_TX_BUF_SIZE, 8 KiB unless the board's Makefile sets it.
  ring = re.search(r'-DLOG_TX_BUF_SIZE=(\d+)', (Path(a.board_dir) / 'Makefile').read_text())
  result['footprint']['log_ring'] = int(ring[1]) if ring else 8192
  text = json.dumps(result, indent=1) + '\n'
  if a.output:
    Path(a.output).write_text(text)
  else:
    sys.stdout.write(text)


if __name__ == '__main__':
  main()
