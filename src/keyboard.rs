//! Interpret the compositor's keymap instead of assuming a US keyboard layout.
use anyhow::{Context, Result, bail};
use egui::{Event, Key, Modifiers};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::fd::OwnedFd,
};
use xkbcommon_dl as xkb;
pub struct Keyboard {
    api: &'static xkb::XkbCommon,
    context: *mut xkb::xkb_context,
    keymap: *mut xkb::xkb_keymap,
    state: *mut xkb::xkb_state,
}
impl Keyboard {
    pub fn new(fd: OwnedFd, size: u32) -> Result<Self> {
        let api = xkb::xkbcommon_option().context("libxkbcommon is required for keyboard input")?;
        let mut bytes = Vec::new();
        let mut file = File::from(fd);
        file.seek(SeekFrom::Start(0))?;
        file.take(u64::from(size)).read_to_end(&mut bytes)?;
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        let text = CString::new(bytes)?;
        // SAFETY: all handles come from libxkbcommon, are checked, and are owned until Drop.
        unsafe {
            let context = (api.xkb_context_new)(xkb::xkb_context_flags::XKB_CONTEXT_NO_FLAGS);
            if context.is_null() {
                bail!("Cannot initialize keyboard context");
            }
            let keymap = (api.xkb_keymap_new_from_string)(
                context,
                text.as_ptr(),
                xkb::xkb_keymap_format::XKB_KEYMAP_FORMAT_TEXT_V1,
                xkb::xkb_keymap_compile_flags::XKB_KEYMAP_COMPILE_NO_FLAGS,
            );
            if keymap.is_null() {
                (api.xkb_context_unref)(context);
                bail!("Invalid compositor keymap");
            }
            let state = (api.xkb_state_new)(keymap);
            if state.is_null() {
                (api.xkb_keymap_unref)(keymap);
                (api.xkb_context_unref)(context);
                bail!("Cannot initialize keyboard state");
            }
            Ok(Self {
                api,
                context,
                keymap,
                state,
            })
        }
    }
    pub fn modifiers(
        &mut self,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) -> Modifiers {
        // SAFETY: the xkb state is live and exclusively owned by this event-loop object.
        unsafe {
            (self.api.xkb_state_update_mask)(self.state, depressed, latched, locked, 0, 0, group);
            let active = |name: &[u8]| {
                (self.api.xkb_state_mod_name_is_active)(
                    self.state,
                    name.as_ptr().cast(),
                    xkb::xkb_state_component::XKB_STATE_MODS_EFFECTIVE,
                ) > 0
            };
            let ctrl = active(xkb::XKB_MOD_NAME_CTRL);
            Modifiers {
                alt: active(xkb::XKB_MOD_NAME_ALT),
                ctrl,
                shift: active(xkb::XKB_MOD_NAME_SHIFT),
                mac_cmd: false,
                command: ctrl,
            }
        }
    }
    pub fn events(&self, key: u32, pressed: bool, modifiers: Modifiers) -> Vec<Event> {
        let mut events = Vec::new();
        // SAFETY: the live xkb state receives a valid Wayland keycode plus the XKB offset.
        let (symbol, character) = unsafe {
            (
                (self.api.xkb_state_key_get_one_sym)(self.state, key + 8),
                (self.api.xkb_state_key_get_utf32)(self.state, key + 8),
            )
        };
        let k = match symbol {
            xkb::keysyms::Escape => Some(Key::Escape),
            xkb::keysyms::Return | xkb::keysyms::KP_Enter => Some(Key::Enter),
            xkb::keysyms::BackSpace => Some(Key::Backspace),
            xkb::keysyms::Delete => Some(Key::Delete),
            xkb::keysyms::Tab => Some(Key::Tab),
            xkb::keysyms::Left => Some(Key::ArrowLeft),
            xkb::keysyms::Right => Some(Key::ArrowRight),
            xkb::keysyms::Up => Some(Key::ArrowUp),
            xkb::keysyms::Down => Some(Key::ArrowDown),
            xkb::keysyms::Home => Some(Key::Home),
            xkb::keysyms::End => Some(Key::End),
            xkb::keysyms::space => Some(Key::Space),
            _ => char::from_u32(symbol)
                .and_then(|c| Key::from_name(&c.to_ascii_uppercase().to_string())),
        };
        if let Some(key) = k {
            events.push(Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            });
        }
        if pressed
            && !modifiers.ctrl
            && !modifiers.alt
            && let Some(c) = char::from_u32(character)
            && !c.is_control()
        {
            events.push(Event::Text(c.to_string()));
        }
        events
    }
}
impl Drop for Keyboard {
    fn drop(&mut self) {
        // SAFETY: release each owned handle once, in reverse construction order.
        unsafe {
            (self.api.xkb_state_unref)(self.state);
            (self.api.xkb_keymap_unref)(self.keymap);
            (self.api.xkb_context_unref)(self.context);
        }
    }
}
