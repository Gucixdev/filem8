//! The three item views (list with columns, grid with thumbnails, compact)
//! of one pane, plus the column model.
use crate::app::*;
use fm8_core::class::{class_label, mime_of};
use fm8_core::listing::{human_size, Entry, SortKey};
use fm8_core::settings::ViewMode;
use fm8_ui::*;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColId { Size, Modified, Type, Items, Perms, Owner, Mime }

impl ColId {
    pub const ALL: [ColId; 7] = [ColId::Size, ColId::Modified, ColId::Type, ColId::Items, ColId::Perms, ColId::Owner, ColId::Mime];
    pub fn label(self) -> &'static str { match self { ColId::Size => "Size", ColId::Modified => "Modified", ColId::Type => "Type", ColId::Items => "Items", ColId::Perms => "Permissions", ColId::Owner => "Owner", ColId::Mime => "MIME type" } }
    pub fn key(self) -> &'static str { match self { ColId::Size => "size", ColId::Modified => "modified", ColId::Type => "type", ColId::Items => "items", ColId::Perms => "perms", ColId::Owner => "owner", ColId::Mime => "mime" } }
    pub fn sort(self) -> SortKey { match self { ColId::Size => SortKey::Size, ColId::Modified => SortKey::Modified, ColId::Type | ColId::Mime => SortKey::Type, ColId::Items => SortKey::Items, ColId::Perms => SortKey::Perms, ColId::Owner => SortKey::Owner } }
    pub fn align(self) -> f32 { match self { ColId::Size | ColId::Items => 1.0, _ => 0.0 } }
    pub fn default_w(self) -> f32 { match self { ColId::Size => 86.0, ColId::Modified => 136.0, ColId::Type => 96.0, ColId::Items => 64.0, ColId::Perms => 96.0, ColId::Owner => 80.0, ColId::Mime => 150.0 } }
}

#[derive(Clone, Debug)]
pub struct Col { pub id: ColId, pub w: f32, pub on: bool }

pub fn default_cols() -> Vec<Col> {
    ColId::ALL.iter().map(|&id| Col { id, w: id.default_w(), on: id != ColId::Mime }).collect()
}
pub fn cols_to_text(c: &[Col]) -> String { c.iter().map(|c| format!("{}\t{}\t{}\n", c.id.key(), c.w as u32, c.on as u8)).collect() }
pub fn cols_from_text(s: &str) -> Vec<Col> {
    let mut out: Vec<Col> = vec![];
    for l in s.lines() {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() != 3 { continue; }
        if let (Some(id), Ok(w)) = (ColId::ALL.iter().find(|c| c.key() == f[0]), f[1].parse::<f32>()) {
            if !out.iter().any(|c| c.id == *id) { out.push(Col { id: *id, w: w.clamp(50.0, 420.0), on: f[2] == "1" }); }
        }
    }
    for id in ColId::ALL { if !out.iter().any(|c| c.id == id) { out.push(Col { id, w: id.default_w(), on: id != ColId::Mime }); } }
    out
}

pub fn fmt_mode(mode: u32, dir: bool) -> String {
    let mut s = String::with_capacity(10);
    s.push(if dir { 'd' } else { '-' });
    for sh in [6, 3, 0] {
        let b = (mode >> sh) & 7;
        s.push(if b & 4 != 0 { 'r' } else { '-' });
        s.push(if b & 2 != 0 { 'w' } else { '-' });
        s.push(if b & 1 != 0 { 'x' } else { '-' });
    }
    s
}

pub fn owner_name(uid: u32) -> String {
    unsafe {
        let p = libc::getpwuid(uid);
        if p.is_null() { return uid.to_string(); }
        std::ffi::CStr::from_ptr((*p).pw_name).to_string_lossy().into_owned()
    }
}

pub fn tile_dims(size: &str) -> (f32, f32) {
    // numeric value = icon edge in px (tile slider); words kept for old configs
    let px: f32 = match size { "small" => 96.0, "large" => 256.0, "medium" => 176.0, n => n.parse::<f32>().unwrap_or(176.0).clamp(64.0, 512.0) };
    (px + 24.0, px + 50.0)
}

