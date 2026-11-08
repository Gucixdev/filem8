//! fm8-ui: a small immediate-mode widget toolkit with a CPU rasteriser
//! (software fallback / headless tests) and a backend-neutral display list.
pub mod draw;
pub mod geom;
pub mod icons;
pub mod soft;
pub mod text;
pub mod theme;
pub mod ui;

pub use draw::{Cmd, DrawList, Image};
pub use geom::*;
pub use icons::Icon;
pub use theme::Theme;
pub use ui::*;

/// Headless helper: run one UI frame and rasterise it with the CPU backend.
pub fn render_frame(ui: &mut Ui, w: u32, h: u32, scale: f32, time: f64, events: &[Event], build: impl FnOnce(&mut Ui)) -> soft::Framebuffer {
    ui.begin(v2(w as f32 / scale, h as f32 / scale), time, events);
    build(ui);
    let dl = ui.end();
    let mut fb = soft::Framebuffer::new(w, h, ui.theme.bg);
    fb.render(&dl, scale);
    fb
}
