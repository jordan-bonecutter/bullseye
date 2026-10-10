//! Native wlr-screencopy capture. No grim/slurp subprocesses or X11 fallback.
use anyhow::{Context, Result, bail};
use image::RgbaImage;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::fd::AsFd,
    sync::mpsc,
    time::Duration,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool},
};
use wayland_protocols_wlr::screencopy::v1::client::{
    zwlr_screencopy_frame_v1 as frame, zwlr_screencopy_manager_v1 as manager,
};

#[derive(Default)]
struct Capture {
    outputs: Vec<(wl_output::WlOutput, String, wl_output::Transform)>,
    shm: Option<wl_shm::WlShm>,
    file: Option<File>,
    buffer: Option<wl_buffer::WlBuffer>,
    layout: Option<(wl_shm::Format, u32, u32, u32)>,
    flipped: bool,
    done: bool,
    error: Option<String>,
}

pub fn screenshot(output: Option<String>, cursor: bool) -> Result<(RgbaImage, String)> {
    // A stalled compositor must never leave the launch command hanging indefinitely.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(capture(output.as_deref(), cursor));
    });
    rx.recv_timeout(Duration::from_secs(10))
        .context("Screenshot timed out after 10 seconds")?
}

pub fn list_outputs() -> Result<Vec<String>> {
    let conn = Connection::connect_to_env().context("Connect to Wayland")?;
    let (globals, mut queue) = registry_queue_init::<Capture>(&conn)?;
    let mut state = Capture::default();
    bind_outputs(&globals, &queue.handle(), &mut state);
    queue.roundtrip(&mut state)?;
    Ok(state.outputs.into_iter().map(|(_, name, _)| name).collect())
}
fn bind_outputs(
    globals: &wayland_client::globals::GlobalList,
    qh: &QueueHandle<Capture>,
    state: &mut Capture,
) {
    for global in globals.contents().clone_list() {
        if global.interface == "wl_output" {
            let proxy = globals.registry().bind::<wl_output::WlOutput, _, _>(
                global.name,
                global.version.min(4),
                qh,
                (),
            );
            state.outputs.push((
                proxy,
                format!("output-{}", global.name),
                wl_output::Transform::Normal,
            ));
        }
    }
}

fn capture(output: Option<&str>, cursor: bool) -> Result<(RgbaImage, String)> {
    let connection = Connection::connect_to_env()
        .context("Connect to Wayland. Run Bullseye inside your Sway session, or use --demo")?;
    let (globals, mut queue) = registry_queue_init::<Capture>(&connection)?;
    let qh = queue.handle();
    let mut state = Capture {
        shm: Some(
            globals
                .bind(&qh, 1..=1, ())
                .context("Compositor has no wl_shm")?,
        ),
        ..Default::default()
    };
    let manager: manager::ZwlrScreencopyManagerV1 = globals.bind(&qh, 1..=3, ())
        .context("This compositor does not support wlr-screencopy. Sway/wlroots is required; GNOME/KDE portal capture is not implemented yet")?;
    bind_outputs(&globals, &qh, &mut state);
    queue.roundtrip(&mut state)?;
    let selected = match output {
        Some(name) => state.outputs.iter().find(|(_, n, _)| n == name),
        None => state.outputs.first(),
    }
    .context("Output not found. Run --list-outputs to find connector names")?;
    let (proxy, name, transform) = selected.clone();
    let frame = manager.capture_output(i32::from(cursor), &proxy, &qh, ());
    while !state.done {
        queue.blocking_dispatch(&mut state)?;
    }
    frame.destroy();
    if let Some(error) = state.error {
        bail!(error);
    }
    let (format, width, height, stride) = state
        .layout
        .context("Compositor did not provide a pixel buffer")?;
    let mut bytes = vec![0; stride as usize * height as usize];
    let file = state.file.as_mut().context("Missing screenshot buffer")?;
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut bytes)?;
    let image = decode(&bytes, format, width, height, stride, state.flipped)?;
    // wl_output transform describes the orientation of the physical buffer.
    let image = match transform {
        wl_output::Transform::_90 => image::imageops::rotate90(&image),
        wl_output::Transform::_180 => image::imageops::rotate180(&image),
        wl_output::Transform::_270 => image::imageops::rotate270(&image),
        wl_output::Transform::Flipped => image::imageops::flip_horizontal(&image),
        wl_output::Transform::Flipped90 => {
            image::imageops::rotate270(&image::imageops::flip_horizontal(&image))
        }
        wl_output::Transform::Flipped180 => {
            image::imageops::rotate180(&image::imageops::flip_horizontal(&image))
        }
        wl_output::Transform::Flipped270 => {
            image::imageops::rotate90(&image::imageops::flip_horizontal(&image))
        }
        _ => image,
    };
    Ok((image, name))
}

