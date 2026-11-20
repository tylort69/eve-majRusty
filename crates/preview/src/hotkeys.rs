//! Hotkeys on Windows: the Zig build's hotkeys.zig HotkeyManager, with its decisions (which client a cycle lands on,
//! exclusions, which profile is next) in eve_maj_core::hotkeys, shared with the Linux build.
//!
//! Keyboard hotkeys go through RegisterHotKey on the timer window; mouse buttons and the wheel go through the
//! WH_MOUSE_LL hook in eve_maj_app::mouse_hook, which re-posts matches as WM_HOTKEY, so both arrive at
//! [`on_wm_hotkey`]. A press runs against a snapshot of the running clients; activations and other effects are
//! carried out afterwards, with no borrow of the manager held, because activating a client re-enters the manager
//! (update_focused_character) through the click path.

use eve_maj_core::accounts_store::{self, Membership};
use eve_maj_core::hotkeys::{Effect, HotkeyHost, HotkeyManager as CoreManager};
use eve_maj_core::log::Scope;
use eve_maj_core::protocol::HotkeyAction as LinkAction;
use eve_maj_core::virtual_keys::{extract_modifiers, extract_vk, format_virtual_key, is_mouse_hook_vk};
use eve_maj_core::types::RegionFitOrder;
use eve_maj_win::window::{is_window, is_window_iconic, is_window_visible, process_exe_path};
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, TRUE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_NOREPEAT};
use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, PostMessageW, ShowWindowAsync, SW_RESTORE};

use crate::globals;
use crate::input;
use crate::tray::WM_HOTKEYS_STATE_CHANGED;

const SLOG: Scope = Scope::new("hotkeys");

pub struct HotkeyManager {
    core: CoreManager<HWND>,
    /// The timer window every binding posts to.
    hwnd: HWND,
    /// Ids currently registered with RegisterHotKey.
    keyboard_ids: Vec<i32>,
    /// Combined keys currently routed through the mouse hook.
    mouse_keys: Vec<u32>,
    /// The settings dialog is recording a key binding; everything is unregistered until it's done.
    dialog_suspended: bool,
}

/// What the hotkey logic sees of the running app at the moment of a press; activations are carried out afterwards.
struct Snapshot {
    windows: Vec<(HWND, String)>,
    not_logged_in: Vec<HWND>,
    focused: Option<HWND>,
    under_cursor: Option<String>,
    activated: Vec<HWND>,
}

impl Snapshot {
    fn take() -> Snapshot {
        let windows: Vec<(HWND, String)> = globals::scout().map(|s| s.windows.iter().map(|w| (w.hwnd, w.character_name.clone())).collect()).unwrap_or_default();
        let foreground = unsafe { GetForegroundWindow() };
        let focused = windows.iter().any(|(w, _)| *w == foreground).then_some(foreground);
        let under_cursor = input::resolve_thumbnail_under_cursor().and_then(|i| globals::painter().map(|p| p.thumbnails[i].character_name.clone()));
        Snapshot {
            windows,
            not_logged_in: globals::scout().map(|s| s.not_logged_in_hwnds()).unwrap_or_default(),
            focused,
            under_cursor,
            activated: Vec::new(),
        }
    }
}

impl HotkeyHost<HWND> for Snapshot {
    fn windows(&self) -> Vec<(HWND, String)> {
        self.windows.clone()
    }
    fn not_logged_in_windows(&self) -> Vec<HWND> {
        self.not_logged_in.clone()
    }
    fn focused(&self) -> Option<HWND> {
        self.focused
    }
    fn activate(&mut self, window: HWND) {
        self.activated.push(window);
    }
    fn thumbnail_under_cursor(&self) -> Option<String> {
        self.under_cursor.clone()
    }
    fn notified_character_names(&self, retention_ms: u64) -> Vec<String> {
        globals::painter().map(|p| p.notified_character_names(retention_ms)).unwrap_or_default()
    }
    fn account_membership(&mut self) -> Membership {
        accounts_store::load_membership()
    }
}

