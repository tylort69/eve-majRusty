//! The Client List view (`viewMode: ClientList`) and the notification History Panel on Windows: two small topmost
//! layered windows drawn with GDI into 32-bit bitmaps, like the Zig build's list_view.zig and notif_info_view.zig.
//! What they show (row order, texts, colours, the history buffer, when the History Panel is up) comes from
//! eve_maj_core::panels, shared with the Linux panels, which this draws the same way.
//!
//! Each panel is dragged by its header (when dragging is enabled), snapping like a thumbnail, and its position is
//! saved when the profile says to remember it. Clicking a list row focuses that client (Shift+click toggles its
//! cycle exclusion); clicking a history row focuses the client it came from, and the footer's buttons filter the
//! history by category. The state lives in its own global, so a profile reload keeps the history.

use eve_maj_core::config::Position;
use eve_maj_core::log::Scope;
use eve_maj_core::panels::{self as model, HistoryPanelVisibility, HistoryRow, ListItem, ListRow, NotificationHistory};
use eve_maj_core::render::{self, now_ms, Metrics};
use eve_maj_core::state::VisibilityState;
use eve_maj_core::types::{FontWeight, NotificationType, ViewMode};
use eve_maj_core::ui_theme::{self, UiPalette};
use eve_maj_win::input::is_shift_pressed;
use eve_maj_win::time::Ticks;
use eve_maj_win::wide;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, GetDC, MonitorFromPoint, ReleaseDC, SelectObject, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, HFONT,
    MONITOR_DEFAULTTONEAREST, OUT_DEFAULT_PRECIS,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::gdi_overlay::{self, OverlayBitmap};
use crate::globals::{self, Global};
use crate::input;
use crate::painter::Painter;
use crate::scout::is_generic_character_name;

const SLOG: Scope = Scope::new("panels");
const CLASS_NAME: &str = "EVEMajPanel";
const TEXT_BUF: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    List,
    History,
}

#[derive(Debug, Clone, PartialEq)]
struct Font {
    name: String,
    size: i32,
    weight: FontWeight,
}

#[derive(Debug, Clone, PartialEq)]
struct ListContent {
    palette: UiPalette,
    rows: Vec<ListRow>,
    columns: u32,
    opacity: u8,
    font: Font,
}

#[derive(Debug, Clone, PartialEq)]
struct HistoryContent {
    palette: UiPalette,
    rows: Vec<HistoryRow<HWND>>,
    empty_text: &'static str,
    opacity: u8,
    font: Font,
    show_filters: bool,
    enabled: [bool; 5],
    footer_top: i32,
}

#[derive(Debug, Clone, PartialEq)]
enum Content {
    List(ListContent),
    History(HistoryContent),
}

impl Content {
    fn font(&self) -> &Font {
        match self {
            Content::List(c) => &c.font,
            Content::History(c) => &c.font,
        }
    }

    fn opacity(&self) -> u8 {
        match self {
            Content::List(c) => c.opacity,
            Content::History(c) => c.opacity,
        }
    }
}

struct Panel {
    hwnd: HWND,
    bitmap: Option<OverlayBitmap>,
    pos: (i32, i32),
    size: (i32, i32),
    /// The configured position this panel was last placed from; a new one (reload, live preview) moves it.
    config_pos: (i32, i32),
    /// What it should show, and what it last drew.
    content: Option<Content>,
    shown: Option<Content>,
}

#[derive(Default)]
struct Panels {
    list: Option<Panel>,
    history_panel: Option<Panel>,
    /// The client behind each list row (display order), and behind each history row drawn.
    list_sources: Vec<HWND>,
    history_sources: Vec<HWND>,
    history: NotificationHistory<HWND>,
    visibility: HistoryPanelVisibility,
    /// Which panel is in a header drag (the system move loop), so updates don't move it meanwhile.
    dragging: Option<Kind>,
    /// Each panel's fonts (they can differ per panel).
    fonts: [Fonts; 2],
    /// The settings window's theme (Preferences), global.settings.json's mtime when it was read, and when that was
    /// last checked; None until first use.
    theme: Option<(UiPalette, Option<std::time::SystemTime>, Ticks)>,
}

/// The fonts one panel draws with: the profile's, a semibold of it, and smaller cuts of each.
#[derive(Default)]
struct Fonts {
    key: Option<Font>,
    normal: Option<HFONT>,
    small: Option<HFONT>,
    bold: Option<HFONT>,
    small_bold: Option<HFONT>,
}

/// Semibold, like the Linux panels' titles and names.
const FW_SEMIBOLD: i32 = 600;

fn create_font(name: &str, size: i32, weight: i32, italic: bool) -> Option<HFONT> {
    let name_w = wide(name);
    let font = unsafe {
        CreateFontW(
            -size.max(6), 0, 0, 0, weight, italic as u32, 0, 0, DEFAULT_CHARSET as u32, OUT_DEFAULT_PRECIS as u32, CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32, DEFAULT_PITCH as u32, name_w.as_ptr(),
        )
    };
    (!font.is_null()).then_some(font)
}

impl Fonts {
    fn release(&mut self) {
        for f in [self.normal.take(), self.small.take(), self.bold.take(), self.small_bold.take()].into_iter().flatten() {
            unsafe { DeleteObject(f) };
        }
        self.key = None;
    }