fn decode(
    bytes: &[u8],
    format: wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
    flipped: bool,
) -> Result<RgbaImage> {
    let bgr = match format {
        wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => true,
        wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888 => false,
        _ => bail!("Unsupported screenshot format: {format:?}"),
    };
    if stride < width.saturating_mul(4) || bytes.len() < stride as usize * height as usize {
        bail!("Invalid screenshot buffer dimensions");
    }
    let mut image = RgbaImage::new(width, height);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let row = if flipped { height - 1 - y } else { y };
        let i = (row * stride + x * 4) as usize;
        // Screenshots are opaque, including XRGB buffers whose unused byte can be zero.
        *pixel = if bgr {
            image::Rgba([bytes[i + 2], bytes[i + 1], bytes[i], 255])
        } else {
            image::Rgba([bytes[i], bytes[i + 1], bytes[i + 2], 255])
        };
    }
    Ok(image)
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Capture {
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
impl Dispatch<wl_output::WlOutput, ()> for Capture {
    fn event(
        state: &mut Self,
        proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let Some((_, name, transform)) = state.outputs.iter_mut().find(|(p, _, _)| p == proxy) {
            match event {
                wl_output::Event::Name { name: n } => *name = n,
                wl_output::Event::Geometry {
                    transform: WEnum::Value(t),
                    ..
                } => *transform = t,
                _ => {}
            }
        }
    }
}
impl Dispatch<frame::ZwlrScreencopyFrameV1, ()> for Capture {
    fn event(
        state: &mut Self,
        proxy: &frame::ZwlrScreencopyFrameV1,
        event: frame::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            frame::Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => {
                state.layout = Some((format, width, height, stride));
                // v1/v2 have no buffer_done; their sole shm offer can be copied immediately.
                if proxy.version() < 3 {
                    state.copy(proxy, qh);
                }
            }
            frame::Event::BufferDone => state.copy(proxy, qh),
            frame::Event::Flags {
                flags: WEnum::Value(flags),
            } => state.flipped = flags.contains(frame::Flags::YInvert),
            frame::Event::Ready { .. } => state.done = true,
            frame::Event::Failed => {
                state.error = Some("Compositor refused screenshot capture".into());
                state.done = true;
            }
            _ => {}
        }
    }
}
impl Capture {
    fn copy(&mut self, frame: &frame::ZwlrScreencopyFrameV1, qh: &QueueHandle<Self>) {
        let result = (|| -> Result<()> {
            let (format, width, height, stride) = self
                .layout
                .context("Compositor offered no supported shared-memory buffer")?;
            if width == 0
                || height == 0
                || width > i32::MAX as u32
                || height > i32::MAX as u32
                || stride > i32::MAX as u32
            {
                bail!("Invalid capture dimensions");
            }
            let size = u64::from(stride) * u64::from(height);
            if size > i32::MAX as u64 {
                bail!("Screenshot exceeds shared-memory limit");
            }
            let file = tempfile::tempfile()?;
            file.set_len(size)?;
            let pool = self.shm.as_ref().context("Missing wl_shm")?.create_pool(
                file.as_fd(),
                size as i32,
                qh,
                (),
            );
            let buffer = pool.create_buffer(
                0,
                width as i32,
                height as i32,
                stride as i32,
                format,
                qh,
                (),
            );
            pool.destroy();
            frame.copy(&buffer);
            self.file = Some(file);
            self.buffer = Some(buffer);
            Ok(())
        })();
        if let Err(error) = result {
            self.error = Some(error.to_string());
            self.done = true;
        }
    }
}
delegate_noop!(Capture: ignore wl_shm::WlShm);
delegate_noop!(Capture: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Capture: ignore wl_buffer::WlBuffer);
delegate_noop!(Capture: ignore manager::ZwlrScreencopyManagerV1);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn padded_inverted_xrgb_is_opaque_and_correct() {
        let bytes = [
            1, 2, 3, 0, 4, 5, 6, 0, 99, 99, 99, 99, 7, 8, 9, 0, 10, 11, 12, 0, 99, 99, 99, 99,
        ];
        let image = decode(&bytes, wl_shm::Format::Xrgb8888, 2, 2, 12, true).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [9, 8, 7, 255]);
        assert_eq!(image.get_pixel(1, 1).0, [6, 5, 4, 255]);
    }
    #[test]
    fn rejects_short_buffer() {
        assert!(decode(&[0; 4], wl_shm::Format::Xrgb8888, 2, 1, 8, false).is_err());
    }
}
