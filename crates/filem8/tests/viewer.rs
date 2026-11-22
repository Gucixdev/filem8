//! The viewer built into the window: Space opens it full window, r rotates, n walks the folder, Esc returns.
use filem8::app::App;
use fm8_ui::*;
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("fm8-view-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    p
}
fn frame(app: &mut App, ui: &mut Ui, t: &mut f64, ev: &[Event]) -> soft::Framebuffer {
    *t += 0.05;
    render_frame(ui, 1000, 600, 1.0, *t, ev, |ui| app.build(ui))
}
fn key(k: Key) -> Vec<Event> { vec![Event::Key(k, Mods::default())] }
fn near(c: Color, r: u8, g: u8, b: u8) -> bool { (c.r as i32 - r as i32).abs() < 12 && (c.g as i32 - g as i32).abs() < 12 && (c.b as i32 - b as i32).abs() < 12 }

#[test]
fn space_views_r_rotates_n_walks_esc_returns() {
    let d = tmp("flow");
    // 64x32: red left half, blue right half
    let mut px = vec![];
    for _y in 0..32 { for x in 0..64 { px.extend(if x < 32 { [255u8, 0, 0, 255] } else { [0, 0, 255, 255] }); } }
    soft::Framebuffer { w: 64, h: 32, px }.save_png(&d.join("a.png")).unwrap();
    std::fs::write(d.join("b.txt"), "hello viewer\nsecond line\n").unwrap();

    let mut app = App::new(d.clone(), false);
    for _ in 0..300 { app.tick(); if !app.tabs[app.active].model.loading { break; } std::thread::sleep(std::time::Duration::from_millis(5)); }
    app.tabs[0].model.cursor = Some(0); // a.png
    let mut ui = Ui::new(Theme::dark());
    let mut t = 0.0;
    let list = frame(&mut app, &mut ui, &mut t, &[]);
    let bg = Theme::dark().bg;
    assert_ne!(list.pixel(100, 300), bg, "the sidebar is there before the viewer opens");

    frame(&mut app, &mut ui, &mut t, &key(Key::Space));
    // the file loads on a worker thread
    let mut fb = frame(&mut app, &mut ui, &mut t, &[]);
    for _ in 0..200 { if near(fb.pixel(484, 287), 255, 0, 0) { break; } std::thread::sleep(std::time::Duration::from_millis(10)); fb = frame(&mut app, &mut ui, &mut t, &[]); }
    assert!(near(fb.pixel(484, 287), 255, 0, 0), "left half is red, got {:?}", fb.pixel(484, 287));
    assert!(near(fb.pixel(516, 287), 0, 0, 255), "right half is blue");
    assert_eq!(fb.pixel(100, 300), bg, "the viewer replaces the file list");

    // clockwise quarter turn: red goes on top
    let fb = frame(&mut app, &mut ui, &mut t, &key(Key::Char('r')));
    let fb = if near(fb.pixel(500, 271), 255, 0, 0) { fb } else { frame(&mut app, &mut ui, &mut t, &[]) };
    assert!(near(fb.pixel(500, 271), 255, 0, 0) && near(fb.pixel(500, 303), 0, 0, 255), "rotated: red above blue");

    if let Ok(dir) = std::env::var("FM8_SHOT_DIR") { fb.save_png(&std::path::Path::new(&dir).join("app_view_rot.png")).unwrap(); }

    // n: the text file replaces the picture
    frame(&mut app, &mut ui, &mut t, &key(Key::Char('n')));
    let mut fb = frame(&mut app, &mut ui, &mut t, &[]);
    for _ in 0..200 { if !near(fb.pixel(500, 271), 255, 0, 0) && !near(fb.pixel(500, 303), 0, 0, 255) { break; } std::thread::sleep(std::time::Duration::from_millis(10)); fb = frame(&mut app, &mut ui, &mut t, &[]); }
    assert!(!near(fb.pixel(500, 271), 255, 0, 0) && !near(fb.pixel(500, 303), 0, 0, 255), "the picture is gone");
    if let Ok(dir) = std::env::var("FM8_SHOT_DIR") { fb.save_png(&std::path::Path::new(&dir).join("app_view_txt.png")).unwrap(); }

    // Esc: back to the file list
    let fb = frame(&mut app, &mut ui, &mut t, &key(Key::Escape));
    let fb = frame(&mut app, &mut ui, &mut t, &[]).diff(&fb, 0).eq(&0).then_some(fb).unwrap_or_else(|| frame(&mut app, &mut ui, &mut t, &[]));
    assert_ne!(fb.pixel(100, 300), bg, "sidebar is back");
}

