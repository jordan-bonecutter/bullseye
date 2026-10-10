//! Virtual-pointer driver for the isolated compositor used by scripts/headless-test.py.
//! Refuses to connect to the user's regular runtime directory.
use anyhow::{Context, Result, bail};
use std::{
    io::{self, BufRead},
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_pointer::ButtonState, wl_registry},
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
struct State;
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
delegate_noop!(State: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ignore ZwlrVirtualPointerV1);
fn main() -> Result<()> {
    let runtime = std::env::var("XDG_RUNTIME_DIR")?;
    if !runtime.starts_with("/tmp/bullseye-headless-")
        || std::env::var("WLR_BACKENDS").as_deref() != Ok("headless")
    {
        bail!("Input testing is restricted to Bullseye's isolated headless compositor");
    }
    let conn = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn)?;
    let qh = queue.handle();
    let mut state = State;
    let manager: ZwlrVirtualPointerManagerV1 = globals
        .bind(&qh, 1..=2, ())
        .context("No virtual-pointer protocol")?;
    let pointer = manager.create_virtual_pointer(None, &qh, ());
    queue.roundtrip(&mut state)?;
    let start = Instant::now();
    for line in io::stdin().lock().lines() {
        let line = line?;
        let parts: Vec<_> = line.split_whitespace().collect();
        let time = start.elapsed().as_millis() as u32;
        match parts.as_slice() {
            ["move", x, y] => pointer.motion_absolute(time, x.parse()?, y.parse()?, 1440, 900),
            ["press"] => pointer.button(time, 0x110, ButtonState::Pressed),
            ["release"] => pointer.button(time, 0x110, ButtonState::Released),
            ["wait", ms] => std::thread::sleep(Duration::from_millis(ms.parse()?)),
            _ => bail!("Unknown input action: {line}"),
        }
        pointer.frame();
        queue.roundtrip(&mut state)?;
        std::thread::sleep(Duration::from_millis(40));
    }
    pointer.destroy();
    manager.destroy();
    conn.flush()?;
    Ok(())
}
