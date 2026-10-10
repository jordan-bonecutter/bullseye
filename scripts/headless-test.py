#!/usr/bin/env python3
"""Exercise the actual layer-shell workflow in a private headless Sway session."""
import json
import os
import select
from pathlib import Path
import subprocess
import tempfile
import time
from PIL import Image, ImageChops

ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / 'target/debug/bullseye'
DRIVER = ROOT / 'target/debug/examples/headless-input'
ARTIFACTS = ROOT / 'target/headless-artifacts'
ARTIFACTS.mkdir(parents=True, exist_ok=True)


def run(command, env, **kwargs):
    result = subprocess.run(command, env=env, capture_output=True, timeout=20, **kwargs)
    assert result.returncode == 0, f'{command}: {result.stderr!r}'
    return result.stdout


def wait_for(check, message, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            result = check()
            if result:
                return result
        except (FileNotFoundError, json.JSONDecodeError):
            pass
        time.sleep(.05)
    raise AssertionError(message)


with tempfile.TemporaryDirectory(prefix='bullseye-headless-', dir='/tmp') as temp:
    runtime = Path(temp)
    config = runtime / 'sway.conf'
    config.write_text('output HEADLESS-1 mode 1440x900\nxwayland disable\nseat seat0 fallback true\ndefault_border none\n')
    env = os.environ.copy()
    for key in ('WAYLAND_DISPLAY', 'DISPLAY', 'SWAYSOCK', 'WAYLAND_SOCKET'):
        env.pop(key, None)
    env.update(XDG_RUNTIME_DIR=temp, WLR_BACKENDS='headless', WLR_RENDERER='pixman', WLR_LIBINPUT_NO_DEVICES='1',XKB_DEFAULT_LAYOUT='us')
    processes = []
    with (ARTIFACTS / 'sway.log').open('w') as log:
        sway = subprocess.Popen(['sway', '-c', str(config)], env=env, stdout=log, stderr=log)
        try:
            socket = wait_for(lambda: next((p for p in runtime.glob('wayland-*') if not p.name.endswith('.lock')), None), 'No headless Wayland socket')
            env['WAYLAND_DISPLAY'] = socket.name
            env['SWAYSOCK'] = str(wait_for(lambda: next(runtime.glob('sway-ipc.*.sock'), None), 'No private IPC socket'))

            def ipc(command):
                return json.loads(run(['swaymsg', '-s', env['SWAYSOCK'], '-r', command], env))

            driver = subprocess.Popen([str(DRIVER)], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
            processes.append((driver, None))
            def input_actions(actions):
                for action in actions.splitlines():
                    driver.stdin.write(action+'\n')
                    driver.stdin.flush()
                    assert select.select([driver.stdout],[],[],10)[0], 'Input driver stalled'
                    assert driver.stdout.readline().strip() == 'OK', 'Input driver stopped'


            def click(x, y):
                input_actions(f'move {int(x)} {int(y)}\npress\nrelease\nwait 150\n')

            def drag(x, y, right, bottom):
                input_actions(f'move {x} {y}\npress\nmove {(x+right)//2} {(y+bottom)//2}\nmove {right} {bottom}\nrelease\nwait 150\n')

            def snapshot(name):
                path = ARTIFACTS / name
                run([str(BINARY), '--save', str(path)], env)
                return Image.open(path).convert('RGB')

            def launch(name, extra=()):
                state_path = ARTIFACTS / f'{name}-state.json'
                state_path.unlink(missing_ok=True)
                child_env = dict(env, BULLSEYE_TEST_STATE=str(state_path), BULLSEYE_SMOKE_SECONDS='40')
                handle = (ARTIFACTS / f'{name}.log').open('w')
                process = subprocess.Popen([str(BINARY), *extra], env=child_env, stdout=handle, stderr=handle)
                processes.append((process, handle))
                def state():
                    assert process.poll() is None, (ARTIFACTS / f'{name}.log').read_text()
                    return json.loads(state_path.read_text())
                wait_for(state, f'{name} overlay did not appear')
                return process, state

            assert run([str(BINARY), '--list-outputs'], env).strip() == b'HEADLESS-1'
            assert snapshot('capture.png').size == (1440, 900)
            ipc('output HEADLESS-1 scale 2')
            assert snapshot('hidpi.png').size == (1440, 900)
            ipc('output HEADLESS-1 scale 1')
            print('PASS native capture and HiDPI preserve physical pixels', flush=True)

            demo_path = ARTIFACTS / 'demo.png'
            run([str(BINARY), '--demo', '--save', str(demo_path)], env)
            base = Image.open(demo_path).convert('RGB')
            exported = ARTIFACTS / 'selection.png'
            exported.unlink(missing_ok=True)
            overlay, state = launch('selection', ('--demo', '--save-to', str(exported)))
            assert state()['phase'] == 'select' and not state()['buttons']
            input_actions('move 200 100\nwait 100\n')
            snapshot('select-overlay.png')
            crosshair_path=ARTIFACTS/'crosshair.png'
            run([str(BINARY),'--cursor','--save',str(crosshair_path)],env)
            assert Image.open(crosshair_path).convert('RGB').getpixel((200,90))==(255,255,255),'Cursor is not a crosshair'

            drag(300, 200, 1000, 600)
            selected = wait_for(lambda: (s := state()) if state()['phase'] == 'annotate' and state()['buttons'] and state().get('toolbar_visible') else None, 'No annotation toolbar')
            assert selected['crop'] == [300, 200, 700, 400], selected
            snapshot('minimal-toolbar.png')
            x, y = selected['buttons']['save']
            click(x, y)
            assert overlay.wait(timeout=10) == 0
            actual = Image.open(wait_for(lambda: exported if exported.exists() else None, 'Selection not saved')).convert('RGB')
            assert ImageChops.difference(actual, base.crop((300,200,1000,600))).getbbox() is None, 'UI leaked into PNG or crop shifted'
            print('PASS crosshair selection → toolbar → save and close; exact crop without UI', flush=True)

            overlay, state = launch('markup', ('--demo',))
            drag(300,200,1000,600)
            selected = wait_for(lambda: (s := state()) if state()['buttons'] and state().get('toolbar_visible') else None, 'No toolbar')
            click(*selected['buttons']['a'])
            drag(450,350,800,480)
            selected = wait_for(lambda: (s := state()) if state()['marks'] == 1 else None, 'Arrow missing')
            snapshot('annotated-overlay.png')
            click(*selected['buttons']['copy'])
            assert overlay.wait(timeout=10) == 0
            png = run(['wl-paste','--no-newline','--type','image/png'], env)
            (ARTIFACTS/'annotated-export.png').write_bytes(png)
            annotated = Image.open(ARTIFACTS/'annotated-export.png').convert('RGB')
            assert annotated.size == (700,400)
            assert ImageChops.difference(annotated,base.crop((300,200,1000,600))).getbbox() is not None
            print('PASS drawing → copy and close; clipboard persists after exit', flush=True)

            # Text entry goes through an actual virtual keyboard and the compositor keymap.
            overlay, state = launch('text', ('--demo',))
            drag(300,200,1000,600)
            selected = wait_for(lambda: state() if state().get('toolbar_visible') else None, 'No text toolbar')
            click(*selected['buttons']['t'])
            click(450,350)
            wait_for(lambda: state() if state().get('text') is not None else None,'Text field did not appear')
            input_actions('wait 400\ntype hello\nwait 400\n')
            wait_for(lambda: state() if state().get('text') == 'hello' else None,'Text entry failed')
            input_actions('key 28\nwait 300\n')
            selected = wait_for(lambda: state() if state()['marks'] == 1 else None, 'Caption was not committed')
            snapshot('text-overlay.png')
            input_actions('shortcut c\nwait 300\n')
            assert overlay.wait(timeout=15) == 0
            png = run(['wl-paste','--no-newline','--type','image/png'], env)
            (ARTIFACTS/'text-export.png').write_bytes(png)
            assert ImageChops.difference(Image.open(ARTIFACTS/'text-export.png').convert('RGB'),base.crop((300,200,1000,600))).getbbox()
            print('PASS text placement, compositor keymap, and Ctrl+C', flush=True)

            # Cover a real changing surface, rather than importing a fixture as the screenshot.
            control = runtime / 'desktop-mode'
            control.write_text('base')
            handle = (ARTIFACTS/'desktop.log').open('w')
            desktop = subprocess.Popen([str(ROOT/'target/debug/examples/headless-desktop'),str(control)], env=env,stdout=handle,stderr=handle)
            processes.append((desktop,handle))
            wait_for(lambda: control.with_suffix('.ready').exists(), 'Changing desktop did not appear')
            input_actions('move 180 150\nwait 500\n')
            def hover_ready():
                image = snapshot('hover-before.png')
                return image if image.getpixel((120,215)) == (255,220,110) else None
            before = wait_for(hover_ready, 'Hover text did not appear')
            frozen_path = ARTIFACTS/'frozen-hover-export.png'
            frozen_path.unlink(missing_ok=True)
            overlay, state = launch('frozen-hover', ('--save-to',str(frozen_path)))
            control.write_text('changed')
            input_actions('wait 500\n')
            drag(80,80,500,350)
            selected = wait_for(lambda: state() if state().get('toolbar_visible') else None,'Frozen selection toolbar missing')
            snapshot('frozen-hover-overlay.png')
            click(*selected['buttons']['save'])
            assert overlay.wait(timeout=15) == 0
            frozen = Image.open(frozen_path).convert('RGB')
            assert ImageChops.difference(frozen,before.crop((80,80,500,350))).getbbox() is None, 'Captured pixels changed after invocation'
            assert snapshot('after-freeze.png').getpixel((20,20)) == (30,120,190), 'Fixture did not change underneath overlay'
            print('PASS desktop freezes before input focus; hover text survives live content changes', flush=True)

            # Selection coordinates remain logical, while the exported crop stays in native pixels.
            ipc('output HEADLESS-1 scale 2')
            input_actions('extent 720 450\n')
            hidpi_path = ARTIFACTS/'hidpi-selection.png'
            hidpi_path.unlink(missing_ok=True)
            overlay, state = launch('hidpi-selection',('--demo','--save-to',str(hidpi_path)))
            drag(100,100,500,300)
            selected=wait_for(lambda:state() if state().get('toolbar_visible') else None,'HiDPI toolbar missing')
            assert selected['crop']==[200,200,800,400],selected
            click(*selected['buttons']['save'])
            assert overlay.wait(timeout=15)==0
            assert ImageChops.difference(Image.open(hidpi_path).convert('RGB'),base.crop((200,200,1000,600))).getbbox() is None
            ipc('output HEADLESS-1 scale 1')
            input_actions('extent 1440 900\n')
            print('PASS HiDPI layer overlay selects and exports the exact native region',flush=True)

            overlay,state=launch('cancel',('--demo',))
            input_actions('key 1\nwait 100\n')
            assert overlay.wait(timeout=10)==0
            print('PASS Escape dismisses the overlay',flush=True)

            ipc('create_output')
            ipc('output HEADLESS-2 mode 800x600')
            ipc('output HEADLESS-1 position 0 0')
            ipc('output HEADLESS-2 position 1440 0')
            outputs=json.loads(run(['swaymsg','-s',env['SWAYSOCK'],'-r','-t','get_outputs'],env))
            layout={o['name']:o['rect'] for o in outputs}
            assert layout['HEADLESS-1']['x']==0 and layout['HEADLESS-2']['x']==1440,layout
            input_actions('extent 2240 900\n')
            second_path=ARTIFACTS/'second-output-selection.png'
            second_path.unlink(missing_ok=True)
            overlay,state=launch('multi-output',('--save-to',str(second_path)))
            wait_for(lambda: snapshot('multi-output-overlay.png').getpixel((20,20))==(15,63,100),'First output is not covered by its frozen snapshot')
            drag(1520,200,1940,500)
            selected=wait_for(lambda:state() if state().get('toolbar_visible') and state()['output']=='HEADLESS-2' else None,'Second output did not accept selection')
            assert selected['crop']==[80,200,420,300],selected
            x,y=selected['buttons']['save']
            click(x+1440,y)
            assert overlay.wait(timeout=15)==0
            second=Image.open(second_path).convert('RGB')
            assert second.size==(420,300) and second.getbbox() is None,'Second output crop is incorrect'
            print('PASS all outputs freeze; selection and export work on the second monitor',flush=True)
        finally:
            for process, handle in processes:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=5)
                if handle:
                    handle.close()
            sway.terminate()
            try:
                sway.wait(timeout=5)
            except subprocess.TimeoutExpired:
                sway.kill()
                sway.wait()