impl App {
    fn cell_text(&self, e: &Entry, c: ColId) -> String {
        match c {
            ColId::Size => if e.is_dir { String::new() } else { human_size(e.size) },
            ColId::Modified => fmt_time(e.modified),
            ColId::Type => class_label(e.class).to_string(),
            ColId::Items => if e.is_dir { e.items.to_string() } else { String::new() },
            ColId::Perms => fmt_mode(e.mode, e.is_dir),
            ColId::Owner => self.owners.borrow_mut().entry(e.uid).or_insert_with(|| owner_name(e.uid)).clone(),
            ColId::Mime => mime_of(&e.name, e.is_dir).to_string(),
        }
    }

    fn visible_cols(&self, w: f32) -> Vec<usize> {
        let avail = w - 34.0 - 190.0;
        let mut sum = 0.0;
        let mut v = vec![];
        for (i, c) in self.cols.iter().enumerate() {
            if !c.on { continue; }
            if sum + c.w > avail { break; }
            sum += c.w;
            v.push(i);
        }
        v
    }

    /// (column index, x, w) from the right edge, plus the x where the name column must end.
    fn layout_cols(&self, vis: &[usize], right: f32) -> Vec<(usize, f32, f32)> {
        let mut x = right;
        let mut out = vec![];
        for &ci in vis.iter().rev() { x -= self.cols[ci].w; out.push((ci, x, self.cols[ci].w)); }
        out.reverse();
        out
    }

