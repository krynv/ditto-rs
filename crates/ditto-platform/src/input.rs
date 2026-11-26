//! Simulated keystrokes (Ditto's `DefaultPasteString` is `^{VKEY86}`, i.e. Ctrl+V).

use enigo::{Direction, Enigo, Key, Keyboard, Settings};

use crate::{PlatformError, Result};

fn input_err(e: impl std::fmt::Display) -> PlatformError {
    PlatformError::Input(e.to_string())
}

/// The platform's "paste" shortcut: Cmd+V on macOS, Ctrl+V elsewhere.
///
/// On macOS this must be called on the main thread (the key code for "v" is looked up in
/// the current keyboard layout through an API that asserts it runs on the main queue);
/// elsewhere it can be called from any thread.
pub fn send_paste() -> Result<()> {
    send_with_modifier('v')
}

/// The platform's "copy" shortcut (for "copy and save current clipboard").
pub fn send_copy() -> Result<()> {
    send_with_modifier('c')
}

fn send_with_modifier(key: char) -> Result<()> {
    #[cfg(target_os = "macos")]
    if !is_main_thread() {
        return Err(PlatformError::Input("on macOS keystrokes must be sent from the main thread".into()));
    }
    #[cfg(target_os = "macos")]
    if !accessibility::trusted(false) {
        return Err(PlatformError::Input(
            "macOS has not granted Accessibility access, so the paste keystroke cannot be sent".into(),
        ));
    }
    let mut enigo = Enigo::new(&Settings::default()).map_err(input_err)?;
    let modifier = if cfg!(target_os = "macos") { Key::Meta } else { Key::Control };
    enigo.key(modifier, Direction::Press).map_err(input_err)?;
    let result = enigo.key(Key::Unicode(key), Direction::Click).map_err(input_err);
    enigo.key(modifier, Direction::Release).map_err(input_err)?;
    result
}

/// Wait (up to `timeout`) until Shift/Ctrl/Alt/Cmd/Win are all released, so a simulated
/// Ctrl+V is not combined with keys the user is still holding (e.g. after Shift+Enter or a
/// Ctrl+Alt+1 hotkey). Returns `false` on timeout.
pub fn wait_for_modifiers_released(timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    let mut probe = modifiers::Probe::new();
    while probe.any_down() {
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    true
}

#[cfg(target_os = "macos")]
mod modifiers {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceFlagsState(state_id: i32) -> u64;
    }
    const COMBINED_SESSION_STATE: i32 = 0;
    // kCGEventFlagMaskShift | Control | Alternate | Command
    const MASK: u64 = 0x0002_0000 | 0x0004_0000 | 0x0008_0000 | 0x0010_0000;

    pub struct Probe;

    impl Probe {
        pub fn new() -> Self {
            Probe
        }

        pub fn any_down(&mut self) -> bool {
            // SAFETY: a pure query of the current modifier state.
            unsafe { CGEventSourceFlagsState(COMBINED_SESSION_STATE) & MASK != 0 }
        }
    }
}

#[cfg(target_os = "windows")]
mod modifiers {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };

    pub struct Probe;

    impl Probe {
        pub fn new() -> Self {
            Probe
        }

        pub fn any_down(&mut self) -> bool {
            [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
                .iter()
                // SAFETY: GetAsyncKeyState only reads global key state.
                .any(|&vk| (unsafe { GetAsyncKeyState(i32::from(vk)) } as u16) & 0x8000 != 0)
        }
    }
}

#[cfg(target_os = "linux")]
mod modifiers {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    use x11rb::rust_connection::RustConnection;

    /// X11 only; under pure Wayland we cannot observe global key state and return "up".
    pub struct Probe {
        conn: Option<RustConnection>,
        keycodes: Vec<u8>,
    }

    impl Probe {
        pub fn new() -> Self {
            let conn = std::env::var_os("DISPLAY").and_then(|_| x11rb::connect(None).ok()).map(|(c, _)| c);
            let mut keycodes = Vec::new();
            if let Some(c) = &conn {
                if let Some(map) = c.get_modifier_mapping().ok().and_then(|cookie| cookie.reply().ok()) {
                    let per = map.keycodes.len() / 8;
                    // Shift (0), Control (2), Mod1/Alt (3), Mod4/Super (6)
                    for m in [0usize, 2, 3, 6] {
                        keycodes.extend(map.keycodes[m * per..(m + 1) * per].iter().copied().filter(|k| *k != 0));
                    }
                }
            }
            let _ = conn.as_ref().map(|c| c.flush());
            Probe { conn, keycodes }
        }

        pub fn any_down(&mut self) -> bool {
            let Some(conn) = &self.conn else { return false };
            let Some(keymap) = conn.query_keymap().ok().and_then(|c| c.reply().ok()) else { return false };
            self.keycodes.iter().any(|&k| keymap.keys[usize::from(k / 8)] & (1 << (k % 8)) != 0)
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod modifiers {
    pub struct Probe;

    impl Probe {
        pub fn new() -> Self {
            Probe
        }

        pub fn any_down(&mut self) -> bool {
            false
        }
    }
}

#[cfg(target_os = "macos")]
fn is_main_thread() -> bool {
    unsafe extern "C" {
        fn pthread_main_np() -> i32;
    }
    // SAFETY: pthread_main_np has no preconditions.
    unsafe { pthread_main_np() != 0 }
}

/// Whether keystrokes can be simulated. On macOS this needs the Accessibility permission;
/// with `prompt` the system dialog asking for it is shown.
pub fn can_send_keystrokes(prompt: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        accessibility::trusted(prompt)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = prompt;
        true
    }
}

#[cfg(target_os = "macos")]
mod accessibility {
    use std::ffi::c_void;

    #[repr(C)]
    struct Opaque {
        _private: [u8; 0],
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
        static kAXTrustedCheckOptionPrompt: *const c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFBooleanTrue: *const c_void;
        static kCFTypeDictionaryKeyCallBacks: Opaque;
        static kCFTypeDictionaryValueCallBacks: Opaque;
        fn CFDictionaryCreate(
            allocator: *const c_void,
            keys: *const *const c_void,
            values: *const *const c_void,
            num_values: isize,
            key_callbacks: *const Opaque,
            value_callbacks: *const Opaque,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    pub fn trusted(prompt: bool) -> bool {
        // SAFETY: documented CoreFoundation/ApplicationServices calls; the dictionary is
        // created with the standard CFType callbacks and released after use.
        unsafe {
            if !prompt {
                return AXIsProcessTrusted();
            }
            let keys = [kAXTrustedCheckOptionPrompt];
            let values = [kCFBooleanTrue];
            let options = CFDictionaryCreate(
                std::ptr::null(),
                keys.as_ptr(),
                values.as_ptr(),
                1,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            );
            let trusted = AXIsProcessTrustedWithOptions(options);
            if !options.is_null() {
                CFRelease(options);
            }
            trusted
        }
    }
}
