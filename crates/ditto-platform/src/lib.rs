//! OS integration for Ditto-RS.
//!
//! Ditto is built directly on Win32: `AddClipboardFormatListener`/`SetClipboardViewer` to
//! watch the clipboard, `GetForegroundWindow`/`SetForegroundWindow` (+ `AttachThreadInput`)
//! to paste into the window that was active before the popup, and `SendInput` for Ctrl+V.
//! This crate provides the portable equivalents:
//!
//! | Ditto (Win32)                       | Here                                              |
//! |-------------------------------------|---------------------------------------------------|
//! | clipboard viewer / format listener  | [`clipboard::watch`] (clipboard-rs)               |
//! | `OpenClipboard`/`GetClipboardData`  | [`clipboard::read`] / [`clipboard::write`]        |
//! | `GetForegroundWindow`               | [`focus::foreground_window`]                      |
//! | `SetForegroundWindow`               | [`focus::activate`]                               |
//! | `SendInput` Ctrl+V                  | [`input::send_paste`] (enigo)                     |
//! | "Clipboard Viewer Ignore" format    | [`clipboard::SELF_MARKER`] + privacy markers      |
//!
//! [`ocr`] has no counterpart in Ditto: it reads the text in image clips so they can be searched.

pub mod clipboard;
pub mod focus;
pub mod input;
pub mod ocr;

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("clipboard: {0}")]
    Clipboard(String),
    #[error("input simulation: {0}")]
    Input(String),
    #[error("text recognition: {0}")]
    Ocr(String),
    #[error("{0}")]
    Unsupported(&'static str),
}

pub type Result<T> = std::result::Result<T, PlatformError>;

/// Whether the system web view can zoom a page, which "Interface size" relies on. WKWebView's
/// `pageZoom` needs macOS 11; calling it on an older system would crash.
pub fn webview_zoom_supported() -> bool {
    #[cfg(target_os = "macos")]
    {
        objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion().majorVersion >= 11
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Release builds on Windows use the GUI subsystem (no console window). Attach to the
/// parent console, if any, so command-line output is visible. No-op elsewhere.
pub fn attach_parent_console() {
    #[cfg(windows)]
    // SAFETY: AttachConsole has no preconditions; failure (no parent console) is harmless.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS);
    }
}
