//! `shot [DIR] OUT.png`: the folder view as the window draws it (list, thumbnail grid, properties window, preview pane), without a compositor.
use fm8_lz::{ui, App};
use lazyui::canvas::{Canvas, Caps};
use lazyui::gui::{png, raster::{Pixels, Raster}};
use lazyui::input::{Event, Key, Mods};
use std::path::PathBuf;

fn main() {
    let mut a = std::env::args().skip(1);
    let dir = PathBuf::from(a.next().expect("DIR"));
    let out = a.next().expect("OUT.png");
    let mut raster = Raster::system(14.0, None).expect("font");
    let (cols, rows) = (100usize, 30usize);
    let mut app = App::open(&dir);
    let mut ui = ui();
    app.show(&mut ui);
    let mut shot = |ui: &mut lazyui::ui::Ui, app: &mut App, name: &str| {
        for _ in 0..100 {
            for e in ui.take_events() {
                if std::env::var("SHOT_DEBUG").is_ok() { eprintln!("{} {} i={} n={}", e.id, e.kind, e.index, e.number); }
                app.handle(ui, &e);
            }
            ui.tick(0.05);
            app.handle(ui, &lazyui::ui::UiEvent::of("app", "poll", ""));
            let mut c = Canvas::new(cols as i32, rows as i32, Caps::FULL);
            ui.render(&mut c);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let mut c = Canvas::new(cols as i32, rows as i32, Caps::FULL);
        ui.render(&mut c);
        let mut px = Pixels::new(cols * raster.cw, rows * raster.ch, raster.default_bg);
        raster.paint(&mut px, None, &c);
        std::fs::write(name, png::encode(px.w as u32, px.h as u32, &px.rgba)).unwrap();
    };
    shot(&mut ui, &mut app, &format!("{out}-list.png"));
    ui.dispatch(&Event::Key(Key::Char('g'), Mods { ctrl: true, ..Mods::default() }));
    shot(&mut ui, &mut app, &format!("{out}-grid.png"));
    ui.dispatch(&Event::Key(Key::Char('i'), Mods { ctrl: true, ..Mods::default() }));
    shot(&mut ui, &mut app, &format!("{out}-props.png"));
    ui.dispatch(&Event::Key(Key::Esc, Mods::default()));
    ui.dispatch(&Event::Key(Key::Char('g'), Mods { ctrl: true, ..Mods::default() }));
    ui.dispatch(&Event::Key(Key::Char('p'), Mods { ctrl: true, ..Mods::default() }));
    shot(&mut ui, &mut app, &format!("{out}-pane-open.png"));
    ui.focus("files");
    for _ in 0..4 {
        ui.dispatch(&Event::Key(Key::Down, Mods::default()));
    }
    shot(&mut ui, &mut app, &format!("{out}-pane-text.png"));
    for _ in 0..2 {
        ui.dispatch(&Event::Key(Key::Down, Mods::default()));
    }
    shot(&mut ui, &mut app, &format!("{out}-pane-picture.png"));
    ui.dispatch(&Event::Key(Key::Char(','), Mods { ctrl: true, ..Mods::default() }));
    shot(&mut ui, &mut app, &format!("{out}-settings.png"));
    ui.dispatch(&Event::Key(Key::Char(','), Mods { ctrl: true, ..Mods::default() }));
    ui.dispatch(&Event::Key(Key::Char('t'), Mods { ctrl: true, ..Mods::default() }));
    shot(&mut ui, &mut app, &format!("{out}-tabs.png"));
}
