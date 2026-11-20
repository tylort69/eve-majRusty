//! The geometry of the settings dialog's "drag to select a region" overlay, shared by the Linux and Windows front ends:
//! what a press at a point does (draw a new rectangle, move it, resize it by an edge, press Save/Cancel), how a drag
//! changes the rectangle, where the buttons go, and which cursor to show. Coordinates are desktop (layout) pixels.

use crate::protocol::Rect;

/// The dialog gives up polling after two minutes; the overlay goes away then too.
pub const TIMEOUT_MS: u64 = 120_000;
/// How far from an edge a press still grabs it.
pub const EDGE_GRAB: i32 = 8;
/// A drag shorter than this is a click, not a new rectangle.
pub const MIN_DRAG: i32 = 4;
pub const BUTTON_H: i32 = 32;
pub const BUTTON_GAP: i32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Save,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub left: bool,
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
}

/// What a press at a point would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Button(Button),
    Resize(Edges),
    Move,
    Draw,
}

/// The drag in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Drawing { anchor: (i32, i32), previous: Option<Rect> },
    Moving { grab: (i32, i32), orig: Rect },
    Resizing { edges: Edges, grab: (i32, i32), orig: Rect },
    Pressing(Button),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Pointer,
    Move,
    Crosshair,
    NwseResize,
    NeswResize,
    EwResize,
    NsResize,
}