fn wait_for(app: &mut App, ui: &mut Ui, t: &mut f64, ok: impl Fn(&soft::Framebuffer) -> bool) -> soft::Framebuffer {
    let mut fb = frame(app, ui, t, &[]);
    for _ in 0..300 { if ok(&fb) { break; } std::thread::sleep(std::time::Duration::from_millis(10)); fb = frame(app, ui, t, &[]); }
    fb
}

#[test]
fn split_view_follows_the_cursor_and_has_buttons() {
    let d = tmp("split");
    let mut px = vec![];
    for _y in 0..32 { for x in 0..64 { px.extend(if x < 32 { [255u8, 0, 0, 255] } else { [0, 0, 255, 255] }); } }
    soft::Framebuffer { w: 64, h: 32, px }.save_png(&d.join("a.png")).unwrap();
    let mut px = vec![];
    for _ in 0..(64 * 32) { px.extend([0u8, 255, 0, 255]); }
    soft::Framebuffer { w: 64, h: 32, px }.save_png(&d.join("b.png")).unwrap();

    let mut app = App::new(d.clone(), false);
    for _ in 0..300 { app.tick(); if !app.tabs[app.active].model.loading { break; } std::thread::sleep(std::time::Duration::from_millis(5)); }
    app.layout = filem8::app::Layout::Preview;
    app.tabs[0].model.cursor = Some(0);
    let mut ui = Ui::new(Theme::dark());
    let mut t = 0.0;
    // the right pane is x 580..1000 (split 0.58 of the area right of the 214px sidebar) - find the picture by colour
    let find = |fb: &soft::Framebuffer, f: &dyn Fn(Color) -> bool| -> Option<(u32, u32)> {
        for y in 60..560u32 { for x in 640..1000u32 { if f(fb.pixel(x, y)) { return Some((x, y)); } } }
        None
    };
    let red = |c: Color| near(c, 255, 0, 0);
    let green = |c: Color| near(c, 0, 255, 0);
    let fb = wait_for(&mut app, &mut ui, &mut t, |fb| find(fb, &red).is_some());
    assert!(find(&fb, &red).is_some(), "a.png shows in the split");
    // the list is still there next to it
    let bg = Theme::dark().bg;
    assert!((100..200u32).any(|y| (230..560u32).any(|x| fb.pixel(x, y) != bg)), "the file list is still there next to it");
    if let Ok(dir) = std::env::var("FM8_SHOT_DIR") { fb.save_png(&std::path::Path::new(&dir).join("app_split.png")).unwrap(); }

    // cursor down: the split follows
    app.tabs[0].model.cursor = Some(1);
    let fb = wait_for(&mut app, &mut ui, &mut t, |fb| find(fb, &green).is_some());
    assert!(find(&fb, &green).is_some() && find(&fb, &red).is_none(), "b.png replaced a.png");

    // plain keys over the viewer half are the viewer's: r turns it a quarter
    let before = find(&fb, &green).unwrap();
    frame(&mut app, &mut ui, &mut t, &[Event::MouseMove(v2(800.0, 300.0))]);
    frame(&mut app, &mut ui, &mut t, &key(Key::Char('r')));
    let fb = frame(&mut app, &mut ui, &mut t, &[]);
    assert_ne!(before, find(&fb, &green).unwrap(), "a 64x32 picture turned a quarter occupies other pixels");

    // the Full button (first in the row) maximises: the sidebar is gone
    let list = frame(&mut app, &mut ui, &mut t, &[]);
    assert_ne!(list.pixel(100, 300), Theme::dark().bg);
    frame(&mut app, &mut ui, &mut t, &[Event::MouseMove(v2(692.0, 557.0)), Event::MouseDown(Button::Left)]);
    frame(&mut app, &mut ui, &mut t, &[Event::MouseUp(Button::Left)]);
    let fb = frame(&mut app, &mut ui, &mut t, &[]);
    assert_eq!(fb.pixel(100, 300), Theme::dark().bg, "full window viewer replaced the file manager");
}
