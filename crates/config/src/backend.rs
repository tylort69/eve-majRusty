//! Answers the dialog page's `webui.call(...)` requests: the Zig build's config_dialog.zig bound functions, on top of
//! eve_maj_core. Replies are the same strings the Zig build returns (mostly JSON); see the call contract in the
//! comments of each arm. Shared by the Linux and Windows builds; what differs (talking to the running preview app,
//! window and display queries, startup entries, updates) is in each platform's `platform` module.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use eve_maj_core::config::{self, clamp_profile_name, parse_hex_color, profile_path, Config, GlobalSettings, Position, DEFAULT_ORE_TABLE, PROFILES_DIR};
use eve_maj_core::log::Scope;
use eve_maj_core::protocol::{Command, RegionSelectRequest};
use eve_maj_core::{esi_prices, eve_accounts, ultra_potato, update};
use serde_json::{json, Value};
use tao::event_loop::EventLoopProxy;

use crate::platform::{self, send_command, EN_HINTS};
use crate::server::{Asset, Handler};
use crate::UiEvent;

const SLOG: Scope = Scope::new("config");
const DEFAULT_PROFILE: &str = "default.json";

/// Which native picker a browse call opens.
#[derive(Debug, Clone, Copy)]
pub enum Browse {
    ChatlogDir,
    GamelogDir,
    SoundFile,
}

pub struct Backend {
    proxy: EventLoopProxy<UiEvent>,
    /// The profile file the dialog edits (not necessarily the one the app runs).
    profile: String,
    page: Vec<u8>,
}

fn ok() -> String {
    json!({ "success": true }).to_string()
}

pub fn fail(error: &str) -> String {
    json!({ "success": false, "error": error }).to_string()
}

