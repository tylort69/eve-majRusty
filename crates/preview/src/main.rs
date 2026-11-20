//! eve-maj-preview: live thumbnails of EVE Online client windows.
//!
//! On Windows (this directory): startup, the hidden timer window that dispatches tray/IPC/hotkey/timer messages, the
//! per-tick scan -> paint pipeline, hotkeys, chat-log monitoring and activity overlays, the Client List and History
//! Panel, region selection, profile reloads, the config dialog's live-preview patches, and crash logging. The
//! Linux/Hyprland front end is `linux/`.

#![cfg_attr(windows, windows_subsystem = "windows")]
// Some ported Zig helpers are kept for parity even where nothing in this build calls them yet.
#![allow(dead_code)]

#[cfg(windows)]
mod fonts;
#[cfg(windows)]
mod gdi_overlay;
#[cfg(windows)]
mod globals;
#[cfg(windows)]
mod hotkeys;
#[cfg(windows)]
mod input;
#[cfg(windows)]
mod manager;
#[cfg(windows)]
mod monitor;
#[cfg(windows)]
mod painter;
#[cfg(windows)]
mod panels;
#[cfg(windows)]
mod region_select;
#[cfg(windows)]
mod scout;
#[cfg(windows)]
mod tray;

#[cfg(windows)]
mod app;

#[cfg(windows)]
fn main() {
    app::main();
}

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
fn main() {
    linux::main();
}

#[cfg(not(any(windows, target_os = "linux")))]
fn main() {
    eprintln!("eve-maj-preview runs on Windows and on Linux under Hyprland");
}
