//! Theme loaded from a plain `key = value` file. The format is the same as
//! colorado's `.theme` files (see docs/colorado-generator.md), so a colorado
//! theme can be used as-is; unknown keys are ignored.

use crate::geom::{rgb, Color};

#[derive(Clone, Debug)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    pub bg: Color,
    pub bg_alt: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub accent: Color,
    pub accent_fg: Color,
    pub border: Color,
    pub border_focus: Color,
    pub urgent: Color,
    pub warn: Color,
    pub ok: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub link: Color,
    pub font_size: f32,
    pub radius: f32,
    pub gap: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::dark()
    }
}

impl Theme {
    pub fn dark() -> Theme {
        Theme {
            name: "dark".into(),
            dark: true,
            bg: rgb(0x1b, 0x1d, 0x23),
            bg_alt: rgb(0x25, 0x28, 0x30),
            fg: rgb(0xe3, 0xe6, 0xed),
            fg_dim: rgb(0x9a, 0xa1, 0xb0),
            accent: rgb(0x5b, 0x9d, 0xff),
            accent_fg: rgb(0x0b, 0x12, 0x20),
            border: rgb(0x3a, 0x3e, 0x49),
            border_focus: rgb(0x5b, 0x9d, 0xff),
            urgent: rgb(0xf0, 0x6a, 0x6a),
            warn: rgb(0xe8, 0xc0, 0x5a),
            ok: rgb(0x6c, 0xcf, 0x8a),
            selection_bg: rgb(0x2f, 0x4a, 0x78),
            selection_fg: rgb(0xff, 0xff, 0xff),
            link: rgb(0x7f, 0xb4, 0xff),
            font_size: 14.0,
            radius: 6.0,
            gap: 6.0,
        }
    }

    pub fn light() -> Theme {
        Theme {
            name: "light".into(),
            dark: false,
            bg: rgb(0xfa, 0xfa, 0xfb),
            bg_alt: rgb(0xee, 0xef, 0xf2),
            fg: rgb(0x1d, 0x20, 0x27),
            fg_dim: rgb(0x5d, 0x64, 0x72),
            accent: rgb(0x1f, 0x6f, 0xeb),
            accent_fg: rgb(0xff, 0xff, 0xff),
            border: rgb(0xd0, 0xd3, 0xda),
            border_focus: rgb(0x1f, 0x6f, 0xeb),
            urgent: rgb(0xc6, 0x28, 0x28),
            warn: rgb(0x9a, 0x6b, 0x00),
            ok: rgb(0x1b, 0x7a, 0x3d),
            selection_bg: rgb(0xcf, 0xe0, 0xfb),
            selection_fg: rgb(0x10, 0x14, 0x1c),
            link: rgb(0x1a, 0x5f, 0xd0),
            font_size: 14.0,
            radius: 6.0,
            gap: 6.0,
        }
    }

    /// Parse theme text. Starts from the dark/light default (per `variant`)
    /// so partial files work. Returns the theme plus a list of problems.
    pub fn parse(text: &str) -> (Theme, Vec<String>) {
        let light = text.lines().any(|l| {
            let l = l.trim();
            l.starts_with("variant") && l.contains("light")
        });
        let mut t = if light { Theme::light() } else { Theme::dark() };
        let mut errs = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                errs.push(format!("line {}: expected key = value", n + 1));
                continue;
            };
            let (k, v) = (k.trim(), v.trim());
            let col = |slot: &mut Color, errs: &mut Vec<String>| match Color::parse(v) {
                Some(c) => *slot = c,
                None => errs.push(format!("line {}: bad colour '{}'", n + 1, v)),
            };
            match k {
                "name" => t.name = v.to_string(),
                "variant" => t.dark = v != "light",
                "bg" => col(&mut t.bg, &mut errs),
                "bg_alt" => col(&mut t.bg_alt, &mut errs),
                "fg" => col(&mut t.fg, &mut errs),
                "fg_dim" => col(&mut t.fg_dim, &mut errs),
                "accent" => col(&mut t.accent, &mut errs),
                "accent_fg" => col(&mut t.accent_fg, &mut errs),
                "border" => col(&mut t.border, &mut errs),
                "border_focus" => col(&mut t.border_focus, &mut errs),
                "urgent" => col(&mut t.urgent, &mut errs),
                "warn" => col(&mut t.warn, &mut errs),
                "ok" => col(&mut t.ok, &mut errs),
                "selection_bg" => col(&mut t.selection_bg, &mut errs),
                "selection_fg" => col(&mut t.selection_fg, &mut errs),
                "link" => col(&mut t.link, &mut errs),
                "font_size" | "radius" | "gap" => match v.parse::<f32>() {
                    Ok(f) if f >= 0.0 => match k {
                        "font_size" => t.font_size = f.clamp(8.0, 40.0),
                        "radius" => t.radius = f,
                        _ => t.gap = f,
                    },
                    _ => errs.push(format!("line {}: bad number '{}'", n + 1, v)),
                },
                _ => {}
            }
        }
        (t, errs)
    }

    pub fn load(path: &std::path::Path) -> Result<Theme, String> {
        let s = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let (t, errs) = Theme::parse(&s);
        if errs.is_empty() { Ok(t) } else { Err(errs.join("; ")) }
    }

    /// Hover / pressed tints derived from fg so any theme works.
    pub fn hover(&self) -> Color {
        self.bg_alt.mix(self.fg, 0.08)
    }
    pub fn pressed(&self) -> Color {
        self.bg_alt.mix(self.fg, 0.16)
    }
}
