//! Linux side of the settings dialog's backend: the running preview app over its Unix socket, Hyprland queries
//! (clients, monitors), XDG autostart, xdg-open, the region-select result file, and manual-only updates.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use eve_maj_core::desktop_icon;
use eve_maj_core::display_info::{self, Display};
use eve_maj_core::log::Scope;
use eve_maj_core::protocol::{region_select_result_path, Command, RegionSelectRequest, RegionSelectResult, WireMessage};
use eve_maj_core::update;
use serde_json::{json, Value};

const SLOG: Scope = Scope::new("config");

/// English hints that describe Windows behaviour, reworded for what the Linux build does. Other languages keep the
/// shared wording until they're translated.
pub const EN_HINTS: &[(&str, &str)] = &[
    ("tab.notifications.section.text-to-speech.hint", "Speak alerts aloud through speech-dispatcher (spd-say) or espeak-ng. Enable individual event types below under each event's \"TTS\" setting."),
    ("tab.behavior.section.startup.hint", "Launch at login (an XDG autostart entry), and register the evemajpreview:// URL protocol for external control (Stream Deck, scripts, etc.)."),
    ("tab.displays.section.displays.hint", "Every display connected to this PC, arranged the way Hyprland lays them out, with the resolution, refresh rate and scaling each one is running. View only: EVE-Maj Preview never changes your display settings. Click a display to highlight its details, or use Identify to show each display's number on screen."),
    ("tab.resources.section.resource-usage-overlay.hint", "Displays each client's CPU% and RAM usage on its thumbnail, read from /proc rather than from EVE's logs. VRAM isn't available on Linux."),
    ("field.hotkeyToggleExclusion.hint", "Applies to whichever EVE window currently has focus, not the thumbnail under your cursor."),
    ("field.regionDisplayUseWorkArea.label", "Leave Room for Bars"),
    ("field.regionDisplayUseWorkArea.hint", "Fills the display's work area so thumbnails don't sit under Waybar or other bars that reserve space."),
    ("tab.accounts.section.characters.hint", "Every character that has logged in on this PC, found in EVE's settings folders (CCP/EVE under the client's Wine prefix, drive_c/users/*/AppData/Local). Accounts are suggested by matching each character's settings file to the account settings file EVE saved at the same moment, so check a suggestion before accepting it. Changes save automatically."),
];

// ------------------------------------------------------------------------------------------------ the preview app --

fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("eve-maj-preview.sock")
}

pub fn main_app_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

/// Sends a dialog command to the running preview app: "evemaj-cmd:<id>\n" then the payload, the same id/payload pair
/// the Windows build sends with WM_COPYDATA.
pub fn send_command(cmd: &Command) -> bool {
    let WireMessage::CopyData { dw_data, payload } = cmd.to_wire() else { return false };
    let Ok(mut s) = UnixStream::connect(socket_path()) else { return false };
    let mut msg = format!("evemaj-cmd:{dw_data}\n").into_bytes();
    if let Some(p) = payload {
        msg.extend_from_slice(&p);
    }
    s.write_all(&msg).is_ok()
}

// ---------------------------------------------------------------------------------------------- Hyprland queries --

fn hyprctl_json(what: &str) -> Option<Value> {
    let out = std::process::Command::new("hyprctl").args(["-j", what]).output().ok()?;
    serde_json::from_slice(&out.stdout).ok()
}

/// Mapped EVE client windows ("EVE - Name"), oldest process first: (name, x, y, minimized).
pub fn eve_clients() -> Vec<(String, i64, i64, bool)> {
    let Some(Value::Array(list)) = hyprctl_json("clients") else { return Vec::new() };
    let mut out: Vec<(i64, String, i64, i64, bool)> = list
        .iter()
        .filter(|c| c["mapped"].as_bool().unwrap_or(false))
        .filter_map(|c| {
            let name = c["title"].as_str()?.strip_prefix("EVE - ")?.trim().to_owned();
            let at = c["at"].as_array()?;
            let minimized = c["workspace"]["name"].as_str().is_some_and(|w| w == "special:evemaj");
            Some((c["pid"].as_i64().unwrap_or(0), name, at.first()?.as_i64()?, at.get(1)?.as_i64()?, minimized))
        })
        .collect();
    out.sort_by_key(|c| c.0);
    out.into_iter().map(|(_, n, x, y, m)| (n, x, y, m)).collect()
}

