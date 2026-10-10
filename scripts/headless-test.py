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
    config.write_text('output HEADLESS-1 mode 1440x900\nxwayland disable\nseat seat0 fallback true\ninput * repeat_delay 150\ninput * repeat_rate 20\ndefault_border none\n')
    env = os.environ.copy()
    for key in ('WAYLAND_DISPLAY', 'DISPLAY', 'SWAYSOCK', 'WAYLAND_SOCKET', 'DBUS_SESSION_BUS_ADDRESS'):
        env.pop(key, None)
    env.update(XDG_RUNTIME_DIR=temp, WLR_BACKENDS='headless', WLR_RENDERER='pixman', WLR_LIBINPUT_NO_DEVICES='1',XKB_DEFAULT_LAYOUT='us',LIBGL_ALWAYS_SOFTWARE='1')
    processes = []
    bus = subprocess.Popen(['dbus-daemon','--session','--nofork',f'--address=unix:path={runtime}/bus','--print-address=1'],stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    env['DBUS_SESSION_BUS_ADDRESS'] = bus.stdout.readline().strip()
    assert str(runtime) in env['DBUS_SESSION_BUS_ADDRESS']
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
            toolbar=snapshot('minimal-toolbar.png')
            px,py=selected['buttons']['p']
            icon_bytes=toolbar.crop((int(px)-10,int(py)-10,int(px)+10,int(py)+10)).tobytes()
            colors={icon_bytes[i:i+3] for i in range(0,len(icon_bytes),3)}
            assert len(colors)>8,'Icon edges have no antialiasing coverage'
            assert 'Bullseye EGL renderer:' in (ARTIFACTS/'selection.log').read_text(),'GPU rendering path not active'
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
            input_actions('wait 400\n')
            caret=snapshot('text-caret.png')
            caret_bounds=ImageChops.difference(caret.crop((446,348,474,386)),base.crop((446,348,474,386))).getbbox()
            assert caret_bounds and caret_bounds[2]-caret_bounds[0]<=3 and caret_bounds[3]-caret_bounds[1]>=18, f'Caret is not a thin vertical line: {caret_bounds}'
            input_actions('type hello\nwait 400\n')
            wait_for(lambda: state() if state().get('text') == 'hello' else None,'Text entry failed')
            preview=snapshot('live-text-preview.png')
            assert ImageChops.difference(preview.crop((450,350,550,390)),base.crop((450,350,550,390))).getbbox(),'Text has no live preview'
            input_actions('key 28\nwait 300\n')
            selected = wait_for(lambda: state() if state()['marks'] == 1 else None, 'Caption was not committed')
            text_preview=snapshot('text-overlay.png')
            input_actions('shortcut c\nwait 300\n')
            assert overlay.wait(timeout=15) == 0
            png = run(['wl-paste','--no-newline','--type','image/png'], env)
            (ARTIFACTS/'text-export.png').write_bytes(png)
            assert ImageChops.difference(Image.open(ARTIFACTS/'text-export.png').convert('RGB'),base.crop((300,200,1000,600))).getbbox()
            text_export=Image.open(ARTIFACTS/'text-export.png').convert('RGB')
            crop_base=base.crop((450,350,550,390))
            preview_bounds=ImageChops.difference(text_preview.crop((450,350,550,390)),crop_base).getbbox()
            export_bounds=ImageChops.difference(text_export.crop((150,150,250,190)),crop_base).getbbox()
            assert all(abs(a-b)<=2 for a,b in zip(preview_bounds,export_bounds)),(preview_bounds,export_bounds)
            print('PASS text placement, compositor keymap, and Ctrl+C', flush=True)

            # A held key must repeat at the compositor's rate, then stop on release.
            overlay,state=launch('repeat',('--demo',))
            drag(300,200,1000,600)
            selected=wait_for(lambda:state() if state().get('toolbar_visible') else None,'No repeat toolbar')
            click(*selected['buttons']['t'])
            click(450,350)
            input_actions('keydown 35\nwait 500\nkeyup 35\nwait 100\n')
            repeated=wait_for(lambda:state()['text'] if state().get('text') and len(state()['text'])>=4 else None,'Held key did not repeat')
            assert set(repeated)=={'h'},repeated
            input_actions('wait 350\n')
            assert state()['text']==repeated,'Text continued repeating after key release'
            input_actions('keydown 14\nwait 300\nkeyup 14\nwait 100\n')
            deleted=state()['text']
            assert len(deleted)<=len(repeated)-3,'Backspace did not repeat'
            input_actions('wait 250\n')
            assert state()['text']==deleted,'Backspace continued after release'
            input_actions('key 1\nkey 1\nwait 100\n')
            assert overlay.wait(timeout=10)==0
            print('PASS compositor key repeat for typing and Backspace, stopping on release',flush=True)

            # Select bundled and installed fonts, without changing an earlier caption's font.
            font_path=ARTIFACTS/'font-export.png'
            overlay,state=launch('fonts',('--demo','--save-to',str(font_path)))
            drag(300,200,1000,600)
            selected=wait_for(lambda:state() if state().get('toolbar_visible') else None,'No font toolbar')
            click(*selected['buttons']['t'])
            selected=wait_for(lambda:state() if 'font' in state()['buttons'] else None,'Font selector missing')
            click(*selected['buttons']['font'])
            selected=wait_for(lambda:state() if 'font-Hack' in state()['buttons'] else None,'Bundled monospace font missing')
            snapshot('font-picker.png')

            def visible_fonts():
                return {k for k in state()['buttons'] if k.startswith('font-') and k not in ('font-search','font-results')}

            # Opening focuses search without another click. Filtered results still scroll.
            input_actions('type a\nwait 150\n')
            wait_for(lambda:'font-Ubuntu-Light' not in visible_fonts() and 'font-Hack' in visible_fonts(),'Opening font picker did not focus search')
            before=visible_fonts()
            x,y=state()['buttons']['font-results']
            input_actions(f'move {int(x)} {int(y)}\nscroll 4\nwait 350\n')
            wait_for(lambda:visible_fonts()!=before,'Mouse wheel did not scroll filtered fonts')
            snapshot('font-picker-filtered-scroll.png')
            # Editing the filter returns to the top, with keyboard focus retained after scrolling.
            input_actions('shortcut a\nkey 14\nwait 200\n')
            wait_for(lambda:'font-Ubuntu-Light' in visible_fonts(),'Clearing search did not reset results to the top')
            before=visible_fonts()
            input_actions('scroll-pixels 120\nwait 350\n')
            wait_for(lambda:visible_fonts()!=before,'Trackpad did not scroll font results')
            snapshot('font-picker-trackpad-scroll.png')
            input_actions('type hack\nwait 200\n')
            selected=wait_for(lambda:state() if visible_fonts()=={'font-Hack'} else None,'Search did not reset scrolled results or find Hack')
            click(*selected['buttons']['font-Hack'])
            wait_for(lambda:state() if state().get('font')=='Hack' else None,'Font did not switch to Hack')
            click(450,350)
            input_actions('type iiiwww\nkey 28\nwait 100\n')
            selected=wait_for(lambda:state() if state()['marks']==1 else None,'Monospace caption missing')
            mono_preview=snapshot('font-monospace.png')
            click(450,450)
            input_actions('type iiiwww\nwait 100\n')
            selected=state()
            click(*selected['buttons']['font'])
            selected=wait_for(lambda:state() if 'font-search' in state()['buttons'] else None,'Font search missing')
            input_actions('type hack\nwait 150\n')
            selected=wait_for(lambda:state() if visible_fonts()=={'font-Hack'} else None,'Reopening font picker did not focus search')
            assert selected['text']=='iiiwww','Font search edited the pending caption'
            click(*selected['buttons']['font-Hack'])
            selected=state()
            click(*selected['buttons']['font'])
            selected=wait_for(lambda:state() if any(k.startswith('font-system-') for k in state()['buttons']) else None,'Installed fonts missing from selector')
            installed=next(k for k in selected['buttons'] if k.startswith('font-system-'))
            click(*selected['buttons'][installed])
            selected=wait_for(lambda:state() if state().get('font') not in ('Hack','Ubuntu Light') else None,'Installed font did not load')
            input_actions('key 28\nwait 100\n')
            selected=wait_for(lambda:state() if state()['marks']==2 else None,'System font caption missing')
            system_preview=snapshot('font-system.png')
            assert ImageChops.difference(mono_preview.crop((450,350,650,390)),system_preview.crop((450,350,650,390))).getbbox() is None,'Changing font altered an earlier caption'
            click(*selected['buttons']['save'])
            assert overlay.wait(timeout=15)==0
            fonts=Image.open(font_path).convert('RGB')
            mono=ImageChops.difference(fonts.crop((150,150,350,190)),base.crop((450,350,650,390)))
            system=ImageChops.difference(fonts.crop((150,250,350,290)),base.crop((450,450,650,490)))
            assert mono.getbbox() and system.getbbox() and mono.tobytes()!=system.tobytes(),'Font selection did not affect PNG export'
            print('PASS font search autofocus, filtered wheel/trackpad scrolling, selection, per-caption retention, and PNG export',flush=True)

            # Stationary pen clicks must produce pixels, even without pointer motion.
            dots_path=ARTIFACTS/'dots-export.png'
            overlay,state=launch('dots',('--demo','--save-to',str(dots_path)))
            drag(300,200,1000,600)
            selected=wait_for(lambda:state() if state().get('toolbar_visible') else None,'No dot toolbar')
            for i in range(12):
                click(430+i*20,350)
            selected=wait_for(lambda:state() if state()['marks']==12 else None,'Stationary clicks dropped dots')
            snapshot('dots-overlay.png')
            click(*selected['buttons']['save'])
            assert overlay.wait(timeout=10)==0
            dots=Image.open(dots_path).convert('RGB')
            for i in range(12):
                assert dots.getpixel((130+i*20,150)) != base.getpixel((430+i*20,350)),'A dot is invisible'
            print('PASS all stationary pen clicks export visible dots',flush=True)

            highlight_path=ARTIFACTS/'highlighter-export.png'
            overlay,state=launch('highlighter',('--demo','--save-to',str(highlight_path)))
            drag(300,200,1000,600)
            selected=wait_for(lambda:state() if state().get('toolbar_visible') else None,'No highlighter toolbar')
            click(*selected['buttons']['h'])
            input_actions('move 450 300\npress\nmove 525 300\nmove 600 300\nmove 600 400\nmove 600 500\nrelease\nwait 200\n')
            selected=wait_for(lambda:state() if state()['marks']==1 else None,'Highlighter stroke missing')
            snapshot('highlighter-overlay.png')
            click(*selected['buttons']['save'])
            assert overlay.wait(timeout=10)==0
            highlight=Image.open(highlight_path).convert('RGB')
            assert highlight.getpixel((225,100)) != base.getpixel((525,300)),'Stroke not highlighted'
            assert highlight.getpixel((150,250)) == base.getpixel((450,450)),'Highlighter filled its bounding rectangle'
            print('PASS highlighter follows a bent pen stroke, leaving its bounding-box interior untouched',flush=True)

            overlay,state=launch('color-picker',('--demo',))
            drag(300,200,1000,600)
            selected=wait_for(lambda:state() if state().get('toolbar_visible') else None,'No color toolbar')
            initial=selected['color']
            click(*selected['buttons']['color'])
            selected=wait_for(lambda:state() if 'color-picker' in state()['buttons'] else None,'Full color picker missing')
            snapshot('color-picker.png')
            click(*selected['buttons']['color-picker'])
            wait_for(lambda:state() if state()['color']!=initial else None,'Color picker did not edit color')
            input_actions('key 1\nwait 200\n')
            if overlay.poll() is None:
                input_actions('key 1\nwait 200\n')
            assert overlay.wait(timeout=10)==0
            print('PASS full color picker changes the drawing color',flush=True)

            # Mock only the portal service, on our own D-Bus daemon. The app performs real D-Bus requests.
            portal_control=runtime/'portal-response'
            portal_path=ARTIFACTS/'portal-export.png'
            portal_path.unlink(missing_ok=True)
            portal_control.write_text(str(portal_path))
            handle=(ARTIFACTS/'portal.log').open('w')
            portal=subprocess.Popen([str(ROOT/'target/debug/examples/headless-portal'),str(portal_control)],env=env,stdout=handle,stderr=handle)
            processes.append((portal,handle))
            wait_for(lambda:portal_control.with_suffix('.ready').exists(),'Private portal did not start')
            overlay,state=launch('portal-save',('--demo',))
            drag(300,200,1000,600)
            selected=wait_for(lambda:state() if state().get('toolbar_visible') else None,'No portal toolbar')
            click(*selected['buttons']['save'])
            assert overlay.wait(timeout=15)==0,'Portal save did not close overlay'
            assert ImageChops.difference(Image.open(portal_path).convert('RGB'),base.crop((300,200,1000,600))).getbbox() is None
            portal_control.write_text('cancel')
            overlay,state=launch('portal-cancel',('--demo',))
            drag(300,200,1000,600)
            selected=wait_for(lambda:state() if state().get('toolbar_visible') else None,'No cancel toolbar')
            before_frame=selected['frame']
            click(*selected['buttons']['save'])
            selected=wait_for(lambda:state() if state()['frame']>before_frame+1 and not state().get('picker') else None,'Portal cancellation did not return')
            assert overlay.poll() is None and selected['crop']==[300,200,700,400]
            click(*selected['buttons']['p'])
            click(500,350)
            wait_for(lambda:state() if state()['marks']==1 else None,'Overlay did not regain input after cancellation')
            input_actions('key 1\nwait 200\n')
            assert overlay.wait(timeout=10)==0
            print('PASS real portal SaveFile request, exact PNG save, cancellation, and restored input',flush=True)

            portal_control.write_text('error')
            overlay,state=launch('portal-fallback',('--demo',))
            drag(300,200,1000,600)
            selected=wait_for(lambda:state() if state().get('toolbar_visible') else None,'No fallback toolbar')
            click(*selected['buttons']['save'])
            wait_for(lambda:state() if state().get('save_dialog') else None,'Missing fallback when portal fails')
            snapshot('save-fallback.png')
            input_actions('key 1\nwait 200\n')
            assert overlay.poll() is None,'Canceling path dialog closed the capture'
            input_actions('key 1\nwait 200\n')
            assert overlay.wait(timeout=10)==0
            print('PASS missing system chooser falls back to a cancellable path dialog',flush=True)

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
            wait_for(lambda: all(abs(a-b)<=1 for a,b in zip(snapshot('multi-output-overlay.png').getpixel((20,20)),(15,63,100))),'First output is not covered by its frozen snapshot')
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
            bus.terminate()
            bus.wait(timeout=5)
            sway.terminate()
            try:
                sway.wait(timeout=5)
            except subprocess.TimeoutExpired:
                sway.kill()
                sway.wait()
