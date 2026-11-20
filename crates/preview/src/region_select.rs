//! The settings dialog's "drag to select a region" overlay on Windows: one dimmed, topmost layered window over the
//! whole virtual desktop where a rectangle is drawn, then moved or resized by its edges, and confirmed with
//! Save/Enter or dropped with Cancel/Esc/right-click. The decisions (hit testing, dragging, button placement) are
//! eve_maj_core::region_select, shared with the Linux overlay. The result goes back to config.exe through the
//! shared-memory mapping its pollRegionSelectResult reads (eve_maj_app::protocol::publish_region_select_result).

use eve_maj_core::log::Scope;
use eve_maj_core::protocol::{label_text, Rect, RegionSelectRequest, RegionSelectStatus};
use eve_maj_core::region_select::{self as region, intersects, Action, Button, Cursor, Hit, Release, TIMEOUT_MS};
use eve_maj_core::types::FontWeight;
use eve_maj_win::time::Ticks;
use eve_maj_win::wide;
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, RECT, TRUE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{DeleteObject, EnumDisplayMonitors, GetDC, GetMonitorInfoW, ReleaseDC, SelectObject, HDC, HFONT, HMONITOR, MONITORINFO};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE, VK_RETURN};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::gdi_overlay::{self, OverlayBitmap};
use crate::globals::{self, Global};

const SLOG: Scope = Scope::new("region-select");
const CLASS_NAME: &str = "EVEMajRegionSelect";
const FONT: &str = "Segoe UI";
const FONT_PX: i32 = 15;
const HANDLE: i32 = 8;
const BUTTON_PAD_X: i32 = 16;
/// Premultiplied 0xAARRGGBB.
const DIM: u32 = 0x7300_0000;
const ACCENT: (u32, u32, u32) = (61, 173, 232);

struct State {
    hwnd: HWND,
    request: RegionSelectRequest,
    /// The virtual desktop's origin; window pixels are desktop pixels offset by this.
    origin: (i32, i32),
    monitors: Vec<Rect>,
    rect: Option<Rect>,
    action: Action,
    pointer: (i32, i32),
    hover: Option<Button>,
    started: Ticks,
    save_label: String,
    cancel_label: String,
    hint_new: String,
    hint_edit: String,
    hint_confirm: String,
    save_w: i32,
    cancel_w: i32,
    font: HFONT,
    bold: HFONT,
    bitmap: Option<OverlayBitmap>,
    /// What was last painted (rect, hover), so a motion that changes nothing doesn't repaint the whole desktop.
    shown: Option<(Option<Rect>, Option<Button>)>,
    /// The area the last frame's moving parts covered (bitmap pixels); None until a full frame was sent.
    drawn: Option<Rect>,
}

static STATE: Global<State> = Global::new();

fn premul(r: u32, g: u32, b: u32, a: u32) -> u32 {
    (a << 24) | ((r * a / 255) << 16) | ((g * a / 255) << 8) | (b * a / 255)
}

unsafe extern "system" fn monitor_enum(monitor: HMONITOR, _: HDC, _: *mut RECT, lparam: LPARAM) -> BOOL {
    let out = &mut *(lparam as *mut Vec<Rect>);
    let mut info: MONITORINFO = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(monitor, &mut info) != 0 {
        let r = info.rcMonitor;
        out.push(Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom });
    }
    TRUE
}

fn monitors() -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    unsafe { EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(monitor_enum), &mut out as *mut Vec<Rect> as LPARAM) };
    out
}

