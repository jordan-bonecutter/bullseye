//! Virtual-pointer driver for the isolated compositor used by scripts/headless-test.py.
//! Refuses to connect to the user's regular runtime directory.
use anyhow::{Context, Result, bail};
use std::{
    io::{self, BufRead, Write},
    os::fd::AsFd,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, delegate_noop,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_pointer::ButtonState, wl_registry, wl_seat},
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
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
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ignore ZwpVirtualKeyboardV1);
fn keymap() -> Result<Vec<u8>> {
    let api = xkbcommon_dl::xkbcommon_option().context("No libxkbcommon")?;
    // SAFETY: valid library-owned handles; the exported string is malloc-allocated.
    unsafe {
        let ctx = (api.xkb_context_new)(xkbcommon_dl::xkb_context_flags::XKB_CONTEXT_NO_FLAGS);
        anyhow::ensure!(!ctx.is_null(), "No XKB context");
        let map = (api.xkb_keymap_new_from_names)(
            ctx,
            std::ptr::null(),
            xkbcommon_dl::xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS,
        );
        if map.is_null() {
            (api.xkb_context_unref)(ctx);
            anyhow::bail!("No XKB keymap");
        }
        let text = (api.xkb_keymap_get_as_string)(
            map,
            xkbcommon_dl::xkb_keymap_format::XKB_KEYMAP_FORMAT_TEXT_V1,
        );
        let bytes = if text.is_null() {
            None
        } else {
            Some(std::ffi::CStr::from_ptr(text).to_bytes_with_nul().to_vec())
        };
        libc::free(text.cast_mut().cast());
        (api.xkb_keymap_unref)(map);
        (api.xkb_context_unref)(ctx);
        bytes.context("No keymap text")
    }
}
fn code(c: char) -> Result<u32> {
    Ok(match c {
        'a' => 30,
        'b' => 48,
        'c' => 46,
        'd' => 32,
        'e' => 18,
        'f' => 33,
        'g' => 34,
        'h' => 35,
        'i' => 23,
        'j' => 36,
        'k' => 37,
        'l' => 38,
        'm' => 50,
        'n' => 49,
        'o' => 24,
        'p' => 25,
        'q' => 16,
        'r' => 19,
        's' => 31,
        't' => 20,
        'u' => 22,
        'v' => 47,
        'w' => 17,
        'x' => 45,
        'y' => 21,
        'z' => 44,
        ' ' => 57,
        _ => bail!("Unsupported fixture character: {c}"),
    })
}
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
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ())?;
    let keyboard_manager: ZwpVirtualKeyboardManagerV1 = globals.bind(&qh, 1..=1, ())?;
    let keyboard = keyboard_manager.create_virtual_keyboard(&seat, &qh, ());
    let bytes = keymap()?;
    let mut file = tempfile::tempfile()?;
    file.write_all(&bytes)?;
    keyboard.keymap(1, file.as_fd(), bytes.len() as u32);
    queue.roundtrip(&mut state)?;
    let mut extent = (1440, 900);
    let start = Instant::now();
    for line in io::stdin().lock().lines() {
        let line = line?;
        let parts: Vec<_> = line.split_whitespace().collect();
        let time = start.elapsed().as_millis() as u32;
        match parts.as_slice() {
            ["move", x, y] => {
                pointer.motion_absolute(time, x.parse()?, y.parse()?, extent.0, extent.1)
            }
            ["extent", w, h] => extent = (w.parse()?, h.parse()?),
            ["press"] => pointer.button(time, 0x110, ButtonState::Pressed),
            ["release"] => pointer.button(time, 0x110, ButtonState::Released),
            ["key", value] => {
                let code = value.parse()?;
                keyboard.key(time, code, 1);
                keyboard.key(time + 1, code, 0);
            }
            ["shortcut", value] => {
                keyboard.modifiers(4, 0, 0, 0);
                keyboard.key(
                    time,
                    code(value.chars().next().context("Missing shortcut")?)?,
                    1,
                );
                keyboard.key(time + 1, code(value.chars().next().unwrap())?, 0);
                keyboard.modifiers(0, 0, 0, 0);
            }
            ["type", words @ ..] => {
                for c in words.join(" ").chars() {
                    let code = code(c)?;
                    keyboard.key(time, code, 1);
                    keyboard.key(time + 1, code, 0);
                }
            }
            ["wait", ms] => std::thread::sleep(Duration::from_millis(ms.parse()?)),
            _ => bail!("Unknown input action: {line}"),
        }
        pointer.frame();
        queue.roundtrip(&mut state)?;
        std::thread::sleep(Duration::from_millis(40));
        println!("OK");
        io::stdout().flush()?;
    }
    keyboard.destroy();

    pointer.destroy();
    manager.destroy();
    conn.flush()?;
    Ok(())
}