fn process_image(pid: i64) -> String {
    // Wine/Proton processes report the Windows exe as argv[0]; native ones are better named by /proc/pid/exe.
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    let argv0 = String::from_utf8_lossy(cmdline.split(|&b| b == 0).next().unwrap_or_default()).into_owned();
    let base = |s: &str| s.rsplit(['/', '\\']).next().unwrap_or("").to_owned();
    if argv0.to_ascii_lowercase().ends_with(".exe") {
        return base(&argv0);
    }
    std::fs::read_link(format!("/proc/{pid}/exe")).map(|p| base(&p.to_string_lossy())).unwrap_or_else(|_| base(&argv0))
}

pub fn running_windows() -> String {
    let Some(Value::Array(list)) = hyprctl_json("clients") else { return "[]".into() };
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut out = Vec::new();
    for c in &list {
        let (Some(class), Some(title)) = (c["class"].as_str(), c["title"].as_str()) else { continue };
        if title.is_empty() || !c["mapped"].as_bool().unwrap_or(false) {
            continue;
        }
        let exe = process_image(c["pid"].as_i64().unwrap_or(0));
        let key = (class.to_owned(), exe.to_ascii_lowercase());
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        out.push(json!({ "class": class, "exe": exe, "title": title }));
    }
    out.sort_by_key(|v| v["exe"].as_str().unwrap_or("").to_ascii_lowercase());
    Value::Array(out).to_string()
}

pub fn displays() -> String {
    let Some(Value::Array(list)) = hyprctl_json("monitors") else { return display_info::EMPTY_DISPLAYS_JSON.into() };
    let mut displays: Vec<Display> = list
        .iter()
        .map(|m| {
            let i = |k: &str| m[k].as_i64().unwrap_or(0) as i32;
            let scale = m["scale"].as_f64().unwrap_or(1.0);
            let transform = m["transform"].as_i64().unwrap_or(0);
            let (w, h) = (i("width"), i("height"));
            // Layout-space size: rotated and scaled like Hyprland places it.
            let (lw, lh) = if transform % 2 == 1 { (h, w) } else { (w, h) };
            let (lw, lh) = ((lw as f64 / scale).round() as i32, (lh as f64 / scale).round() as i32);
            let reserved: Vec<i32> = m["reserved"].as_array().map(|r| r.iter().map(|v| v.as_i64().unwrap_or(0) as i32).collect()).unwrap_or_default();
            let res = |k: usize| reserved.get(k).copied().unwrap_or(0);
            let bounds = display_info::Rect { x: i("x"), y: i("y"), width: lw, height: lh };
            let name = format!("{} {}", m["make"].as_str().unwrap_or(""), m["model"].as_str().unwrap_or("")).trim().to_owned();
            Display {
                number: i("id") as u32 + 1,
                id: m["name"].as_str().unwrap_or("").to_owned(),
                gdi_name: m["name"].as_str().unwrap_or("").to_owned(),
                name: if name.is_empty() { m["name"].as_str().unwrap_or("").to_owned() } else { name },
                gpu: None,
                primary: i("id") == 0,
                bounds,
                work_area: display_info::Rect { x: bounds.x + res(0), y: bounds.y + res(1), width: lw - res(0) - res(2), height: lh - res(1) - res(3) },
                mode_width: w as u32,
                mode_height: h as u32,
                refresh_hz: m["refreshRate"].as_f64().unwrap_or(0.0).round() as u32,
                bits_per_pixel: 32,
                orientation: (transform % 4) as u32 * 90,
                scale_percent: (scale * 100.0).round() as u32,
                max_width: w as u32,
                max_height: h as u32,
                max_refresh_hz: m["refreshRate"].as_f64().unwrap_or(0.0).round() as u32,
            }
        })
        .collect();
    displays.sort_by_key(|d| d.number);
    display_info::displays_json(&displays)
}

/// The Updates tab's check: whether GitHub has a newer release, with `manual` set so the tab links to it instead of
/// offering the Windows zip.
pub fn check_for_update_now() -> String {
    match update::UpdateChecker::new().check_for_updates() {
        Ok(None) => json!({ "success": true, "available": false, "currentVersion": update::CURRENT_VERSION }).to_string(),
        Ok(Some(info)) => {
            update::UPDATE_STATUS.set(&info.version, &info.url, info.notes.as_deref());
            json!({
                "success": true, "available": true, "manual": true, "currentVersion": update::CURRENT_VERSION,
                "version": info.version, "url": info.url, "notes": info.notes,
                "assetName": null, "assetSize": 0, "hasDigest": false, "staged": false,
            })
            .to_string()
        }
        Err(e) => {
            SLOG.warn(format_args!("Manual update check failed: {e}"));
            json!({ "success": false, "error": "check_failed" }).to_string()
        }
    }
}

