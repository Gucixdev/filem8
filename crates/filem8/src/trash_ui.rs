//! The Trash view: a virtual place built on `fm8_core::trashbin` (home trash plus the
//! `.Trash-UID` of every mount). Shows name, original folder, deletion date and size;
//! Restore / Delete permanently / Empty trash with confirmations. Thumbnails and icons
//! come from the trashed file itself, so previews work exactly like in a normal folder.
use crate::app::*;
use fm8_core::fileops;
use fm8_core::listing::{human_size, Entry};
use fm8_core::trashbin::{self, TrashItem};
use fm8_ui::*;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Sidebar/breadcrumb sentinel: not a real directory.
pub fn sentinel() -> PathBuf { PathBuf::from("trash:///") }

pub(crate) struct TrashRow {
    pub it: TrashItem,
    pub entry: Option<Entry>,
    pub size: u64,
    pub name: String,
}

#[derive(Default)]
pub(crate) struct TrashView {
    pub rows: Vec<TrashRow>,
    pub sel: BTreeSet<usize>,
    pub cursor: usize,
    pub total: u64,
}

pub(crate) enum TrashDlg {
    Delete(Vec<TrashItem>),
    Empty,
    Conflict { item: TrashItem, rest: Vec<TrashItem> },
}

fn load_rows(roots: &[(PathBuf, Option<PathBuf>)]) -> TrashView {
    let rows: Vec<TrashRow> = trashbin::list_roots(roots).into_iter().map(|it| {
        let entry = Entry::from_path(&it.trashed).ok();
        let size = if entry.as_ref().map(|e| e.is_dir).unwrap_or(false) { fileops::tree_size(std::slice::from_ref(&it.trashed)) } else { entry.as_ref().map(|e| e.size).unwrap_or(0) };
        let name = it.original.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| it.original.to_string_lossy().into_owned());
        TrashRow { it, entry, size, name }
    }).collect();
    let total = rows.iter().map(|r| r.size).sum();
    TrashView { rows, sel: BTreeSet::new(), cursor: 0, total }
}

/// "2026-11-09T12:30:05" -> "2026-11-09 12:30".
pub fn fmt_deleted(s: &str) -> String { s.get(..16).map(|x| x.replace('T', " ")).unwrap_or_else(|| s.replace('T', " ")) }

impl App {
    pub fn set_trash_roots(&mut self, roots: Vec<(PathBuf, Option<PathBuf>)>) { self.trash_roots = roots; self.trash_place = true; self.refresh_places(); }

    /// (item count, bytes) for the sidebar entry.
    pub(crate) fn trash_stats(&self) -> (usize, u64) {
        let items = trashbin::list_roots(&self.trash_roots);
        let bytes = items.iter().map(|i| std::fs::symlink_metadata(&i.trashed).map(|m| if m.is_dir() { 0 } else { m.len() }).unwrap_or(0)).sum();
        (items.len(), bytes)
    }

    pub fn open_trash(&mut self) {
        self.trash = Some(load_rows(&self.trash_roots));
        self.search_open = false;
        self.refresh_places();
        let v = self.trash.as_ref().unwrap();
        self.status = format!("Trash: {} item(s), {}", v.rows.len(), human_size(v.total));
    }

    pub fn trash_reload(&mut self) { if self.trash.is_some() { let keep = self.trash.as_ref().map(|v| v.cursor).unwrap_or(0); self.trash = Some(load_rows(&self.trash_roots)); if let Some(v) = self.trash.as_mut() { v.cursor = keep.min(v.rows.len().saturating_sub(1)); } } self.refresh_places(); }

    pub fn trash_rows(&self) -> usize { self.trash.as_ref().map(|v| v.rows.len()).unwrap_or(0) }
    pub fn trash_select(&mut self, idx: &[usize]) { if let Some(v) = self.trash.as_mut() { v.sel = idx.iter().copied().collect(); v.cursor = idx.first().copied().unwrap_or(0); } }
    pub fn trash_dialog_open(&self) -> bool { self.trash_dlg.is_some() }

    /// Selected rows, or the cursor row when nothing is selected.
    pub(crate) fn trash_picked(&self) -> Vec<TrashItem> {
        let Some(v) = &self.trash else { return vec![] };
        let idx: Vec<usize> = if v.sel.is_empty() { if v.cursor < v.rows.len() { vec![v.cursor] } else { vec![] } } else { v.sel.iter().copied().collect() };
        idx.into_iter().filter_map(|i| v.rows.get(i).map(|r| r.it.clone())).collect()
    }

