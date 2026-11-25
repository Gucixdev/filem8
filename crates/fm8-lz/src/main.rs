//! filem8 on lazyUI-rs in a native Wayland window: `fm8-lz [DIR]`. Follows finedesktop's theme when it runs.
use fm8_lz::{ui, App};
use lazyui::gui::{panel, raster::Raster};
use lazyui::wl::{self, seat::Seat, window::Window, wire};
use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let dir = std::env::args().nth(1).map(PathBuf::from).or_else(|| std::env::current_dir().ok()).unwrap_or_else(|| PathBuf::from("/"));
    let mut conn = wire::connect()?;
    let (reg, globals) = wl::registry(&mut conn)?;
    let mut raster = Raster::system(14.0, None).map_err(std::io::Error::other)?;
    let mut win = Window::open(&mut conn, reg, &globals, "filem8", "filem8", ((raster.cw * 100) as u32, (raster.ch * 30) as u32))?;
    let mut seat = Seat::bind(&mut conn, reg, &globals, raster.cw, raster.ch)?;
    let mut app = App::open(&dir);
    let mut ui = ui();
    app.show(&mut ui);
    let follow = panel::Follow::start();
    panel::run(&mut conn, &mut win, &mut seat, &mut ui, &mut raster, follow.as_ref(), &mut |ui, e| app.handle(ui, &e))?;
    win.close(&mut conn)
}