fn arg(args: &[Value], i: usize) -> String {
    match args.get(i) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

const LANGS: &[(&str, &str, &str)] = &[
    ("en", "English", include_str!("../ui/lang/en.json")),
    ("de", "Deutsch", include_str!("../ui/lang/de.json")),
    ("ru", "Русский", include_str!("../ui/lang/ru.json")),
    ("zh", "简体中文", include_str!("../ui/lang/zh.json")),
    ("pt", "Português", include_str!("../ui/lang/pt.json")),
    ("es", "Español", include_str!("../ui/lang/es.json")),
    ("fr", "Français", include_str!("../ui/lang/fr.json")),
    ("pl", "Polski", include_str!("../ui/lang/pl.json")),
];

/// A language's catalog JSON, with the platform's rewording applied to English.
fn catalog_json(code: &str, catalog: &str) -> String {
    if code != "en" {
        return catalog.to_owned();
    }
    let Ok(Value::Object(mut map)) = serde_json::from_str::<Value>(catalog) else { return catalog.to_owned() };
    for (key, text) in EN_HINTS {
        if map.contains_key(*key) {
            map.insert((*key).to_owned(), Value::String((*text).to_owned()));
        } else {
            SLOG.warn(format_args!("Platform wording for a missing translation key: {key}"));
        }
    }
    Value::Object(map).to_string()
}

/// The page with styles, script, fonts, images and translations inlined, as the Zig build's injectResources does.
fn assemble_page(language: &str) -> Vec<u8> {
    let (lang, _, catalog) = LANGS.iter().find(|(code, _, _)| *code == language).unwrap_or(&LANGS[0]);
    let catalog = catalog_json(lang, catalog);
    let mut html = include_str!("../ui/config_dialog.html").to_owned();
    let replace = |html: &mut String, needle: &str, with: &str| {
        if html.contains(needle) {
            *html = html.replacen(needle, with, 1);
        } else {
            SLOG.warn(format_args!("Dialog page template is missing {needle:?}"));
        }
    };
    replace(&mut html, "/* Styles will be injected here by WebUI */", include_str!("../ui/config_dialog.css"));
    replace(&mut html, "LAYOUT_PREVIEW_IMAGE_PLACEHOLDER", &base64(include_bytes!("../ui/layout_preview.jpg")));
    replace(&mut html, "CASCADIA_CODE_WOFF2_PLACEHOLDER", &base64(include_bytes!("../ui/CascadiaCode.woff2")));
    // The saved window theme goes in ahead of the script so the page paints in it from the first frame instead of
    // switching once loadGlobalSettings answers. "<" is escaped so no stored value can close the script element.
    let theme = serde_json::to_string(&GlobalSettings::load().ui_theme).unwrap_or_else(|_| "null".into()).replace('<', "\\u003c");
    // The page stays hidden (html.pre-init) until the backend reveals it; here that happens as soon as it's set up.
    let script = format!(
        "window.__EVEMAJ_THEME__ = {theme};\n{}\nwindow.addEventListener('DOMContentLoaded', () => setTimeout(() => document.documentElement.classList.remove('pre-init'), 0));",
        include_str!("../ui/config_dialog.js")
    );
    replace(&mut html, "// Script will be injected here by WebUI", &script);
    replace(
        &mut html,
        "<!-- Favicon will be injected here by WebUI -->",
        &format!("<link rel=\"icon\" type=\"image/x-icon\" href=\"data:image/x-icon;base64,{}\">", base64(include_bytes!("../../../assets/icon.ico"))),
    );
    replace(&mut html, "window.__I18N__ = {}; /* Translations will be injected here by WebUI */", &format!("window.__I18N__ = {catalog};"));
    let names: serde_json::Map<String, Value> = LANGS.iter().map(|(c, n, _)| (c.to_string(), Value::String(n.to_string()))).collect();
    replace(
        &mut html,
        "window.__I18N_LANGS__ = {}; /* Language list will be injected here by WebUI */",
        &format!("window.__I18N_LANGS__ = {};", Value::Object(names)),
    );
    let all = LANGS.iter().map(|(c, _, cat)| format!("\"{c}\":{}", catalog_json(c, cat))).collect::<Vec<_>>().join(",");
    replace(&mut html, "window.__I18N_ALL__ = {}; /* All translations will be injected here by WebUI */", &format!("window.__I18N_ALL__ = {{{all}}};"));
    replace(&mut html, "<html lang=\"en\" class=\"pre-init\">", &format!("<html lang=\"{lang}\" class=\"pre-init\">"));
    html.into_bytes()
}

/// The startup notice's status. The dialog only asks when update checks are enabled, so the first ask starts the
/// background check (it asks again a few seconds later if nothing was known yet).
fn update_status() -> String {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        if let Err(e) = std::thread::Builder::new().name("update_check".into()).spawn(update::UpdateChecker::check_for_updates_background) {
            SLOG.warn(format_args!("Failed to start update check thread: {e}"));
        }
    });
    let s = &update::UPDATE_STATUS;
    json!({ "available": s.is_available(), "version": s.version(), "url": s.url(), "notes": s.notes() }).to_string()
}

/// A profile file, or a backup to restore ("backup\<file>" / "backup/<file>"), as a path; None if the name is unsafe.
fn source_profile_path(source: &str) -> Option<PathBuf> {
    match source.strip_prefix("backup\\").or_else(|| source.strip_prefix("backup/")) {
        Some(file) => config::is_safe_profile_name(file).then(|| Path::new(PROFILES_DIR).join("backup").join(file)),
        None => profile_path(source).ok(),
    }
}

impl Backend {
    pub fn new(proxy: EventLoopProxy<UiEvent>, profile: String, language: &str) -> Self {
        Backend { proxy, profile, page: assemble_page(language) }
    }

    fn browse(&self, kind: Browse) -> String {
        let (tx, rx) = mpsc::channel();
        if self.proxy.send_event(UiEvent::Browse(kind, tx)).is_err() {
            return String::new();
        }
        rx.recv().unwrap_or_default()
    }

