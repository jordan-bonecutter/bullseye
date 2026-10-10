# Bullseye

A native Rust screenshot editor for **Sway and wlroots**, inspired by Flameshot.
Dark slate surfaces, mint accents, a quiet canvas, and tools that stay out of the way.

Capture talks directly to Wayland through `wlr-screencopy`; the editor uses egui/eframe's
Wayland backend. No GTK, Qt, X11 window backend, or `grim`/`slurp`/`wl-copy` subprocesses
are used by the app. A small process running the same Rust binary serves the clipboard
until another application replaces it, so copying still works after closing the editor.

## Run

```sh
cargo run -- --demo                 # Explore the UI with a sample desktop
cargo run                          # Capture, then open the editor
cargo run -- --list-outputs
cargo run -- --output DP-1          # Choose a monitor
cargo run -- --fullscreen           # Fullscreen editor
cargo run -- --open screenshot.png  # Annotate an existing PNG/JPEG
```

The app uses Rust edition 2024 (Rust 1.88+ recommended for the current dependencies).
A working Wayland session and EGL/OpenGL drivers are required for the editor; software
rendering works with Mesa. Headless capture/export does not need a graphics context.

## Tools

- Region selection with draggable corner handles and native pixel dimensions.
- Freehand, arrows, rectangles, ellipses, translucent highlights, and opaque redaction.
- Text captions and numbered pins.
- Six annotation colors, adjustable stroke width, zoom, undo, and redo.
- Atomic PNG saves and a persistent Wayland image clipboard.
- Delayed capture, optional cursor capture, and explicit monitor selection.

Draw a selection, choose a tool on the left, and drag or click on the image.
For text, enter your caption in the inspector, then click to place it.
Redaction completely replaces covered pixels with an opaque block.
The exported PNG contains the selected region and all annotations at the original resolution.

Save location defaults to `~/Pictures/Bullseye/`. Edit it in the inspector or launch with
`--save-to /path/to/shot.png`. **New capture** closes the editor before taking a fresh shot;
save or copy your current edits first.

| Key | Action |
| --- | --- |
| S / P / A | Select / pen / arrow |
| R / E / H | Rectangle / ellipse / highlight |
| B / T / N | Redact / text / numbered pin |
| Ctrl+S / Ctrl+C | Save PNG / copy image |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z or Ctrl+Y | Redo |
| Ctrl+A | Reset to the full image |
| F1 | Show shortcuts |
| Esc | Cancel a drag, dismiss help, or close |

For a capture without opening the editor:

```sh
bullseye --delay 3 --cursor --save /tmp/shot.png
bullseye --output DP-1 --copy
bullseye --save /tmp/shot.png --copy
```

## Install and bind in Sway

```sh
cargo install --path . --locked
```

Add to your Sway config (adjust the path if Cargo's bin directory differs):

```sway
bindsym Print exec ~/.cargo/bin/bullseye
bindsym $mod+Print exec ~/.cargo/bin/bullseye --copy
for_window [app_id="bullseye"] floating enable
```

The optional desktop launcher is in `packaging/bullseye.desktop`, with an SVG icon in
`assets/bullseye.svg`. Install them to your desktop's applications and icon directories,
and ensure `bullseye` is on the launcher’s PATH.

## Scope

This is a first working implementation focused on Sway. The capture backend requires
`wlr-screencopy`; compositors that expose it should work, but only headless Sway has
been integration tested. **GNOME/KDE portal capture and ext-image-copy-capture are not
implemented.** Importing an image and demo mode still work on their Wayland sessions.

Capture selects one output at a time; the default is the first advertised output.
Multi-monitor stitching, window selection, blur/pixelation, and layer-shell desktop
selection overlays are future work. Selection currently happens on the captured image
in the editor. Text uses the bundled Ubuntu font; complex-script shaping is not implemented.

## Verify without touching your desktop

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --examples
cargo build
python3 scripts/headless-test.py
```

The integration harness requires **Sway, grim, wl-paste, Python 3, and Pillow** for testing
only. It starts a software-rendered headless Sway compositor with a private runtime
directory and explicitly removes the host display/socket variables. Its Rust virtual
pointer driver refuses to connect outside that private headless runtime.

The harness checks native capture, output enumeration, HiDPI pixel preservation,
clipboard persistence after process exit, editor startup/shutdown, and real pointer-driven
cropping, annotation, saving, and copying. Rotated and mirrored outputs are compared
pixel for pixel against grim as an independent reference. Screenshots and logs go to
`target/headless-artifacts/`; it never opens windows in your desktop session.

## Implementation

- `src/capture.rs`: Wayland output discovery, shared-memory screencopy, stride handling,
  vertical inversion, output transforms, and a bounded capture timeout.
- `src/document.rs`: image-space edits, crop/history state, Rust rasterization, and export.
- `src/app.rs`: native editor and keyboard/pointer interactions.
- `src/clipboard.rs`: PNG clipboard serving with the Rust data-control client.
- `src/main.rs`: CLI, atomic export, and launch lifecycle.

MIT licensed. Fonts are supplied by `epaint_default_fonts` under their upstream licenses.