/// Opens the overlay (replacing one already up, without a result: config.exe already reset it for this request).
pub fn start(request: &RegionSelectRequest) {
    if STATE.is_set() {
        SLOG.info(format_args!("Region selection restarted"));
        close();
    }
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    static REGISTERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !REGISTERED.load(std::sync::atomic::Ordering::Relaxed) {
        if !gdi_overlay::register_window_class(instance, Some(window_proc), CLASS_NAME, std::ptr::null_mut()) {
            SLOG.err(format_args!("Failed to register the region-select window class"));
            eve_maj_app::protocol::publish_region_select_result(RegionSelectStatus::Cancelled, RECT { left: 0, top: 0, right: 0, bottom: 0 });
            return;
        }
        REGISTERED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let (x, y, w, h) = unsafe { (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN), GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN)) };
    let monitors = monitors();
    if monitors.is_empty() || w <= 0 || h <= 0 {
        SLOG.warn(format_args!("No monitors to select a region on"));
        eve_maj_app::protocol::publish_region_select_result(RegionSelectStatus::Cancelled, RECT { left: 0, top: 0, right: 0, bottom: 0 });
        return;
    }

    let text = |b: &[u8]| String::from_utf8_lossy(label_text(b)).into_owned();
    let font = gdi_overlay::create_font(FONT, FONT_PX, FontWeight::Regular);
    let bold = gdi_overlay::create_font(FONT, FONT_PX, FontWeight::Bold);
    let (save_label, cancel_label) = (text(&request.labels.save), text(&request.labels.cancel));
    let (save_w, cancel_w) = unsafe {
        let dc = GetDC(std::ptr::null_mut());
        let old = SelectObject(dc, bold);
        let m = |t: &str| gdi_overlay::measure_text_width(64, dc, t) as i32 + 2 * BUTTON_PAD_X;
        let widths = (m(&save_label), m(&cancel_label));
        SelectObject(dc, old);
        ReleaseDC(std::ptr::null_mut(), dc);
        widths
    };

    let class_w = wide(CLASS_NAME);
    let title_w = wide("EVE-Maj Region Select");
    let hwnd = unsafe {
        CreateWindowExW(WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW, class_w.as_ptr(), title_w.as_ptr(), WS_POPUP, x, y, w, h, std::ptr::null_mut(), std::ptr::null_mut(), instance, std::ptr::null())
    };
    if hwnd.is_null() {
        SLOG.err(format_args!("Failed to create the region-select window"));
        unsafe {
            DeleteObject(font);
            DeleteObject(bold);
        }
        eve_maj_app::protocol::publish_region_select_result(RegionSelectStatus::Cancelled, RECT { left: 0, top: 0, right: 0, bottom: 0 });
        return;
    }

    let mut pointer = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut pointer) };
    STATE.set(State {
        hwnd,
        request: *request,
        origin: (x, y),
        monitors,
        rect: request.edit_region,
        action: Action::None,
        pointer: (pointer.x, pointer.y),
        hover: None,
        started: Ticks::now(),
        save_label,
        cancel_label,
        hint_new: text(&request.labels.hint_new),
        hint_edit: text(&request.labels.hint_edit),
        hint_confirm: text(&request.labels.hint_confirm),
        save_w,
        cancel_w,
        font,
        bold,
        bitmap: None,
        shown: None,
        drawn: None,
    });
    if request.hide_thumbnails {
        if let Some(p) = globals::painter() {
            p.hide_thumbnails_for_region_select();
        }
    }
    redraw();
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
    }
    SLOG.info(format_args!(
        "Region selection started ({}, {w}x{h} desktop at ({x}, {y}), {} monitor(s), hide thumbnails: {})",
        if request.edit_region.is_some() { "edit" } else { "new" },
        STATE.get().map_or(0, |s| s.monitors.len()),
        request.hide_thumbnails
    ));
}

/// Takes the overlay down and brings back thumbnails it hid; false if none was up.
fn close() -> bool {
    let Some(state) = STATE.take() else { return false };
    unsafe {
        ReleaseCapture();
        DestroyWindow(state.hwnd);
        DeleteObject(state.font);
        DeleteObject(state.bold);
    }
    if state.request.hide_thumbnails {
        if let Some(p) = globals::painter() {
            p.restore_thumbnails_after_region_select();
        }
    }
    true
}

fn finish(status: RegionSelectStatus, rect: Option<Rect>) {
    if !close() {
        return;
    }
    let r = rect.unwrap_or_default();
    // Same size rule as the Linux build and the dialog: confirmed() flags a too-small region.
    let status = match (status, rect) {
        (RegionSelectStatus::Success, Some(r)) => eve_maj_core::protocol::RegionSelectResult::confirmed(r).status,
        (s, _) => s,
    };
    SLOG.info(format_args!("Region selection finished: {status:?} {}x{} at ({}, {})", r.width(), r.height(), r.left, r.top));
    eve_maj_app::protocol::publish_region_select_result(status, RECT { left: r.left, top: r.top, right: r.right, bottom: r.bottom });
}

fn confirm() {
    let Some(rect) = STATE.get().and_then(|s| s.rect) else { return };
    finish(RegionSelectStatus::Success, Some(rect));
}

/// Per timer tick: the dialog stops polling after two minutes, so the overlay goes too.
pub fn tick() {
    if STATE.get().is_some_and(|s| Ticks::now().elapsed_since(s.started) > TIMEOUT_MS) {
        SLOG.info(format_args!("Region selection timed out"));
        finish(RegionSelectStatus::Cancelled, None);
    }
}