impl HotkeyManager {
    /// Builds the bindings for the live profile and registers them on `hwnd` (the timer window).
    pub fn new(hwnd: HWND) -> HotkeyManager {
        let mut m = HotkeyManager { core: CoreManager::default(), hwnd, keyboard_ids: Vec::new(), mouse_keys: Vec::new(), dialog_suspended: false };
        m.rebuild_and_register();
        m
    }

    /// Re-reads bindings from the live profile and global settings (after a profile reload), keeping cycle cursors.
    pub fn rebuild_and_register(&mut self) {
        let (cfg, gs) = (globals::config(), globals::GLOBAL_SETTINGS.get().expect("global settings are loaded first"));
        self.core.rebuild(cfg, gs);
        self.register_all();
    }

    /// (Re)registers every live binding: all of them, only the suspend key while suspended, none while the dialog records a key.
    fn register_all(&mut self) {
        self.unregister_all();
        if self.dialog_suspended {
            SLOG.info(format_args!("Hotkeys paused while the settings dialog records a key"));
            return;
        }
        let allow_repeat = globals::config().hotkeys.allow_hotkey_auto_repeat;
        let (mut ok, mut total) = (0, 0);
        let bindings: Vec<_> = self.core.active_bindings().cloned().collect();
        for b in bindings {
            total += 1;
            let name = format_virtual_key(b.key);
            if is_mouse_hook_vk(extract_vk(b.key)) {
                match eve_maj_app::mouse_hook::register(self.hwnd, b.key, b.id as i32) {
                    Ok(()) => {
                        self.mouse_keys.push(b.key);
                        ok += 1;
                        SLOG.debug(format_args!("Registered mouse hotkey {name} for {}", b.description));
                    }
                    Err(err) => SLOG.err(format_args!("Failed to register mouse hotkey {name} for {}: {err}", b.description)),
                }
                continue;
            }
            let modifiers = extract_modifiers(b.key) | if allow_repeat { 0 } else { MOD_NOREPEAT };
            if unsafe { RegisterHotKey(self.hwnd, b.id as i32, modifiers, extract_vk(b.key)) } != 0 {
                self.keyboard_ids.push(b.id as i32);
                ok += 1;
                SLOG.debug(format_args!("Registered hotkey {name} (id {}) for {}", b.id, b.description));
            } else {
                // Most often another program (or a second binding in this profile) already owns the combination.
                SLOG.err(format_args!("Failed to register hotkey {name} for {}: it is probably already in use by another program", b.description));
            }
        }
        SLOG.info(format_args!("Registered {ok} of {total} hotkey(s){}", if self.core.is_suspended() { " (suspended: only the suspend key is live)" } else { "" }));
    }

    pub fn unregister_all(&mut self) {
        for id in self.keyboard_ids.drain(..) {
            unsafe { UnregisterHotKey(self.hwnd, id) };
        }
        for key in self.mouse_keys.drain(..) {
            eve_maj_app::mouse_hook::unregister(key);
        }
    }

    pub fn is_character_excluded(&self, character_name: &str) -> bool {
        self.core.is_character_excluded(globals::config(), character_name)
    }

    pub fn update_focused_character(&mut self, character_name: &str, hwnd: HWND) {
        self.core.update_focused_character(globals::config(), character_name, hwnd);
    }

    pub fn toggle_character_exclusion(&mut self, character_name: &str) {
        self.core.toggle_character_exclusion(globals::config(), character_name);
    }

    pub fn are_hotkeys_suspended(&self) -> bool {
        self.core.is_suspended()
    }

    /// The tray's "Suspend Hotkeys".
    pub fn handle_suspend_hotkeys_request(&mut self) {
        run_link_action(LinkAction::SuspendHotkeys);
    }