    fn load_config(&self) -> String {
        match Config::load_profile(&self.profile).and_then(|c| c.to_json_string()) {
            Ok(s) => s,
            Err(e) => {
                SLOG.err(format_args!("Failed to load {}: {e}", self.profile));
                json!({ "error": "Failed to read config file" }).to_string()
            }
        }
    }

    fn save_config(&self, text: &str) -> String {
        let cfg = match Config::build_config_from_json(text.as_bytes(), &self.profile) {
            Ok(c) => c,
            Err(e) => {
                SLOG.err(format_args!("Rejected configuration: {e}"));
                return fail("Invalid configuration");
            }
        };
        let Ok(path) = profile_path(&self.profile) else { return fail("Failed to write file") };
        if let Err(e) = cfg.save_to_json_file(path) {
            SLOG.err(format_args!("Failed to save {}: {e}", self.profile));
            return fail("Failed to write file");
        }
        // The running app reloads a changed profile file by itself; this makes the saved one the live one.
        send_command(&Command::Profile(self.profile.clone().into_bytes()));
        ok()
    }

    fn list_profiles(&self) -> String {
        let mut profiles = GlobalSettings::enumerate_profiles().unwrap_or_default();
        profiles.retain(|p| p != eve_maj_core::accounts_store::ACCOUNTS_FILE);
        profiles.sort();
        json!({ "profiles": profiles, "current": self.profile }).to_string()
    }