    /// Recreated only when the profile's font changes; false if GDI couldn't make one.
    fn ensure(&mut self, font: &Font) -> bool {
        if self.key.as_ref() == Some(font) {
            return true;
        }
        self.release();
        let name = if font.name.trim().is_empty() { "Segoe UI" } else { font.name.as_str() };
        let italic = font.weight.is_italic();
        let weight = font.weight.to_win32_weight();
        let bold_weight = weight.max(FW_SEMIBOLD);
        self.normal = create_font(name, font.size, weight, italic);
        self.small = create_font(name, font.size - 1, weight, italic);
        self.bold = create_font(name, font.size, bold_weight, italic);
        self.small_bold = create_font(name, font.size - 2, bold_weight, italic);
        if self.normal.is_none() || self.small.is_none() || self.bold.is_none() || self.small_bold.is_none() {
            SLOG.err(format_args!("Failed to create the panel font '{name}'"));
            self.release();
            return false;
        }
        self.key = Some(font.clone());
        true
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        self.release();
    }
}

/// How often global.settings.json is checked for a theme change made in the settings window.
const THEME_CHECK_MS: u64 = 1000;

/// The panels' colors: the theme picked in the settings window, re-read when global.settings.json changes (so a
/// theme change in Preferences recolors the panels live, as on Linux).
fn panel_palette() -> UiPalette {
    let p = panels();
    if let Some((palette, _, checked)) = p.theme {
        if Ticks::now().elapsed_since(checked) < THEME_CHECK_MS {
            return palette;
        }
    }
    let mtime = std::fs::metadata(eve_maj_core::config::GLOBAL_SETTINGS_FILE).and_then(|m| m.modified()).ok();
    match p.theme {
        Some((palette, seen, _)) if seen == mtime => {
            p.theme = Some((palette, seen, Ticks::now()));
            palette
        }
        _ => {
            let first = p.theme.is_none();
            let theme = match (first, globals::GLOBAL_SETTINGS.get()) {
                (true, Some(gs)) => gs.ui_theme.clone(),
                _ => eve_maj_core::config::GlobalSettings::load().ui_theme,
            };
            let palette = UiPalette::for_theme(&theme);
            if !first {
                SLOG.info(format_args!("Settings theme changed to {:?}, recoloring the panels", if theme.id.is_empty() { "default" } else { &theme.id }));
            }
            if let Some(gs) = globals::GLOBAL_SETTINGS.get() {
                gs.ui_theme = theme;
            }
            p.theme = Some((palette, mtime, Ticks::now()));
            palette
        }
    }
}

impl Panels {
    fn slot(&mut self, kind: Kind) -> &mut Option<Panel> {
        match kind {
            Kind::List => &mut self.list,
            Kind::History => &mut self.history_panel,
        }
    }

    fn kind_of(&self, hwnd: HWND) -> Option<Kind> {
        if self.list.as_ref().is_some_and(|p| p.hwnd == hwnd) {
            Some(Kind::List)
        } else if self.history_panel.as_ref().is_some_and(|p| p.hwnd == hwnd) {
            Some(Kind::History)
        } else {
            None
        }
    }
}

static PANELS: Global<Panels> = Global::new();

fn panels() -> &'static mut Panels {
    if !PANELS.is_set() {
        PANELS.set(Panels::default());
    }
    PANELS.get().expect("set above")
}

// ------------------------------------------------------------------------------------------------ public API --

/// Records a notification that was shown (called by the painter's show_notification).
pub fn push_history(source: HWND, character_name: &str, character_color: Option<u32>, text: &str, ntype: NotificationType) {
    panels().history.push(source, character_name, character_color, text, ntype, now_ms());
}

/// The tray's "Clear Notification History".
pub fn clear_history() {
    // Repainted on the next tick; this runs inside a painter call, so it doesn't reach back into the painter.
    panels().history.clear();
    SLOG.info(format_args!("Notification history cleared"));
}

fn any_logged_in() -> bool {
    globals::painter().is_some_and(|p| p.thumbnails.iter().any(|t| !is_generic_character_name(&t.character_name)))
}

/// The open panel windows, for the painter's topmost reassert.
pub fn hwnds() -> Vec<HWND> {
    PANELS.get().map(|p| [&p.list, &p.history_panel].into_iter().flatten().map(|panel| panel.hwnd).collect()).unwrap_or_default()
}

pub fn history_visible() -> bool {
    panels().visibility.is_visible(&globals::config().display, any_logged_in())
}

/// The tray's "Show History Panel": toggled and saved to the profile.
pub fn toggle_history() {
    let config = globals::config();
    panels().visibility.toggle(&mut config.display, any_logged_in());
    SLOG.info(format_args!("History Panel {}", if config.display.show_notif_info_panel { "shown" } else { "hidden" }));
    if let Err(err) = config.save_current_profile() {
        SLOG.err(format_args!("Failed to save profile after toggling the History Panel: {err}"));
    }
    update();
}

/// Destroys both panel windows (before the app's windows go away).
pub fn shutdown() {
    if let Some(mut p) = PANELS.take() {
        for slot in [p.list.take(), p.history_panel.take()] {
            if let Some(panel) = slot {
                unsafe { DestroyWindow(panel.hwnd) };
            }
        }
    }
}

/// Per timer tick: brings both panels in line with the config and the tracked clients. Cheap when nothing changed.
pub fn update() {
    let Some(painter) = globals::painter() else { return };
    let list = list_content(painter);
    place(Kind::List, list);
    let any = painter.thumbnails.iter().any(|t| !is_generic_character_name(&t.character_name));
    let p = panels();
    p.visibility.update(any);
    let history = history_visible().then(history_content);
    place(Kind::History, history);
    redraw(Kind::List);
    redraw(Kind::History);
}

// ---------------------------------------------------------------------------------------------- content --

