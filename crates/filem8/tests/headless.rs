use filem8::app::App;
use fm8_ui::*;
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("fm8-app-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    p
}
fn settle(app: &mut App) { for _ in 0..300 { app.tick(); if !app.tabs[app.active].model.loading { break; } std::thread::sleep(std::time::Duration::from_millis(5)); } app.tick(); }
fn frame(app: &mut App, ui: &mut Ui, t: f64, ev: &[Event]) -> soft::Framebuffer { render_frame(ui, 1000, 600, 1.0, t, ev, |ui| app.build(ui)) }
fn click(x: f32, y: f32) -> Vec<Event> { vec![Event::MouseMove(v2(x, y)), Event::MouseDown(Button::Left)] }

#[test]
fn navigate_tabs_keyboard_and_golden() {
    let d = tmp("nav");
    std::fs::create_dir_all(d.join("alpha/inner")).unwrap();
    std::fs::create_dir_all(d.join("beta")).unwrap();
    std::fs::write(d.join("a.txt"), "x").unwrap();
    let mut app = App::new(d.clone(), false);
    settle(&mut app);
    let mut ui = Ui::new(Theme::dark());
    let fb = frame(&mut app, &mut ui, 0.0, &[]);
    assert_eq!(app.tabs[0].model.view.len(), 3);
    // golden of the full window
    let g = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/window.png");
    // paths differ per run (temp name in breadcrumbs), so only compare the list area
    let crop = |f: &soft::Framebuffer| { let mut o = vec![]; for y in 90..300u32 { for x in 215..1000u32 { let c = f.pixel(x, y); o.extend([c.r, c.g, c.b, 255]); } } soft::Framebuffer { w: 785, h: 210, px: o } };
    if std::env::var("UPDATE_GOLDEN").is_ok() || !g.exists() { std::fs::create_dir_all(g.parent().unwrap()).unwrap(); crop(&fb).save_png(&g).unwrap(); }
    let gold = soft::Framebuffer::load_png(&g).unwrap();
    // modification times differ between runs: compare only left half (names, icons, selection)
    let left = |f: soft::Framebuffer| { let mut o = vec![]; for y in 0..f.h { for x in 0..190 { let c = f.pixel(x, y); o.extend([c.r, c.g, c.b, 255]); } } soft::Framebuffer { w: 190, h: f.h, px: o } };
    assert_eq!(left(crop(&fb)).diff(&left(gold), 2), 0, "list area differs from golden");

    // click row 0 (alpha) then Enter (via Activate on list focus) navigates in
    frame(&mut app, &mut ui, 0.1, &click(300.0, 129.0));
    frame(&mut app, &mut ui, 0.2, &[Event::MouseUp(Button::Left)]);
    assert_eq!(app.tabs[0].model.selection(), vec![d.join("alpha")]);
    frame(&mut app, &mut ui, 0.3, &[Event::Key(Key::Enter, Mods::default())]);
    settle(&mut app);
    assert_eq!(app.tabs[0].model.dir, std::fs::canonicalize(d.join("alpha")).unwrap());
    // Backspace goes back
    frame(&mut app, &mut ui, 0.4, &[Event::Key(Key::Backspace, Mods::default())]);
    settle(&mut app);
    assert_eq!(app.tabs[0].model.dir, std::fs::canonicalize(&d).unwrap());
    // arrows move the cursor
    frame(&mut app, &mut ui, 0.5, &[Event::Key(Key::Down, Mods::default())]);
    frame(&mut app, &mut ui, 0.6, &[Event::Key(Key::Down, Mods::default())]);
    assert_eq!(app.tabs[0].model.cursor, Some(1));
    // ctrl+t / ctrl+w
    let ctrl = Mods { ctrl: true, ..Default::default() };
    frame(&mut app, &mut ui, 0.7, &[Event::Key(Key::Char('t'), ctrl)]);
    assert_eq!(app.tabs.len(), 2);
    frame(&mut app, &mut ui, 0.8, &[Event::Key(Key::Char('w'), ctrl)]);
    assert_eq!(app.tabs.len(), 1);
    // ctrl+l path edit, type, enter
    frame(&mut app, &mut ui, 0.9, &[Event::Key(Key::Char('l'), ctrl)]);
    assert!(app.path_edit.is_some());
    frame(&mut app, &mut ui, 1.0, &[Event::Key(Key::Char('a'), ctrl), Event::Text(format!("{}/beta", d.display()))]);
    frame(&mut app, &mut ui, 1.1, &[Event::Key(Key::Enter, Mods::default())]);
    settle(&mut app);
    assert!(app.tabs[0].model.dir.ends_with("beta"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn hi_dpi_renders() {
    let d = tmp("dpi");
    std::fs::write(d.join("x"), "").unwrap();
    let mut app = App::new(d.clone(), false);
    settle(&mut app);
    let mut ui = Ui::new(Theme::dark());
    let fb = render_frame(&mut ui, 2000, 1200, 2.0, 0.0, &[], |ui| app.build(ui));
    assert_eq!((fb.w, fb.h), (2000, 1200));
    assert_ne!(fb.pixel(1200, 300), ui.theme.bg_alt);
    let _ = std::fs::remove_dir_all(&d);
}