    /// The tray's "Restore Saved Positions".
    pub fn handle_move_to_saved_positions_request(&mut self) {
        apply_effect(Effect::MoveToSavedPositions);
    }

    /// The settings dialog started or stopped recording a key binding.
    pub fn set_dialog_suspended(&mut self, suspended: bool) {
        if self.dialog_suspended == suspended {
            return;
        }
        self.dialog_suspended = suspended;
        SLOG.info(format_args!("Settings dialog {} hotkeys", if suspended { "paused" } else { "resumed" }));
        self.register_all();
    }

    /// Called each tick: config.exe pauses the hotkeys while it records a key and resumes them when it closes, so if it
    /// crashes or is killed in between, nothing would ever resume them. Resume once its window is gone.
    pub fn check_dialog_alive(&mut self) {
        if !self.dialog_suspended {
            return;
        }
        let title = eve_maj_win::wide(CONFIG_WINDOW_TITLE);
        if unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) }.is_null() {
            SLOG.warn(format_args!("The settings window is gone without resuming the hotkeys; resuming them"));
            self.set_dialog_suspended(false);
        }
    }
}

/// The title of config.exe's window (crates/config/src/windows/mod.rs).
const CONFIG_WINDOW_TITLE: &str = "EVE-Maj Preview Configuration";

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.unregister_all();
    }
}

/// A WM_HOTKEY from RegisterHotKey or the mouse hook.
pub fn on_wm_hotkey(id: u32) {
    let Some(manager) = globals::hotkey_manager() else { return };
    let mut snap = Snapshot::take();
    let gs = globals::GLOBAL_SETTINGS.get().expect("global settings are loaded first");
    SLOG.debug(format_args!("Hotkey pressed: id {id}"));
    let effect = manager.core.handle_press(id, globals::config(), gs, &mut snap);
    finish(snap, effect);
}

/// An evemajpreview://hotkey/<action> link or a tray action; bypasses requireEveFocus and suspension, like the Zig build.
pub fn run_link_action(action: LinkAction) {
    let Some(manager) = globals::hotkey_manager() else {
        SLOG.warn(format_args!("Hotkey action {} ignored: hotkeys are not initialised", action.name()));
        return;
    };
    let mut snap = Snapshot::take();
    let gs = globals::GLOBAL_SETTINGS.get().expect("global settings are loaded first");
    let effect = manager.core.run_global_action(action, globals::config(), gs, &mut snap);
    finish(snap, effect);
}

fn finish(snap: Snapshot, effect: Option<Effect>) {
    if let Some(&window) = snap.activated.last() {
        let name = snap.windows.iter().find(|(w, _)| *w == window).map_or("", |(_, n)| n.as_str());
        SLOG.debug(format_args!("Hotkey activates {name}"));
        input::handle_thumbnail_click(window);
    }
    if let Some(effect) = effect {
        apply_effect(effect);
    }
}