    fn list_backups(&self) -> String {
        let mut backups: Vec<String> = std::fs::read_dir(Path::new(PROFILES_DIR).join("backup"))
            .map(|d| d.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.ends_with(".json")).collect())
            .unwrap_or_default();
        backups.sort_by(|a, b| b.cmp(a));
        json!({ "backups": backups }).to_string()
    }

    fn switch_profile(&mut self, name: &str) -> String {
        if profile_path(name).is_err() {
            return fail("Failed to allocate path");
        }
        self.profile = name.to_owned();
        let mut gs = GlobalSettings::load();
        if let Err(e) = gs.update_last_used(name) {
            SLOG.warn(format_args!("Couldn't record last used profile: {e}"));
        }
        ok()
    }

    fn create_profile(&self, name: &str, accent: &str) -> String {
        let file = format!("{}.json", clamp_profile_name(name));
        let Ok(path) = profile_path(&file) else { return fail("Memory allocation failed") };
        if path.exists() {
            return fail("Profile already exists");
        }
        let Ok(mut cfg) = Config::defaults_with_profile(&file) else { return fail("Failed to write profile") };
        if let Ok(color) = parse_hex_color(accent) {
            cfg.accent_color = color;
        }
        match cfg.save_to_json_file(&path) {
            Ok(()) => ok(),
            Err(_) => fail("Failed to write profile"),
        }
    }

    fn copy_profile(&self, request: &str) -> String {
        let Ok(v) = serde_json::from_str::<Value>(request) else { return fail("Invalid JSON") };
        if !v.is_object() {
            return fail("Expected JSON object");
        }
        let Some(source) = v["source"].as_str() else { return fail("Missing source") };
        let Some(target) = v["target"].as_str() else { return fail("Missing target") };
        let target_file = format!("{}.json", clamp_profile_name(target));
        let (Some(src), Ok(dst)) = (source_profile_path(source), profile_path(&target_file)) else { return fail("Memory allocation failed") };
        if dst.exists() {
            return fail("Target profile already exists");
        }
        if std::fs::copy(&src, &dst).is_err() {
            return fail("Failed to copy file");
        }
        if let Some(Ok(color)) = v["accentColor"].as_str().map(parse_hex_color) {
            if let Ok(mut cfg) = Config::load_profile(&target_file) {
                cfg.accent_color = color;
                if let Err(e) = cfg.save_to_json_file(&dst) {
                    SLOG.warn(format_args!("Couldn't set the copy's accent color: {e}"));
                }
            }
        }
        ok()
    }

    fn delete_profile(&self, name: &str) -> String {
        if name == DEFAULT_PROFILE {
            return fail("Cannot delete default profile");
        }
        let Ok(path) = profile_path(name) else { return fail("Memory allocation failed") };
        let backup_dir = Path::new(PROFILES_DIR).join("backup");
        if std::fs::create_dir_all(&backup_dir).is_err() {
            return fail("Failed to create backup directory");
        }
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        match std::fs::rename(&path, backup_dir.join(format!("{secs}_{name}"))) {
            Ok(()) => ok(),
            Err(e) => fail(&format!("Failed to delete: {:?}", e.kind())),
        }
    }

    fn reset_profile(&self, name: &str) -> String {
        let (Ok(path), Ok(cfg)) = (profile_path(name), Config::defaults_with_profile(name)) else { return fail("Memory allocation failed") };
        match cfg.save_to_json_file(path) {
            Ok(()) => ok(),
            Err(_) => fail("Failed to write profile"),
        }
    }

    fn load_global_settings(&self) -> String {
        let gs = GlobalSettings::load();
        let Some(mut v) = gs.to_json_string().ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else { return "{}".into() };
        let table: Vec<Value> = DEFAULT_ORE_TABLE
            .iter()
            .map(|e| json!({ "name": e.name, "category": e.category, "volumeM3": e.volume_m3, "price": gs.ore_price(e.name).unwrap_or(e.price) }))
            .collect();
        v["oreTable"] = Value::Array(table);
        v.to_string()
    }

    fn save_global_settings(&self, text: &str) -> String {
        let Ok(_) = serde_json::from_str::<Value>(text) else { return fail("Invalid JSON") };
        let mut gs: GlobalSettings = match serde_json::from_str(text) {
            Ok(g) => g,
            Err(e) => {
                SLOG.err(format_args!("Rejected global settings: {e}"));
                return fail("Invalid settings format");
            }
        };
        let on_disk = GlobalSettings::load();
        gs.dialog_x = on_disk.dialog_x;
        gs.dialog_y = on_disk.dialog_y;
        // IDs the preview app learned while the dialog was open aren't in the page's copy; keep them.
        for (name, id) in on_disk.character_id_map_snapshot() {
            if !gs.has_character_id(&name) {
                let _ = gs.update_character_id(&name, &id);
            }
        }
        if gs.save().is_err() {
            return fail("Failed to write file");
        }
        platform::apply_run_on_startup(gs.run_on_startup);
        ok()
    }

    /// Loads the current profile from disk, changes it with `f` (which saves) and reports like the Zig build.
    fn edit_profile(&self, f: impl FnOnce(&mut Config) -> Result<(), config::ConfigError>, extra: Value) -> String {
        let Ok(mut cfg) = Config::load_profile(&self.profile) else { return fail("Failed to load profile") };
        if f(&mut cfg).is_err() {
            return fail("Failed to save");
        }
        let mut out = json!({ "success": true });
        if let (Value::Object(o), Value::Object(e)) = (&mut out, extra) {
            o.extend(e);
        }
        out.to_string()
    }

    fn set_window_position(&self, name: &str, all: bool) -> String {
        let Some((_, x, y, minimized)) = platform::eve_clients().into_iter().find(|c| c.0 == name) else {
            return fail(if all { "Selected character is not currently open" } else { "Character is not currently open" });
        };
        if minimized {
            return fail(if all { "Selected character's window is minimized" } else { "Character's window is minimized" });
        }
        let pos = Position { x: x as i32, y: y as i32 };
        if all {
            self.edit_profile(|c| c.save_all_character_window_positions(pos), json!({ "x": x, "y": y }))
        } else {
            self.edit_profile(|c| c.save_character_window_position(name, pos), json!({ "x": x, "y": y }))
        }
    }
}

impl Handler for Backend {
    fn call_without_state(name: &str) -> Option<String> {
        // A GitHub request can take a while when offline; it mustn't block saves and status polls meanwhile.
        (name == "checkForUpdateNow").then(platform::check_for_update_now)
    }