    /// Restore to the original paths; a name conflict stops and asks (dialog).
    pub fn trash_restore(&mut self, items: Vec<TrashItem>) {
        let mut done = 0;
        let mut it = items.into_iter();
        while let Some(item) = it.next() {
            if item.original.symlink_metadata().is_ok() {
                self.trash_dlg = Some(TrashDlg::Conflict { item, rest: it.collect() });
                break;
            }
            match trashbin::restore(&item) { Ok(()) => done += 1, Err(e) => { self.status = format!("Restore failed: {e}"); } }
        }
        if done > 0 { self.status = format!("Restored {done} item(s)"); }
        self.trash_reload();
        self.reload_all();
    }

    pub fn trash_purge(&mut self, items: Vec<TrashItem>) {
        let mut n = 0;
        for i in &items { match trashbin::purge(i) { Ok(()) => n += 1, Err(e) => self.status = format!("Delete failed: {e}") } }
        if n > 0 { self.status = format!("Permanently deleted {n} item(s)"); }
        self.trash_reload();
    }

    pub fn trash_empty(&mut self) {
        let all = trashbin::list_roots(&self.trash_roots);
        self.trash_purge(all);
    }

    pub fn trash_ask_delete(&mut self) { let p = self.trash_picked(); if !p.is_empty() { self.trash_dlg = Some(TrashDlg::Delete(p)); } }
    pub fn trash_ask_empty(&mut self) { if self.trash_rows() > 0 { self.trash_dlg = Some(TrashDlg::Empty); } }

    /// Keys of the trash view. Returns true when one was used.
    pub(crate) fn trash_keys(&mut self, ui: &mut Ui) -> bool {
        if ui.take_key(|k, m| k == Key::Delete && m.ctrl && m.shift) { self.trash_ask_empty(); return true; }
        if ui.take_key(|k, m| k == Key::Delete && !m.ctrl) { self.trash_ask_delete(); return true; }
        if ui.take_key(|k, m| (k == Key::Char('r') || k == Key::F(2)) && !m.ctrl && !m.alt) { let p = self.trash_picked(); self.trash_restore(p); return true; }
        if ui.take_key(|k, m| k == Key::Char('a') && m.ctrl) { if let Some(v) = self.trash.as_mut() { v.sel = (0..v.rows.len()).collect(); } return true; }
        false
    }

