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
    frames: Vec<FrameBuffer>,
}
#[derive(Default)]
struct FrameBuffer {
    file: Option<File>,
    buffer: Option<wl_buffer::WlBuffer>,
    layout: Option<(wl_shm::Format, u32, u32, u32)>,
    flipped: bool,
    done: bool,
    error: Option<String>,
}

pub fn screenshot(output: Option<String>, cursor: bool) -> Result<(RgbaImage, String)> {
    Ok(capture_timeout(output, cursor, false)?.remove(0))
}

pub fn screenshots(cursor: bool) -> Result<Vec<(RgbaImage, String)>> {
    capture_timeout(None, cursor, true)
}

fn capture_timeout(
    output: Option<String>,
    cursor: bool,
    all: bool,
) -> Result<Vec<(RgbaImage, String)>> {
    // A stalled compositor must never leave the launch command hanging indefinitely.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(capture(output.as_deref(), cursor, all));
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

fn capture(output: Option<&str>, cursor: bool, all: bool) -> Result<Vec<(RgbaImage, String)>> {
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
    let selected: Vec<_> = if all {
        state.outputs.clone()
    } else {
        vec![
            match output {
                Some(name) => state.outputs.iter().find(|(_, n, _)| n == name),
                None => state.outputs.first(),
            }
            .context("Output not found. Run --list-outputs to find connector names")?
            .clone(),
        ]
    };
    if selected.is_empty() {
        bail!("No Wayland outputs available for capture");
    }
    let mut frames = Vec::with_capacity(selected.len());
    for (idx, (proxy, _, _)) in selected.iter().enumerate() {
        state.frames.push(FrameBuffer::default());
        frames.push(manager.capture_output(i32::from(cursor), proxy, &qh, idx));
    }
    // Request every output before dispatching, so monitors can be copied in the same compositor cycle.
    while state.frames.iter().any(|frame| !frame.done) {
        queue.blocking_dispatch(&mut state)?;
    }
    for frame in frames {
        frame.destroy();
    }
    selected
        .into_iter()
        .zip(state.frames)
        .map(|((_, name, transform), mut frame)| {
            if let Some(error) = frame.error {
                bail!(error);
            }
            let (format, width, height, stride) = frame
                .layout
                .context("Compositor did not provide a pixel buffer")?;
            let mut bytes = vec![0; stride as usize * height as usize];
            let file = frame.file.as_mut().context("Missing screenshot buffer")?;
            file.seek(SeekFrom::Start(0))?;
            file.read_exact(&mut bytes)?;
            let image = decode(&bytes, format, width, height, stride, frame.flipped)?;
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
        })
        .collect()
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
    for (y, destination) in image
        .as_mut()
        .chunks_exact_mut(width as usize * 4)
        .enumerate()
    {
        let row = if flipped { height as usize - 1 - y } else { y };
        let source = &bytes[row * stride as usize..row * stride as usize + width as usize * 4];
        for (src, dst) in source
            .as_chunks::<4>()
            .0
            .iter()
            .zip(destination.as_chunks_mut::<4>().0)
        {
            // Row-wise traversal avoids coordinate division and repeated stride arithmetic.
            let pixel = if bgr {
                [src[2], src[1], src[0], 255]
            } else {
                [src[0], src[1], src[2], 255]
            };
            dst.copy_from_slice(&pixel);
        }
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
impl Dispatch<frame::ZwlrScreencopyFrameV1, usize> for Capture {
    fn event(
        state: &mut Self,
        proxy: &frame::ZwlrScreencopyFrameV1,
        event: frame::Event,
        idx: &usize,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let frame = &mut state.frames[*idx];
        match event {
            frame::Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => {
                frame.layout = Some((format, width, height, stride));
                // v1/v2 have no buffer_done; their sole shm offer can be copied immediately.
                if proxy.version() < 3 {
                    state.copy(*idx, proxy, qh);
                }
            }
            frame::Event::BufferDone => state.copy(*idx, proxy, qh),
            frame::Event::Flags {
                flags: WEnum::Value(flags),
            } => frame.flipped = flags.contains(frame::Flags::YInvert),
            frame::Event::Ready { .. } => frame.done = true,
            frame::Event::Failed => {
                frame.error = Some("Compositor refused screenshot capture".into());
                frame.done = true;
            }
            _ => {}
        }
    }
}
impl Capture {
    fn copy(&mut self, idx: usize, frame: &frame::ZwlrScreencopyFrameV1, qh: &QueueHandle<Self>) {
        let result = (|| -> Result<()> {
            let (format, width, height, stride) = self.frames[idx]
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
            self.frames[idx].file = Some(file);
            self.frames[idx].buffer = Some(buffer);
            Ok(())
        })();
        if let Err(error) = result {
            self.frames[idx].error = Some(error.to_string());
            self.frames[idx].done = true;
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
    #[test]
    fn padded_abgr_keeps_rgb_and_discards_unused_alpha() {
        let bytes = [1, 2, 3, 0, 4, 5, 6, 12, 99, 99, 99, 99];
        let image = decode(&bytes, wl_shm::Format::Abgr8888, 2, 1, 12, false).unwrap();
        assert_eq!(image.as_raw(), &[1, 2, 3, 255, 4, 5, 6, 255]);
    }
}
