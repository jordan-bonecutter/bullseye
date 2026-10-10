//! Real layer-shell overlays: capture first, then cover each output with its frozen image.
use crate::{app::OverlayUi, gpu::Gpu, keyboard::Keyboard};
use anyhow::{Context, Result, bail};
use egui::{Event, Modifiers, Pos2, Vec2};
use image::RgbaImage;
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    os::fd::AsFd,
    path::PathBuf,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry,
        wl_seat, wl_shm, wl_shm_pool, wl_surface,
    },
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1 as shell, zwlr_layer_surface_v1 as layer,
};

struct Slot {
    file: File,
    buffer: wl_buffer::WlBuffer,
    busy: bool,
}
struct View {
    name: String,
    output: wl_output::WlOutput,
    surface: Option<wl_surface::WlSurface>,
    layer: Option<layer::ZwlrLayerSurfaceV1>,
    viewport: Option<wp_viewport::WpViewport>,
    width: u32,
    height: u32,
    ui: OverlayUi,
    gpu: Option<Gpu>,
    input: Vec<Event>,
    dirty: bool,
    frame_pending: bool,
    repaint_at: Option<Instant>,
}
struct State {
    views: Vec<View>,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard_proxy: Option<wl_keyboard::WlKeyboard>,
    keyboard: Option<Keyboard>,
    modifiers: Modifiers,
    pointer_view: Option<usize>,
    keyboard_view: Option<usize>,
    selected: Option<usize>,
    position: Pos2,
    primary_down: bool,
    picking: bool,
    cursor_surface: wl_surface::WlSurface,
    cursor_slot: Slot,
    exit: bool,
    error: Option<String>,
    started: Instant,
}
#[derive(Clone, Copy)]
struct BufferData {
    view: usize,
}

