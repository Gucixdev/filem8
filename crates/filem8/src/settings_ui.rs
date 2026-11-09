//! Settings panel (Ctrl+,): schema-driven sections of checkboxes / choices,
//! the column list and the shortcut editor. Every change is applied live and
//! written to `~/.config/filem8/settings` immediately.
use crate::app::*;
use fm8_ui::*;

#[derive(Clone, Copy)]
enum Kind { Bool, Choice(&'static [&'static str]), IconThemes }
struct Field { key: &'static str, label: &'static str, hint: &'static str, kind: Kind }

const VIEWS: &[&str] = &["list", "grid", "compact"];
const SORTS: &[&str] = &["name", "size", "modified", "type", "perms", "owner", "items"];
const SIZES: &[&str] = &["small", "medium", "large"];
const THEMES: &[&str] = &["file", "dark", "light"];
const BACKENDS: &[&str] = &["auto", "vulkan", "gl", "soft"];

const GENERAL: &[Field] = &[
    Field { key: "view", label: "Default view", hint: "for folders you have not changed", kind: Kind::Choice(VIEWS) },
    Field { key: "show_hidden", label: "Show hidden files", hint: "", kind: Kind::Bool },
    Field { key: "sort", label: "Sort by", hint: "", kind: Kind::Choice(SORTS) },
    Field { key: "sort_ascending", label: "Ascending order", hint: "", kind: Kind::Bool },
    Field { key: "confirm_trash", label: "Confirm before moving to trash", hint: "Ctrl+Z always undoes", kind: Kind::Bool },
    Field { key: "preview", label: "Preview panel available", hint: "F3 cycles single / dual / preview", kind: Kind::Bool },
];
const APPEARANCE: &[Field] = &[
    Field { key: "theme", label: "Theme", hint: "file = ~/.config/filem8/theme (colorado)", kind: Kind::Choice(THEMES) },
    Field { key: "icon_theme", label: "Icon theme", hint: "installed freedesktop themes; run `filem8 --get-icons` to fetch one", kind: Kind::IconThemes },
    Field { key: "thumb_size", label: "Icon / thumbnail size", hint: "grid view", kind: Kind::Choice(SIZES) },
    Field { key: "thumbnails", label: "Image thumbnails", hint: "grid view", kind: Kind::Bool },
];
const ADVANCED: &[Field] = &[
    Field { key: "backend", label: "Graphics backend", hint: "applies on next start (FM8_BACKEND overrides)", kind: Kind::Choice(BACKENDS) },
];

pub const ACTIONS: &[(&str, &str)] = &[
    ("new_tab", "New tab"), ("close_tab", "Close tab"), ("path", "Edit path"), ("hidden", "Toggle hidden files"),
    ("layout", "Cycle layout"), ("rename", "Rename"), ("copy", "Copy"), ("cut", "Cut"), ("paste", "Paste"),
    ("bookmark", "Bookmark this folder"), ("settings", "Settings"), ("reload", "Reload"),
];
const SECTIONS: &[&str] = &["General", "Columns", "Appearance", "Shortcuts", "Advanced"];

#[derive(Default)]
pub struct SettingsState {
    pub section: usize,
    pub capture: Option<&'static str>,
}

pub fn bind_text(k: Key, m: Mods) -> Option<String> {
    let name = match k {
        Key::Char(c) => c.to_ascii_lowercase().to_string(),
        Key::F(n) => format!("f{n}"),
        Key::Tab => "tab".into(), Key::Delete => "delete".into(), Key::Enter => "enter".into(), Key::Space => "space".into(),
        Key::Backspace => "backspace".into(), Key::Home => "home".into(), Key::End => "end".into(),
        _ => return None,
    };
    let mut s = String::new();
    if m.ctrl { s.push_str("ctrl+"); }
    if m.alt { s.push_str("alt+"); }
    if m.shift { s.push_str("shift+"); }
    s.push_str(&name);
    Some(s)
}

/// True when `(k, m)` is the shortcut written as `spec` (e.g. "ctrl+shift+t", "f2").
pub fn bind_matches(spec: &str, k: Key, m: Mods) -> bool {
    bind_text(k, m).map(|s| s == spec).unwrap_or(false)
}

fn checkbox(ui: &mut Ui, id: Id, r: Rect, on: bool) -> bool {
    let t = ui.theme.clone();
    let b = rect(r.x, r.y + (r.h - 20.0) / 2.0, 20.0, 20.0);
    let (clicked, hov, _) = ui.interact(id, r.inset_xy(0.0, 0.0));
    ui.draw().rounded(b, if on { t.accent } else if hov { t.hover() } else { t.bg }, 5.0);
    ui.draw().stroke(b, if on { t.accent } else { t.border }, 5.0, 1.0);
    if on {
        ui.draw().line((b.x + 5.0, b.y + 10.5), (b.x + 8.5, b.y + 14.0), 2.0, t.accent_fg);
        ui.draw().line((b.x + 8.5, b.y + 14.0), (b.x + 15.0, b.y + 6.0), 2.0, t.accent_fg);
    }
    clicked
}

/// Segmented choice; wraps onto more lines when it does not fit. Returns (picked, lines used).
fn choice(ui: &mut Ui, id: Id, r: Rect, opts: &[&str], cur: &str) -> (Option<usize>, usize) {
    let t = ui.theme.clone();
    let mut x = r.x;
    let mut line = 0usize;
    let mut out = None;
    for (i, o) in opts.iter().enumerate() {
        let w = fm8_ui::text::measure(o, t.font_size, false) + 22.0;
        if x > r.x && x + w > r.right() { x = r.x; line += 1; }
        let br = rect(x, r.y + 4.0 + line as f32 * 36.0, w, 28.0);
        let (clicked, hov, _) = ui.interact(id.with(i as u64), br);
        let on = *o == cur;
        ui.draw().rounded(br, if on { t.accent } else if hov { t.hover() } else { t.bg }, 6.0);
        if !on { ui.draw().stroke(br, t.border, 6.0, 1.0); }
        ui.label_ex(br, o, if on { t.accent_fg } else { t.fg }, false, 0.5);
        if clicked { out = Some(i); }
        x += w + 6.0;
    }
    (out, line + 1)
}

impl App {
    pub fn open_settings(&mut self) { self.settings = Some(SettingsState::default()); }

