use std::collections::HashMap;

use crate::settings::{KeyBind, MODIFIER_ALT, MODIFIER_CONTROL, MODIFIER_SHIFT};

pub const ESCAPE: u8 = 0x1B;
pub const ENTER: u8 = 0x0D;

const SHIFT: i32 = 0x10;
const CONTROL: i32 = 0x11;
const ALT: i32 = 0x12;
const LEFT_MOUSE: i32 = 0x01;
const RIGHT_MOUSE: i32 = 0x02;
const MODIFIER_KEYS: [u8; 9] = [0x10, 0x11, 0x12, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5];

#[link(name = "user32")]
unsafe extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
    fn GetForegroundWindow() -> *mut std::ffi::c_void;
    fn GetWindowThreadProcessId(window: *mut std::ffi::c_void, process: *mut u32) -> u32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcessId() -> u32;
}

fn down(key: i32) -> bool {
    let state = unsafe { GetAsyncKeyState(key) };
    state < 0
}

fn game_focused() -> bool {
    let mut process = 0u32;
    unsafe {
        GetWindowThreadProcessId(GetForegroundWindow(), &mut process);
        process == GetCurrentProcessId()
    }
}

fn held_modifiers() -> u8 {
    let mut held = 0;
    if down(ALT) {
        held |= MODIFIER_ALT;
    }
    if down(CONTROL) {
        held |= MODIFIER_CONTROL;
    }
    if down(SHIFT) {
        held |= MODIFIER_SHIFT;
    }
    held
}

/// Reads the keyboard directly once a frame, so bindings work whatever the game's input mode is.
#[derive(Default)]
pub struct Keys {
    focused: bool,
    mouse_held: bool,
    modifiers: u8,
    was_down: HashMap<u8, bool>,
    pressed: Vec<u8>,
}

impl Keys {
    /// Call once per frame before asking about presses.
    pub fn poll(&mut self, watched: impl IntoIterator<Item = u8>) {
        self.pressed.clear();
        self.focused = game_focused();
        self.modifiers = if self.focused { held_modifiers() } else { 0 };
        self.mouse_held = down(LEFT_MOUSE) || down(RIGHT_MOUSE);
        for key in watched {
            let now = self.focused && down(i32::from(key));
            let before = self.was_down.insert(key, now).unwrap_or(false);
            if now && !before && !self.pressed.contains(&key) {
                self.pressed.push(key);
            }
        }
    }

    pub fn mouse_held(&self) -> bool {
        self.mouse_held
    }

    /// Exactly these modifiers held, and the key went down this frame.
    pub fn bind_pressed(&self, bind: &KeyBind) -> bool {
        self.pressed.contains(&bind.key) && self.modifiers == bind.modifiers
    }

    pub fn any_pressed(&self, binds: &[KeyBind]) -> bool {
        binds.iter().any(|bind| self.bind_pressed(bind))
    }

    /// A plain key with no modifiers held.
    pub fn plain_pressed(&self, key: u8) -> bool {
        self.pressed.contains(&key) && self.modifiers == 0
    }

    /// The first new press this frame of a key in `allowed`, with the modifiers held, for rebinding.
    pub fn captured(&self, allowed: &[u8]) -> Option<KeyBind> {
        self.pressed
            .iter()
            .find(|key| !MODIFIER_KEYS.contains(key) && allowed.contains(key))
            .map(|&key| KeyBind { key, modifiers: self.modifiers })
    }
}
