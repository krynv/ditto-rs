//! Remember which window had focus before the popup opened and give it focus back before
//! pasting (Ditto: `ExternalWindowTracker`), plus the name of the app that made a copy.

/// A window (or, on macOS, an application) outside Ditto-RS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignWindow {
    /// HWND on Windows, X11 window id on Linux, pid on macOS.
    pub handle: u64,
    pub pid: Option<u32>,
    /// Executable / application name, e.g. `chrome.exe`, `Safari`, `firefox`.
    pub app_name: Option<String>,
}

impl ForeignWindow {
    pub fn is_own_process(&self) -> bool {
        self.pid == Some(std::process::id())
    }
}

/// The window that currently has keyboard focus, if it can be determined.
pub fn foreground_window() -> Option<ForeignWindow> {
    imp::foreground_window()
}

/// Bring `window` back to the foreground. Returns `false` when that was not possible.
pub fn activate(window: &ForeignWindow) -> bool {
    imp::activate(window)
}

/// Wait until `window` (or, on macOS, its application) is in the foreground again.
/// Returns `false` if that did not happen within `timeout` or cannot be determined.
pub fn wait_until_foreground(window: &ForeignWindow, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let current = foreground_window();
        if current.is_some_and(|w| w.handle == window.handle) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
}

/// Name of the application that currently has focus (used for the include/exclude lists).
pub fn foreground_app_name() -> Option<String> {
    foreground_window().filter(|w| !w.is_own_process()).and_then(|w| w.app_name)
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};

    use super::ForeignWindow;

    pub fn foreground_window() -> Option<ForeignWindow> {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        let pid = app.processIdentifier();
        if pid <= 0 {
            return None;
        }
        Some(ForeignWindow { handle: pid as u64, pid: Some(pid as u32), app_name: app.localizedName().map(|n| n.to_string()) })
    }

    pub fn activate(window: &ForeignWindow) -> bool {
        let Some(pid) = window.pid else { return false };
        match NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32) {
            // Ignoring other apps is a no-op since macOS 14, where activation is cooperative;
            // we are the active app at this point, so plain activation is allowed.
            #[allow(deprecated)]
            Some(app) => app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps),
            None => false,
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use windows_sys::Win32::Foundation::{CloseHandle, HWND};
    use windows_sys::Win32::System::Threading::{
        AttachThreadInput, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow, SW_RESTORE,
        SetForegroundWindow, ShowWindow,
    };

    use super::ForeignWindow;

    pub fn foreground_window() -> Option<ForeignWindow> {
        // SAFETY: plain Win32 calls with valid out-pointers.
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            Some(ForeignWindow { handle: hwnd as usize as u64, pid: (pid != 0).then_some(pid), app_name: process_name(pid) })
        }
    }

    fn process_name(pid: u32) -> Option<String> {
        if pid == 0 {
            return None;
        }
        // SAFETY: the handle is checked and closed; the buffer length is passed in/out.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
            CloseHandle(process);
            if ok == 0 {
                return None;
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            path.rsplit(['\\', '/']).next().map(str::to_string)
        }
    }

    pub fn activate(window: &ForeignWindow) -> bool {
        let hwnd = window.handle as usize as HWND;
        // SAFETY: Win32 calls on a window handle that is validated with IsWindow first.
        unsafe {
            if IsWindow(hwnd) == 0 {
                return false;
            }
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            if SetForegroundWindow(hwnd) != 0 {
                return true;
            }
            // Like Ditto: attach to the foreground thread's input queue and retry.
            let foreground = GetForegroundWindow();
            let fg_thread = GetWindowThreadProcessId(foreground, std::ptr::null_mut());
            let me = GetCurrentThreadId();
            let attach = fg_thread != 0 && fg_thread != me;
            if attach {
                AttachThreadInput(me, fg_thread, 1);
            }
            let ok = SetForegroundWindow(hwnd) != 0;
            BringWindowToTop(hwnd);
            if attach {
                AttachThreadInput(me, fg_thread, 0);
            }
            ok
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    //! X11 via EWMH (`_NET_ACTIVE_WINDOW`). Under Wayland there is no portable way to
    //! query or set focus; the compositor gives focus back when our window hides.
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask};
    use x11rb::rust_connection::RustConnection;

    use super::ForeignWindow;

    fn connect() -> Option<(RustConnection, u32)> {
        std::env::var_os("DISPLAY")?;
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        Some((conn, root))
    }

    fn atom(conn: &RustConnection, name: &str) -> Option<u32> {
        Some(conn.intern_atom(false, name.as_bytes()).ok()?.reply().ok()?.atom)
    }

    fn property32(conn: &RustConnection, window: u32, name: &str, ty: AtomEnum) -> Option<u32> {
        let prop = atom(conn, name)?;
        let reply = conn.get_property(false, window, prop, ty, 0, 1).ok()?.reply().ok()?;
        reply.value32()?.next()
    }

    pub fn foreground_window() -> Option<ForeignWindow> {
        let (conn, root) = connect()?;
        let window = property32(&conn, root, "_NET_ACTIVE_WINDOW", AtomEnum::WINDOW).filter(|w| *w != 0)?;
        let pid = property32(&conn, window, "_NET_WM_PID", AtomEnum::CARDINAL);
        let app_name = pid
            .and_then(|p| std::fs::read_to_string(format!("/proc/{p}/comm")).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Some(ForeignWindow { handle: u64::from(window), pid, app_name })
    }

    pub fn activate(window: &ForeignWindow) -> bool {
        let Some((conn, root)) = connect() else { return false };
        let Some(active) = atom(&conn, "_NET_ACTIVE_WINDOW") else { return false };
        // Source indication 2 ("pager") is honoured by most window managers' focus-stealing rules.
        let event = ClientMessageEvent::new(32, window.handle as u32, active, [2u32, 0, 0, 0, 0]);
        let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
        conn.send_event(false, root, mask, event).is_ok() && conn.flush().is_ok()
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod imp {
    use super::ForeignWindow;

    pub fn foreground_window() -> Option<ForeignWindow> {
        None
    }

    pub fn activate(_: &ForeignWindow) -> bool {
        false
    }
}
