#!/usr/bin/env python3
"""Test in a private, software-rendered Wayland compositor. Never use the host UI."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / 'target/debug/bullseye'
ARTIFACTS = ROOT / 'target/headless-artifacts'
ARTIFACTS.mkdir(parents=True, exist_ok=True)


def run(command, env, **kwargs):
    result = subprocess.run(command, env=env, capture_output=True, timeout=20, **kwargs)
    assert result.returncode == 0, f'{command}: {result.stderr!r}'
    return result.stdout


def dimensions(path):
    data = path.read_bytes()
    assert data[:8] == b'\x89PNG\r\n\x1a\n', 'Not a PNG'
    return int.from_bytes(data[16:20], 'big'), int.from_bytes(data[20:24], 'big')


with tempfile.TemporaryDirectory(prefix='bullseye-headless-', dir='/tmp') as temp:
    runtime = Path(temp)
    config = runtime / 'sway.conf'
    config.write_text('''
output HEADLESS-1 mode 1440x900
xwayland disable
seat seat0 fallback true
default_border none
for_window [app_id="bullseye"] floating enable, resize set 1280 820, move position center
''')
    env = os.environ.copy()
    for key in ('WAYLAND_DISPLAY', 'DISPLAY', 'SWAYSOCK', 'WAYLAND_SOCKET'):
        env.pop(key, None)
    env.update(XDG_RUNTIME_DIR=temp, WLR_BACKENDS='headless', WLR_RENDERER='pixman',
               WLR_LIBINPUT_NO_DEVICES='1', LIBGL_ALWAYS_SOFTWARE='1')
    with (ARTIFACTS / 'sway.log').open('w') as log:
        sway = subprocess.Popen(['sway', '-c', str(config)], env=env, stdout=log, stderr=log)
        editor = None
        try:
            for _ in range(100):
                sockets = [p for p in runtime.glob('wayland-*') if not p.name.endswith('.lock')]
                if sockets and list(runtime.glob('sway-ipc.*.sock')):
                    break
                assert sway.poll() is None, (ARTIFACTS / 'sway.log').read_text()
                time.sleep(0.1)
            else:
                raise AssertionError('Headless Sway did not start')
            env['WAYLAND_DISPLAY'] = sockets[0].name
            env['SWAYSOCK'] = str(next(runtime.glob('sway-ipc.*.sock')))

            def ipc(command):
                return json.loads(run(['swaymsg', '-s', env['SWAYSOCK'], '-r', command], env))

            assert run([str(BINARY), '--list-outputs'], env).strip() == b'HEADLESS-1'
            shot = ARTIFACTS / 'capture.png'
            run([str(BINARY), '--output', 'HEADLESS-1', '--save', str(shot)], env)
            assert dimensions(shot) == (1440, 900), dimensions(shot)
            print('PASS native headless capture (1440 × 900)', flush=True)

            demo = ARTIFACTS / 'demo.png'
            run([str(BINARY), '--demo', '--save', str(demo), '--copy'], env)
            clipboard = run(['wl-paste', '--no-newline', '--type', 'image/png'], env)
            assert clipboard == demo.read_bytes(), 'Clipboard changed after the source process exited'
            print('PASS clipboard PNG survives source process exit', flush=True)

            ipc('output HEADLESS-1 scale 2')
            run([str(BINARY), '--output', 'HEADLESS-1', '--save', str(shot)], env)
            assert dimensions(shot) == (1440, 900), 'Scale must not discard native pixels'
            ipc('output HEADLESS-1 scale 1')
            print('PASS HiDPI capture preserves physical pixels', flush=True)

            editor_env = dict(env, BULLSEYE_SMOKE_SECONDS='25')
            export = ARTIFACTS / 'ui-export.png'
            export.unlink(missing_ok=True)
            with (ARTIFACTS / 'editor.log').open('w') as editor_log:
                editor = subprocess.Popen([str(BINARY), '--demo', '--save-to', str(export)], env=editor_env, stdout=editor_log, stderr=editor_log)
                time.sleep(2)
                assert editor.poll() is None, (ARTIFACTS / 'editor.log').read_text()
                run([str(BINARY), '--save', str(ARTIFACTS / 'editor.png')], env)
                # Drive the real Wayland UI through wlr-virtual-pointer, within this socket only.
                actions = '''move 350 350
press
move 500 430
move 700 530
move 950 650
release
wait 150
move 120 298
press
release
move 520 450
press
move 650 490
move 780 520
release
wait 150
move 1270 84
press
release
wait 200
'''
                run([str(ROOT / 'target/debug/examples/headless-input')], env, input=actions.encode())
                for _ in range(50):
                    if export.exists():
                        break
                    time.sleep(0.1)
                assert export.exists(), 'UI save button did not export'
                width, height = dimensions(export)
                assert 900 < width < 1200 and 400 < height < 600, (width, height)
                from PIL import Image
                image = Image.open(export).convert('RGB')
                mint_pixels = sum(pixel == (132, 239, 205) for pixel in (image.get_flattened_data() if hasattr(image, 'get_flattened_data') else image.getdata()))
                assert mint_pixels > 1000, f'Arrow was not rendered: {mint_pixels} mint pixels'
                print('PASS virtual-pointer crop, arrow annotation, and PNG save', flush=True)
                run([str(BINARY), '--save', str(ARTIFACTS / 'editor-annotated.png')], env)
                run([str(ROOT / 'target/debug/examples/headless-input')], env,
                    input=b'move 1157 84\npress\nrelease\nwait 200\n')
                for _ in range(50):
                    clipboard = run(['wl-paste', '--no-newline', '--type', 'image/png'], env)
                    if clipboard == export.read_bytes():
                        break
                    time.sleep(0.1)
                assert clipboard == export.read_bytes(), 'UI clipboard differs from saved annotations'
                print('PASS editor clipboard matches annotated export', flush=True)
                # Compare transformed capture against grim as an independent test oracle.
                from PIL import ImageChops
                for transform in ('90', 'flipped', 'flipped-90'):
                    ipc(f'output HEADLESS-1 transform {transform}')
                    time.sleep(0.5)
                    ours = ARTIFACTS / f'transform-{transform}.png'
                    reference = ARTIFACTS / f'reference-{transform}.png'
                    run([str(BINARY), '--save', str(ours)], env)
                    run(['grim', '-o', 'HEADLESS-1', str(reference)], env)
                    actual = Image.open(ours).convert('RGB')
                    expected = Image.open(reference).convert('RGB')
                    assert actual.size == expected.size, (transform, actual.size, expected.size)
                    assert ImageChops.difference(actual, expected).getbbox() is None, f'Incorrect {transform} transform'
                print('PASS rotated and mirrored capture matches grim pixel for pixel', flush=True)
                ipc('output HEADLESS-1 transform normal')
                assert editor.wait(timeout=30) == 0, (ARTIFACTS / 'editor.log').read_text()
                editor = None
            print('PASS software-rendered Wayland editor opens and closes', flush=True)
            print(f'UI screenshot: {ARTIFACTS / "editor.png"}', flush=True)
        finally:
            if editor is not None and editor.poll() is None:
                editor.terminate()
                editor.wait(timeout=5)
            sway.terminate()
            try:
                sway.wait(timeout=5)
            except subprocess.TimeoutExpired:
                sway.kill()
                sway.wait()