    pub(crate) fn trash_view(&mut self, ui: &mut Ui, r: Rect) {
        let t = ui.theme.clone();
        let (bar, rest) = r.cut_top(40.0);
        let n = self.trash_rows();
        let any = n > 0;
        let picked = self.trash_picked().len();
        let b1 = rect(bar.x + 8.0, bar.y + 5.0, 110.0, 30.0);
        let b2 = rect(b1.right() + 8.0, b1.y, 170.0, 30.0);
        let b3 = rect(bar.right() - 128.0, b1.y, 120.0, 30.0);
        if ui.button(Id::new("tr-restore"), b1, "Restore", if picked > 0 { BtnStyle::Primary } else { BtnStyle::Normal }) && picked > 0 { let p = self.trash_picked(); self.trash_restore(p); }
        if ui.button(Id::new("tr-delete"), b2, "Delete permanently", BtnStyle::Normal) && picked > 0 { self.trash_ask_delete(); }
        if ui.button(Id::new("tr-empty"), b3, "Empty trash", BtnStyle::Normal) && any { self.trash_ask_empty(); }
        // columns
        let (hdr, body) = rest.cut_top(26.0);
        ui.draw().rect(hdr, t.bg_alt);
        let (w_size, w_date) = (84.0, 138.0);
        let orig_w = ((hdr.w - w_size - w_date - 60.0) * 0.4).max(80.0);
        let x_size = hdr.right() - w_size - 14.0;
        let x_date = x_size - w_date - 8.0;
        let x_orig = x_date - orig_w - 8.0;
        let name_w = (x_orig - hdr.x - 44.0 - 8.0).max(40.0);
        for (x, label) in [(hdr.x + 40.0, "Name"), (x_orig, "Original location"), (x_date, "Deleted"), (x_size, "Size")] { ui.draw().text(x, hdr.y + 6.0, label, 12.0, t.fg_dim, true); }
        ui.separator(rect(hdr.x, hdr.bottom() - 1.0, hdr.w, 1.0));
        if !any {
            ui.label_ex(body, "Trash is empty", t.fg_dim, false, 0.5);
            return;
        }
        let lid = Id::new("trashlist");
        let rows_h = 32.0;
        ui.push_clip(body);
        let resp = ui.virt(lid, body, n, None, rows_h);
        let mut ev_ctx: Option<usize> = None;
        let mut restore_now = false;
        let (mut ev_sel, mut ev_cur): (Option<BTreeSet<usize>>, Option<usize>) = (None, None);
        {
            let v = self.trash.as_ref().unwrap();
            for ev in &resp.events {
                match ev {
                    ListEvent::Click { i, mods } => {
                        let mut s = if mods.ctrl { v.sel.clone() } else { BTreeSet::new() };
                        if mods.shift { let (a, b) = (v.cursor.min(*i), v.cursor.max(*i)); s.extend(a..=b); }
                        else if mods.ctrl && !s.remove(i) { s.insert(*i); } else if !mods.ctrl { s.insert(*i); }
                        ev_sel = Some(s); ev_cur = Some(*i);
                    }
                    ListEvent::RightClick { i } => { if !v.sel.contains(i) { ev_sel = Some([*i].into()); ev_cur = Some(*i); } ev_ctx = Some(*i); }
                    ListEvent::DoubleClick { i } => { ev_sel = Some([*i].into()); ev_cur = Some(*i); restore_now = true; }
                    ListEvent::ClickEmpty => ev_sel = Some(BTreeSet::new()),
                    ListEvent::Nav(nav, mods) => {
                        let last = n - 1;
                        let to = match nav { Nav::Up => v.cursor.saturating_sub(1), Nav::Down => (v.cursor + 1).min(last), Nav::PageUp => v.cursor.saturating_sub(resp.page_rows()), Nav::PageDown => (v.cursor + resp.page_rows()).min(last), Nav::Home => 0, Nav::End => last, _ => v.cursor };
                        let mut s: BTreeSet<usize> = if mods.shift { v.sel.clone() } else { BTreeSet::new() };
                        if mods.shift { let (a, b) = (v.cursor.min(to), v.cursor.max(to)); s.extend(a..=b); } else { s.insert(to); }
                        ev_sel = Some(s); ev_cur = Some(to);
                        ui.reveal(lid, to);
                    }
                    ListEvent::Activate => restore_now = true,
                    _ => {}
                }
            }
        }
        if let Some(s) = ev_sel { if let Some(v) = self.trash.as_mut() { v.sel = s; } }
        if let Some(c) = ev_cur { if let Some(v) = self.trash.as_mut() { v.cursor = c; } }
        let thumbs_on = self.prefs.get("thumbnails") != "off";
        for i in resp.first..resp.last {
            let rr = resp.cell_rect(i);
            let (sel, cur, size, name, orig, date, entry) = {
                let v = self.trash.as_ref().unwrap();
                let row = &v.rows[i];
                (v.sel.contains(&i), v.cursor == i, row.size, row.name.clone(), row.it.original.parent().unwrap_or(Path::new("/")).to_string_lossy().into_owned(), fmt_deleted(&row.it.deleted), row.entry.clone())
            };
            if sel { ui.draw().rect(rr, t.selection_bg); } else if resp.hovered == Some(i) { ui.draw().rect(rr, t.hover()); }
            if cur && !sel { ui.draw().stroke(rr.inset(1.0), t.accent, 3.0, 1.0); }
            let fg = if sel { t.selection_fg } else { t.fg };
            let dim = if sel { t.selection_fg } else { t.fg_dim };
            let ir = rect(rr.x + 10.0, rr.y + 4.0, 24.0, 24.0);
            if let Some(e) = &entry {
                let th = if thumbs_on && !e.is_dir && e.size < 400_000_000 { self.thumbs.get(&e.path) } else { None };
                if let Some(th) = th { self.icons.draw(ui, Some(th.img.clone()), ir); }
                else {
                    let mut ne = e.clone();
                    ne.name = name.clone();
                    if !self.icons.entry(ui, &ne, ir) { ui.icon2(icon_for(e.class), ir, dim, t.bg_alt); }
                }
            }
            ui.label(rect(rr.x + 40.0, rr.y, name_w, rows_h), &name, fg);
            ui.label(rect(x_orig, rr.y, orig_w, rows_h), &orig, dim);
            ui.label(rect(x_date, rr.y, w_date, rows_h), &date, dim);
            ui.label_ex(rect(x_size, rr.y, w_size, rows_h), &human_size(size), dim, false, 1.0);
        }
        ui.pop_clip();
        if self.thumbs.busy() || self.icons.busy() { ui.request_redraw(); }
        if let Some(i) = ev_ctx { let _ = i; self.ctx = Some((ui.mouse, Ctx::Trash)); }
        else if resp.rect.contains(ui.mouse) && ui.frame.press[1] && resp.hovered.is_none() { self.ctx = Some((ui.mouse, Ctx::Trash)); }
        if restore_now { let p = self.trash_picked(); self.trash_restore(p); }
    }