pub fn contains(r: Rect, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

pub fn intersects(a: Rect, b: Rect) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

pub fn normalized((x0, y0): (i32, i32), (x1, y1): (i32, i32)) -> Rect {
    Rect { left: x0.min(x1), top: y0.min(y1), right: x0.max(x1), bottom: y0.max(y1) }
}

pub fn hit_rect(r: Rect, x: i32, y: i32) -> Hit {
    let in_x = x >= r.left - EDGE_GRAB && x <= r.right + EDGE_GRAB;
    let in_y = y >= r.top - EDGE_GRAB && y <= r.bottom + EDGE_GRAB;
    let edges = Edges {
        left: in_y && (x - r.left).abs() <= EDGE_GRAB,
        right: in_y && (x - r.right).abs() <= EDGE_GRAB,
        top: in_x && (y - r.top).abs() <= EDGE_GRAB,
        bottom: in_x && (y - r.bottom).abs() <= EDGE_GRAB,
    };
    // A rectangle thinner than the grab zone: prefer the far edges so it can still grow.
    let edges = Edges { left: edges.left && !edges.right, top: edges.top && !edges.bottom, ..edges };
    if edges != Edges::default() {
        Hit::Resize(edges)
    } else if contains(r, x, y) {
        Hit::Move
    } else {
        Hit::Draw
    }
}

/// `orig` with the grabbed edges moved by the pointer's travel; it flips over rather than inverting.
pub fn resized(orig: Rect, edges: Edges, (dx, dy): (i32, i32)) -> Rect {
    let mut r = orig;
    if edges.left {
        r.left += dx;
    }
    if edges.right {
        r.right += dx;
    }
    if edges.top {
        r.top += dy;
    }
    if edges.bottom {
        r.bottom += dy;
    }
    normalized((r.left, r.top), (r.right, r.bottom))
}

pub fn cursor_for(hit: Hit) -> Cursor {
    match hit {
        Hit::Button(_) => Cursor::Pointer,
        Hit::Move => Cursor::Move,
        Hit::Draw => Cursor::Crosshair,
        Hit::Resize(e) => match (e.left, e.top, e.right, e.bottom) {
            (true, true, _, _) | (_, _, true, true) => Cursor::NwseResize,
            (true, _, _, true) | (_, true, true, _) => Cursor::NeswResize,
            (true, _, _, _) | (_, _, true, _) => Cursor::EwResize,
            _ => Cursor::NsResize,
        },
    }
}

/// The cursor while `action` is under way, with `hit` under the pointer.
pub fn cursor_during(action: Action, hit: Hit) -> Cursor {
    match action {
        Action::None | Action::Pressing(_) => cursor_for(hit),
        Action::Drawing { .. } => Cursor::Crosshair,
        Action::Moving { .. } => Cursor::Move,
        Action::Resizing { edges, .. } => cursor_for(Hit::Resize(edges)),
    }
}

/// Save and Cancel, right-aligned under the rectangle, or inside its bottom edge when the monitor ends there.
pub fn button_rects(rect: Option<Rect>, save_w: i32, cancel_w: i32, monitor: Option<Rect>) -> Option<(Rect, Rect)> {
    let r = rect?;
    let width = save_w + BUTTON_GAP + cancel_w;
    let below = r.bottom + BUTTON_GAP;
    let top = match monitor {
        Some(m) if below + BUTTON_H > m.bottom => (r.bottom - BUTTON_GAP - BUTTON_H).max(r.top),
        _ => below,
    };
    let mut left = r.right - width;
    if let Some(m) = monitor {
        left = left.clamp(m.left, (m.right - width).max(m.left));
    }
    let save = Rect { left, top, right: left + save_w, bottom: top + BUTTON_H };
    let cancel = Rect { left: save.right + BUTTON_GAP, top, right: save.right + BUTTON_GAP + cancel_w, bottom: top + BUTTON_H };
    Some((save, cancel))
}

/// The monitor the buttons go on: the one holding the rectangle's bottom-right corner, else the nearest.
pub fn buttons_monitor(rect: Option<Rect>, monitors: &[Rect]) -> Option<Rect> {
    let r = rect?;
    let (x, y) = (r.right - 1, r.bottom - 1);
    monitors.iter().copied().find(|b| contains(*b, x, y)).or_else(|| {
        monitors.iter().copied().min_by_key(|b| {
            let dx = (b.left - x).max(0).max(x - b.right) as i64;
            let dy = (b.top - y).max(0).max(y - b.bottom) as i64;
            dx * dx + dy * dy
        })
    })
}

/// What a press at `(x, y)` does, buttons first.
pub fn hit(rect: Option<Rect>, buttons: Option<(Rect, Rect)>, (x, y): (i32, i32)) -> Hit {
    if let Some((save, cancel)) = buttons {
        if contains(save, x, y) {
            return Hit::Button(Button::Save);
        }
        if contains(cancel, x, y) {
            return Hit::Button(Button::Cancel);
        }
    }
    match rect {
        Some(r) => hit_rect(r, x, y),
        None => Hit::Draw,
    }
}

/// The action a left press starts.
pub fn press(rect: Option<Rect>, hit: Hit, p: (i32, i32)) -> Action {
    match (hit, rect) {
        (Hit::Button(b), _) => Action::Pressing(b),
        (Hit::Move, Some(orig)) => Action::Moving { grab: p, orig },
        (Hit::Resize(edges), Some(orig)) => Action::Resizing { edges, grab: p, orig },
        _ => Action::Drawing { anchor: p, previous: rect },
    }
}

/// The rectangle while `action` is under way with the pointer at `p`; `current` when nothing is being dragged.
pub fn dragged(action: Action, current: Option<Rect>, p: (i32, i32)) -> Option<Rect> {
    match action {
        Action::Drawing { anchor, .. } => Some(normalized(anchor, p)),
        Action::Moving { grab, orig } => {
            let (dx, dy) = (p.0 - grab.0, p.1 - grab.1);
            Some(Rect { left: orig.left + dx, top: orig.top + dy, right: orig.right + dx, bottom: orig.bottom + dy })
        }
        Action::Resizing { edges, grab, orig } => Some(resized(orig, edges, (p.0 - grab.0, p.1 - grab.1))),
        Action::Pressing(_) | Action::None => current,
    }
}

/// What a left release does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    Confirm,
    Cancel,
    /// The drag ended; this is the rectangle now (a too-short new drag puts the previous one back).
    Rect(Option<Rect>),
}

