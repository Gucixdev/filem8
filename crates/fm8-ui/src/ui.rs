//! Immediate-mode UI core.
//!
//! Why immediate mode: a file manager's UI is a pure function of app state
//! (current dir, selection, tabs). Rebuilding the tree each frame removes a
//! whole class of stale-widget bugs, virtualised lists are just "loop over the
//! visible range", and the display list is trivially testable headlessly.
//! The few things that must persist between frames (scroll offsets, split
//! ratios, text carets, focus) live in `Ui` keyed by a hashed `Id`.

use crate::draw::{Cmd, DrawList};
use crate::geom::*;
use crate::icons::{self, Icon};
use crate::text;
use crate::theme::Theme;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Id(pub u64);

impl Id {
    pub fn new(s: &str) -> Id {
        let mut h = 0xcbf29ce484222325u64;
        for b in s.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        Id(h)
    }
    pub fn with(self, n: u64) -> Id {
        Id((self.0 ^ n.wrapping_add(0x9e3779b97f4a7c15)).wrapping_mul(0x100000001b3))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left, Right, Up, Down, Home, End, PageUp, PageDown,
    Enter, Escape, Backspace, Delete, Tab, Space, F(u8),
    Char(char),
}

#[derive(Clone, Debug)]
pub enum Event {
    MouseMove(Vec2),
    MouseDown(Button),
    MouseUp(Button),
    /// Scroll in lines; positive = content moves up (scroll down).
    Wheel(f32),
    Key(Key, Mods),
    Text(String),
    Leave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Cursor {
    #[default]
    Default,
    Pointer,
    Text,
    ResizeH,
    ResizeV,
}

#[derive(Default, Debug)]
pub struct Frame {
    pub press: [bool; 3],
    pub release: [bool; 3],
    pub double: bool,
    pub wheel: f32,
    pub keys: Vec<(Key, Mods)>,
    pub text: String,
}

fn bi(b: Button) -> usize {
    match b {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
    }
}

pub struct Ui {
    pub theme: Theme,
    pub size: Vec2,
    pub time: f64,
    pub dt: f32,
    pub mouse: Vec2,
    pub mouse_down: [bool; 3],
    /// One-shot: the next `icon_button` draws this picture instead of the procedural glyph.
    pub icon_img: Option<std::sync::Arc<crate::draw::Image>>,
    pub mods: Mods,
    pub frame: Frame,
    pub cursor: Cursor,
    /// Set when something is animating; the app should keep redrawing.
    pub animating: bool,
    pub dl: DrawList,
    overlay: DrawList,
    in_overlay: bool,
    clips: Vec<Rect>,
    active: Option<Id>,
    pub focus: Option<Id>,
    focus_claimed: bool,
    pub typing: bool,
    typing_now: bool,
    floats: HashMap<Id, f32>,
    texts: HashMap<Id, (usize, usize)>,
    scroll: HashMap<Id, (f32, f32)>,
    reveal: HashMap<Id, usize>,
    last_click: (f64, Vec2),
    popup_prev: Option<Rect>,
    popup_cur: Option<Rect>,
    tip: Option<(Id, f64)>,
    tip_draw: Option<(String, Rect)>,
    drag_origin: Option<(Id, Vec2)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BtnStyle {
    Normal,
    Primary,
    Ghost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Up, Down, Left, Right, PageUp, PageDown, Home, End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ListEvent {
    Click { i: usize, mods: Mods },
    RightClick { i: usize },
    DoubleClick { i: usize },
    MiddleClick { i: usize },
    /// Click on blank space.
    ClickEmpty,
    Nav(Nav, Mods),
    Activate,
    /// Mouse pressed on row `i` and dragged > 6px (start of a drag-and-drop).
    DragStart { i: usize },
}

pub struct ListResp {
    pub rect: Rect,
    pub first: usize,
    /// Exclusive.
    pub last: usize,
    pub cols: usize,
    pub cell: (f32, f32),
    pub offset: f32,
    pub events: Vec<ListEvent>,
    pub hovered: Option<usize>,
}

impl ListResp {
    pub fn cell_rect(&self, i: usize) -> Rect {
        let (c, r) = (i % self.cols, i / self.cols);
        rect(self.rect.x + c as f32 * self.cell.0, self.rect.y + r as f32 * self.cell.1 - self.offset, self.cell.0, self.cell.1)
    }
    /// Rows visible per page (for PageUp/PageDown handling).
    pub fn page_rows(&self) -> usize {
        ((self.rect.h / self.cell.1) as usize).max(1)
    }
}

#[derive(Default, Debug)]
pub struct TabsResp {
    pub select: Option<usize>,
    pub close: Option<usize>,
    pub new: bool,
}

#[derive(Clone)]
pub struct MenuItem {
    pub label: String,
    pub shortcut: String,
    pub enabled: bool,
    pub separator: bool,
}

impl MenuItem {
    pub fn new(label: &str, shortcut: &str) -> MenuItem {
        MenuItem { label: label.into(), shortcut: shortcut.into(), enabled: true, separator: false }
    }
    pub fn disabled(mut self) -> MenuItem {
        self.enabled = false;
        self
    }
    pub fn sep() -> MenuItem {
        MenuItem { label: String::new(), shortcut: String::new(), enabled: false, separator: true }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum MenuResp {
    Open,
    Chosen(usize),
    Dismiss,
}

#[derive(Default, Debug)]
pub struct TextResp {
    pub changed: bool,
    pub submitted: bool,
    pub cancelled: bool,
}

impl Ui {
    pub fn new(theme: Theme) -> Ui {
        Ui {
            theme,
            size: v2(800.0, 600.0),
            time: 0.0,
            dt: 0.016,
            mouse: v2(-1.0, -1.0),
            mouse_down: [false; 3],
            icon_img: None,
            mods: Mods::default(),
            frame: Frame::default(),
            cursor: Cursor::Default,
            animating: false,
            dl: DrawList::default(),
            overlay: DrawList::default(),
            in_overlay: false,
            clips: vec![],
            active: None,
            focus: None,
            focus_claimed: false,
            typing: false,
            typing_now: false,
            floats: HashMap::new(),
            texts: HashMap::new(),
            scroll: HashMap::new(),
            reveal: HashMap::new(),
            last_click: (-10.0, v2(0.0, 0.0)),
            popup_prev: None,
            popup_cur: None,
            tip: None,
            tip_draw: None,
            drag_origin: None,
        }
    }

    // ---- frame lifecycle -------------------------------------------------

    pub fn begin(&mut self, size: Vec2, time: f64, events: &[Event]) {
        self.dt = ((time - self.time) as f32).clamp(0.0, 0.1);
        self.time = time;
        self.size = size;
        self.frame = Frame::default();
        self.cursor = Cursor::Default;
        self.animating = false;
        self.dl = DrawList::default();
        self.overlay = DrawList::default();
        self.clips.clear();
        self.focus_claimed = false;
        self.typing_now = false;
        self.popup_cur = None;
        self.tip_draw = None;
        for e in events {
            match e {
                Event::MouseMove(p) => self.mouse = *p,
                Event::Leave => self.mouse = v2(-1.0, -1.0),
                Event::MouseDown(b) => {
                    self.mouse_down[bi(*b)] = true;
                    self.frame.press[bi(*b)] = true;
                    if *b == Button::Left {
                        let (t, p) = self.last_click;
                        if time - t < 0.4 && (p.x - self.mouse.x).abs() < 5.0 && (p.y - self.mouse.y).abs() < 5.0 {
                            self.frame.double = true;
                            self.last_click = (-10.0, self.mouse);
                        } else {
                            self.last_click = (time, self.mouse);
                        }
                    }
                }
                Event::MouseUp(b) => {
                    self.mouse_down[bi(*b)] = false;
                    self.frame.release[bi(*b)] = true;
                }
                Event::Wheel(w) => self.frame.wheel += w,
                Event::Key(k, m) => {
                    self.mods = *m;
                    self.frame.keys.push((*k, *m));
                }
                Event::Text(t) => self.frame.text.push_str(t),
            }
        }
    }

    pub fn end(&mut self) -> DrawList {
        if let Some((t, anchor)) = self.tip_draw.take() {
            self.in_overlay = true;
            let tw = text::measure(&t, 12.5, false) + 16.0;
            let mut r = rect(anchor.x, anchor.bottom() + 6.0, tw, 24.0);
            r.x = r.x.min(self.size.x - tw - 4.0).max(4.0);
            if r.bottom() > self.size.y {
                r.y = anchor.y - 30.0;
            }
            self.out().cmds.push(Cmd::Shadow { r, radius: 5.0, blur: 10.0, color: rgba(0, 0, 0, 90) });
            let (bg, fg, bd) = (self.theme.bg_alt.mix(self.theme.fg, 0.1), self.theme.fg, self.theme.border);
            self.out().rounded(r, bg, 5.0);
            self.out().stroke(r, bd, 5.0, 1.0);
            self.out().text(r.x + 8.0, r.y + 5.0, &t, 12.5, fg, false);
            self.in_overlay = false;
        }
        // a left press nobody claimed drops keyboard focus
        if self.frame.press[0] && !self.focus_claimed {
            self.focus = None;
        }
        if !self.mouse_down[0] && !self.frame.press[0] {
            self.active = None;
            self.drag_origin = None;
        }
        self.typing = self.typing_now;
        self.popup_prev = self.popup_cur;
        let mut d = std::mem::take(&mut self.dl);
        d.extend(std::mem::take(&mut self.overlay));
        d
    }

    fn out(&mut self) -> &mut DrawList {
        if self.in_overlay { &mut self.overlay } else { &mut self.dl }
    }

    /// Draw list for custom drawing (current layer).
    pub fn set_active(&mut self, id: Id) { self.active = Some(id); }
    pub fn is_active(&self, id: Id) -> bool { self.active == Some(id) }

    pub fn draw(&mut self) -> &mut DrawList {
        self.out()
    }

    pub fn push_clip(&mut self, r: Rect) {
        let r = match self.clips.last() { Some(c) => c.intersect(r), None => r };
        self.clips.push(r);
        self.out().push_clip(r);
    }
    pub fn pop_clip(&mut self) {
        self.clips.pop();
        self.out().pop_clip();
    }

    pub fn take_key(&mut self, pred: impl Fn(Key, Mods) -> bool) -> bool {
        if let Some(i) = self.frame.keys.iter().position(|(k, m)| pred(*k, *m)) {
            self.frame.keys.remove(i);
            true
        } else {
            false
        }
    }
    pub fn key_pressed(&mut self, k: Key) -> bool {
        self.take_key(|kk, m| kk == k && !m.ctrl && !m.alt)
    }
    pub fn ctrl_key(&mut self, c: char) -> bool {
        self.take_key(|k, m| m.ctrl && !m.alt && k == Key::Char(c))
    }

    pub fn hovered(&self, r: Rect) -> bool {
        if !r.contains(self.mouse) {
            return false;
        }
        if let Some(c) = self.clips.last() {
            if !c.contains(self.mouse) {
                return false;
            }
        }
        if !self.in_overlay {
            if let Some(p) = self.popup_prev {
                if p.contains(self.mouse) {
                    return false;
                }
            }
        }
        true
    }

    pub fn f(&mut self, id: Id, default: f32) -> &mut f32 {
        self.floats.entry(id).or_insert(default)
    }

    pub fn request_redraw(&mut self) {
        self.animating = true;
    }

    pub fn set_focus(&mut self, id: Option<Id>) {
        self.focus = id;
        self.focus_claimed = true;
    }

    // ---- primitives ------------------------------------------------------

    pub fn text_size(&self) -> f32 {
        self.theme.font_size
    }

    pub fn label(&mut self, r: Rect, s: &str, color: Color) {
        self.label_ex(r, s, color, false, 0.0);
    }

    /// Vertically centred, ellipsised text. `align`: 0 left, 0.5 centre, 1 right.
    pub fn label_ex(&mut self, r: Rect, s: &str, color: Color, bold: bool, align: f32) {
        let size = self.theme.font_size;
        let shown = text::ellipsize(s, size, bold, r.w);
        let w = text::measure(&shown, size, bold);
        let x = r.x + (r.w - w) * align;
        let y = r.y + (r.h - text::line_height(size, bold)) / 2.0;
        self.out().text(x, y, &shown, size, color, bold);
    }

    pub fn icon(&mut self, i: Icon, r: Rect, color: Color) {
        let c2 = self.theme.bg;
        let mut dl = DrawList::default();
        icons::draw(&mut dl, i, r, color, c2);
        self.out().cmds.extend(dl.cmds);
    }

    pub fn icon2(&mut self, i: Icon, r: Rect, c: Color, c2: Color) {
        let mut dl = DrawList::default();
        icons::draw(&mut dl, i, r, c, c2);
        self.out().cmds.extend(dl.cmds);
    }

    pub fn separator(&mut self, r: Rect) {
        let c = self.theme.border;
        self.out().rect(r, c);
    }

    pub fn focus_ring(&mut self, r: Rect, radius: f32) {
        let c = self.theme.border_focus;
        self.out().stroke(r.inset(-1.0), c, radius + 1.0, 2.0);
    }

    // ---- widgets ---------------------------------------------------------

    /// Returns true on click (release over the button after pressing it).
    pub fn button(&mut self, id: Id, r: Rect, label: &str, style: BtnStyle) -> bool {
        let (clicked, hov, down) = self.interact(id, r);
        let t = self.theme.clone();
        let (bg, fg) = match style {
            BtnStyle::Primary => (if down { t.accent.mix(t.bg, 0.25) } else if hov { t.accent.mix(t.fg, 0.12) } else { t.accent }, t.accent_fg),
            BtnStyle::Normal => (if down { t.pressed() } else if hov { t.hover() } else { t.bg_alt }, t.fg),
            BtnStyle::Ghost => (if down { t.pressed() } else if hov { t.hover() } else { Color::TRANSPARENT }, t.fg),
        };
        self.out().rounded(r, bg, t.radius);
        if style == BtnStyle::Normal {
            self.out().stroke(r, t.border, t.radius, 1.0);
        }
        self.label_ex(r.inset_xy(10.0, 0.0), label, fg, false, 0.5);
        if self.focus == Some(id) {
            self.focus_ring(r, t.radius);
        }
        clicked
    }

    /// Horizontal drag handle: returns the 0..1 position while the mouse is
    /// pressed on (or dragging from) `r`.
    pub fn drag_frac(&mut self, id: Id, r: Rect) -> Option<f32> {
        if self.hovered(r) { self.cursor = Cursor::Pointer; if self.frame.press[0] { self.active = Some(id); } }
        if self.active == Some(id) && self.mouse_down[0] { Some(((self.mouse.x - r.x) / r.w).clamp(0.0, 1.0)) } else { None }
    }

    pub fn icon_button(&mut self, id: Id, r: Rect, icon: Icon, tip: &str, enabled: bool, active: bool) -> bool {
        let (clicked, hov, down) = if enabled { self.interact(id, r) } else { (false, false, false) };
        let t = self.theme.clone();
        let bg = if active { t.selection_bg } else if down { t.pressed() } else if hov { t.hover() } else { Color::TRANSPARENT };
        self.out().rounded(r, bg, t.radius);
        let fg = if !enabled { t.fg_dim.with_alpha(110) } else if active { t.selection_fg } else { t.fg };
        let ir = r.centered(18.0_f32.min(r.w - 4.0), 18.0_f32.min(r.h - 4.0));
        match self.icon_img.take() {
            Some(img) => {
                let k = (ir.w / img.w as f32).min(ir.h / img.h as f32);
                let (w, h) = (img.w as f32 * k, img.h as f32 * k);
                self.out().cmds.push(Cmd::Image { r: rect(ir.x + (ir.w - w) / 2.0, ir.y + (ir.h - h) / 2.0, w, h), img, smooth: true });
            }
            None => self.icon(icon, ir, fg),
        }
        if hov && !tip.is_empty() {
            self.tooltip(id, r, tip);
        }
        if self.focus == Some(id) {
            self.focus_ring(r, t.radius);
        }
        clicked && enabled
    }

    /// Generic click interaction: (clicked, hovered, held).
    pub fn interact(&mut self, id: Id, r: Rect) -> (bool, bool, bool) {
        let hov = self.hovered(r);
        if hov {
            self.cursor = Cursor::Pointer;
        }
        if hov && self.frame.press[0] {
            self.active = Some(id);
            self.focus = Some(id);
            self.focus_claimed = true;
        }
        let held = self.active == Some(id) && self.mouse_down[0];
        let mut clicked = self.active == Some(id) && self.frame.release[0] && hov;
        if self.focus == Some(id) && !self.typing && (self.key_pressed(Key::Enter) || self.key_pressed(Key::Space)) {
            clicked = true;
        }
        (clicked, hov, held)
    }

    /// Show `s` near `anchor` after the cursor rested on it for 0.5 s.
    pub fn tooltip(&mut self, id: Id, anchor: Rect, s: &str) {
        if !self.hovered(anchor) {
            return;
        }
        match self.tip {
            Some((tid, since)) if tid == id => {
                if self.time - since > 0.5 {
                    self.tip_draw = Some((s.to_string(), anchor));
                } else {
                    self.animating = true;
                }
            }
            _ => self.tip = Some((id, self.time)),
        }
        if self.mouse_down[0] {
            self.tip = None;
            self.tip_draw = None;
        }
    }

    /// Single-line text input. `text` is edited in place.
    pub fn text_input(&mut self, id: Id, r: Rect, text: &mut String, placeholder: &str) -> TextResp {
        let mut resp = TextResp::default();
        let t = self.theme.clone();
        let size = t.font_size;
        let hov = self.hovered(r);
        if hov {
            self.cursor = Cursor::Text;
        }
        let n = text.chars().count();
        let st = *self.texts.entry(id).or_insert((n, n));
        let (mut cur, mut anc) = (st.0.min(n), st.1.min(n));
        if hov && self.frame.press[0] {
            self.focus = Some(id);
            self.focus_claimed = true;
            self.active = Some(id);
            cur = text::index_at(text, size, self.mouse.x - r.x - 8.0);
            anc = if self.mods.shift { anc } else { cur };
        } else if self.active == Some(id) && self.mouse_down[0] {
            cur = text::index_at(text, size, self.mouse.x - r.x - 8.0);
        }
        let focused = self.focus == Some(id);
        if focused {
            self.typing_now = true;
            let to_byte = |s: &str, ci: usize| s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len());
            let keys: Vec<(Key, Mods)> = self.frame.keys.drain(..).collect();
            let mut rest = vec![];
            for (k, m) in keys {
                let (lo, hi) = (cur.min(anc), cur.max(anc));
                let mut handled = true;
                match k {
                    Key::Left => { cur = if cur != anc && !m.shift { lo } else { cur.saturating_sub(1) }; if !m.shift { anc = cur } }
                    Key::Right => { let nn = text.chars().count(); cur = if cur != anc && !m.shift { hi } else { (cur + 1).min(nn) }; if !m.shift { anc = cur } }
                    Key::Home => { cur = 0; if !m.shift { anc = 0 } }
                    Key::End => { cur = text.chars().count(); if !m.shift { anc = cur } }
                    Key::Backspace | Key::Delete => {
                        let (mut a, mut b) = (lo, hi);
                        if a == b {
                            if k == Key::Backspace { a = a.saturating_sub(1) } else { b = (b + 1).min(text.chars().count()) }
                        }
                        if a != b {
                            let (ba, bb) = (to_byte(text, a), to_byte(text, b));
                            text.replace_range(ba..bb, "");
                            cur = a; anc = a; resp.changed = true;
                        }
                    }
                    Key::Char('a') if m.ctrl => { anc = 0; cur = text.chars().count(); }
                    Key::Enter => resp.submitted = true,
                    Key::Escape => resp.cancelled = true,
                    _ => handled = false,
                }
                if !handled { rest.push((k, m)); }
            }
            self.frame.keys = rest;
            if !self.frame.text.is_empty() {
                let ins: String = self.frame.text.chars().filter(|c| !c.is_control()).collect();
                self.frame.text.clear();
                if !ins.is_empty() {
                    let (lo, hi) = (cur.min(anc), cur.max(anc));
                    let (ba, bb) = (to_byte(text, lo), to_byte(text, hi));
                    text.replace_range(ba..bb, &ins);
                    cur = lo + ins.chars().count();
                    anc = cur;
                    resp.changed = true;
                }
            }
        }
        self.texts.insert(id, (cur, anc));
        // draw
        let bg = t.bg;
        self.out().rounded(r, bg, t.radius);
        self.out().stroke(r, if focused { t.border_focus } else { t.border }, t.radius, if focused { 2.0 } else { 1.0 });
        let inner = r.inset_xy(8.0, 0.0);
        self.push_clip(inner);
        let ty = r.y + (r.h - text::line_height(size, false)) / 2.0;
        if text.is_empty() {
            self.out().text(inner.x, ty, placeholder, size, t.fg_dim, false);
        } else {
            if cur != anc && focused {
                let pre = |n: usize| text::measure(&text.chars().take(n).collect::<String>(), size, false);
                let (x0, x1) = (pre(cur.min(anc)), pre(cur.max(anc)));
                self.out().rect(rect(inner.x + x0, ty, x1 - x0, text::line_height(size, false)), t.selection_bg);
            }
            self.out().text(inner.x, ty, text, size, t.fg, false);
        }
        if focused && ((self.time * 1.8) as u64) % 2 == 0 {
            let cx = inner.x + text::measure(&text.chars().take(cur).collect::<String>(), size, false);
            self.out().rect(rect(cx, ty, 1.5, text::line_height(size, false)), t.fg);
        }
        if focused {
            self.animating = true;
        }
        self.pop_clip();
        resp
    }

    /// Ask the next `virt` call with this id to scroll so item `i` is visible.
    pub fn reveal(&mut self, id: Id, i: usize) {
        self.reveal.insert(id, i);
    }

    /// Virtualised list / grid. Only the visible cells are reported through
    /// `first..last`; the caller draws them with `cell_rect`. Cost is O(visible),
    /// independent of `count`. `cell_w == None` gives a one-column list.
    pub fn virt(&mut self, id: Id, r: Rect, count: usize, cell_w: Option<f32>, cell_h: f32) -> ListResp {
        let cols = match cell_w { Some(w) => ((r.w - 10.0) / w).floor().max(1.0) as usize, None => 1 };
        let cw = match cell_w { Some(_) => (r.w - 10.0) / cols as f32, None => r.w };
        let rows = count.div_ceil(cols);
        let content_h = rows as f32 * cell_h;
        let max_off = (content_h - r.h).max(0.0);
        let hov = self.hovered(r);
        let mut events = vec![];
        let (mut cur, mut target) = *self.scroll.entry(id).or_insert((0.0, 0.0));
        if hov && self.frame.wheel != 0.0 {
            target -= self.frame.wheel * -cell_h.max(24.0) * 1.5;
            self.frame.wheel = 0.0;
        }
        if let Some(i) = self.reveal.remove(&id) {
            let top = (i / cols) as f32 * cell_h;
            if top < target { target = top } else if top + cell_h > target + r.h { target = top + cell_h - r.h }
        }
        // scrollbar
        let sb = rect(r.right() - 10.0, r.y, 10.0, r.h);
        let sb_id = id.with(77);
        let show_sb = max_off > 0.0;
        let thumb_h = if show_sb { (r.h * r.h / content_h).clamp(28.0, r.h) } else { 0.0 };
        if show_sb {
            let travel = r.h - thumb_h;
            let thumb_y = r.y + travel * (cur / max_off);
            let thumb = rect(sb.x, thumb_y, sb.w, thumb_h);
            if self.hovered(thumb) && self.frame.press[0] {
                self.active = Some(sb_id);
                self.drag_origin = Some((sb_id, v2(0.0, self.mouse.y - thumb_y)));
            }
            if self.hovered(sb) && !self.hovered(thumb) && self.frame.press[0] {
                target = (target + if self.mouse.y < thumb_y { -r.h } else { r.h }).clamp(0.0, max_off);
            }
            if self.active == Some(sb_id) && self.mouse_down[0] {
                if let Some((_, off)) = self.drag_origin {
                    let ty = (self.mouse.y - off.y - r.y).clamp(0.0, travel);
                    target = if travel > 0.0 { ty / travel * max_off } else { 0.0 };
                    cur = target;
                }
            }
        }
        target = target.clamp(0.0, max_off);
        let diff = target - cur;
        if diff.abs() > 0.5 {
            cur += diff * (1.0 - (-self.dt * 24.0).exp());
            self.animating = true;
        } else {
            cur = target;
        }
        self.scroll.insert(id, (cur, target));
        let offset = cur;
        // hit testing
        let hit = |p: Vec2| -> Option<usize> {
            if !r.contains(p) || (show_sb && p.x >= sb.x) { return None; }
            let c = ((p.x - r.x) / cw) as usize;
            let rr = ((p.y - r.y + offset) / cell_h) as usize;
            let i = rr * cols + c;
            if c < cols && i < count { Some(i) } else { None }
        };
        let hovered = if hov { hit(self.mouse) } else { None };
        let on_sb = show_sb && sb.contains(self.mouse);
        if hov && !on_sb {
            for (b, press) in self.frame.press.clone().into_iter().enumerate() {
                if !press { continue; }
                self.focus = Some(id);
                self.focus_claimed = true;
                match (b, hovered) {
                    (0, Some(i)) => {
                        events.push(if self.frame.double { ListEvent::DoubleClick { i } } else { ListEvent::Click { i, mods: self.mods } });
                        self.drag_origin = Some((id.with(i as u64), self.mouse));
                    }
                    (0, None) => events.push(ListEvent::ClickEmpty),
                    (1, Some(i)) => events.push(ListEvent::RightClick { i }),
                    (2, Some(i)) => events.push(ListEvent::MiddleClick { i }),
                    _ => {}
                }
            }
        }
        if let Some((did, p0)) = self.drag_origin {
            if self.mouse_down[0] && self.active != Some(sb_id) && (self.mouse.x - p0.x).hypot(self.mouse.y - p0.y) > 6.0 {
                for i in hit(p0).into_iter() {
                    if did == id.with(i as u64) {
                        events.push(ListEvent::DragStart { i });
                        self.drag_origin = None;
                    }
                }
            }
        }
        if self.focus == Some(id) && !self.typing {
            let keys: Vec<(Key, Mods)> = self.frame.keys.drain(..).collect();
            let mut rest = vec![];
            for (k, m) in keys {
                let nav = match k {
                    Key::Up => Some(Nav::Up), Key::Down => Some(Nav::Down), Key::Left => Some(Nav::Left), Key::Right => Some(Nav::Right),
                    Key::PageUp => Some(Nav::PageUp), Key::PageDown => Some(Nav::PageDown), Key::Home => Some(Nav::Home), Key::End => Some(Nav::End),
                    _ => None,
                };
                if let (Some(n), false) = (nav, m.alt) {
                    events.push(ListEvent::Nav(n, m));
                } else if k == Key::Enter && !m.alt {
                    events.push(ListEvent::Activate);
                } else {
                    rest.push((k, m));
                }
            }
            self.frame.keys = rest;
        }
        let first = ((offset / cell_h) as usize * cols).min(count);
        let last = (((offset + r.h) / cell_h).ceil() as usize * cols).min(count);
        // scrollbar paint (after clip push by caller? draw here, unclipped)
        if show_sb {
            let travel = r.h - thumb_h;
            let thumb_y = r.y + travel * (offset / max_off);
            let hot = self.active == Some(sb_id) || on_sb;
            let c = self.theme.fg_dim.with_alpha(if hot { 190 } else { 90 });
            let w = if hot { 6.0 } else { 4.0 };
            self.out().rounded(rect(sb.right() - w - 2.0, thumb_y + 1.0, w, thumb_h - 2.0), c, w / 2.0);
        }
        if self.focus == Some(id) { /* focus is shown by the selection colours */ }
        ListResp { rect: r, first, last, cols, cell: (cw, cell_h), offset, events, hovered }
    }

    /// Tab strip. Titles are ellipsised; each tab has a close button on hover.
    pub fn tabs(&mut self, id: Id, r: Rect, titles: &[String], active: usize, closable: bool) -> TabsResp {
        let t = self.theme.clone();
        let mut resp = TabsResp::default();
        let plus_w = 32.0;
        let n = titles.len().max(1);
        let tw = ((r.w - plus_w - 4.0) / n as f32).clamp(60.0, 200.0);
        self.out().rect(r, t.bg_alt);
        self.out().rect(rect(r.x, r.bottom() - 1.0, r.w, 1.0), t.border);
        for (i, title) in titles.iter().enumerate() {
            let tr = rect(r.x + 4.0 + i as f32 * tw, r.y + 4.0, tw - 3.0, r.h - 4.0);
            let tid = id.with(i as u64);
            let hov = self.hovered(tr);
            let (bg, fg) = if i == active { (t.bg, t.fg) } else if hov { (t.hover(), t.fg) } else { (Color::TRANSPARENT, t.fg_dim) };
            self.out().rounded(rect(tr.x, tr.y, tr.w, tr.h + 2.0), bg, 8.0);
            if i == active {
                self.out().rect(rect(tr.x + 8.0, tr.y, tr.w - 16.0, 2.0), t.accent);
            }
            let xr = rect(tr.right() - 24.0, tr.y + (tr.h - 18.0) / 2.0, 18.0, 18.0);
            let show_x = closable && (hov || i == active) && titles.len() > 1;
            let lab = rect(tr.x + 10.0, tr.y, tr.w - 20.0 - if show_x { 20.0 } else { 0.0 }, tr.h);
            self.label_ex(lab, title, fg, false, 0.0);
            if show_x {
                let xh = self.hovered(xr);
                if xh {
                    self.out().rounded(xr, t.pressed(), 4.0);
                }
                self.icon(Icon::Close, xr.inset(3.0), t.fg_dim);
                if xh && self.frame.press[0] {
                    resp.close = Some(i);
                    self.focus_claimed = true;
                    continue;
                }
            }
            if hov && !(show_x && self.hovered(xr)) {
                if self.frame.press[0] { resp.select = Some(i); }
                if self.frame.press[2] { resp.close = Some(i); }
                self.tooltip(tid, tr, title);
            }
        }
        let pr = rect(r.x + 4.0 + titles.len() as f32 * tw, r.y + 6.0, 28.0, r.h - 8.0);
        if self.icon_button(id.with(9999), pr, Icon::Plus, "New tab (Ctrl+T)", true, false) {
            resp.new = true;
        }
        resp
    }

    /// Two panes separated by a draggable divider. Ratio persists in `Ui`.
    pub fn split(&mut self, id: Id, r: Rect, axis: Axis, default_ratio: f32, min_px: f32) -> (Rect, Rect) {
        let ratio = *self.f(id, default_ratio);
        let total = if axis == Axis::Horizontal { r.w } else { r.h };
        let pos = (total * ratio).clamp(min_px.min(total / 2.0), (total - min_px).max(total / 2.0));
        let grip = 6.0;
        let hit = match axis {
            Axis::Horizontal => rect(r.x + pos - grip / 2.0, r.y, grip, r.h),
            Axis::Vertical => rect(r.x, r.y + pos - grip / 2.0, r.w, grip),
        };
        let hov = self.hovered(hit);
        if hov {
            self.cursor = if axis == Axis::Horizontal { Cursor::ResizeH } else { Cursor::ResizeV };
            if self.frame.press[0] {
                self.active = Some(id);
            }
        }
        if self.active == Some(id) && self.mouse_down[0] {
            self.cursor = if axis == Axis::Horizontal { Cursor::ResizeH } else { Cursor::ResizeV };
            let m = if axis == Axis::Horizontal { self.mouse.x - r.x } else { self.mouse.y - r.y };
            let p = m.clamp(min_px.min(total / 2.0), (total - min_px).max(total / 2.0));
            *self.f(id, default_ratio) = p / total.max(1.0);
        }
        let (a, b) = match axis {
            Axis::Horizontal => (rect(r.x, r.y, pos - 0.5, r.h), rect(r.x + pos + 0.5, r.y, total - pos - 0.5, r.h)),
            Axis::Vertical => (rect(r.x, r.y, r.w, pos - 0.5), rect(r.x, r.y + pos + 0.5, r.w, total - pos - 0.5)),
        };
        let c = if hov || self.active == Some(id) { self.theme.accent } else { self.theme.border };
        match axis {
            Axis::Horizontal => self.out().rect(rect(r.x + pos - 0.5, r.y, 1.0, r.h), c),
            Axis::Vertical => self.out().rect(rect(r.x, r.y + pos - 0.5, r.w, 1.0), c),
        }
        (a, b)
    }

    /// Popup menu at `pos` (drawn in the overlay layer, blocks input below).
    /// Call every frame while it is open.
    pub fn menu(&mut self, id: Id, pos: Vec2, items: &[MenuItem]) -> MenuResp {
        let t = self.theme.clone();
        let size = t.font_size;
        let lw = items.iter().map(|i| text::measure(&i.label, size, false)).fold(0.0, f32::max);
        let sw = items.iter().map(|i| text::measure(&i.shortcut, size - 1.0, false)).fold(0.0, f32::max);
        let w = (lw + sw + 70.0).max(180.0);
        let h: f32 = items.iter().map(|i| if i.separator { 9.0 } else { 30.0 }).sum::<f32>() + 8.0;
        let x = pos.x.min(self.size.x - w - 4.0).max(4.0);
        let y = pos.y.min(self.size.y - h - 4.0).max(4.0);
        let mr = rect(x, y, w, h);
        self.popup_cur = Some(mr);
        let was = self.in_overlay;
        self.in_overlay = true;
        let saved_clips = std::mem::take(&mut self.clips);
        self.out().cmds.push(Cmd::Shadow { r: mr, radius: t.radius + 2.0, blur: 14.0, color: rgba(0, 0, 0, 100) });
        self.out().rounded(mr, t.bg_alt, t.radius + 2.0);
        self.out().stroke(mr, t.border, t.radius + 2.0, 1.0);
        let mut sel = *self.f(id, -1.0);
        let mut resp = MenuResp::Open;
        let mut yy = mr.y + 4.0;
        let (pmx, pmy) = (*self.f(id.with(1), -1.0), *self.f(id.with(2), -1.0));
        let moved = pmx != self.mouse.x || pmy != self.mouse.y;
        *self.f(id.with(1), -1.0) = self.mouse.x;
        *self.f(id.with(2), -1.0) = self.mouse.y;
        let enabled: Vec<usize> = items.iter().enumerate().filter(|(_, i)| i.enabled).map(|(n, _)| n).collect();
        if self.key_pressed(Key::Down) {
            sel = enabled.iter().find(|&&n| n as f32 > sel).or(enabled.first()).map(|&n| n as f32).unwrap_or(-1.0);
        }
        if self.key_pressed(Key::Up) {
            sel = enabled.iter().rev().find(|&&n| (n as f32) < sel).or(enabled.last()).map(|&n| n as f32).unwrap_or(-1.0);
        }
        if self.key_pressed(Key::Enter) && sel >= 0.0 {
            resp = MenuResp::Chosen(sel as usize);
        }
        if self.key_pressed(Key::Escape) {
            resp = MenuResp::Dismiss;
        }
        for (n, it) in items.iter().enumerate() {
            if it.separator {
                self.out().rect(rect(mr.x + 8.0, yy + 4.0, mr.w - 16.0, 1.0), t.border);
                yy += 9.0;
                continue;
            }
            let ir = rect(mr.x + 4.0, yy, mr.w - 8.0, 30.0);
            let hov = self.mouse_in(ir);
            if hov && it.enabled && moved {
                sel = n as f32;
            }
            if sel == n as f32 && it.enabled {
                self.out().rounded(ir, t.selection_bg, t.radius);
            }
            let fg = if it.enabled { if sel == n as f32 { t.selection_fg } else { t.fg } } else { t.fg_dim.with_alpha(120) };
            self.label_ex(rect(ir.x + 10.0, ir.y, lw + 10.0, ir.h), &it.label, fg, false, 0.0);
            if !it.shortcut.is_empty() {
                let sr = rect(ir.right() - sw - 10.0, ir.y, sw + 4.0, ir.h);
                let c = t.fg_dim;
                self.out().text(sr.x, sr.y + (sr.h - text::line_height(size - 1.0, false)) / 2.0, &it.shortcut, size - 1.0, c, false);
            }
            if hov && it.enabled && self.frame.release[0] {
                resp = MenuResp::Chosen(n);
            }
            yy += 30.0;
        }
        if (self.frame.press[0] || self.frame.press[1]) && !mr.contains(self.mouse) {
            resp = MenuResp::Dismiss;
        }
        *self.f(id, -1.0) = sel;
        self.clips = saved_clips;
        self.in_overlay = was;
        if resp != MenuResp::Open {
            self.floats.remove(&id);
            self.floats.remove(&id.with(1));
            self.floats.remove(&id.with(2));
            self.focus_claimed = true;
        }
        resp
    }

    fn mouse_in(&self, r: Rect) -> bool {
        r.contains(self.mouse)
    }

    /// Thin horizontal progress bar.
    pub fn progress(&mut self, r: Rect, frac: f32) {
        let t = self.theme.clone();
        self.out().rounded(r, t.border, r.h / 2.0);
        self.out().rounded(rect(r.x, r.y, r.w * frac.clamp(0.0, 1.0), r.h), t.accent, r.h / 2.0);
    }

    /// Modal-ish centred dialog frame; returns the content rect.
    pub fn dialog_frame(&mut self, w: f32, h: f32, title: &str) -> Rect {
        let t = self.theme.clone();
        let full = rect(0.0, 0.0, self.size.x, self.size.y);
        let was = self.in_overlay;
        self.in_overlay = true;
        self.popup_cur = Some(full);
        self.out().rect(full, rgba(0, 0, 0, 120));
        let r = full.centered(w.min(self.size.x - 20.0), h);
        self.out().cmds.push(Cmd::Shadow { r, radius: 10.0, blur: 24.0, color: rgba(0, 0, 0, 120) });
        self.out().rounded(r, t.bg_alt, 10.0);
        self.out().stroke(r, t.border, 10.0, 1.0);
        let (head, body) = r.cut_top(44.0);
        self.label_ex(head.inset_xy(16.0, 0.0), title, t.fg, true, 0.0);
        self.in_overlay = was;
        body.inset(14.0)
    }
    pub fn begin_overlay(&mut self) {
        self.in_overlay = true;
        self.popup_cur = Some(rect(0.0, 0.0, self.size.x, self.size.y));
    }
    pub fn end_overlay(&mut self) {
        self.in_overlay = false;
    }
}