    fn asset(&self, path: &str) -> Option<Asset> {
        match path {
            "/" | "/index.html" => Some(Asset { content_type: "text/html; charset=utf-8", body: self.page.clone() }),
            "/webui.js" => Some(Asset { content_type: "text/javascript; charset=utf-8", body: include_bytes!("../ui/webui.js").to_vec() }),
            _ => None,
        }
    }

    fn call(&mut self, name: &str, args: Vec<Value>) -> String {
        let a0 = arg(&args, 0);
        match name {
            "closeDialog" => {
                let _ = self.proxy.send_event(UiEvent::Close);
                String::new()
            }
            "getConfigData" => json!({
                "configPath": format!("{PROFILES_DIR}/{}", self.profile),
                "version": env!("CARGO_PKG_VERSION"),
                "categories": [
                    {"id": "general", "name": "General"}, {"id": "thumbnails", "name": "Thumbnails"},
                    {"id": "display", "name": "Display"}, {"id": "hotkeys", "name": "Hotkeys"},
                    {"id": "colors", "name": "Colors"}, {"id": "states", "name": "Visual States"},
                    {"id": "notifications", "name": "Notifications"}, {"id": "chatlog", "name": "Chatlog"},
                    {"id": "characters", "name": "Characters"}
                ]
            })
            .to_string(),
            "loadConfig" => self.load_config(),
            "getDefaultConfig" => Config::defaults_with_profile(&self.profile)
                .and_then(|c| c.to_json_string())
                .unwrap_or_else(|_| json!({ "error": "Failed to build defaults" }).to_string()),
            "saveConfig" => self.save_config(&a0),
            "getValidationRanges" => Config::build_validation_ranges_json(),
            "listProfiles" => self.list_profiles(),
            "listProfileBackups" => self.list_backups(),
            "switchProfile" => self.switch_profile(&a0),
            "createProfile" => self.create_profile(&a0, &arg(&args, 1)),
            "copyProfile" => self.copy_profile(&a0),
            "deleteProfile" => self.delete_profile(&a0),
            "resetProfile" => self.reset_profile(&a0),
            "loadGlobalSettings" => self.load_global_settings(),
            "saveGlobalSettings" => self.save_global_settings(&a0),
            "logClientMessage" => {
                let msg = arg(&args, 1);
                if a0 == "warn" {
                    SLOG.warn(format_args!("[js] {msg}"));
                } else {
                    SLOG.err(format_args!("[js] {msg}"));
                }
                String::new()
            }
            "scaleLegacyPositions" => {
                let Ok(Value::Array(list)) = serde_json::from_str::<Value>(&a0) else { return "[]".into() };
                let scaled: Vec<Value> = list
                    .iter()
                    .map(|p| {
                        let pos = Position { x: p["x"].as_i64().unwrap_or(0) as i32, y: p["y"].as_i64().unwrap_or(0) as i32 }.scale_from_legacy_dpi_unaware();
                        json!({ "x": pos.x, "y": pos.y })
                    })
                    .collect();
                Value::Array(scaled).to_string()
            }
            "setCharacterWindowPosition" => self.set_window_position(&a0, false),
            "setAllCharacterWindowPositions" => self.set_window_position(&a0, true),
            "clearCharacterWindowPosition" => self.edit_profile(|c| c.clear_character_window_position(&a0), json!({})),
            "clearAllCharacterWindowPositions" => self.edit_profile(|c| c.clear_all_character_window_positions(), json!({})),
            "getOpenClients" => Value::Array(platform::eve_clients().into_iter().map(|c| Value::String(c.0)).collect()).to_string(),
            "getRunningWindows" => platform::running_windows(),
            "getDisplays" => platform::displays(),
            "identifyDisplays" => json!({ "success": platform::identify_displays() }).to_string(),

            // Live: the running preview app.
            "getMainAppStatus" => json!({ "running": platform::main_app_running() }).to_string(),
            "switchProfileLive" => {
                send_command(&Command::Profile(a0.into_bytes()));
                ok()
            }
            "previewThumbnailConfig" => {
                send_command(&Command::PreviewThumbnail(a0));
                ok()
            }
            "testNotification" => json!({ "success": send_command(&Command::TestNotification(a0)) }).to_string(),
            "suspendHotkeysForRecording" => {
                send_command(&Command::DialogSuspendHotkeys);
                ok()
            }
            "resumeHotkeysAfterRecording" => {
                send_command(&Command::DialogResumeHotkeys);
                ok()
            }
            "startRegionSelect" => {
                if platform::start_region_select(RegionSelectRequest::from_dialog_json(&a0)) {
                    ok()
                } else {
                    fail("EVE-Maj Preview isn't running")
                }
            }
            "pollRegionSelectResult" => platform::poll_region_select(),

            // Desktop integration.
            "browseChatlogDir" => self.browse(Browse::ChatlogDir),
            "browseGamelogDir" => self.browse(Browse::GamelogDir),
            "browseSoundFile" => self.browse(Browse::SoundFile),
            "setAlwaysOnTop" => {
                let _ = self.proxy.send_event(UiEvent::SetAlwaysOnTop(args.first().and_then(Value::as_bool).unwrap_or(false)));
                String::new()
            }
            "openUrlInBrowser" => {
                let lower = a0.to_ascii_lowercase();
                let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"));
                let valid = rest.is_some_and(|r| !r.is_empty() && !r.starts_with(['/', '\\'])) && a0.len() < 1024 && a0.bytes().all(|b| b > 0x20 && b != 0x7F);
                if valid {
                    platform::open_url(&a0);
                } else {
                    SLOG.warn(format_args!("Refusing to open {a0:?}"));
                }
                String::new()
            }

            // Updates: GitHub is checked and the release linked; downloading and installing the portable zip is Windows-only.
            "getUpdateStatus" => update_status(),
            "downloadUpdate" => platform::download_update(),
            "installUpdate" => platform::install_update(),

            // EVE files and network.
            "loadAccounts" => eve_accounts::load_accounts(),
            "saveAccounts" => eve_accounts::save_accounts(&a0),
            "scanEveAccounts" => eve_accounts::scan_eve_accounts(),
            "fetchOrePrices" => esi_prices::fetch_ore_prices(&a0),
            "scanUltraPotatoProfiles" => serde_json::to_string(&ultra_potato::scan_profiles()).unwrap_or_else(|_| "[]".into()),
            "applyUltraPotatoMode" => {
                let Ok(paths) = serde_json::from_str::<Vec<String>>(&a0) else { return fail("Invalid request") };
                if paths.is_empty() {
                    return fail("No profiles selected");
                }
                let known: Vec<String> = ultra_potato::scan_profiles().into_iter().map(|p| p.path).collect();
                if !paths.iter().all(|p| known.contains(p)) {
                    return fail("Invalid request");
                }
                let results = ultra_potato::apply_to_files(&paths);
                serde_json::to_string(&json!({ "success": true, "results": results })).unwrap_or_else(|_| fail("Failed to build response"))
            }
            _ => {
                SLOG.warn(format_args!("Unhandled call {name} ({} args)", args.len()));
                String::new()
            }
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn linux_wording_replaces_existing_english_keys_only() {
        let en: Value = serde_json::from_str(LANGS[0].2).unwrap();
        for (key, _) in EN_HINTS {
            assert!(en.get(*key).is_some(), "en.json no longer has {key}");
        }
        let patched: Value = serde_json::from_str(&catalog_json("en", LANGS[0].2)).unwrap();
        assert_eq!(patched["field.regionDisplayUseWorkArea.label"], "Leave Room for Bars");
        assert!(!patched["tab.notifications.section.text-to-speech.hint"].as_str().unwrap().contains("Windows"));
        assert_eq!(catalog_json("de", LANGS[1].2), LANGS[1].2);
    }
}
