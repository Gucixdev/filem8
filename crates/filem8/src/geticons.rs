//! `filem8 --get-icons`: show what would be downloaded, ask, then fetch an open
//! icon theme into ~/.local/share/icons with curl + tar. Nothing happens without a "y".
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Plan {
    pub name: &'static str,
    pub url: &'static str,
    /// Directory inside the tarball that is the theme (relative to the single top directory).
    pub subdir: &'static str,
    pub license: &'static str,
    pub needs_svg: bool,
    pub dest: PathBuf,
}

pub fn plan(data_home: &Path) -> Plan {
    Plan {
        name: "Papirus",
        url: "https://github.com/PapirusDevelopmentTeam/papirus-icon-theme/archive/refs/heads/master.tar.gz",
        subdir: "Papirus",
        license: "GPL-3.0 (free software; see LICENSE in the archive, copied next to the theme)",
        needs_svg: true,
        dest: data_home.join("icons").join("Papirus"),
    }
}

pub fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| fm8_core::paths::home().join(".local/share"))
}

/// The commands that will run, as shown to the user.
pub fn describe(p: &Plan, tmp: &Path) -> Vec<String> {
    vec![
        format!("curl -fL --progress-bar -o {}/theme.tar.gz {}", tmp.display(), p.url),
        format!("tar -xzf {}/theme.tar.gz -C {}", tmp.display(), tmp.display()),
        format!("move <archive>/{} -> {}", p.subdir, p.dest.display()),
    ]
}

fn ask(q: &str, input: &mut dyn BufRead, out: &mut dyn Write) -> bool {
    let _ = write!(out, "{q} [y/N] ");
    let _ = out.flush();
    let mut l = String::new();
    input.read_line(&mut l).is_ok() && matches!(l.trim().to_lowercase().as_str(), "y" | "yes")
}

/// Run the interactive flow; returns the process exit code.
pub fn run(input: &mut dyn BufRead, out: &mut dyn Write, data_home: &Path, can_svg: bool) -> i32 {
    let mut themes = fm8_core::icontheme::IconThemes::system();
    let have = themes.installed();
    let _ = writeln!(out, "Installed icon themes: {}", if have.is_empty() { "none".into() } else { have.join(", ") });
    let usable: Vec<&String> = have.iter().filter(|t| { let mut t2 = fm8_core::icontheme::IconThemes::system(); t2.usable(t, can_svg) }).collect();
    let _ = writeln!(out, "Usable by this build ({}): {}", if can_svg { "PNG + SVG" } else { "PNG only; build with FM8_FULL=1 for SVG" }, if usable.is_empty() { "none".into() } else { usable.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ") });
    let p = plan(data_home);
    if p.dest.exists() { let _ = writeln!(out, "{} is already installed at {}.", p.name, p.dest.display()); return 0 }
    let tmp = data_home.join("filem8-icons-download");
    let _ = writeln!(out, "\nWould download the open icon theme {}:\n  licence: {}\n  source:  {}\n  install: {}", p.name, p.license, p.url, p.dest.display());
    if p.needs_svg && !can_svg { let _ = writeln!(out, "  note: {} is SVG-only; this build cannot draw SVG (rebuild with FM8_FULL=1), it would be ignored.", p.name); }
    let _ = writeln!(out, "\nCommands:");
    for c in describe(&p, &tmp) { let _ = writeln!(out, "  {c}"); }
    if !ask("Download and install?", input, out) { let _ = writeln!(out, "Nothing was downloaded."); return 0 }
    if let Err(e) = fetch(&p, &tmp) { let _ = writeln!(out, "failed: {e}"); let _ = std::fs::remove_dir_all(&tmp); return 1 }
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = writeln!(out, "Installed {} in {}. Select it in Settings (Ctrl+,) -> Icon theme.", p.name, p.dest.display());
    0
}

fn fetch(p: &Plan, tmp: &Path) -> Result<(), String> {
    std::fs::create_dir_all(tmp).map_err(|e| e.to_string())?;
    let tgz = tmp.join("theme.tar.gz");
    let ok = Command::new("curl").args(["-fL", "--progress-bar", "-o"]).arg(&tgz).arg(p.url).status().map_err(|e| format!("curl: {e}"))?.success();
    if !ok { return Err("curl failed".into()) }
    if !Command::new("tar").arg("-xzf").arg(&tgz).arg("-C").arg(tmp).status().map_err(|e| format!("tar: {e}"))?.success() { return Err("tar failed".into()) }
    let top = std::fs::read_dir(tmp).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).find(|p| p.is_dir()).ok_or("empty archive")?;
    let src = top.join(p.subdir);
    if !src.join("index.theme").is_file() { return Err(format!("{} has no index.theme", src.display())) }
    std::fs::create_dir_all(p.dest.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::rename(&src, &p.dest).map_err(|e| e.to_string())?;
    for l in ["LICENSE", "COPYING"] { let f = top.join(l); if f.is_file() { let _ = std::fs::copy(&f, p.dest.join(l)); } }
    Ok(())
}