impl State {
    fn buttons(&self) -> Option<(Rect, Rect)> {
        region::button_rects(self.rect, self.save_w, self.cancel_w, region::buttons_monitor(self.rect, &self.monitors))
    }

    fn hit(&self) -> Hit {
        region::hit(self.rect, self.buttons(), self.pointer)
    }
}

fn desktop_point(lparam: LPARAM) -> (i32, i32) {
    let s = STATE.get().map_or((0, 0), |s| s.origin);
    let x = (lparam & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x + s.0, y + s.1)
}

fn cursor(c: Cursor) -> *const u16 {
    match c {
        Cursor::Pointer => IDC_HAND,
        Cursor::Move => IDC_SIZEALL,
        Cursor::Crosshair => IDC_CROSS,
        Cursor::NwseResize => IDC_SIZENWSE,
        Cursor::NeswResize => IDC_SIZENESW,
        Cursor::EwResize => IDC_SIZEWE,
        Cursor::NsResize => IDC_SIZENS,
    }
}

fn pointer_moved() {
    let Some(s) = STATE.get() else { return };
    s.rect = region::dragged(s.action, s.rect, s.pointer);
    let hit = s.hit();
    s.hover = region::hover(s.action, hit);
    unsafe { SetCursor(LoadCursorW(std::ptr::null_mut(), cursor(region::cursor_during(s.action, hit)))) };
    redraw();
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_SETCURSOR => {
            if let Some(s) = STATE.get() {
                SetCursor(LoadCursorW(std::ptr::null_mut(), cursor(region::cursor_during(s.action, s.hit()))));
                return TRUE as LRESULT;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_MOUSEMOVE => {
            if let Some(s) = STATE.get() {
                s.pointer = desktop_point(lparam);
                pointer_moved();
            }
            0
        }
        WM_LBUTTONDOWN => {
            if let Some(s) = STATE.get() {
                s.pointer = desktop_point(lparam);
                let hit = s.hit();
                s.action = region::press(s.rect, hit, s.pointer);
                SetCapture(hwnd);
                pointer_moved();
            }
            0
        }
        WM_LBUTTONUP => {
            ReleaseCapture();
            if let Some(s) = STATE.get() {
                s.pointer = desktop_point(lparam);
                let action = std::mem::replace(&mut s.action, Action::None);
                let rect = region::dragged(action, s.rect, s.pointer);
                match region::release(action, rect, s.hit(), s.pointer) {
                    Release::Confirm => confirm(),
                    Release::Cancel => finish(RegionSelectStatus::Cancelled, None),
                    Release::Rect(r) => {
                        s.rect = r;
                        pointer_moved();
                    }
                }
            }
            0
        }
        WM_RBUTTONDOWN => {
            finish(RegionSelectStatus::Cancelled, None);
            0
        }
        WM_KEYDOWN => {
            match wparam as u16 {
                VK_ESCAPE => finish(RegionSelectStatus::Cancelled, None),
                VK_RETURN => confirm(),
                _ => {}
            }
            0
        }
        // Alt+F4 or the window being closed some other way counts as Cancel.
        WM_CLOSE => {
            finish(RegionSelectStatus::Cancelled, None);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Draws text with `font` at desktop-relative (x, y) in bitmap space and makes its pixels opaque.
fn text(bmp: &mut OverlayBitmap, font: HFONT, x: i32, y: i32, s: &str, color: u32) -> (i32, i32) {
    let dc = bmp.mem_dc;
    let old = unsafe { SelectObject(dc, font) };
    let size = gdi_overlay::measure_text_size(256, dc, s);
    gdi_overlay::draw_text(256, dc, x, y, s, color);
    let (w, h) = (bmp.width, bmp.height);
    gdi_overlay::fix_text_alpha_rect(bmp.pixels(), w, h, x, y, size.cx.max(0) as usize, size.cy.max(0) as usize);
    unsafe { SelectObject(dc, old) };
    (size.cx, size.cy)
}

fn measure(bmp: &OverlayBitmap, font: HFONT, s: &str) -> (i32, i32) {
    let old = unsafe { SelectObject(bmp.mem_dc, font) };
    let size = gdi_overlay::measure_text_size(256, bmp.mem_dc, s);
    unsafe { SelectObject(bmp.mem_dc, old) };
    (size.cx, size.cy)
}

fn fill(bmp: &mut OverlayBitmap, x: i32, y: i32, w: i32, h: i32, argb: u32) {
    let (bw, bh) = (bmp.width as i32, bmp.height as i32);
    let (x0, y0, x1, y1) = (x.max(0), y.max(0), (x + w).min(bw), (y + h).min(bh));
    if x1 > x0 && y1 > y0 {
        gdi_overlay::fill_rect(bmp.pixels(), bw as usize, bh as usize, x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize, argb);
    }
}

fn rounded(bmp: &mut OverlayBitmap, x: i32, y: i32, w: i32, h: i32, radius: usize, argb: u32) {
    let (bw, bh) = (bmp.width as i32, bmp.height as i32);
    if x < 0 || y < 0 || x + w > bw || y + h > bh {
        return fill(bmp, x, y, w, h, argb);
    }
    gdi_overlay::fill_rounded_rect(bmp.pixels(), bw as usize, bh as usize, x as usize, y as usize, w.max(0) as usize, h.max(0) as usize, radius, argb);
}

fn union(a: Option<Rect>, b: Rect) -> Rect {
    match a {
        Some(a) => Rect { left: a.left.min(b.left), top: a.top.min(b.top), right: a.right.max(b.right), bottom: a.bottom.max(b.bottom) },
        None => b,
    }
}

fn redraw() {
    let Some(s) = STATE.get() else { return };
    let scene = (s.rect, s.hover);
    if s.shown == Some(scene) {
        return;
    }
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN)) };
    if OverlayBitmap::needs_resize(&s.bitmap, w, h) {
        let dc = unsafe { GetDC(std::ptr::null_mut()) };
        let ok = OverlayBitmap::recreate(&mut s.bitmap, dc, w, h);
        unsafe { ReleaseDC(std::ptr::null_mut(), dc) };
        if !ok {
            SLOG.err(format_args!("Can't allocate a {w}x{h} region-select bitmap"));
            return;
        }
        s.drawn = None;
    }
    let buttons = s.buttons();
    let (ox, oy) = s.origin;
    let (font, bold) = (s.font, s.bold);
    let (line1, line2) = (if s.rect.is_some() { s.hint_edit.clone() } else { s.hint_new.clone() }, s.hint_confirm.clone());
    let monitors = s.monitors.clone();
    let (rect, hover) = (s.rect, s.hover);
    let (save_label, cancel_label) = (s.save_label.clone(), s.cancel_label.clone());
    let previous = s.drawn;
    let bmp = s.bitmap.as_mut().expect("allocated above");
    let accent = premul(ACCENT.0, ACCENT.1, ACCENT.2, 255);

    // Where this frame's moving parts (rect, handles, size label, buttons) go, in bitmap pixels.
    let label = rect.map(|r| {
        let text = format!("{} \u{00d7} {}   ({}, {})", r.width(), r.height(), r.left, r.top);
        let (lw, lh) = measure(bmp, font, &text);
        let monitor_top = monitors.iter().find(|m| region::contains(**m, r.left, r.top)).map_or(oy, |m| m.top);
        let (x, y) = (r.left - ox, r.top - oy);
        let ly = if r.top - lh - 14 >= monitor_top { y - lh - 12 } else { y + 8 };
        (text, Rect { left: x, top: ly, right: x + lw + 12, bottom: ly + lh + 6 }, lh)
    });
    let mut moving: Option<Rect> = None;
    if let Some(r) = rect {
        let grow = HANDLE / 2 + 1;
        moving = Some(union(moving, Rect { left: r.left - ox - grow, top: r.top - oy - grow, right: r.right - ox + grow, bottom: r.bottom - oy + grow }));
    }
    if let Some((_, l, _)) = &label {
        moving = Some(union(moving, *l));
    }
    if let Some((save, cancel)) = buttons {
        for b in [save, cancel] {
            moving = Some(union(moving, Rect { left: b.left - ox, top: b.top - oy, right: b.right - ox, bottom: b.bottom - oy }));
        }
    }

    // The hint boxes, top centre of every monitor; they never move.
    let (w1, h1) = measure(bmp, font, &line1);
    let (w2, h2) = measure(bmp, font, &line2);
    let (tw, th) = (w1.max(w2), h1 + h2 + 2);
    let hints: Vec<Rect> = monitors
        .iter()
        .map(|m| {
            let (px, py) = (m.left - ox + (m.width() - tw) / 2 - 16, m.top - oy + 48);
            Rect { left: px, top: py, right: px + tw + 32, bottom: py + th + 20 }
        })
        .collect();

    // Only the part of the desktop that changed is repainted and sent to the window: everything the last frame's and
    // this frame's moving parts cover, plus any hint box they touch (it's redrawn whole). Repainting the whole
    // desktop on every mouse move would push tens of MB per move on multi-monitor setups.
    let full = Rect { left: 0, top: 0, right: bmp.width as i32, bottom: bmp.height as i32 };
    let mut dirty = match (previous, moving) {
        (None, _) => full,
        (Some(p), Some(m)) => union(Some(p), m),
        (Some(p), None) => p,
    };
    // A hint box whose text changed (new rect vs editing) is repainted too.
    if s.shown.is_some_and(|(r, _)| r.is_some() != rect.is_some()) {
        for hint in &hints {
            dirty = union(Some(dirty), *hint);
        }
    }
    for hint in &hints {
        if intersects(*hint, dirty) {
            dirty = union(Some(dirty), *hint);
        }
    }
    let dirty = Rect { left: dirty.left.max(0), top: dirty.top.max(0), right: dirty.right.min(full.right), bottom: dirty.bottom.min(full.bottom) };
    if dirty.width() <= 0 || dirty.height() <= 0 {
        s.shown = Some(scene);
        s.drawn = moving;
        return;
    }
    fill(bmp, dirty.left, dirty.top, dirty.width(), dirty.height(), DIM);

    if let Some(r) = rect {
        let (x, y, rw, rh) = (r.left - ox, r.top - oy, r.width(), r.height());
        // Barely tinted, but never fully transparent: a zero-alpha pixel would let clicks fall through the window.
        fill(bmp, x, y, rw, rh, premul(ACCENT.0, ACCENT.1, ACCENT.2, 20));
        let (bw, bh) = (bmp.width, bmp.height);
        gdi_overlay::draw_rect_outline(bmp.pixels(), bw, bh, x, y, rw.max(0) as usize, rh.max(0) as usize, 2, accent);
        for (hx, hy) in [(x, y), (x + rw / 2, y), (x + rw, y), (x, y + rh / 2), (x + rw, y + rh / 2), (x, y + rh), (x + rw / 2, y + rh), (x + rw, y + rh)] {
            fill(bmp, hx - HANDLE / 2, hy - HANDLE / 2, HANDLE, HANDLE, accent);
            fill(bmp, hx - HANDLE / 2 + 2, hy - HANDLE / 2 + 2, HANDLE - 4, HANDLE - 4, 0xFFFF_FFFF);
        }
    }
    // Size and position, above the top-left corner (inside it when there's no room).
    if let Some((text_s, l, _)) = &label {
        rounded(bmp, l.left, l.top, l.width(), l.height(), 4, premul(20, 20, 20, 217));
        text(bmp, font, l.left + 6, l.top + 3, text_s, 0xFFFF_FFFF);
    }

    for hint in hints.iter().filter(|h| intersects(**h, dirty)) {
        let (px, py) = (hint.left, hint.top);
        rounded(bmp, px, py, hint.width(), hint.height(), 8, premul(20, 20, 20, 224));
        text(bmp, font, px + 16 + (tw - w1) / 2, py + 10, &line1, 0xFFFF_FFFF);
        text(bmp, font, px + 16 + (tw - w2) / 2, py + 12 + h1, &line2, 0xFFFF_FFFF);
    }

    if let Some((save, cancel)) = buttons {
        for (b, r, label, primary) in [(Button::Save, save, &save_label, true), (Button::Cancel, cancel, &cancel_label, false)] {
            if !monitors.iter().any(|m| intersects(r, *m)) {
                continue;
            }
            let hovered = hover == Some(b);
            let color = match (primary, hovered) {
                (true, false) => accent,
                (true, true) => premul(73, 199, 250, 255),
                (false, false) => premul(38, 38, 38, 235),
                (false, true) => premul(71, 71, 71, 242),
            };
            let (x, y) = (r.left - ox, r.top - oy);
            rounded(bmp, x, y, r.width(), r.height(), 5, color);
            let (lw, lh) = measure(bmp, bold, label);
            text(bmp, bold, x + (r.width() - lw) / 2, y + (r.height() - lh) / 2, label, 0xFFFF_FFFF);
        }
    }
    let hwnd = s.hwnd;
    let dirty_rect = RECT { left: dirty.left, top: dirty.top, right: dirty.right, bottom: dirty.bottom };
    gdi_overlay::present_layered_dirty(hwnd, s.bitmap.as_ref().expect("allocated above"), 255, if previous.is_some() { Some(&dirty_rect) } else { None });
    s.shown = Some(scene);
    s.drawn = moving;
}