pub fn release(action: Action, rect: Option<Rect>, hit_now: Hit, p: (i32, i32)) -> Release {
    match action {
        Action::Pressing(b) if hit_now == Hit::Button(b) => match b {
            Button::Save => Release::Confirm,
            Button::Cancel => Release::Cancel,
        },
        Action::Drawing { anchor, previous } => {
            let (dx, dy) = (p.0 - anchor.0, p.1 - anchor.1);
            Release::Rect(if dx.abs() < MIN_DRAG || dy.abs() < MIN_DRAG { previous } else { rect })
        }
        _ => Release::Rect(rect),
    }
}

/// The hovered button, if any (none while dragging).
pub fn hover(action: Action, hit: Hit) -> Option<Button> {
    match (action, hit) {
        (Action::None | Action::Pressing(_), Hit::Button(b)) => Some(b),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect { left: 100, top: 100, right: 300, bottom: 200 };

    #[test]
    fn hits_edges_corners_inside_and_outside() {
        assert_eq!(hit_rect(R, 100, 150), Hit::Resize(Edges { left: true, ..Default::default() }));
        assert_eq!(hit_rect(R, 305, 150), Hit::Resize(Edges { right: true, ..Default::default() }));
        assert_eq!(hit_rect(R, 200, 96), Hit::Resize(Edges { top: true, ..Default::default() }));
        assert_eq!(hit_rect(R, 299, 199), Hit::Resize(Edges { right: true, bottom: true, ..Default::default() }));
        assert_eq!(hit_rect(R, 200, 150), Hit::Move);
        assert_eq!(hit_rect(R, 50, 150), Hit::Draw);
        assert_eq!(hit_rect(R, 200, 220), Hit::Draw);
        // Too thin to tell the edges apart: the far ones win.
        let thin = Rect { left: 100, top: 100, right: 104, bottom: 200 };
        assert_eq!(hit_rect(thin, 102, 150), Hit::Resize(Edges { right: true, ..Default::default() }));
    }

    #[test]
    fn resizing_moves_only_grabbed_edges_and_flips() {
        let e = Edges { right: true, bottom: true, ..Default::default() };
        assert_eq!(resized(R, e, (50, -20)), Rect { left: 100, top: 100, right: 350, bottom: 180 });
        let l = Edges { left: true, ..Default::default() };
        assert_eq!(resized(R, l, (250, 0)), Rect { left: 300, top: 100, right: 350, bottom: 200 });
    }

    #[test]
    fn cursors_match_edges() {
        let r = |l, t, rt, b| cursor_for(Hit::Resize(Edges { left: l, top: t, right: rt, bottom: b }));
        assert_eq!(r(true, true, false, false), Cursor::NwseResize);
        assert_eq!(r(false, true, true, false), Cursor::NeswResize);
        assert_eq!(r(false, false, true, false), Cursor::EwResize);
        assert_eq!(r(false, false, false, true), Cursor::NsResize);
    }

    #[test]
    fn a_short_new_drag_keeps_the_previous_rect() {
        let a = press(Some(R), Hit::Draw, (10, 10));
        assert_eq!(release(a, dragged(a, Some(R), (12, 40)), Hit::Draw, (12, 40)), Release::Rect(Some(R)));
        let drawn = dragged(a, Some(R), (60, 50));
        assert_eq!(release(a, drawn, Hit::Draw, (60, 50)), Release::Rect(Some(Rect { left: 10, top: 10, right: 60, bottom: 50 })));
    }

    #[test]
    fn buttons_sit_under_the_rect_or_inside_at_the_monitor_bottom() {
        let m = Rect { left: 0, top: 0, right: 1000, bottom: 800 };
        let (save, cancel) = button_rects(Some(R), 80, 90, Some(m)).unwrap();
        assert_eq!((save.top, cancel.right), (208, 300));
        let low = Rect { left: 100, top: 700, right: 300, bottom: 790 };
        let (save, _) = button_rects(Some(low), 80, 90, Some(m)).unwrap();
        assert_eq!(save.bottom, 790 - BUTTON_GAP);
    }
}
