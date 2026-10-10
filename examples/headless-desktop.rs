//! A changing desktop with hover content, exclusively for the private headless tests.
#[allow(dead_code)]
#[path = "../src/document.rs"]
mod document;
#[allow(dead_code)]
#[path = "../src/fonts.rs"]
mod fonts;
use anyhow::{Context, Result, bail};
use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    os::fd::AsFd,
    path::PathBuf,
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer, wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
        wl_surface,
    },
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1 as shell, zwlr_layer_surface_v1 as layer,
};
struct State {
    width: u32,
    height: u32,
    dirty: bool,
    hover: bool,
    pointer: Option<wl_pointer::WlPointer>,
    buffers: Vec<(usize, File, wl_buffer::WlBuffer)>,
    next: usize,
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
impl Dispatch<layer::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        s: &mut Self,
        l: &layer::ZwlrLayerSurfaceV1,
        e: layer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let layer::Event::Configure {
            serial,
            width,
            height,
        } = e
        {
            l.ack_configure(serial);
            s.width = width;
            s.height = height;
            s.dirty = true;
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        s: &mut Self,
        seat: &wl_seat::WlSeat,
        e: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = e
            && caps.contains(wl_seat::Capability::Pointer)
            && s.pointer.is_none()
        {
            s.pointer = Some(seat.get_pointer(qh, ()));
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        s: &mut Self,
        _: &wl_pointer::WlPointer,
        e: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            wl_pointer::Event::Enter {
                surface_x,
                surface_y,
                ..
            }
            | wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                let hover =
                    (100.0..=300.0).contains(&surface_x) && (100.0..=200.0).contains(&surface_y);
                if s.hover != hover {
                    s.hover = hover;
                    s.dirty = true;
                }
            }
            wl_pointer::Event::Leave { .. } => {
                s.hover = false;
                s.dirty = true;
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_buffer::WlBuffer, usize> for State {
    fn event(
        s: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        e: wl_buffer::Event,
        id: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = e {
            buffer.destroy();
            s.buffers.retain(|(i, _, _)| i != id);
        }
    }
}
delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_output::WlOutput);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore shell::ZwlrLayerShellV1);
fn main() -> Result<()> {
    let runtime = std::env::var("XDG_RUNTIME_DIR")?;
    if !runtime.starts_with("/tmp/bullseye-headless-")
        || std::env::var("WLR_BACKENDS").as_deref() != Ok("headless")
    {
        bail!("Fixture restricted to private headless Sway");
    }
    let control = PathBuf::from(std::env::args().nth(1).context("Missing control file")?);
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ())?;
    let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ())?;
    let shell: shell::ZwlrLayerShellV1 = globals.bind(&qh, 1..=4, ())?;
    let output: wl_output::WlOutput = globals.bind(&qh, 1..=4, ())?;
    let _seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ())?;
    let surface = compositor.create_surface(&qh, ());
    let layer = shell.get_layer_surface(
        &surface,
        Some(&output),
        shell::Layer::Background,
        "bullseye-test-desktop".into(),
        &qh,
        (),
    );
    layer.set_anchor(layer::Anchor::all());
    layer.set_size(0, 0);
    surface.commit();
    let mut s = State {
        width: 0,
        height: 0,
        dirty: false,
        hover: false,
        pointer: None,
        buffers: vec![],
        next: 0,
    };
    let mut previous = String::new();
    loop {
        queue.dispatch_pending(&mut s)?;
        let mode = std::fs::read_to_string(&control).unwrap_or_default();
        if mode != previous {
            previous = mode;
            s.dirty = true;
        }
        if s.dirty && s.width > 0 {
            let mut image = image::imageops::resize(
                &document::demo_image(),
                s.width,
                s.height,
                image::imageops::FilterType::Triangle,
            );
            if previous.trim() == "changed" {
                for p in image.pixels_mut() {
                    p.0 = [30, 120, 190, 255];
                }
            }
            if s.hover {
                for y in 210..290.min(s.height) {
                    for x in 100..440.min(s.width) {
                        image.put_pixel(x, y, image::Rgba([255, 220, 110, 255]));
                    }
                }
                document::draw_text(
                    &mut image,
                    "Hover text — keep this!",
                    egui::pos2(115., 230.),
                    22.,
                    egui::Color32::BLACK,
                );
            }
            let mut file = tempfile::tempfile()?;
            file.set_len(u64::from(s.width) * u64::from(s.height) * 4)?;
            let bytes: Vec<u8> = image
                .pixels()
                .flat_map(|p| u32::from_be_bytes([255, p[0], p[1], p[2]]).to_ne_bytes())
                .collect();
            file.seek(SeekFrom::Start(0))?;
            file.write_all(&bytes)?;
            let pool = shm.create_pool(file.as_fd(), (s.width * s.height * 4) as i32, &qh, ());
            let buffer = pool.create_buffer(
                0,
                s.width as i32,
                s.height as i32,
                s.width as i32 * 4,
                wl_shm::Format::Xrgb8888,
                &qh,
                s.next,
            );
            pool.destroy();
            surface.attach(Some(&buffer), 0, 0);
            surface.damage_buffer(0, 0, s.width as i32, s.height as i32);
            surface.commit();
            s.buffers.push((s.next, file, buffer));
            s.next += 1;
            s.dirty = false;
            std::fs::write(control.with_extension("ready"), b"ready")?;
        }
        conn.flush()?;
        if let Some(guard) = queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut polls = [rustix::event::PollFd::new(
                &fd,
                rustix::event::PollFlags::IN,
            )];
            let timeout = rustix::event::Timespec {
                tv_sec: 0,
                tv_nsec: 100_000_000,
            };
            if rustix::event::poll(&mut polls, Some(&timeout))? > 0 {
                guard.read()?;
            }
        }
    }
}
