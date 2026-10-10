# Bullseye

A Rust screenshot tool for Sway: **freeze the desktop → drag a region → annotate → copy or save**.

Invoking Bullseye captures the displays before creating any UI. Native Wayland layer-shell
overlays then show those frozen pixels, with a crosshair cursor. Drag to select; a small
floating toolbar appears next to the selection. You keep working on the captured desktop.
Copying or saving successfully dismisses the overlay.

The application uses Rust Wayland clients and EGL/OpenGL ES to render directly into
the layer-shell surfaces. The frozen screenshot is uploaded once; selection shading,
strokes, text previews, and antialiased controls are drawn on the GPU. Compositor frame
callbacks pace rendering and coalesce pointer updates. Runtime does not invoke `grim`,
`slurp`, `wl-copy`, or other capture helpers, and the app UI uses no GTK or Qt.

## Run

```sh
cargo run                    # Freeze your displays and select a region
cargo run -- --demo          # Try the same flow with a sample desktop
cargo run -- --output DP-1   # Limit capture to one display
cargo run -- --delay 3       # Wait before freezing, useful for menus and hover text
cargo run -- --list-outputs
```

Requires a recent Rust toolchain (edition 2024; dependencies require Rust 1.88+), a Wayland
session with **wlr-screencopy and wlr-layer-shell**, and `libxkbcommon` for keyboard input.
Sway is the tested compositor. Rendering requires libwayland-client, libwayland-egl,
and an EGL driver supporting OpenGL ES 3. The compositor may use its normal hardware
driver; headless tests exercise the same rendering path with Mesa llvmpipe.

## Use

1. Invoke Bullseye while the content you want is visible. Hover text is captured before
   the overlay takes input focus; the original application can keep changing underneath.
2. Drag out a region. Resize it with the corner handles, or choose the selection tool
   to move it. Selection is contained within one display.
3. Pick pen, arrow, rectangle, ellipse, highlight, redaction, text, or numbered pins.
   The highlighter draws a broad translucent pen stroke. Stationary pen clicks make dots.
   The color button opens a full hue/saturation/value, RGBA, and opacity picker, plus
   the stroke-width control.
4. For text, click the text tool. Its toolbar shows a searchable font selector with
   bundled Ubuntu Light/Hack and installed system fonts. Opening it focuses search;
   scroll the results with a wheel or trackpad. Each name previews its own font
   with crisp, supersampled text; the dropdown sizes itself to fit the names.
   Click inside the selection
   and type a caption; its preview appears directly on the image. Press Enter, click
   elsewhere, or export to
   finish it; Escape discards the pending caption. Each caption keeps its chosen font.
   Held keys repeat using the compositor’s configured delay and rate.
5. Copy to the Wayland clipboard, or save as PNG. The overlay closes after success.
   Escape or right-click cancels without exporting.

Save asks the system file chooser through the XDG desktop portal. Install a portal
frontend and a backend providing FileChooser, such as your desktop’s portal backend.
The capture yields its overlay layer and keyboard focus while the chooser is open;
canceling restores the frozen capture. If the portal is unavailable, a small path dialog
is offered, initially pointing into `~/Pictures/Bullseye/`.
Use `--save-to /path/to/shot.png` to choose a destination in advance and save immediately
when clicking Save. PNG export contains only the selected native-resolution pixels and
annotations. UI, dimming, and the crosshair are excluded. `--cursor` includes the original
pointer in the captured desktop, before Bullseye replaces it with the crosshair.

The clipboard is served by a small process running the same Rust binary. It remains
available after the overlay exits, until another application replaces the selection.
Saves encode into a temporary file before replacing the destination.

| Key | Action |
| --- | --- |
| S / P / A | Select or move region / pen / arrow |
| R / E / H | Rectangle / ellipse / highlight |
| B / T / N | Opaque redaction / text / numbered pin |
| Ctrl+C | Copy and close |
| Ctrl+S | Save and close |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z or Ctrl+Y | Redo |
| Enter | Commit text or save in the path dialog |
| Esc | Cancel text/dialog/drag, or dismiss the capture |

Full-output automation, without the selection overlay:

```sh
bullseye --save /tmp/shot.png
bullseye --output DP-1 --copy
bullseye --delay 3 --cursor --save /tmp/shot.png --copy
```

`--open image.png` uses an existing PNG/JPEG in the overlay. `--fullscreen` is accepted
for compatibility; interactive capture always covers the desktop.

## Install and bind in Sway

```sh
cargo install --path . --locked
```

```sway
bindsym Print exec ~/.cargo/bin/bullseye
bindsym $mod+Print exec ~/.cargo/bin/bullseye --copy
```

No floating-window rule is needed: selection is a layer-shell overlay, independent of
window tiling. The optional desktop launcher and icon are in `packaging/` and `assets/`.

## Headless verification

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
cargo build --examples
python3 scripts/headless-test.py
```

The integration tests require Sway, wl-paste, dbus-daemon, Mesa EGL, Python 3, and Pillow. They start an isolated,
software-rendered headless compositor in a private `/tmp` runtime directory and remove
all host Wayland/display/IPC and D-Bus socket variables. The Rust input driver, changing
desktop fixture, and mock portal refuse to run outside that private environment. A
private D-Bus daemon serves test SaveFile responses; the host portal is never contacted.
Testing never opens a window in your desktop session.

The suite checks the actual crosshair, exact region selection, toolbar actions, drawings,
inline text previews and held-key repeat through a virtual keyboard, bundled/system
font search autofocus, filtered wheel/trackpad scrolling, selection and export,
stationary dots, bent highlighter
strokes, icon antialiasing, full color selection, clipboard persistence, portal saving,
cancellation, unavailable-portal fallback, and dismissal,
HiDPI mapping, and multiple displays. A real hover-content fixture changes behind the
capture overlay; its exported pixels must still exactly match the original hover content.
Screenshots and logs are written to `target/headless-artifacts/`.

Unit tests additionally cover native screenshot buffer decoding, crop/history behavior,
redaction edges, color alpha compositing, atomic PNG export, batched pointer events,
text/toolbar interactions, and reuse of the frozen texture during dragging.

## Current limits

Sway/wlroots is the focus. GNOME/KDE portal capture and ext-image-copy-capture are not
implemented. Selection cannot span displays; choose a region on one frozen display.
Text uses the selected font without complex-script shaping, IME composition,
or dead-key composition. Blur/pixelation and selecting a window automatically are future
work. Desktop capture supports the common 8-bit RGB shared-memory formats.

MIT licensed. Fonts are supplied by `epaint_default_fonts` under their upstream licenses.