    pub(crate) fn settings_panel(&mut self, ui: &mut Ui) {
        let Some(mut st) = self.settings.take() else { return };
        let t = ui.theme.clone();
        let body = ui.dialog_frame(800.0, 540.0, "Settings");
        ui.begin_overlay();
        let (nav, content) = body.cut_left(170.0);
        let mut close = false;
        for (i, s) in SECTIONS.iter().enumerate() {
            let r = rect(nav.x, nav.y + i as f32 * 36.0, nav.w - 10.0, 32.0);
            let (clicked, hov, _) = ui.interact(Id::new("sset").with(i as u64), r);
            if st.section == i { ui.draw().rounded(r, t.selection_bg, t.radius); } else if hov { ui.draw().rounded(r, t.hover(), t.radius); }
            ui.label(r.inset_xy(12.0, 0.0), s, if st.section == i { t.selection_fg } else { t.fg });
            if clicked { st.section = i; st.capture = None; }
        }
        if ui.button(Id::new("sclose"), rect(nav.x, nav.bottom() - 34.0, nav.w - 10.0, 32.0), "Close (Esc)", BtnStyle::Normal) { close = true; }
        ui.draw().rect(rect(content.x - 8.0, content.y, 1.0, content.h), t.border);
        let content = content.inset_xy(10.0, 0.0);
        let fields: Option<&[Field]> = match st.section { 0 => Some(GENERAL), 2 => Some(APPEARANCE), 4 => Some(ADVANCED), _ => None };
        let mut changed: Option<&'static str> = None;
        if let Some(fields) = fields {
            let mut ry = content.y;
            for (i, f) in fields.iter().enumerate() {
                let row = rect(content.x, ry, content.w, 48.0);
                ry += 52.0;
                ui.label(rect(row.x, row.y + 2.0, 270.0, 24.0), f.label, t.fg);
                if !f.hint.is_empty() { ui.draw().text(row.x, row.y + 26.0, f.hint, 11.0, t.fg_dim, false); }
                let ctl = rect(row.x + 290.0, row.y, row.w - 290.0, 36.0);
                let cur = self.prefs.get(f.key);
                match f.kind {
                    Kind::Bool => if checkbox(ui, Id::new("sf").with(i as u64 + 100 * st.section as u64), ctl, cur == "true") { self.prefs.set(f.key, if cur == "true" { "false" } else { "true" }); changed = Some(f.key); },
                    Kind::Choice(_) | Kind::IconThemes => { let opts: &[&str] = match f.kind { Kind::Choice(o) => o, _ => crate::iconset::theme_choices() }; let (pick, lines) = choice(ui, Id::new("sf").with(i as u64 + 100 * st.section as u64), ctl, opts, &cur); ry += (lines as f32 - 1.0) * 36.0; if let Some(k) = pick { self.prefs.set(f.key, opts[k]); changed = Some(f.key); } }
                }
            }
        } else if st.section == 1 {
            ui.draw().text(content.x, content.y, "Columns of the list view. Drag a header edge in the list to resize.", 12.0, t.fg_dim, false);
            for i in 0..self.cols.len() {
                let row = rect(content.x, content.y + 28.0 + i as f32 * 40.0, content.w, 36.0);
                let on = self.cols[i].on;
                if checkbox(ui, Id::new("scol").with(i as u64), rect(row.x, row.y, 240.0, 36.0), on) { self.cols[i].on = !on; self.cols_dirty = true; }
                ui.label(rect(row.x + 32.0, row.y, 200.0, 36.0), crate::views::ColId::label(self.cols[i].id), t.fg);
                ui.draw().text(row.x + 260.0, row.y + 10.0, &format!("{} px", self.cols[i].w as u32), 12.0, t.fg_dim, false);
                let rid = Id::new("scolw").with(i as u64);
                if ui.button(rid, rect(row.x + 340.0, row.y + 3.0, 34.0, 30.0), "-", BtnStyle::Normal) { self.cols[i].w = (self.cols[i].w - 10.0).max(50.0); self.cols_dirty = true; }
                if ui.button(rid.with(1), rect(row.x + 380.0, row.y + 3.0, 34.0, 30.0), "+", BtnStyle::Normal) { self.cols[i].w = (self.cols[i].w + 10.0).min(420.0); self.cols_dirty = true; }
            }
            ui.draw().text(content.x, content.y + 28.0 + self.cols.len() as f32 * 40.0 + 6.0, "Name is always shown. Columns that do not fit are hidden.", 11.0, t.fg_dim, false);
        } else {
            ui.draw().text(content.x, content.y, "Click a shortcut, then press the new keys. Esc cancels.", 12.0, t.fg_dim, false);
            if let Some(a) = st.capture {
                let keys: Vec<(Key, Mods)> = std::mem::take(&mut ui.frame.keys);
                for (k, m) in keys {
                    if k == Key::Escape { st.capture = None; break; }
                    if let Some(s) = bind_text(k, m) {
                        let kk = format!("key.{a}");
                        self.prefs.set(&kk, &s);
                        st.capture = None;
                        changed = Some("key");
                        break;
                    }
                }
            }
            for (i, (act, label)) in ACTIONS.iter().enumerate() {
                let row = rect(content.x, content.y + 26.0 + i as f32 * 33.0, content.w, 30.0);
                ui.label(rect(row.x, row.y, 260.0, 30.0), label, t.fg);
                let cur = self.prefs.get(&format!("key.{act}"));
                let capturing = st.capture == Some(*act);
                if ui.button(Id::new("skey").with(i as u64), rect(row.x + 280.0, row.y, 180.0, 28.0), if capturing { "press keys..." } else { &cur }, if capturing { BtnStyle::Primary } else { BtnStyle::Normal }) { st.capture = Some(act); }
            }
            if ui.button(Id::new("skeyreset"), rect(content.x, content.bottom() - 34.0, 160.0, 30.0), "Reset shortcuts", BtnStyle::Normal) {
                for (act, _) in ACTIONS { let k = format!("key.{act}"); self.prefs.map.remove(&k); }
                changed = Some("key");
            }
        }
        ui.end_overlay();
        if ui.take_key(|k, _| k == Key::Escape) && st.capture.is_none() { close = true; }
        ui.frame.keys.clear();
        ui.frame.text.clear();
        if let Some(k) = changed { self.prefs_changed(k); }
        if !close { self.settings = Some(st); }
    }
}
