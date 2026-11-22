//! Windows front end of the settings dialog (config.exe): a native window (tao + wry/WebView2) showing the dialog page
//! served by [`crate::server`], with [`crate::backend`] answering the page's calls against the files next to the
//! exe, the same ones eve-maj-preview.exe uses. Needs the Microsoft Edge WebView2 Runtime (part of Windows 11, and
//! installed on Windows 10 by Edge updates).

pub mod platform;

use std::path::PathBuf;

use eve_maj_core::config::GlobalSettings;
use eve_maj_core::log::{self, Scope};
use eve_maj_win::wide;
use tao::dpi::{LogicalSize, PhysicalSize};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::windows::{IconExtWindows, WindowBuilderExtWindows, WindowExtWindows};
use tao::window::{Icon, WindowBuilder};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HWND, TRUE};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetSystemMetrics, IsIconic, MessageBoxW, SetForegroundWindow, ShowWindow, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
    SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, SW_RESTORE,
};
use wry::{WebContext, WebViewBuilder};

use crate::{backend, server, UiEvent};

const SLOG: Scope = Scope::new("config");
const TITLE: &str = "EVE-Maj Preview Configuration";
/// The icon in assets/app.rc (IDI_ICON1).
const ICON_RESOURCE: u16 = 101;

/// A Windows-subsystem build has no console, so failures are shown in a message box (and logged).
pub fn show_error(message: &str) {
    SLOG.err(format_args!("{message}"));
    log::flush();
    let body = wide(&format!("The settings window could not open:\n\n{message}"));
    let title = wide(TITLE);
    unsafe { MessageBoxW(std::ptr::null_mut(), body.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR | MB_TOPMOST | MB_SETFOREGROUND) };
}

/// Native folder/file picker, owned by the dialog window so it stays above it (the window may be always-on-top).
fn pick(kind: backend::Browse, owner: HWND) -> String {
    use eve_maj_win::shell::{show_file_picker, show_folder_picker};
    let result = match kind {
        backend::Browse::ChatlogDir => show_folder_picker("Select Chatlog Directory", Some(owner)),
        backend::Browse::GamelogDir => show_folder_picker("Select Gamelog Directory", Some(owner)),
        backend::Browse::SoundFile => show_file_picker("Select Sound File", "Audio Files (*.wav, *.mp3)", "*.wav;*.mp3", Some(owner)),
    };
    match result {
        Ok(path) => path.unwrap_or_default(),
        Err(e) => {
            SLOG.warn(format_args!("The file picker failed: {e}"));
            String::new()
        }
    }
}

/// A second config.exe brings the open one forward instead of starting another dialog on the same files.
fn focus_existing() -> bool {
    let title = wide(TITLE);
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd.is_null() {
        return false;
    }
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
    true
}

/// WebView2 keeps its cache and cookies here rather than next to the exe, which may not be writable.
fn webview_data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    Some(base.join("EVE-Maj Preview").join("WebView2"))
}

pub fn run() -> Result<(), String> {
    // Profiles, global settings and eve-maj.log live next to the exe, as for eve-maj-preview.exe.
    let dir = eve_maj_win::window::self_exe_dir().ok_or("can't find this program's folder")?;
    std::env::set_current_dir(&dir).map_err(|e| format!("can't enter {}: {e}", dir.display()))?;
    std::fs::create_dir_all("profiles").map_err(|e| format!("can't create {}\\profiles: {e}", dir.display()))?;

    let mutex_name = wide("Local\\EVE-Maj-Config-SingleInstance");
    let mutex = unsafe { CreateMutexW(std::ptr::null(), TRUE, mutex_name.as_ptr()) };
    if !mutex.is_null() && unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe { CloseHandle(mutex) };
        if focus_existing() {
            return Ok(());
        }
    }

    let global = GlobalSettings::load();
    log::set_level(global.log_level);
    // A profile file name on the command line opens that one; otherwise the last used.
    let profile = std::env::args()
        .nth(1)
        .filter(|a| eve_maj_core::config::is_safe_profile_name(a))
        .unwrap_or_else(|| if global.last_used_profile.is_empty() { "default.json".to_owned() } else { global.last_used_profile.clone() });
    SLOG.info(format_args!("EVE-Maj Config v{} (Windows), editing {profile}", env!("CARGO_PKG_VERSION")));
    platform::init_updater();

    let event_loop = EventLoopBuilder::<UiEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let server = server::Server::start(backend::Backend::new(proxy, profile, &global.language)).map_err(|e| format!("can't start the dialog server: {e}"))?;
    let url = format!("http://127.0.0.1:{}/", server.port);
    SLOG.debug(format_args!("Dialog page served on 127.0.0.1:{}", server.port));

    let mut builder = WindowBuilder::new()
        .with_title(TITLE)
        .with_inner_size(LogicalSize::new(1100.0, 800.0))
        .with_min_inner_size(LogicalSize::new(760.0, 520.0))
        .with_always_on_top(global.always_on_top);
    // tao sets the window icon as ICON_SMALL only (title bar); without an ICON_BIG, Alt+Tab and the taskbar show a
    // generic one. Load each at its own size so neither is scaled from the other.
    let icon_at = |w, h| Icon::from_resource(ICON_RESOURCE, Some(PhysicalSize::new(unsafe { GetSystemMetrics(w) } as u32, unsafe { GetSystemMetrics(h) } as u32)));
    match icon_at(SM_CXSMICON, SM_CYSMICON) {
        Ok(icon) => builder = builder.with_window_icon(Some(icon)),
        Err(e) => SLOG.warn(format_args!("No window icon: {e}")),
    }
    match icon_at(SM_CXICON, SM_CYICON) {
        Ok(icon) => builder = builder.with_taskbar_icon(Some(icon)),
        Err(e) => SLOG.warn(format_args!("No taskbar icon: {e}")),
    }
    let window = builder.build(&event_loop).map_err(|e| format!("can't open the window: {e}"))?;
    let owner = window.hwnd() as HWND;

    let mut context = WebContext::new(webview_data_dir());
    let token = server.token.clone();
    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(&url)
        // The shim reads the call token from here; it never appears in a URL or a page another origin can load.
        .with_initialization_script(format!("window.__EVEMAJ_TOKEN__ = {};", serde_json::Value::String(token)))
        .with_devtools(std::env::var_os("EVE_MAJ_DEVTOOLS").is_some())
        .build(&window)
        .map_err(|e| {
            format!(
                "can't create the web view: {e}\n\nThe settings window needs the Microsoft Edge WebView2 Runtime. Install it from https://developer.microsoft.com/microsoft-edge/webview2/ (the \"Evergreen Bootstrapper\") and try again."
            )
        })?;
    SLOG.info(format_args!("Settings window open"));

    event_loop.run(move |event, _, control_flow| {
        // Keep the web view (and its context) alive for as long as the loop runs.
        let _ = (&webview, &context);
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } | Event::UserEvent(UiEvent::Close) => {
                // Safety net, as in the Zig build: drop any unsaved live preview and give the app its hotkeys back.
                platform::send_command(&eve_maj_core::protocol::Command::RevertPreview);
                platform::send_command(&eve_maj_core::protocol::Command::DialogResumeHotkeys);
                SLOG.info(format_args!("Settings window closed"));
                log::flush();
                *control_flow = ControlFlow::Exit;
            }
            Event::UserEvent(UiEvent::SetAlwaysOnTop(on)) => window.set_always_on_top(on),
            Event::UserEvent(UiEvent::Browse(kind, reply)) => {
                let _ = reply.send(pick(kind, owner));
            }
            _ => {}
        }
    });
}
