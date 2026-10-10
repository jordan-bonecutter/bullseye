#!/usr/bin/env python3
"""Measure process launch to the first submitted overlay frame in private headless Sway."""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, default=Path('target/release/bullseye'))
parser.add_argument('--runs', type=int, default=7)
parser.add_argument('--large', action='store_true', help='Also measure 4K and two 4K outputs')
parser.add_argument('--compare', type=Path, help='Alternate with an older binary for a controlled comparison')
parser.add_argument('--profile', action='store_true', help='Include the final run’s startup stage timings')
args = parser.parse_args()
binary = args.binary.resolve()
assert binary.is_file() and args.runs > 0

with tempfile.TemporaryDirectory(prefix='bullseye-headless-startup-', dir='/tmp') as temp:
    runtime = Path(temp)
    config = runtime / 'sway.conf'
    config.write_text('output HEADLESS-1 mode 1440x900\nxwayland disable\nseat seat0 fallback true\n')
    env = os.environ.copy()
    for key in ('WAYLAND_DISPLAY', 'DISPLAY', 'SWAYSOCK', 'WAYLAND_SOCKET', 'DBUS_SESSION_BUS_ADDRESS'):
        env.pop(key, None)
    env.update(XDG_RUNTIME_DIR=temp, WLR_BACKENDS='headless', WLR_RENDERER='pixman',
               WLR_LIBINPUT_NO_DEVICES='1', LIBGL_ALWAYS_SOFTWARE='1', LP_NUM_THREADS='4')
    if args.profile:
        env['BULLSEYE_PROFILE_STARTUP'] = '1'
    with (runtime / 'sway.log').open('w') as log:
        sway = subprocess.Popen(['sway', '-c', str(config)], env=env, stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 10
            while not list(runtime.glob('sway-ipc.*.sock')):
                assert sway.poll() is None and time.monotonic() < deadline, 'Headless Sway did not start'
                time.sleep(.01)
            env['WAYLAND_DISPLAY'] = next(p.name for p in runtime.glob('wayland-*') if not p.name.endswith('.lock'))
            env['SWAYSOCK'] = str(next(runtime.glob('sway-ipc.*.sock')))

            def ipc(command):
                subprocess.run(['swaymsg', '-s', env['SWAYSOCK'], command], env=env,
                               check=True, stdout=subprocess.DEVNULL, timeout=10)

            cases = [('1440x900', 'output HEADLESS-1 mode 1440x900'),
                                   ('1920x1080', 'output HEADLESS-1 mode 1920x1080'),
                                   ('two 1080p outputs', 'create_output')]
            if args.large:
                cases += [('3840x2160', 'output HEADLESS-2 disable'),
                          ('two 4K outputs', 'output HEADLESS-2 enable')]
            for label, command in cases:
                ipc(command)
                if label == 'two 1080p outputs':
                    ipc('output HEADLESS-2 mode 1920x1080 position 1920 0')
                if label in ('3840x2160', 'two 4K outputs'):
                    ipc('output HEADLESS-1 mode 3840x2160')
                if label == 'two 4K outputs':
                    ipc('output HEADLESS-2 mode 3840x2160 position 3840 0')
                time.sleep(.1)
                binaries = [binary]
                if args.compare and label not in ('3840x2160', 'two 4K outputs'):
                    binaries.append(args.compare.resolve())
                timings = {path: [] for path in binaries}
                profiles = {}
                for iteration in range(args.runs):
                    for path in binaries[::(-1 if iteration % 2 else 1)]:
                        state = runtime / 'state.json'
                        state.unlink(missing_ok=True)
                        with (runtime / 'app.log').open('w') as app_log:
                            started = time.monotonic()
                            child = subprocess.Popen([str(path)], env=dict(env, BULLSEYE_TEST_STATE=str(state)),
                                                     stdout=app_log, stderr=app_log)
                            try:
                                while not state.exists():
                                    assert child.poll() is None, (runtime / 'app.log').read_text()
                                    assert time.monotonic() - started < 20, 'Startup timed out'
                                    time.sleep(.002)
                                timings[path].append((time.monotonic() - started) * 1000)
                            finally:
                                child.terminate()
                                child.wait(timeout=10)
                        profiles[path] = [line for line in (runtime / 'app.log').read_text().splitlines()
                                          if line.startswith('Bullseye startup')]
                for path, values in timings.items():
                    result = {'output': label, 'binary': str(path), 'runs': args.runs,
                              'median_ms': round(statistics.median(values), 1),
                              'min_ms': round(min(values), 1), 'max_ms': round(max(values), 1)}
                    if args.profile:
                        result['stages'] = profiles[path]
                    print(json.dumps(result), flush=True)
        finally:
            sway.terminate()
            sway.wait(timeout=10)
