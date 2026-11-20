//! eve-maj-config: the settings dialog (config.exe in the Windows/Zig build). The page is the Zig build's own
//! HTML/CSS/JS, unchanged; this binary serves it to a native web view (WebKitGTK on Linux, WebView2 on Windows) and
//! answers its `webui.call(...)` requests.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(any(target_os = "linux", windows))]
mod backend;
#[cfg(any(target_os = "linux", windows))]
mod server;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::platform;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::platform;

/// Requests from the backend thread to the window.
#[cfg(any(target_os = "linux", windows))]
#[derive(Debug)]
pub enum UiEvent {
    Close,
    SetAlwaysOnTop(bool),
    /// Open a native picker and send back the chosen path ("" when cancelled).
    Browse(backend::Browse, std::sync::mpsc::Sender<String>),
}

fn main() {
    #[cfg(target_os = "linux")]
    if let Err(e) = linux::run() {
        eprintln!("eve-maj-config: {e}");
        std::process::exit(1);
    }
    #[cfg(windows)]
    if let Err(e) = windows::run() {
        windows::show_error(&e);
        std::process::exit(1);
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        eprintln!("eve-maj-config runs on Linux and Windows");
        std::process::exit(1);
    }
}