/// What a panel should show, where it's configured to be, and its size.
type Wanted = (Content, (i32, i32), (i32, i32));

fn metrics_of(t: &crate::painter::ThumbnailWindow) -> Metrics {
    Metrics {
        incoming_dps: t.last_incoming_dps,
        outgoing_dps: t.last_outgoing_dps,
        mining_rate: t.last_mining_rate,
        mining_isk_rate: t.last_mining_isk_rate,
        bounty_isk_rate: t.last_bounty_isk_rate,
        cpu_percent: t.last_cpu_percent,
        ram_mb: t.last_ram_mb,
        vram_mb: t.last_vram_mb,
        has_dps_data: t.has_dps_data,
        has_mining_data: t.has_mining_data,
        has_bounty_data: t.has_bounty_data,
        has_resource_data: t.has_resource_data,
        has_vram_data: t.has_vram_data,
    }
}

/// The painter's newest notification in core's form (only its type and text decide a list row).
fn core_notification(n: &crate::painter::ActiveNotification) -> render::ActiveNotification {
    let now = now_ms();
    render::ActiveNotification {
        text: n.text.clone(),
        notification_type: n.notification_type,
        start_ms: now.saturating_sub(Ticks::now().elapsed_since(n.start_time)),
        duration_ms: n.duration_ms,
        suppress_when_focused: n.suppress_when_focused,
        suppress_when_clicked: n.suppress_when_clicked,
        border_color_override: n.border_color_override,
        text_color_override: n.text_color_override,
        show_border: n.show_border,
        flash_border: n.flash_border,
    }
}

fn list_content(painter: &Painter) -> Option<Wanted> {
    let config = globals::config();
    let display = &config.display;
    if display.view_mode != ViewMode::ClientList {
        return None;
    }
    // Per-character hidden clients get no row; with every client hidden (visibility hotkey, auto-hide), no list.
    let shown: Vec<usize> = (0..painter.thumbnails.len()).filter(|&i| !painter.thumbnails[i].cached_hide_thumbnail).collect();
    // Region select's temporary hide counts as visible, so the list isn't torn down (and its dragged spot lost) meanwhile.
    let visible = |i: usize| {
        let t = &painter.thumbnails[i];
        t.visibility_state == VisibilityState::Visible || painter.region_select_hidden_hwnds.contains(&t.hwnd)
    };
    if shown.is_empty() || !shown.iter().any(|&i| visible(i)) {
        return None;
    }
    let active = painter.active_source_hwnd;
    let metrics: Vec<Metrics> = shown.iter().map(|&i| metrics_of(&painter.thumbnails[i])).collect();
    let newest: Vec<Option<render::ActiveNotification>> = shown.iter().map(|&i| painter.thumbnails[i].active_notifications.first().map(core_notification)).collect();
    let items: Vec<ListItem<'_>> = shown
        .iter()
        .enumerate()
        .map(|(k, &i)| {
            let t = &painter.thumbnails[i];
            ListItem {
                character_name: &t.character_name,
                display_name: &t.cached_display_name,
                state: t.effective_render_state(active),
                excluded: t.is_excluded_from_cycle,
                newest_notification: newest[k].as_ref(),
                system_name: &t.system_name,
                system_color: t.cached_system_color,
                character_color: t.cached_character_color,
                active_border_override: t.cached_border_colors.and_then(|b| b.active_border_color),
                metrics: &metrics[k],
            }
        })
        .collect();
    let rows = model::list_rows(config, &items);
    panels().list_sources = rows.iter().map(|r| painter.thumbnails[shown[r.source]].source_hwnd).collect();
    let size = model::list_size(display.list_view_columns, rows.len());
    let content = Content::List(ListContent {
        palette: panel_palette(),
        rows,
        columns: display.list_view_columns,
        opacity: display.list_view_opacity,
        font: Font { name: display.list_view_font_name.clone(), size: display.list_view_font_size, weight: display.list_view_font_weight },
    });
    Some((content, (display.start_x, display.start_y), size))
}

fn history_content() -> Wanted {
    let palette = panel_palette();
    let config = globals::config();
    let display = &config.display;
    let p = panels();
    let size = (display.notif_info_panel_width.max(1), display.notif_info_panel_height.max(1));
    let rows = model::history_rows(config, &p.history, size.1, now_ms());
    p.history_sources = rows.iter().map(|r| r.source).collect();
    let content = Content::History(HistoryContent {
        palette,
        rows,
        empty_text: model::history_empty_text(p.history.len()),
        opacity: display.notif_info_panel_opacity,
        font: Font { name: display.notif_info_panel_font_name.clone(), size: display.notif_info_panel_font_size, weight: display.notif_info_panel_font_weight },
        show_filters: display.notif_info_panel_show_category_filters,
        enabled: model::CATEGORIES.map(|(c, _)| model::category_enabled(display, c)),
        footer_top: model::history_footer_top(display, size.1),
    });
    (content, (display.notif_info_panel_x, display.notif_info_panel_y), size)
}

// -------------------------------------------------------------------------------------------- placement --

/// A configured position pulled onto the nearest monitor, so a panel saved on an unplugged display stays reachable.
fn on_screen((x, y): (i32, i32), (w, h): (i32, i32)) -> (i32, i32) {
    let monitor = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    match eve_maj_win::geometry::monitor_rect(monitor) {
        Some(b) => (x.clamp(b.left, (b.right - w).max(b.left)), y.clamp(b.top, (b.bottom - h).max(b.top))),
        None => (x, y),
    }
}

