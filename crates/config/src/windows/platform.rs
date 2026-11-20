//! Windows side of the settings dialog's backend: the running preview app over WM_COPYDATA (eve_maj_app::protocol),
//! EVE client windows and running apps from EnumWindows, displays and Identify (eve_maj_app::displays), the Run key
//! for start-on-login, the shell for links, the region-select result mapping, and the in-place updater
//! (eve_maj_app::updater). This is the Zig build's config.exe behaviour.

use std::sync::atomic::{AtomicU32, Ordering};

use eve_maj_app::protocol as ipc;
use eve_maj_core::log::Scope;
use eve_maj_core::protocol::{Command, RegionSelectRequest, RegionSelectResult, TIMER_CLASS_NAME};
use eve_maj_win::window::{class_name, is_window_iconic, is_window_visible, process_exe_path, window_title};
use eve_maj_win::wide;
use serde_json::{json, Value};
use windows_sys::Win32::Foundation::{BOOL, ERROR_SUCCESS, HWND, LPARAM, RECT, TRUE};
use windows_sys::Win32::System::Registry::{RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ};
use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, EnumWindows, GetWindow, GetWindowRect, GetWindowThreadProcessId, GW_OWNER};

const SLOG: Scope = Scope::new("config");

/// The page's English text already describes the Windows build.
pub const EN_HINTS: &[(&str, &str)] = &[];

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "EVE-Maj Preview";

fn main_app() -> Option<HWND> {
    ipc::find_existing_instance(TIMER_CLASS_NAME)
}

pub fn main_app_running() -> bool {
    main_app().is_some()
}

/// Sends a dialog command to the running preview app; false when it isn't running.
pub fn send_command(cmd: &Command) -> bool {
    match main_app() {
        Some(hwnd) => {
            ipc::send_command_to_instance(hwnd, cmd);
            true
        }
        None => false,
    }
}

struct TopLevel {
    hwnd: HWND,
    pid: u32,
    title: String,
    class: String,
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = &mut *(lparam as *mut Vec<TopLevel>);
    // Visible, unowned top-level windows: what the taskbar and Alt+Tab show.
    if !is_window_visible(hwnd) || !GetWindow(hwnd, GW_OWNER).is_null() {
        return TRUE;
    }
    let Some(title) = window_title(hwnd).filter(|t| !t.is_empty()) else { return TRUE };
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    out.push(TopLevel { hwnd, pid, title, class: class_name(hwnd).unwrap_or_default() });
    TRUE
}

fn top_levels() -> Vec<TopLevel> {
    let mut out: Vec<TopLevel> = Vec::new();
    unsafe { EnumWindows(Some(collect), &mut out as *mut Vec<TopLevel> as LPARAM) };
    out
}

fn exe_name(pid: u32) -> String {
    process_exe_path(pid).map(|p| p.rsplit(['\\', '/']).next().unwrap_or("").to_owned()).unwrap_or_default()
}

/// Logged-in EVE client windows ("EVE - Name"), oldest process first: (name, x, y, minimized).
pub fn eve_clients() -> Vec<(String, i64, i64, bool)> {
    let mut out: Vec<(u32, String, i64, i64, bool)> = top_levels()
        .into_iter()
        .filter(|w| exe_name(w.pid).eq_ignore_ascii_case("exefile.exe"))
        .filter_map(|w| {
            let name = w.title.strip_prefix("EVE - ")?.trim().to_owned();
            let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            unsafe { GetWindowRect(w.hwnd, &mut r) };
            Some((w.pid, name, r.left as i64, r.top as i64, is_window_iconic(w.hwnd)))
        })
        .collect();
    out.sort_by_key(|c| c.0);
    out.into_iter().map(|(_, n, x, y, m)| (n, x, y, m)).collect()
}

/// Running apps for the App Hotkeys picker: one entry per (class, exe), sorted by exe.
pub fn running_windows() -> String {
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut out = Vec::new();
    for w in top_levels() {
        let exe = exe_name(w.pid);
        if exe.is_empty() {
            continue;
        }
        let key = (w.class.clone(), exe.to_ascii_lowercase());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(json!({ "class": w.class, "exe": exe, "title": w.title }));
    }
    out.sort_by_key(|v| v["exe"].as_str().unwrap_or("").to_ascii_lowercase());
    Value::Array(out).to_string()
}

pub fn displays() -> String {
    eve_maj_app::displays::get_displays()
}

pub fn identify_displays() -> bool {
    eve_maj_app::displays::identify_displays().contains("true")
}

/// runOnStartup: the current user's Run key, pointing at eve-maj-preview.exe next to this binary.
pub fn apply_run_on_startup(enabled: bool) {
    let key_w = wide(RUN_KEY);
    let value_w = wide(RUN_VALUE);
    let mut key: HKEY = std::ptr::null_mut();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, key_w.as_ptr(), 0, KEY_SET_VALUE, &mut key) } != ERROR_SUCCESS {
        SLOG.warn(format_args!("Couldn't open the Run key to update start-on-login"));
        return;
    }
    if enabled {
        match eve_maj_win::window::self_exe_dir().map(|d| d.join("eve-maj-preview.exe")) {
            Some(exe) => {
                let data = wide(&format!("\"{}\"", exe.display()));
                let status = unsafe { RegSetValueExW(key, value_w.as_ptr(), 0, REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) };
                if status == ERROR_SUCCESS {
                    SLOG.info(format_args!("Start on login enabled: {}", exe.display()));
                } else {
                    SLOG.warn(format_args!("Couldn't write the start-on-login entry (error {status})"));
                }
            }
            None => SLOG.warn(format_args!("Couldn't find this program's folder for start-on-login")),
        }
    } else {
        // Missing already is fine.
        unsafe { RegDeleteValueW(key, value_w.as_ptr()) };
    }
    unsafe { RegCloseKey(key) };
}

pub fn open_url(url: &str) {
    if !eve_maj_win::shell::shell_open(url, None) {
        SLOG.warn(format_args!("Couldn't open {url}"));
    }
}

/// The result sequence when the current selection started; a later one is its answer.
static REGION_SEQUENCE: AtomicU32 = AtomicU32::new(0);

pub fn start_region_select(request: RegionSelectRequest) -> bool {
    REGION_SEQUENCE.store(ipc::read_region_select_result().map_or(0, |r| r.sequence), Ordering::Release);
    // This window is in front; let the app's overlay take the foreground so it gets Esc and Enter.
    if let Some(hwnd) = main_app() {
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut pid);
            AllowSetForegroundWindow(pid);
        }
    }
    send_command(&Command::StartRegionSelect(Box::new(request)))
}

pub fn poll_region_select() -> String {
    match ipc::read_region_select_result() {
        Some(r) if r.sequence != REGION_SEQUENCE.load(Ordering::Acquire) => {
            REGION_SEQUENCE.store(r.sequence, Ordering::Release);
            r.to_dialog_json()
        }
        // The app stopped (or never answered) while the dialog was waiting.
        None if !main_app_running() => RegionSelectResult::cancelled().to_dialog_json(),
        _ => json!({ "done": false }).to_string(),
    }
}

pub fn check_for_update_now() -> String {
    eve_maj_app::updater::check_for_update_now()
}

pub fn download_update() -> String {
    eve_maj_app::updater::download_update()
}

pub fn install_update() -> String {
    eve_maj_app::updater::install_update()
}

/// Registers how the updater finds the main app (to stop and restart it around an install).
pub fn init_updater() {
    eve_maj_app::updater::init(main_app);
}
