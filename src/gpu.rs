//! EGL rendering on the existing layer-shell surface, without creating a toolkit window.
use anyhow::{Context, Result};
use glow::HasContext;
use glutin::{
    config::ConfigTemplateBuilder,
    context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext, Version},
    display::{Display, DisplayApiPreference},
    prelude::*,
    surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface},
};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use std::{ffi::CString, num::NonZeroU32, ptr::NonNull, sync::Arc};
use wayland_client::{Connection, Proxy, protocol::wl_surface::WlSurface};

pub struct Gpu {
    painter: egui_glow::Painter,
    context: PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
    gl: Arc<glow::Context>,
    size: (u32, u32),
    // Keep the native proxies alive until EGL has destroyed its resources.
    _wayland_surface: WlSurface,
    _connection: Connection,
}
impl Gpu {
    pub fn new(conn: &Connection, surface: &WlSurface, size: (u32, u32)) -> Result<Self> {
        let display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            NonNull::new(conn.backend().display_ptr().cast())
                .context("Missing native Wayland display")?,
        ));
        let window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
            NonNull::new(surface.id().as_ptr().cast()).context("Missing native Wayland surface")?,
        ));
        // SAFETY: the connection and surface proxies are retained for the complete EGL lifetime.
        let display = unsafe { Display::new(display_handle, DisplayApiPreference::Egl)? };
        let template = ConfigTemplateBuilder::new().with_alpha_size(8).build();
        let config = unsafe { display.find_configs(template)? }
            .next()
            .context("No EGL window configuration")?;
        let attrs = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
            .build(Some(window_handle));
        let context = unsafe { display.create_context(&config, &attrs)? };
        let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            window_handle,
            NonZeroU32::new(size.0).unwrap(),
            NonZeroU32::new(size.1).unwrap(),
        );
        let egl_surface = unsafe { display.create_window_surface(&config, &attrs)? };
        let context = context.make_current(&egl_surface)?;
        egl_surface.set_swap_interval(&context, SwapInterval::DontWait)?;
        let gl = Arc::new(unsafe {
            glow::Context::from_loader_function(|name| {
                display
                    .get_proc_address(&CString::new(name).unwrap())
                    .cast()
            })
        });
        eprintln!("Bullseye EGL renderer: {}", unsafe {
            gl.get_parameter_string(glow::RENDERER)
        });
        let painter = egui_glow::Painter::new(gl.clone(), "", None, false)
            .map_err(|e| anyhow::anyhow!("GPU painter: {e}"))?;
        Ok(Self {
            painter,
            context,
            surface: egl_surface,
            gl,
            size,
            _wayland_surface: surface.clone(),
            _connection: conn.clone(),
        })
    }
    pub fn paint(
        &mut self,
        size: (u32, u32),
        ppp: f32,
        primitives: &[egui::ClippedPrimitive],
        textures: &egui::TexturesDelta,
    ) -> Result<()> {
        self.context.make_current(&self.surface)?;
        if size != self.size {
            self.surface.resize(
                &self.context,
                NonZeroU32::new(size.0).unwrap(),
                NonZeroU32::new(size.1).unwrap(),
            );
            self.size = size;
        }
        unsafe {
            self.gl.clear_color(0., 0., 0., 1.);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
        }
        self.painter
            .paint_and_update_textures([size.0, size.1], ppp, primitives, textures);
        self.surface.swap_buffers(&self.context)?;
        Ok(())
    }
}
impl Drop for Gpu {
    fn drop(&mut self) {
        if self.context.make_current(&self.surface).is_ok() {
            self.painter.destroy();
        }
    }
}