fn apply_effect(effect: Effect) {
    SLOG.debug(format_args!("Hotkey effect: {effect:?}"));
    let config = globals::config();
    match effect {
        Effect::MinimizeAll => {
            if let Some(scout) = globals::scout() {
                crate::manager::minimize_all_clients(&scout.windows);
            }
        }
        Effect::CloseAll => {
            if let Some(scout) = globals::scout() {
                crate::manager::close_all_clients(&scout.windows, config);
            }
        }
        Effect::ToggleVisibility => {
            if let Some(painter) = globals::painter() {
                painter.toggle_all_thumbnails_visibility();
            }
        }
        Effect::ToggleAutoMinimize => {
            config.auto_minimize.enabled = !config.auto_minimize.enabled;
            SLOG.info(format_args!("Auto-minimize {}", if config.auto_minimize.enabled { "enabled" } else { "disabled" }));
        }
        Effect::MoveToSavedPositions => {
            if let Some(scout) = globals::scout() {
                crate::manager::move_all_clients_to_saved_positions(&scout.windows, config);
            }
        }
        Effect::SwitchProfile(name) => crate::tray::request_profile_switch(name),
        Effect::ToggleExclusionOfFocused => {
            let foreground = unsafe { GetForegroundWindow() };
            if globals::painter().is_some_and(|p| p.has_thumbnail(foreground)) {
                input::handle_thumbnail_shift_click_forced(foreground);
            }
        }
        Effect::ReturnToLastApp => {
            let target = globals::painter().and_then(|p| p.last_non_eve_foreground);
            match target {
                Some(hwnd) if is_window(hwnd) => {
                    SLOG.info(format_args!("Returning to the last non-EVE window"));
                    focus_window(hwnd);
                }
                _ => {
                    SLOG.debug(format_args!("No previous non-EVE window to return to"));
                    if let Some(p) = globals::painter() {
                        p.last_non_eve_foreground = None;
                    }
                }
            }
        }
        Effect::ActivateApp(exe) => match find_window_by_executable(&exe) {
            Some(hwnd) => {
                SLOG.info(format_args!("Activating app {exe}"));
                focus_window(hwnd);
            }
            None => SLOG.debug(format_args!("Activate app hotkey: no running window found for {exe}")),
        },
        Effect::OpenUrl { url, upload_clipboard } => {
            if url.is_empty() {
                SLOG.debug(format_args!("Open URL hotkey pressed - no URL set"));
            } else if upload_clipboard {
                SLOG.info(format_args!("Open URL hotkey (uploading clipboard): {url}"));
                eve_maj_app::paste_upload::upload_clipboard_and_open_async(&url);
            } else {
                SLOG.info(format_args!("Open URL hotkey: {url}"));
                if !eve_maj_win::shell::shell_open(&url, None) {
                    SLOG.err(format_args!("Failed to open URL '{url}'"));
                }
            }
        }
        Effect::SuspendChanged(_) => {
            if let Some(manager) = globals::hotkey_manager() {
                manager.register_all();
            }
            if let Some(timer) = globals::timer_hwnd() {
                unsafe { PostMessageW(timer, WM_HOTKEYS_STATE_CHANGED, 0, 0) };
            }
        }
        Effect::GroupMembershipChanged(name) => {
            if let Some(painter) = globals::painter() {
                for i in 0..painter.thumbnails.len() {
                    if painter.thumbnails[i].character_name == name {
                        painter.refresh_group_badge(i);
                        painter.thumbnails[i].needs_render = true;
                    }
                }
                if config.display.region_fit_order == RegionFitOrder::HotkeyGroups {
                    painter.reposition_all_thumbnails();
                }
                painter.render_dirty_thumbnails(None);
            }
        }
    }
}

fn focus_window(hwnd: HWND) {
    if is_window_iconic(hwnd) {
        unsafe { ShowWindowAsync(hwnd, SW_RESTORE) };
    }
    input::force_set_foreground_window(hwnd);
}

struct FindByExe<'a> {
    exe: &'a str,
    found: Option<HWND>,
}

unsafe extern "system" fn find_by_exe_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam as *mut FindByExe);
    if !is_window_visible(hwnd) {
        return TRUE;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    let Some(path) = process_exe_path(pid) else { return TRUE };
    let base = path.rsplit(['\\', '/']).next().unwrap_or("");
    if base.eq_ignore_ascii_case(ctx.exe) {
        ctx.found = Some(hwnd);
        return 0;
    }
    TRUE
}

/// First visible top-level window (in Z-order, so effectively the frontmost) owned by `exe`.
fn find_window_by_executable(exe: &str) -> Option<HWND> {
    let mut ctx = FindByExe { exe, found: None };
    unsafe { EnumWindows(Some(find_by_exe_callback), &mut ctx as *mut FindByExe as LPARAM) };
    ctx.found
}