    fn header(&mut self, ui: &mut Ui, hdr: Rect, body: Rect, vis: &[usize]) {
        let t = ui.theme.clone();
        let ai = self.active;
        ui.draw().rect(hdr, t.bg);
        let cols = self.layout_cols(vis, body.right() - 22.0);
        let first_x = cols.first().map(|c| c.1).unwrap_or(body.right() - 22.0);
        let mut cells: Vec<(Option<usize>, Rect, SortKey, &'static str, f32)> = vec![(None, rect(body.x + 12.0, hdr.y, first_x - body.x - 20.0, hdr.h), SortKey::Name, "Name", 0.0)];
        for &(ci, x, w) in &cols { let id = self.cols[ci].id; cells.push((Some(ci), rect(x, hdr.y, w, hdr.h), id.sort(), id.label(), id.align())); }
        for (n, (ci, c, key, label, align)) in cells.iter().enumerate() {
            let sort = self.tabs[ai].model.sort;
            let (clicked, hov, _) = ui.interact(Id::new("col").with(n as u64), *c);
            if hov { ui.draw().rounded(c.inset_xy(0.0, 3.0), t.hover(), 5.0); }
            let active = sort.key == *key && !(label == &"MIME type" && false);
            let lw = fm8_ui::text::measure(label, t.font_size, true);
            let ellip = fm8_ui::text::ellipsize(label, t.font_size, true, c.w - 22.0);
            let tx = if *align > 0.5 { c.right() - 6.0 - lw - if active { 16.0 } else { 0.0 } } else { c.x + 6.0 };
            ui.label_ex(rect(tx, c.y, c.w.min(lw + 2.0), c.h), &ellip, if active { t.fg } else { t.fg_dim }, true, 0.0);
            if active {
                let (ax, ay) = (tx + lw.min(c.w - 22.0) + 9.0, c.y + c.h / 2.0);
                let d = if sort.ascending { -1.0 } else { 1.0 };
                ui.draw().line((ax - 4.0, ay - 2.0 * d), (ax, ay + 2.0 * d), 1.6, t.fg);
                ui.draw().line((ax, ay + 2.0 * d), (ax + 4.0, ay - 2.0 * d), 1.6, t.fg);
            }
            if clicked {
                let mut s = sort;
                if s.key == *key { s.ascending = !s.ascending } else { s.key = *key; s.ascending = true }
                self.tabs[ai].model.set_sort(s);
                self.remember();
            }
            // drag the left edge of a column to resize it
            if let Some(ci) = ci {
                let handle = rect(c.x - 4.0, hdr.y + 4.0, 8.0, hdr.h - 8.0);
                let hid = Id::new("colresize").with(*ci as u64);
                if ui.hovered(handle) { ui.cursor = Cursor::ResizeH; if ui.frame.press[0] { ui.set_active(hid); } }
                if ui.is_active(hid) && ui.mouse_down[0] {
                    ui.cursor = Cursor::ResizeH;
                    self.cols[*ci].w = (c.right() - ui.mouse.x).clamp(50.0, 420.0);
                    self.cols_dirty = true;
                }
                if n > 0 { ui.draw().rect(rect(c.x - 0.5, hdr.y + 8.0, 1.0, hdr.h - 16.0), t.border); }
            }
        }
        if ui.hovered(hdr) && ui.frame.press[1] { self.ctx = Some((ui.mouse, Ctx::Columns)); }
        ui.separator(rect(hdr.x, hdr.bottom() - 1.0, hdr.w, 1.0));
    }

    pub(crate) fn items_view(&mut self, ui: &mut Ui, r: Rect, focused: bool) {
        let t = ui.theme.clone();
        let ai = self.active;
        let mode = self.tabs[ai].mode;
        let tile = tile_dims(&self.prefs.get("thumb_size"));
        let (body, vis) = if mode == ViewMode::List {
            let (hdr, body) = r.cut_top(30.0);
            let vis = self.visible_cols(body.w);
            self.header(ui, hdr, body, &vis);
            (body, vis)
        } else { (r, vec![]) };
        let id = self.list_id();
        let n = self.tabs[ai].model.view.len();
        if let Some(e) = &self.tabs[ai].model.error { ui.label_ex(body.inset(16.0), e, t.urgent, false, 0.0); }
        else if n == 0 && !self.tabs[ai].model.loading { ui.label_ex(body, "This folder is empty", t.fg_dim, false, 0.5); }
        if focused && !ui.typing && ui.focus == Some(id) && !ui.frame.text.is_empty() {
            if ui.time - self.find_at > 1.0 { self.find.clear(); }
            let txt: String = ui.frame.text.chars().filter(|c| !c.is_control()).collect();
            ui.frame.text.clear();
            self.find.push_str(&txt);
            self.find_at = ui.time;
            let f = self.find.clone();
            if let Some(pos) = self.tabs[ai].model.find_prefix(&f) { self.tabs[ai].model.click(pos, false, false); ui.reveal(id, pos); }
        }
        if focused && !ui.typing && (ui.focus.is_none() || ui.focus == self.tabs[ai].last_list) { ui.focus = Some(id); }
        self.tabs[ai].last_list = Some(id);
        let (cw, ch) = match mode { ViewMode::List => (None, 30.0), ViewMode::Grid => (Some(tile.0), tile.1), ViewMode::Compact => (Some(236.0), 30.0) };
        let resp = ui.virt(id, body, n, cw, ch);
        let mut ctx = None;
        let mut open: Option<(PathBuf, bool)> = None;
        let mut newtab: Option<PathBuf> = None;
        let mut drag: Option<usize> = None;
        for ev in &resp.events {
            let m = &mut self.tabs[ai].model;
            match ev {
                ListEvent::Click { i, mods } => m.click(*i, mods.ctrl, mods.shift),
                ListEvent::DoubleClick { i } => { if let Some(e) = m.entry(*i) { open = Some((e.path.clone(), e.is_dir)); } }
                ListEvent::MiddleClick { i } => { if let Some(e) = m.entry(*i) { if e.is_dir { newtab = Some(e.path.clone()); } } }
                ListEvent::RightClick { i } => {
                    if !m.is_selected(*i) { m.click(*i, false, false); }
                    if let Some(e) = m.entry(*i) { ctx = Some(Ctx::Entry(e.path.clone(), e.is_dir)); }
                }
                ListEvent::ClickEmpty => m.clear_selection(),
                ListEvent::Nav(k, mods) => {
                    let page = (resp.page_rows() * resp.cols) as isize;
                    let c = resp.cols as isize;
                    let d = match k {
                        Nav::Up => -c, Nav::Down => c,
                        Nav::Left => if c > 1 { -1 } else { 0 }, Nav::Right => if c > 1 { 1 } else { 0 },
                        Nav::PageUp => -page, Nav::PageDown => page, Nav::Home => -(n as isize), Nav::End => n as isize,
                    };
                    if d != 0 { m.move_cursor(d, mods.shift); if let Some(c) = m.cursor { ui.reveal(id, c); } }
                }
                ListEvent::Activate => self.pending_open = true,
                ListEvent::DragStart { i } => drag = Some(*i),
            }
        }
        if let Some(i) = drag {
            let m = &self.tabs[ai].model;
            if let Some(e) = m.entry(i) {
                let paths = if m.is_selected(i) { m.selection() } else { vec![e.path.clone()] };
                self.drag = Some(Drag { paths });
            }
        }
        if ctx.is_none() && ui.hovered(body) && ui.frame.press[1] && resp.hovered.is_none() { ctx = Some(Ctx::Empty); }
        if let Some(c) = ctx { self.ctx = Some((ui.mouse, c)); }
        if let Some((p, d)) = open { self.open_path(p, d, false); }
        if let Some(p) = newtab { self.new_tab(p); }
        let dragging = self.drag.is_some();
        if dragging { let d = self.tabs[ai].model.dir.clone(); self.drops.push((body, d)); }
        // ---- cells
        let m = &self.tabs[ai].model;
        let cols = self.layout_cols(&vis, body.right() - 22.0);
        let thumbs_on = self.prefs.flag("thumbnails");
        self.thumbs.set_px((tile.0 - 24.0) as u32);
        let mut rename_done: Option<(bool, String)> = None;
        ui.push_clip(body);
        for pos in resp.first..resp.last {
            let Some(e) = m.entry(pos) else { break };
            let cr = resp.cell_rect(pos);
            let rr = rect(cr.x + 3.0, cr.y + 1.0, cr.w - if mode == ViewMode::List { 17.0 } else { 6.0 }, cr.h - 2.0);
            let sel = m.is_selected(pos);
            let hov = resp.hovered == Some(pos);
            if sel { ui.draw().rounded(rr, t.selection_bg, t.radius); } else if hov { ui.draw().rounded(rr, t.hover(), t.radius); }
            if m.cursor == Some(pos) && ui.focus == Some(id) { ui.draw().stroke(rr, t.border_focus, t.radius, 1.5); }
            if dragging && hov && e.is_dir { self.drops.push((rr, e.path.clone())); }
            let (fg, dim) = if sel { (t.selection_fg, t.selection_fg) } else { (t.fg, t.fg_dim) };
            let icol = if sel { t.selection_fg } else if e.is_dir { t.accent } else { t.fg_dim };
            let ibg = if sel { t.selection_bg } else { t.bg };
            let renaming = self.rename.as_ref().map(|(p, _)| *p == e.path).unwrap_or(false);
            let name_col = if e.hidden { dim } else { fg };
            let name_r;
            match mode {
                ViewMode::List => {
                    let first_x = cols.first().map(|c| c.1).unwrap_or(rr.right() - 8.0);
                    { let ir = rect(rr.x + 8.0, rr.y + 5.0, 20.0, 20.0); if !self.icons.entry(ui, e, ir) { ui.icon2(icon_for(e.class), ir, icol, ibg); } }
                    name_r = rect(rr.x + 38.0, rr.y, (first_x - rr.x - 46.0).max(20.0), rr.h);
                    if !renaming { ui.label(name_r, &e.name, name_col); }
                    for &(ci, x, w) in &cols {
                        let id = self.cols[ci].id;
                        let s = self.cell_text(e, id);
                        let a = id.align();
                        ui.label_ex(rect(x + if a > 0.5 { 0.0 } else { 4.0 }, rr.y, w - 6.0, rr.h), &s, dim, false, a);
                    }
                }
                ViewMode::Compact => {
                    { let ir = rect(rr.x + 8.0, rr.y + 5.0, 20.0, 20.0); if !self.icons.entry(ui, e, ir) { ui.icon2(icon_for(e.class), ir, icol, ibg); } }
                    name_r = rect(rr.x + 38.0, rr.y, rr.w - 44.0, rr.h);
                    if !renaming { ui.label(name_r, &e.name, name_col); }
                }
                ViewMode::Grid => {
                    let isz = (rr.w - 24.0).min(rr.h - 50.0).max(24.0);
                    let ir = rect(rr.x + (rr.w - isz) / 2.0, rr.y + 6.0, isz, isz);
                    let th = if thumbs_on && (e.is_dir || e.size < 400_000_000) { self.thumbs.get(&e.path) } else { None };
                    let mut info = String::new();
                    let mut drew = false;
                    if let Some(tb) = th {
                        let mut img = tb.img.clone();
                        info = tb.info.clone();
                        if e.is_dir {
                            // folder: theme icon with a 2x2 collage of its images in the middle
                            let ir2 = ir.inset(isz * 0.04);
                            if !self.icons.entry(ui, e, ir2) { ui.icon2(icon_for(e.class), ir2, icol, ibg); }
                            let c = ir.inset(isz * 0.24);
                            let cr = rect(c.x, c.y + isz * 0.06, c.w, c.h * 0.9);
                            ui.draw().cmds.push(Cmd::Image { r: { let sd = cr.w.min(cr.h); rect(cr.x + (cr.w - sd) / 2.0, cr.y, sd, sd) }, img, smooth: true });
                            drew = true;
                        } else {
                            if hov {
                                if let Some(fr) = self.thumbs.anim(&e.path) {
                                    let total: u32 = fr.iter().map(|f| f.1).sum::<u32>().max(1);
                                    let mut at = ((ui.time * 1000.0) as u64 % total as u64) as u32;
                                    for f in fr.iter() { if at < f.1 { img = f.0.clone(); break; } at -= f.1; }
                                    ui.request_redraw();
                                }
                            }
                            let s = (ir.w / img.w as f32).min(ir.h / img.h as f32);
                            let (w, h) = (img.w as f32 * s, img.h as f32 * s);
                            let fr = rect(ir.x + (ir.w - w) / 2.0, ir.y + (ir.h - h) / 2.0, w, h);
                            if tb.alpha { checker(ui, fr, &t_dark(&t)); }
                            ui.draw().cmds.push(Cmd::Image { r: fr, img, smooth: true });
                            drew = true;
                        }
                    }
                    if !drew {
                        { let ir2 = ir.inset(isz * 0.1); if !self.icons.entry(ui, e, ir2) { ui.icon2(icon_for(e.class), ir2, icol, ibg); } }
                    }
                    if !info.is_empty() { ui.label_ex(rect(rr.x + 4.0, rr.bottom() - 20.0, rr.w - 8.0, 16.0), &info, dim, false, 0.5); }
                    name_r = rect(rr.x + 4.0, rr.bottom() - 42.0, rr.w - 8.0, 22.0);
                    if !renaming { ui.label_ex(name_r, &e.name, name_col, false, 0.5); }
                }
            }
            if renaming {
                if let Some((_, buf)) = self.rename.as_mut() {
                    let rid = Id::new("rename");
                    let resp = ui.text_input(rid, name_r.inset_xy(-4.0, 1.0), buf, "");
                    if resp.submitted { rename_done = Some((true, buf.clone())); }
                    else if resp.cancelled || ui.focus != Some(rid) { rename_done = Some((false, String::new())); }
                }
            }
        }
        ui.pop_clip();
        if let Some((ok, name)) = rename_done { self.finish_rename(ok, name); ui.focus = None; }
        if self.tabs[ai].model.loading || self.thumbs.busy() || self.icons.busy() { ui.request_redraw(); }
    }
}

fn t_dark(t: &Theme) -> [Color; 2] { [t.bg_alt, t.border] }

/// Checkerboard behind transparent thumbnails (cell size grows with the tile, capped at 12x12 cells).
fn checker(ui: &mut Ui, r: Rect, c: &[Color; 2]) {
    let cell = (r.w.max(r.h) / 12.0).max(8.0);
    ui.draw().rect(r, c[0]);
    let (mut y, mut j) = (r.y, 0);
    while y < r.bottom() {
        let (mut x, mut i) = (r.x, 0);
        while x < r.right() {
            if (i + j) % 2 == 1 { ui.draw().rect(rect(x, y, cell.min(r.right() - x), cell.min(r.bottom() - y)), c[1]); }
            x += cell; i += 1;
        }
        y += cell; j += 1;
    }
}