pub fn run(
    images: Vec<(RgbaImage, String)>,
    destination: PathBuf,
    explicit_destination: bool,
) -> Result<()> {
    let conn = Connection::connect_to_env().context("Connect to Wayland for selection")?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals
        .bind(&qh, 1..=4, ())
        .context("Missing wl_compositor")?;
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).context("Missing wl_shm")?;
    let shell: shell::ZwlrLayerShellV1 = globals
        .bind(&qh, 1..=4, ())
        .context("The compositor must support wlr-layer-shell for desktop selection")?;
    let viewporter: Option<wp_viewporter::WpViewporter> = globals.bind(&qh, 1..=1, ()).ok();
    let cursor_surface = compositor.create_surface(&qh, ());
    let mut cursor_slot = allocate(&shm, &qh, 32, 32, BufferData { view: usize::MAX })?;
    let mut cursor = RgbaImage::new(32, 32);
    for y in 0..32 {
        for x in 0..32 {
            let dx = (x as i32 - 15).abs();
            let dy = (y as i32 - 15).abs();
            if (dx <= 1 && (4..=12).contains(&dy)) || (dy <= 1 && (4..=12).contains(&dx)) {
                cursor.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
            if (dx == 0 && (4..=12).contains(&dy)) || (dy == 0 && (4..=12).contains(&dx)) {
                cursor.put_pixel(x, y, image::Rgba([255, 255, 255, 255]));
            }
        }
    }
    write_buffer(&mut cursor_slot, &cursor)?;
    cursor_surface.attach(Some(&cursor_slot.buffer), 0, 0);
    cursor_surface.damage_buffer(0, 0, 32, 32);
    cursor_surface.commit();
    let mut state = State {
        views: vec![],
        pointer: None,
        keyboard_proxy: None,
        keyboard: None,
        modifiers: Modifiers::default(),
        pointer_view: None,
        keyboard_view: None,
        selected: None,
        position: Pos2::ZERO,
        primary_down: false,
        picking: false,
        cursor_surface,
        cursor_slot,
        exit: false,
        error: None,
        started: Instant::now(),
    };
    for global in globals.contents().clone_list() {
        if global.interface == "wl_output" {
            let idx = state.views.len();
            let output = globals.registry().bind::<wl_output::WlOutput, _, _>(
                global.name,
                global.version.min(4),
                &qh,
                idx,
            );
            state.views.push(View {
                name: format!("output-{}", global.name),
                output,
                surface: None,
                layer: None,
                viewport: None,
                width: 0,
                height: 0,
                ui: OverlayUi::new(
                    RgbaImage::new(1, 1),
                    destination.clone(),
                    explicit_destination,
                ),
                gpu: None,
                input: vec![],
                dirty: false,
                frame_pending: false,
                repaint_at: None,
            });
        }
    }
    let _seat: wl_seat::WlSeat = globals
        .bind(&qh, 1..=7, ())
        .context("Missing Wayland seat")?;
    queue.roundtrip(&mut state)?;
    let mut mapped = 0;
    for view in &mut state.views {
        if let Some((image, _)) = images.iter().find(|(_, name)| name == &view.name) {
            view.ui = OverlayUi::new(image.clone(), destination.clone(), explicit_destination);
            mapped += 1;
        } else {
            continue;
        }
    }
    if mapped == 0 {
        bail!("Captured outputs disappeared before selection started");
    }
    for (idx, view) in state.views.iter_mut().enumerate() {
        if !images.iter().any(|(_, name)| name == &view.name) {
            continue;
        }
        let surface = compositor.create_surface(&qh, ());
        let layer = shell.get_layer_surface(
            &surface,
            Some(&view.output),
            shell::Layer::Overlay,
            "bullseye".into(),
            &qh,
            idx,
        );
        layer.set_size(0, 0);
        layer.set_anchor(layer::Anchor::all());
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(layer::KeyboardInteractivity::Exclusive);
        view.viewport = viewporter
            .as_ref()
            .map(|v| v.get_viewport(&surface, &qh, ()));
        surface.commit();
        view.surface = Some(surface);
        view.layer = Some(layer);
    }
    conn.flush()?;
    while !state.exit {
        queue.dispatch_pending(&mut state)?;
        state.render(&conn, &qh)?;
        conn.flush()?;
        if state.exit {
            break;
        }
        if let Some(seconds) = std::env::var("BULLSEYE_SMOKE_SECONDS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            && state.started.elapsed() >= Duration::from_secs(seconds)
        {
            break;
        }
        if let Some(guard) = queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut poll = [rustix::event::PollFd::new(
                &fd,
                rustix::event::PollFlags::IN,
            )];
            let delay = state
                .views
                .iter()
                .filter_map(|v| v.repaint_at)
                .map(|at| at.saturating_duration_since(Instant::now()))
                .min()
                .unwrap_or(Duration::from_millis(100))
                .min(Duration::from_millis(100));
            let timeout = rustix::event::Timespec {
                tv_sec: 0,
                tv_nsec: delay.as_nanos() as i64,
            };
            if rustix::event::poll(&mut poll, Some(&timeout))? > 0 {
                guard.read()?;
            }
        }
    }
    for view in &mut state.views {
        view.gpu.take();
        if let Some(layer) = &view.layer {
            layer.destroy();
        }
        if let Some(surface) = &view.surface {
            surface.destroy();
        }
    }
    conn.flush()?;
    if let Some(error) = state.error {
        bail!(error);
    }
    Ok(())
}
fn allocate(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<State>,
    width: u32,
    height: u32,
    data: BufferData,
) -> Result<Slot> {
    let size = u64::from(width) * u64::from(height) * 4;
    if size > i32::MAX as u64 || size == 0 {
        bail!("Overlay buffer dimensions are invalid");
    }
    let file = tempfile::tempfile()?;
    file.set_len(size)?;
    let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
    let buffer = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        width as i32 * 4,
        wl_shm::Format::Argb8888,
        qh,
        data,
    );
    pool.destroy();
    Ok(Slot {
        file,
        buffer,
        busy: false,
    })
}
fn write_buffer(slot: &mut Slot, image: &RgbaImage) -> Result<()> {
    let mut bytes = Vec::with_capacity(image.as_raw().len());
    for p in image.pixels() {
        let alpha = u32::from(p[3]);
        let argb = (alpha << 24)
            | ((u32::from(p[0]) * alpha / 255) << 16)
            | ((u32::from(p[1]) * alpha / 255) << 8)
            | (u32::from(p[2]) * alpha / 255);
        bytes.extend_from_slice(&argb.to_ne_bytes());
    }
    slot.file.seek(SeekFrom::Start(0))?;
    slot.file.write_all(&bytes)?;
    Ok(())
}
impl State {
    fn render(&mut self, conn: &Connection, qh: &QueueHandle<Self>) -> Result<()> {
        let metadata = std::env::var_os("BULLSEYE_TEST_STATE").map(|_|self.views.iter().map(|v|serde_json::json!({"name":v.name,"logical":[v.width,v.height],"native":[v.ui.doc.original.width(),v.ui.doc.original.height()],"mapped":v.surface.is_some()})).collect::<Vec<_>>());
        for idx in 0..self.views.len() {
            let view = &mut self.views[idx];
            if view.ui.picker_active() {
                view.ui.poll_picker();
                view.repaint_at = Some(Instant::now() + Duration::from_millis(30));
                view.dirty = true;
                if view.ui.done {
                    self.exit = true;
                }
            }
            if view.repaint_at.is_some_and(|at| Instant::now() >= at) {
                view.dirty = true;
                view.repaint_at = None;
            }
            if !view.dirty || view.frame_pending || view.width == 0 || view.height == 0 {
                continue;
            }
            let Some(surface) = view.surface.clone() else {
                continue;
            };
            let native = view.ui.doc.original.dimensions();
            let buffer_size = if view.viewport.is_some() {
                (native.0.max(view.width), native.1.max(view.height))
            } else {
                (view.width, view.height)
            };
            if view.gpu.is_none() {
                view.gpu = Some(Gpu::new(conn, &surface, buffer_size)?);
            }
            let active = self.selected.is_none_or(|selected| selected == idx);
            let ppp = buffer_size.0 as f32 / view.width as f32;
            let logical = Vec2::new(view.width as f32, view.height as f32);
            let mut raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, logical)),
                events: std::mem::take(&mut view.input),
                modifiers: self.modifiers,
                time: Some(self.started.elapsed().as_secs_f64()),
                focused: true,
                ..Default::default()
            };
            raw.viewports
                .get_mut(&egui::ViewportId::ROOT)
                .unwrap()
                .native_pixels_per_point = Some(ppp);
            let frame_started = Instant::now();
            let output = view.ui.frame(raw, logical, active);
            let repaint_delay = output
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .map(|v| v.repaint_delay)
                .unwrap_or(Duration::MAX);
            view.repaint_at = Instant::now().checked_add(repaint_delay);
            let primitives = view
                .ui
                .ctx
                .tessellate(output.shapes, output.pixels_per_point);
            if let Some(viewport) = &view.viewport {
                viewport.set_destination(view.width as i32, view.height as i32);
            }
            surface.frame(qh, idx);
            view.frame_pending = true;
            view.gpu.as_mut().unwrap().paint(
                buffer_size,
                output.pixels_per_point,
                &primitives,
                &output.textures_delta,
            )?;
            view.dirty = false;
            if let Ok(path) = std::env::var("BULLSEYE_TEST_STATE")
                && std::env::var("XDG_RUNTIME_DIR")
                    .unwrap_or_default()
                    .starts_with("/tmp/bullseye-headless-")
            {
                let hits: serde_json::Map<String, serde_json::Value> = view
                    .ui
                    .hits
                    .iter()
                    .map(|(k, r)| (k.clone(), serde_json::json!([r.center().x, r.center().y])))
                    .collect();
                let (x, y, w, h) = view.ui.doc.crop_pixels();
                let data = serde_json::json!({"views":metadata,"output":view.name,"phase":if view.ui.doc.crop.is_some(){"annotate"}else{"select"},"buttons":hits,"crop":[x,y,w,h],"marks":view.ui.doc.marks.len(),"logical":[view.width,view.height],"frame":view.ui.ctx.cumulative_frame_nr(),"primitives":primitives.len(),"toolbar_visible":view.ui.toolbar_visible,"text":view.ui.pending_text(),"tool":view.ui.tool(),"color":view.ui.color(),"render_us":frame_started.elapsed().as_micros(),"texture_updates":output.textures_delta.set.len(),"picker":view.ui.picker_active(),"save_dialog":view.ui.fallback_dialog(),"repaint_ms":repaint_delay.as_millis().min(1000000)});
                let parent = std::path::Path::new(&path)
                    .parent()
                    .unwrap_or(std::path::Path::new("."));
                let mut file = tempfile::NamedTempFile::new_in(parent)?;
                file.write_all(&serde_json::to_vec(&data)?)?;
                file.persist(path)?;
            }
            view.ui.start_picker();
            if view.ui.picker_active() {
                view.repaint_at = Some(Instant::now() + Duration::from_millis(30));
            }
            let claimed = view.ui.claimed;
            if view.ui.done {
                self.exit = true;
            }
            if claimed {
                if let Some(old) = self.selected
                    && old != idx
                {
                    self.views[old].ui.deactivate();
                    self.views[old].dirty = true;
                }
                self.selected = Some(idx);
                self.views[idx].dirty = true;
                for (other, v) in self.views.iter_mut().enumerate() {
                    if other != idx {
                        v.dirty = true;
                    }
                }
            }
        }
        let picking = self.views.iter().any(|v| v.ui.picker_active());
        if self.picking != picking {
            self.picking = picking;
            for v in &self.views {
                if let Some(layer) = &v.layer {
                    layer.set_layer(if picking {
                        shell::Layer::Bottom
                    } else {
                        shell::Layer::Overlay
                    });
                    layer.set_keyboard_interactivity(if picking {
                        layer::KeyboardInteractivity::None
                    } else {
                        layer::KeyboardInteractivity::Exclusive
                    });
                    if let Some(surface) = &v.surface {
                        surface.commit();
                    }
                }
            }
        }
        Ok(())
    }
    fn input(&mut self, idx: usize, event: Event) {
        if self.picking {
            return;
        }
        if let Some(view) = self.views.get_mut(idx) {
            view.input.push(event);
            view.dirty = true;
        }
    }
}
impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        idx: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            state.views[*idx].name = name;
        }
    }
}
impl Dispatch<layer::ZwlrLayerSurfaceV1, usize> for State {
    fn event(
        state: &mut Self,
        layer: &layer::ZwlrLayerSurfaceV1,
        event: layer::Event,
        idx: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            layer::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer.ack_configure(serial);
                let view = &mut state.views[*idx];
                view.width = width;
                view.height = height;
                view.dirty = true;
            }
            layer::Event::Closed => state.exit = true,
            _ => {}
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Keyboard) && state.keyboard_proxy.is_none() {
                state.keyboard_proxy = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
            } => {
                state.pointer_view = state
                    .views
                    .iter()
                    .position(|v| v.surface.as_ref() == Some(&surface));
                state.position = Pos2::new(surface_x as f32, surface_y as f32);
                pointer.set_cursor(serial, Some(&state.cursor_surface), 15, 15);
                if let Some(idx) = state.pointer_view {
                    state.input(idx, Event::PointerMoved(state.position));
                }
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.position = Pos2::new(surface_x as f32, surface_y as f32);
                if let Some(idx) = state.pointer_view {
                    state.input(idx, Event::PointerMoved(state.position));
                }
            }
            wl_pointer::Event::Leave { .. } => {
                if let Some(idx) = state.pointer_view {
                    if state.primary_down {
                        state.input(
                            idx,
                            Event::PointerButton {
                                pos: state.position,
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                modifiers: state.modifiers,
                            },
                        );
                    }
                    state.input(idx, Event::PointerGone);
                }
                state.pointer_view = None;
            }
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(button_state),
                ..
            } => {
                if let Some(idx) = state.pointer_view {
                    if button == 0x110 {
                        state.primary_down = button_state == wl_pointer::ButtonState::Pressed;
                        state.input(
                            idx,
                            Event::PointerButton {
                                pos: state.position,
                                button: egui::PointerButton::Primary,
                                pressed: button_state == wl_pointer::ButtonState::Pressed,
                                modifiers: state.modifiers,
                            },
                        );
                    }
                    if button == 0x111 && button_state == wl_pointer::ButtonState::Pressed {
                        state.exit = true;
                    }
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Keymap {
                format: WEnum::Value(wl_keyboard::KeymapFormat::XkbV1),
                fd,
                size,
            } => match Keyboard::new(fd, size) {
                Ok(k) => state.keyboard = Some(k),
                Err(e) => {
                    state.error = Some(e.to_string());
                    state.exit = true;
                }
            },
            wl_keyboard::Event::Enter { surface, .. } => {
                state.keyboard_view = state
                    .views
                    .iter()
                    .position(|v| v.surface.as_ref() == Some(&surface))
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                if let Some(keyboard) = &mut state.keyboard {
                    state.modifiers =
                        keyboard.modifiers(mods_depressed, mods_latched, mods_locked, group);
                }
            }
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(key_state),
                ..
            } => {
                if let Some(idx) = state
                    .selected
                    .or(state.keyboard_view)
                    .or(state.pointer_view)
                    && let Some(keyboard) = &state.keyboard
                {
                    let events = keyboard.events(
                        key,
                        key_state == wl_keyboard::KeyState::Pressed,
                        state.modifiers,
                    );
                    for event in events {
                        state.input(idx, event);
                    }
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_buffer::WlBuffer, BufferData> for State {
    fn event(
        state: &mut Self,
        _: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        data: &BufferData,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event
            && data.view == usize::MAX
        {
            state.cursor_slot.busy = false;
        }
    }
}
delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore shell::ZwlrLayerShellV1);
delegate_noop!(State: ignore wp_viewporter::WpViewporter);
delegate_noop!(State: ignore wp_viewport::WpViewport);

impl Dispatch<wl_callback::WlCallback, usize> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        idx: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.views[*idx].frame_pending = false;
        }
    }
}