/// Shows each monitor's number (the ones getDisplays lists) for a few seconds, through the preview binary's
/// `--identify-displays` helper; a click while it's still up is ignored, like the Windows build.
pub fn identify_displays() -> bool {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return true;
    }
    let targets: Vec<String> = match hyprctl_json("monitors") {
        Some(Value::Array(list)) => list.iter().filter_map(|m| Some(format!("{}={}", m["name"].as_str()?, m["id"].as_i64()? + 1))).collect(),
        _ => Vec::new(),
    };
    // Next to this binary, else on PATH (as the preview app finds this one).
    let sibling = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("eve-maj-preview"))).filter(|p| p.is_file());
    let exe = sibling.unwrap_or_else(|| PathBuf::from("eve-maj-preview"));
    let child = if targets.is_empty() {
        Err(std::io::Error::other("hyprctl listed no monitors"))
    } else {
        std::process::Command::new(&exe).arg("--identify-displays").args(&targets).stdin(std::process::Stdio::null()).spawn()
    };
    match child {
        // Waited on from its own thread, so it's reaped as soon as it exits and the next click can show it again.
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
                RUNNING.store(false, Ordering::Release);
            });
            true
        }
        Err(e) => {
            SLOG.warn(format_args!("Couldn't start {}: {e}", exe.display()));
            RUNNING.store(false, Ordering::Release);
            false
        }
    }
}

/// Points LOCALAPPDATA at the running EVE client's Wine prefix (WINEPREFIX or Proton's STEAM_COMPAT_DATA_PATH/pfx),
/// so the EVE settings-folder scans (accounts, Ultra Potato) find the client's files.
pub fn set_local_app_data_from_wine() {
    if std::env::var_os("LOCALAPPDATA").is_some() {
        return;
    }
    let Ok(procs) = std::fs::read_dir("/proc") else { return };
    for p in procs.filter_map(|e| e.ok()) {
        let dir = p.path();
        let cmdline = std::fs::read(dir.join("cmdline")).unwrap_or_default().to_ascii_lowercase();
        if !cmdline.windows(11).any(|w| w == b"exefile.exe") && !cmdline.windows(15).any(|w| w == b"eve-online.exe") {
            continue;
        }
        let Ok(environ) = std::fs::read(dir.join("environ")) else { continue };
        let var = |name: &str| environ.split(|&b| b == 0).find_map(|kv| kv.strip_prefix(format!("{name}=").as_bytes())).map(|v| PathBuf::from(String::from_utf8_lossy(v).into_owned()));
        for prefix in [var("WINEPREFIX"), var("STEAM_COMPAT_DATA_PATH").map(|p| p.join("pfx"))].into_iter().flatten() {
            let Ok(users) = std::fs::read_dir(prefix.join("drive_c/users")) else { continue };
            for u in users.filter_map(|e| e.ok()) {
                let local = u.path().join("AppData/Local");
                if local.join("CCP/EVE").is_dir() {
                    SLOG.info(format_args!("Using EVE's Wine prefix for settings scans: {}", local.display()));
                    std::env::set_var("LOCALAPPDATA", &local);
                    return;
                }
            }
        }
    }
}

/// Run on login (runOnStartup): an XDG autostart entry for the preview app next to this binary.
pub fn apply_run_on_startup(enabled: bool) {
    let dir = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(x) => PathBuf::from(x).join("autostart"),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config/autostart"),
    };
    let file = dir.join("eve-maj-preview.desktop");
    if !enabled {
        let _ = std::fs::remove_file(&file);
        return;
    }
    let Some(exe) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("eve-maj-preview"))) else { return };
    if let Err(e) = desktop_icon::install() {
        SLOG.warn(format_args!("Couldn't install the app icon: {e}"));
    }
    let contents = format!(
        "[Desktop Entry]\nType=Application\nName=EVE-Maj Preview\nExec={}\nIcon={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exe.display(),
        desktop_icon::ICON_NAME
    );
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, contents)) {
        SLOG.warn(format_args!("Couldn't write {}: {e}", file.display()));
    }
}


pub fn open_url(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// The app draws the overlay and leaves the outcome in a file; any stale one is cleared first.
pub fn start_region_select(request: RegionSelectRequest) -> bool {
    let _ = std::fs::remove_file(region_select_result_path());
    send_command(&Command::StartRegionSelect(Box::new(request)))
}

pub fn poll_region_select() -> String {
    let path = region_select_result_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let _ = std::fs::remove_file(&path);
            RegionSelectResult::from_file_json(&text).unwrap_or_else(RegionSelectResult::cancelled).to_dialog_json()
        }
        Err(_) => json!({ "done": false }).to_string(),
    }
}

// Releases ship a Windows zip, so the download/install half stays Windows-only; a Linux build is updated by
// rebuilding or its package.
pub fn download_update() -> String {
    crate::backend::fail("unsupported")
}

pub fn install_update() -> String {
    crate::backend::fail("unsupported")
}