fn ensure_class() -> bool {
    static REGISTERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if REGISTERED.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    let ok = gdi_overlay::register_window_class(instance, Some(window_proc), CLASS_NAME, std::ptr::null_mut());
    REGISTERED.store(ok, std::sync::atomic::Ordering::Relaxed);
    ok
}

fn place(kind: Kind, want: Option<Wanted>) {
    let p = panels();
    let Some((content, config_pos, size)) = want else {
        if let Some(panel) = p.slot(kind).take() {
            SLOG.debug(format_args!("{kind:?} panel hidden"));
            unsafe { DestroyWindow(panel.hwnd) };
        }
        if p.dragging == Some(kind) {
            p.dragging = None;
        }
        return;
    };
    let dragging = p.dragging == Some(kind);
    if p.slot(kind).is_none() {
        if !ensure_class() {
            SLOG.err(format_args!("Failed to register the panel window class"));
            return;
        }
        let pos = on_screen(config_pos, size);
        let class_w = wide(CLASS_NAME);
        let title_w = wide(match kind {
            Kind::List => "EVE-Maj Client List",
            Kind::History => "EVE-Maj Notification History",
        });
        let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class_w.as_ptr(),
                title_w.as_ptr(),
                WS_POPUP,
                pos.0,
                pos.1,
                size.0,
                size.1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            SLOG.err(format_args!("Failed to create the {kind:?} panel window"));
            return;
        }
        SLOG.info(format_args!("{kind:?} panel shown at ({}, {}), {}x{}", pos.0, pos.1, size.0, size.1));
        *p.slot(kind) = Some(Panel { hwnd, bitmap: None, pos, size, config_pos, content: None, shown: None });
    }
    let panel = p.slot(kind).as_mut().expect("created above");
    let mut pos = panel.pos;
    // Keep a dragged (or dragged-but-not-remembered) panel where it is until the configured position changes.
    if !dragging && panel.config_pos != config_pos {
        pos = on_screen(config_pos, size);
    }
    panel.config_pos = config_pos;
    if (pos != panel.pos || size != panel.size) && !dragging {
        panel.pos = pos;
        panel.size = size;
        panel.shown = None;
        unsafe { SetWindowPos(panel.hwnd, HWND_TOPMOST, pos.0, pos.1, size.0, size.1, SWP_NOACTIVATE) };
    }
    panel.content = Some(content);
}

fn redraw(kind: Kind) {
    let p = panels();
    let Some(content) = p.slot(kind).as_ref().and_then(|panel| panel.content.clone()) else { return };
    if p.slot(kind).as_ref().is_some_and(|panel| panel.shown.as_ref() == Some(&content)) {
        return;
    }
    let font_spec = content.font().clone();
    let slot = match kind {
        Kind::List => 0,
        Kind::History => 1,
    };
    if !p.fonts[slot].ensure(&font_spec) {
        return;
    }
    let fonts = &p.fonts[slot];
    let fonts = PanelFonts {
        normal: fonts.normal.expect("ensured"),
        small: fonts.small.expect("ensured"),
        bold: fonts.bold.expect("ensured"),
        small_bold: fonts.small_bold.expect("ensured"),
    };
    let Some(panel) = p.slot(kind).as_mut() else { return };
    let (w, h) = panel.size;
    if OverlayBitmap::needs_resize(&panel.bitmap, w, h) {
        let dc = unsafe { GetDC(std::ptr::null_mut()) };
        let ok = OverlayBitmap::recreate(&mut panel.bitmap, dc, w, h);
        unsafe { ReleaseDC(std::ptr::null_mut(), dc) };
        if !ok {
            SLOG.err(format_args!("Can't allocate a {w}x{h} panel bitmap"));
            return;
        }
    }
    let bmp = panel.bitmap.as_mut().expect("allocated above");
    let old = unsafe { SelectObject(bmp.mem_dc, fonts.normal) };
    bmp.pixels().fill(0);
    paint(bmp, &content, &fonts, w, h);
    unsafe { SelectObject(bmp.mem_dc, old) };
    // The whole panel fades with its opacity.
    gdi_overlay::present_layered(panel.hwnd, bmp, content.opacity());
    unsafe { ShowWindow(panel.hwnd, SW_SHOWNOACTIVATE) };
    panel.shown = Some(content);
}

// ------------------------------------------------------------------------------------------------- input --

fn save_position(kind: Kind) {
    let p = panels();
    let Some(panel) = p.slot(kind).as_mut() else { return };
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    unsafe { GetWindowRect(panel.hwnd, &mut r) };
    panel.pos = (r.left, r.top);
    let config = globals::config();
    let pos = Position { x: r.left, y: r.top };
    let saved = match kind {
        Kind::List if config.display.remember_list_view_position => Some(config.save_list_view_position(pos)),
        Kind::History if config.display.remember_notif_info_panel_position => Some(config.save_notif_info_panel_position(pos)),
        _ => None,
    };
    match saved {
        Some(Err(err)) => SLOG.err(format_args!("Failed to save the {kind:?} panel position: {err}")),
        // The saved position is where it is now; don't jump.
        Some(Ok(())) => {
            panel.config_pos = (r.left, r.top);
            SLOG.info(format_args!("Saved {kind:?} panel position ({}, {})", r.left, r.top));
        }
        None => {}
    }
}