    pub(crate) fn trash_dialog(&mut self, ui: &mut Ui) {
        let Some(d) = self.trash_dlg.take() else { return };
        let t = ui.theme.clone();
        let enter = |ui: &mut Ui| ui.take_key(|k, _| k == Key::Enter);
        let esc = |ui: &mut Ui| ui.take_key(|k, _| k == Key::Escape);
        match d {
            TrashDlg::Delete(items) => {
                let body = ui.dialog_frame(460.0, 170.0, "Delete permanently?");
                ui.begin_overlay();
                ui.label(rect(body.x, body.y, body.w, 28.0), &format!("{} item(s) will be deleted for good.", items.len()), t.fg);
                ui.draw().text(body.x, body.y + 30.0, "This cannot be undone.", 12.0, t.warn, false);
                let by = body.bottom() - 40.0;
                let yes = ui.button(Id::new("td-yes"), rect(body.right() - 150.0, by, 150.0, 34.0), "Delete permanently", BtnStyle::Primary);
                let no = ui.button(Id::new("td-no"), rect(body.right() - 240.0, by, 80.0, 34.0), "Cancel", BtnStyle::Normal);
                ui.end_overlay();
                if yes || enter(ui) { self.trash_purge(items); } else if no || esc(ui) {} else { self.trash_dlg = Some(TrashDlg::Delete(items)); }
            }
            TrashDlg::Empty => {
                let all = trashbin::list_roots(&self.trash_roots);
                let bytes: u64 = self.trash.as_ref().map(|v| v.total).unwrap_or(0);
                let body = ui.dialog_frame(460.0, 170.0, "Empty trash?");
                ui.begin_overlay();
                ui.label(rect(body.x, body.y, body.w, 28.0), &format!("{} item(s), {} will be deleted for good.", all.len(), human_size(bytes)), t.fg);
                ui.draw().text(body.x, body.y + 30.0, "This cannot be undone.", 12.0, t.warn, false);
                let by = body.bottom() - 40.0;
                let yes = ui.button(Id::new("te-yes"), rect(body.right() - 130.0, by, 130.0, 34.0), "Empty trash", BtnStyle::Primary);
                let no = ui.button(Id::new("te-no"), rect(body.right() - 220.0, by, 80.0, 34.0), "Cancel", BtnStyle::Normal);
                ui.end_overlay();
                if yes || enter(ui) { self.trash_empty(); } else if no || esc(ui) {} else { self.trash_dlg = Some(TrashDlg::Empty); }
            }
            TrashDlg::Conflict { item, rest } => {
                let body = ui.dialog_frame(500.0, 180.0, "Name already exists");
                ui.begin_overlay();
                let name = item.original.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                ui.label(rect(body.x, body.y, body.w, 28.0), &format!("\"{name}\" already exists in the original folder."), t.fg);
                ui.draw().text(body.x, body.y + 30.0, "Nothing is overwritten. Restore it under a new name?", 12.0, t.fg_dim, false);
                let by = body.bottom() - 40.0;
                let yes = ui.button(Id::new("tc-yes"), rect(body.right() - 170.0, by, 170.0, 34.0), "Restore as new name", BtnStyle::Primary);
                let skip = ui.button(Id::new("tc-skip"), rect(body.right() - 250.0, by, 70.0, 34.0), "Skip", BtnStyle::Normal);
                ui.end_overlay();
                if yes || enter(ui) {
                    let dest = trashbin::free_name(&item.original);
                    match trashbin::restore_to(&item, &dest) { Ok(()) => self.status = format!("Restored as {}", dest.display()), Err(e) => self.status = format!("Restore failed: {e}") }
                    self.trash_restore(rest);
                } else if skip || esc(ui) { self.trash_restore(rest); } else { self.trash_dlg = Some(TrashDlg::Conflict { item, rest }); }
            }
        }
    }
}