fn click(kind: Kind, x: i32, y: i32) {
    let config = globals::config();
    match kind {
        Kind::List => {
            let sources = panels().list_sources.clone();
            let Some(row) = model::list_row_at(config.display.list_view_columns, sources.len(), x, y) else { return };
            let source = sources[row];
            if is_shift_pressed() {
                input::handle_thumbnail_shift_click(source);
            } else {
                input::handle_thumbnail_click(source);
            }
        }
        Kind::History => {
            let Some(size) = panels().history_panel.as_ref().map(|p| p.size) else { return };
            let footer_top = model::history_footer_top(&config.display, size.1);
            if config.display.notif_info_panel_show_category_filters && y >= footer_top {
                let spans = model::history_button_spans(size.0);
                if let Some(i) = spans.iter().position(|&(l, r)| x >= l && x < r) {
                    let category = model::CATEGORIES[i].0;
                    let on = model::category_enabled(&config.display, category);
                    model::set_category_enabled(&mut config.display, category, !on);
                    if let Err(err) = config.save_notif_info_panel_category_filter() {
                        SLOG.err(format_args!("Failed to save the History Panel's category filter: {err}"));
                    }
                    update();
                }
                return;
            }
            // The header drags the panel (or does nothing with dragging off); it never picks a row.
            if y < model::HISTORY_HEADER_HEIGHT {
                return;
            }
            let row = ((y - model::HISTORY_HEADER_HEIGHT) / model::HISTORY_ROW_HEIGHT) as usize;
            let Some(&source) = panels().history_sources.get(row) else { return };
            input::handle_thumbnail_click(source);
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let kind = PANELS.get().and_then(|p| p.kind_of(hwnd));
    match (msg, kind) {
        (WM_NCHITTEST, Some(kind)) => {
            let header = match kind {
                Kind::List => model::LIST_HEADER_HEIGHT,
                Kind::History => model::HISTORY_HEADER_HEIGHT,
            };
            gdi_overlay::panel_header_hit_test(hwnd, lparam, header, globals::config().interaction.enable_dragging)
        }
        // Clicks never take focus from the EVE client.
        (WM_MOUSEACTIVATE, _) => MA_NOACTIVATE as LRESULT,
        (WM_ENTERSIZEMOVE, Some(kind)) => {
            gdi_overlay::begin_panel_drag(hwnd);
            panels().dragging = Some(kind);
            0
        }
        (WM_MOVING, Some(_)) => {
            gdi_overlay::update_panel_drag_rect(hwnd, &mut *(lparam as *mut RECT));
            TRUE_RESULT
        }
        (WM_EXITSIZEMOVE, Some(kind)) => {
            panels().dragging = None;
            save_position(kind);
            0
        }
        (WM_LBUTTONDOWN, Some(kind)) => {
            let x = (lparam & 0xFFFF) as u16 as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
            click(kind, x, y);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

const TRUE_RESULT: LRESULT = 1;

// --------------------------------------------------------------------------------------------------- drawing --
//
// Drawn like the Linux panels (crates/preview/src/linux/panels.rs): a card in the settings window's theme colors
// with a semibold header, pill highlights for the active and alerting clients, and filter chips. Colors the profile
// sets itself (character colors, overlay stat colors, notification colors) are kept; the model's stock defaults
// become theme roles. Shapes are anti-aliased in software (GDI's own aren't); text is GDI's.

const CORNER: f32 = 12.0;

/// The fonts one paint uses (see [`Fonts`]).
struct PanelFonts {
    normal: HFONT,
    small: HFONT,
    bold: HFONT,
    small_bold: HFONT,
}

/// Premultiplied 0xAARRGGBB from 0xRRGGBB and an alpha.
fn argb(color: u32, alpha: u32) -> u32 {
    let ch = |shift: u32| (((color >> shift) & 0xFF) * alpha / 255) << shift;
    (alpha << 24) | ch(16) | ch(8) | ch(0)
}

/// `color` at `alpha` (0..=1) composited over the pixel (both premultiplied).
fn blend(p: &mut u32, color: u32, alpha: f32) {
    let a = (alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    if a == 0 {
        return;
    }
    let src = argb(color, a);
    if a == 255 {
        *p = src;
        return;
    }
    let dst = *p;
    let inv = 255 - a;
    let ch = |shift: u32| ((((src >> shift) & 0xFF) + ((dst >> shift) & 0xFF) * inv / 255).min(255)) << shift;
    *p = ch(24) | ch(16) | ch(8) | ch(0);
}

/// Paints `color` over the pixels in the bounds, each at `alpha` times the coverage `cover` gives its centre.
fn shade(bmp: &mut OverlayBitmap, bounds: (f32, f32, f32, f32), color: u32, alpha: f32, cover: impl Fn(f32, f32) -> f32) {
    let (bw, bh) = (bmp.width as i32, bmp.height as i32);
    let x0 = (bounds.0.floor() as i32 - 1).max(0);
    let y0 = (bounds.1.floor() as i32 - 1).max(0);
    let x1 = (bounds.2.ceil() as i32 + 1).min(bw);
    let y1 = (bounds.3.ceil() as i32 + 1).min(bh);
    let stride = bw as usize;
    let pixels = bmp.pixels();
    for y in y0..y1 {
        for x in x0..x1 {
            let c = cover(x as f32 + 0.5, y as f32 + 0.5);
            if c > 0.0 {
                blend(&mut pixels[y as usize * stride + x as usize], color, alpha * c.min(1.0));
            }
        }
    }
}

/// Signed distance from (px, py) to a rounded rect (negative inside).
fn rounded_distance(px: f32, py: f32, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let qx = (px - cx).abs() - (w / 2.0 - r);
    let qy = (py - cy).abs() - (h / 2.0 - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

fn coverage(distance: f32) -> f32 {
    (0.5 - distance).clamp(0.0, 1.0)
}

#[allow(clippy::too_many_arguments)]
fn fill_rounded(bmp: &mut OverlayBitmap, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32, alpha: f32) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    shade(bmp, (x, y, x + w, y + h), color, alpha, |px, py| coverage(rounded_distance(px, py, x, y, w, h, r)));
}

/// A 1px outline just inside the rounded rect.
fn stroke_rounded(bmp: &mut OverlayBitmap, x: f32, y: f32, w: f32, h: f32, r: f32, color: u32) {
    if w <= 2.0 || h <= 2.0 {
        return;
    }
    shade(bmp, (x, y, x + w, y + h), color, 1.0, |px, py| {
        let outer = coverage(rounded_distance(px, py, x, y, w, h, r));
        let inner = coverage(rounded_distance(px, py, x + 1.0, y + 1.0, w - 2.0, h - 2.0, (r - 1.0).max(0.0)));
        (outer - inner).max(0.0)
    });
}

fn fill_circle(bmp: &mut OverlayBitmap, cx: f32, cy: f32, r: f32, color: u32) {
    shade(bmp, (cx - r, cy - r, cx + r, cy + r), color, 1.0, |px, py| coverage(((px - cx).powi(2) + (py - cy).powi(2)).sqrt() - r));
}

/// A ring of `width` centred on radius `r`.
fn ring(bmp: &mut OverlayBitmap, cx: f32, cy: f32, r: f32, width: f32, color: u32) {
    let outer = r + width / 2.0;
    shade(bmp, (cx - outer, cy - outer, cx + outer, cy + outer), color, 1.0, |px, py| {
        let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
        (width / 2.0 + 0.5 - (d - r).abs()).clamp(0.0, 1.0)
    });
}

/// A line of `width` from (x0, y0) to (x1, y1), with butt-ish ends.
#[allow(clippy::too_many_arguments)]
fn line(bmp: &mut OverlayBitmap, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: u32) {
    let (dx, dy) = (x1 - x0, y1 - y0);
    let len2 = (dx * dx + dy * dy).max(f32::EPSILON);
    let pad = width;
    shade(bmp, (x0.min(x1) - pad, y0.min(y1) - pad, x0.max(x1) + pad, y0.max(y1) + pad), color, 1.0, |px, py| {
        let t = (((px - x0) * dx + (py - y0) * dy) / len2).clamp(0.0, 1.0);
        let d = ((px - x0 - t * dx).powi(2) + (py - y0 - t * dy).powi(2)).sqrt();
        (width / 2.0 + 0.5 - d).clamp(0.0, 1.0)
    });
}

fn fill(bmp: &mut OverlayBitmap, x: i32, y: i32, w: i32, h: i32, color: u32) {
    let (bw, bh) = (bmp.width as i32, bmp.height as i32);
    let (x0, y0, x1, y1) = (x.max(0), y.max(0), (x + w).min(bw), (y + h).min(bh));
    if x1 > x0 && y1 > y0 {
        gdi_overlay::fill_rect(bmp.pixels(), bw as usize, bh as usize, x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize, argb(color, 255));
    }
}

fn use_font(bmp: &OverlayBitmap, font: HFONT) {
    unsafe { SelectObject(bmp.mem_dc, font) };
}

fn width_of(bmp: &OverlayBitmap, text: &str) -> i32 {
    gdi_overlay::measure_text_width(TEXT_BUF, bmp.mem_dc, text) as i32
}

fn size_of(bmp: &OverlayBitmap, text: &str) -> (i32, i32) {
    let s = gdi_overlay::measure_text_size(TEXT_BUF, bmp.mem_dc, text);
    (s.cx, s.cy)
}

/// `text` shortened with "..." to fit `max_w` (unchanged when it fits).
fn fit(bmp: &OverlayBitmap, text: &str, max_w: i32) -> String {
    if width_of(bmp, text) <= max_w {
        return text.to_owned();
    }
    gdi_overlay::truncate_text_to_fit(TEXT_BUF, bmp.mem_dc, text, max_w.max(0) as usize)
}

fn show(bmp: &mut OverlayBitmap, text: &str, x: i32, y: i32, color: u32) {
    if text.is_empty() {
        return;
    }
    let size = gdi_overlay::measure_text_size(TEXT_BUF, bmp.mem_dc, text);
    gdi_overlay::draw_text(TEXT_BUF, bmp.mem_dc, x, y, text, color | 0xFF00_0000);
    let (bw, bh) = (bmp.width, bmp.height);
    gdi_overlay::fix_text_alpha_rect(bmp.pixels(), bw, bh, x, y, size.cx.max(0) as usize, size.cy.max(0) as usize);
}

/// `text` vertically centred in the band from `top` of `height`.
fn show_centered(bmp: &mut OverlayBitmap, text: &str, x: i32, top: i32, height: i32, color: u32) {
    let (_, th) = size_of(bmp, text);
    show(bmp, text, x, top + ((height - th) / 2).max(0), color);
}

/// A stock model color replaced by a theme role; anything the profile chose is kept.
fn themed(color: u32, stock: u32, role: u32) -> u32 {
    if color & 0xFF_FFFF == stock { role } else { color & 0xFF_FFFF }
}

/// The overlays' stock stat colors (incoming red, outgoing green, mining blue, bounty gold) at the M3 tone the
/// panel's other text uses; colors the profile set itself are kept.
fn stat_tone(color: u32) -> u32 {
    match color {
        0xFF4444 => ui_theme::ERROR,
        0x44FF44 => ui_theme::SUCCESS,
        0x44AAFF => 0x9ECAFF,
        0xFFD700 => 0xF2C94C,
        c => c,
    }
}

/// The notification border color behind a model alert tint (which mixes it into a dark grey), close enough to tint
/// the pill with.
fn alert_tint_source(bg: u32) -> u32 {
    let up = |v: u32, base: u32| (v.saturating_sub(base) * 255 / 35).min(255);
    (up((bg >> 16) & 0xFF, 0x1A) << 16) | (up((bg >> 8) & 0xFF, 0x1A) << 8) | up(bg & 0xFF, 0x20)
}

/// A small rounded count chip ending at `right`, centred in the header; returns its left edge. Uses the selected font.
fn count_chip(bmp: &mut OverlayBitmap, text: &str, right: i32, header: i32, pal: &UiPalette) -> i32 {
    let (tw, th) = size_of(bmp, text);
    let (w, h) = (tw + 16, th + 4);
    let (x, y) = (right - w, (header - h) / 2);
    fill_rounded(bmp, x as f32, y as f32, w as f32, h as f32, h as f32 / 2.0, pal.secondary_container, 1.0);
    show(bmp, text, x + 8, y + 2, pal.on_secondary_container);
    x
}

fn paint(bmp: &mut OverlayBitmap, content: &Content, fonts: &PanelFonts, w: i32, h: i32) {
    let pal = match content {
        Content::List(c) => c.palette,
        Content::History(c) => c.palette,
    };
    let (wf, hf) = (w as f32, h as f32);
    // The card. Everything inside is inset from its rounded corners, so nothing needs clipping to them.
    fill_rounded(bmp, 0.0, 0.0, wf, hf, CORNER, pal.surface_container_low, 1.0);
    match content {
        Content::List(c) => paint_list(bmp, c, fonts, w),
        Content::History(c) => paint_history(bmp, c, fonts, w, h),
    }
    stroke_rounded(bmp, 0.0, 0.0, wf, hf, CORNER, pal.outline_variant);
}

fn paint_list(bmp: &mut OverlayBitmap, c: &ListContent, fonts: &PanelFonts, w: i32) {
    use model::*;
    let pal = &c.palette;
    let n = c.rows.len() as i32;
    let columns = effective_columns(c.columns, c.rows.len());

    // Header: title on the left, the client count in a chip on the right.
    use_font(bmp, fonts.small_bold);
    let chip_left = count_chip(bmp, &n.to_string(), w - LIST_RIGHT_MARGIN, LIST_HEADER_HEIGHT, pal);
    use_font(bmp, fonts.bold);
    let title = fit(bmp, "EVE-Maj Preview", chip_left - LIST_BADGE_LEFT - 8);
    show_centered(bmp, &title, LIST_BADGE_LEFT, 0, LIST_HEADER_HEIGHT, pal.on_surface);
    fill(bmp, 1, LIST_HEADER_HEIGHT - 1, w - 2, 1, pal.outline_variant);

    for (i, row) in c.rows.iter().enumerate() {
        let (r, col) = (i as i32 / columns, i as i32 % columns);
        let top = LIST_HEADER_HEIGHT + r * LIST_ROW_HEIGHT;
        let left = col * LIST_WIDTH;
        // The model marks an alerting row by its background, and the focused one by its badge color.
        let alert = row.background != LIST_ROW_BG;
        let active = !alert && matches!(row.badge, Badge::Dot(c) if c != BADGE_INACTIVE && c != BADGE_MINIMIZED && c != BADGE_ALERT);
        // Pill highlight, inset like the settings window's selected navigation item.
        let (px, py, pw, ph) = ((left + 6) as f32, (top + 3) as f32, (LIST_WIDTH - 12) as f32, (LIST_ROW_HEIGHT - 6) as f32);
        if active {
            fill_rounded(bmp, px, py, pw, ph, ph / 2.0, pal.secondary_container, 1.0);
        } else if alert {
            let tint = if row.background == LIST_ROW_ALERT_BG { ui_theme::ERROR_CONTAINER } else { alert_tint_source(row.background) };
            fill_rounded(bmp, px, py, pw, ph, ph / 2.0, tint, 0.3);
        }

        let (cx, cy) = ((left + LIST_BADGE_LEFT + LIST_BADGE_RADIUS) as f32 + 0.5, (top + LIST_ROW_HEIGHT / 2) as f32);
        let radius = LIST_BADGE_RADIUS as f32;
        match row.badge {
            // Not focused: a hollow ring (dimmer when minimized).
            Badge::Dot(color) if color == BADGE_INACTIVE || color == BADGE_MINIMIZED => {
                ring(bmp, cx, cy, radius - 0.5, 1.5, if color == BADGE_MINIMIZED { pal.outline_variant } else { pal.outline });
            }
            Badge::Dot(color) => {
                let color = match color {
                    BADGE_ACTIVE => pal.primary,
                    BADGE_ALERT => ui_theme::ERROR,
                    c => c,
                };
                fill_circle(bmp, cx, cy, radius, color);
            }
            Badge::Excluded => {
                line(bmp, cx - radius, cy - radius, cx + radius, cy + radius, 1.6, ui_theme::ERROR);
                line(bmp, cx - radius, cy + radius, cx + radius, cy - radius, 1.6, ui_theme::ERROR);
            }
        }

        let text_x = left + LIST_TEXT_LEFT;
        use_font(bmp, if active { fonts.bold } else { fonts.normal });
        let name_color = themed(row.name_color, LIST_NAME_TEXT, if active { pal.on_secondary_container } else { pal.on_surface });
        let name_max = LIST_WIDTH - LIST_TEXT_LEFT - LIST_RIGHT_MARGIN - LIST_NAME_RESERVE;
        let name = fit(bmp, &row.name, name_max);
        let name_w = width_of(bmp, &name);
        show_centered(bmp, &name, text_x, top, LIST_ROW_HEIGHT, name_color);
        if !row.right_text.is_empty() {
            // Right-aligned after the name, shortened with "..." rather than drawn over it.
            let right_edge = left + LIST_WIDTH - LIST_RIGHT_MARGIN - 4;
            let room = right_edge - (text_x + name_w + 10);
            use_font(bmp, fonts.small);
            let color = match row.right_color {
                LIST_NOTIFICATION_TEXT => pal.primary,
                LIST_SYSTEM_TEXT => pal.on_surface_variant,
                c => stat_tone(c),
            };
            if room >= 16 {
                let right = fit(bmp, &row.right_text, room);
                let rw = width_of(bmp, &right);
                show_centered(bmp, &right, right_edge - rw, top, LIST_ROW_HEIGHT, color);
            }
        }
    }

    for col in 1..columns {
        let rows = items_in_column(n, columns, col - 1).max(items_in_column(n, columns, col));
        let (y0, y1) = (LIST_HEADER_HEIGHT + 6, LIST_HEADER_HEIGHT + rows * LIST_ROW_HEIGHT - 6);
        fill(bmp, col * LIST_WIDTH, y0, 1, y1 - y0, pal.outline_variant);
    }
}

fn paint_history(bmp: &mut OverlayBitmap, c: &HistoryContent, fonts: &PanelFonts, w: i32, h: i32) {
    use model::*;
    let pal = &c.palette;

    use_font(bmp, fonts.bold);
    let title = fit(bmp, "Notifications", w - 2 * HISTORY_TEXT_LEFT);
    show_centered(bmp, &title, HISTORY_TEXT_LEFT, 0, HISTORY_HEADER_HEIGHT, pal.on_surface);
    fill(bmp, 1, HISTORY_HEADER_HEIGHT - 1, w - 2, 1, pal.outline_variant);

    let max_w = (w - HISTORY_TEXT_LEFT - HISTORY_RIGHT_MARGIN).max(0);
    for (i, row) in c.rows.iter().enumerate() {
        let top = HISTORY_HEADER_HEIGHT + 4 + i as i32 * HISTORY_ROW_HEIGHT;
        draw_history_row(bmp, fonts, pal, row, HISTORY_TEXT_LEFT, top, max_w);
    }
    if c.rows.is_empty() {
        use_font(bmp, fonts.normal);
        let text = fit(bmp, c.empty_text, max_w);
        show(bmp, &text, HISTORY_TEXT_LEFT, HISTORY_HEADER_HEIGHT + 10, pal.on_surface_muted);
    }

    if c.show_filters {
        fill(bmp, 1, c.footer_top, w - 2, 1, pal.outline_variant);
        let band = h - c.footer_top;
        use_font(bmp, fonts.small_bold);
        for (i, &(l, r)) in history_button_spans(w).iter().enumerate() {
            // Chips inset within each click span.
            let x = (l + 3) as f32 + if i == 0 { 5.0 } else { 0.0 };
            let cw = (r - l - 6) as f32 - if i == 0 || i == 4 { 5.0 } else { 0.0 };
            let ch = 24.0_f32.min(band as f32 - 8.0);
            let y = c.footer_top as f32 + (band as f32 - ch) / 2.0;
            let label = CATEGORIES[i].1;
            let (lw, lh) = size_of(bmp, label);
            let lx = (x + (cw - lw as f32) / 2.0).round() as i32;
            let ly = (y + (ch - lh as f32) / 2.0).round() as i32;
            if c.enabled[i] {
                fill_rounded(bmp, x, y, cw, ch, 8.0, pal.secondary_container, 1.0);
                show(bmp, label, lx, ly, pal.on_secondary_container);
            } else {
                stroke_rounded(bmp, x, y, cw, ch, 8.0, pal.outline_variant);
                show(bmp, label, lx, ly, pal.on_surface_muted);
            }
        }
    }
}

/// "Name  message", cut short to fit, with the timestamp (when shown) right-aligned and never dropped.
#[allow(clippy::too_many_arguments)]
fn draw_history_row(bmp: &mut OverlayBitmap, fonts: &PanelFonts, pal: &UiPalette, row: &HistoryRow<HWND>, x: i32, top: i32, max_w: i32) {
    let mut remaining = max_w;
    if let Some(ts) = &row.timestamp {
        use_font(bmp, fonts.small);
        let ts_w = width_of(bmp, ts).min(max_w);
        remaining = (max_w - ts_w - 10).max(0);
        show_centered(bmp, ts, x + max_w - ts_w, top, model::HISTORY_ROW_HEIGHT, pal.on_surface_muted);
    }
    let name_color = themed(row.name_color, model::HISTORY_NAME_TEXT, pal.on_surface);
    let text_color = themed(row.text_color, 0xFFFFFF, pal.on_surface_variant);
    use_font(bmp, fonts.bold);
    let name_w = width_of(bmp, &row.name);
    if name_w > remaining {
        let name = fit(bmp, &row.name, remaining);
        show_centered(bmp, &name, x, top, model::HISTORY_ROW_HEIGHT, name_color);
        return;
    }
    show_centered(bmp, &row.name, x, top, model::HISTORY_ROW_HEIGHT, name_color);
    let gap = 8;
    let room = remaining - name_w - gap;
    if room > 12 {
        use_font(bmp, fonts.normal);
        let text = fit(bmp, &row.text, room);
        show_centered(bmp, &text, x + name_w + gap, top, model::HISTORY_ROW_HEIGHT, text_color);
    }
}
