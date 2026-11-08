use fm8_ui::*;

fn mv(x: f32, y: f32) -> Event { Event::MouseMove(v2(x, y)) }
fn run(ui: &mut Ui, t: f64, ev: &[Event], f: impl FnOnce(&mut Ui)) {
    ui.begin(v2(800.0, 600.0), t, ev);
    f(ui);
    ui.end();
}

#[test]
fn flex_layout() {
    let r = rect(0.0, 0.0, 300.0, 40.0);
    let c = r.row(10.0, &[Size::Fixed(50.0), Size::Flex(1.0), Size::Flex(2.0)]);
    assert_eq!(c[0].w, 50.0);
    assert!((c[1].w - 230.0 / 3.0).abs() < 1e-3 && (c[2].w - 460.0 / 3.0).abs() < 1e-3);
    assert!((c[2].right() - 300.0).abs() < 1e-3);
    let v = r.col(0.0, &[Size::Flex(1.0), Size::Flex(1.0)]);
    assert_eq!(v[1].y, 20.0);
}

#[test]
fn theme_parses_colorado_file() {
    let (t, errs) = Theme::parse("# colorado theme\nname = x\nvariant = light\nbg = #ffffff\naccent = #4f9cff\nfont_size = 12\ncolor1 = #ff0000\n");
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(t.bg, rgb(255, 255, 255));
    assert_eq!(t.accent, rgb(0x4f, 0x9c, 0xff));
    assert!(!t.dark);
    assert_eq!(t.font_size, 12.0);
    let (_, errs) = Theme::parse("bg = nope\nfont_size = abc\njunk");
    assert_eq!(errs.len(), 3);
}

#[test]
fn virtual_list_100k_is_cheap() {
    let mut ui = Ui::new(Theme::dark());
    let id = Id::new("l");
    let mut first_last = (0, 0);
    let start = std::time::Instant::now();
    for f in 0..200 {
        run(&mut ui, f as f64 * 0.016, &[mv(100.0, 100.0), Event::Wheel(5.0)], |ui| {
            let r = ui.virt(id, rect(0.0, 0.0, 400.0, 500.0), 100_000, None, 28.0);
            first_last = (r.first, r.last);
            assert!(r.last - r.first <= 20, "only visible rows");
        });
    }
    assert!(first_last.0 > 0, "scrolled");
    assert!(start.elapsed().as_millis() < 500, "200 frames of 100k rows took {:?}", start.elapsed());
    // scroll to the very end
    run(&mut ui, 100.0, &[Event::Wheel(1e7)], |ui| { ui.virt(id, rect(0.0, 0.0, 400.0, 500.0), 100_000, None, 28.0); });
    for f in 0..200 {
        run(&mut ui, 101.0 + f as f64 * 0.016, &[], |ui| {
            let r = ui.virt(id, rect(0.0, 0.0, 400.0, 500.0), 100_000, None, 28.0);
            first_last = (r.first, r.last);
        });
    }
    assert_eq!(first_last.1, 100_000);
}

#[test]
fn list_click_nav_and_grid() {
    let mut ui = Ui::new(Theme::dark());
    let id = Id::new("l");
    let r = rect(0.0, 0.0, 400.0, 300.0);
    let mut evs = vec![];
    run(&mut ui, 0.0, &[mv(50.0, 70.0), Event::MouseDown(Button::Left)], |ui| { evs = ui.virt(id, r, 50, None, 28.0).events; });
    assert_eq!(evs, vec![ListEvent::Click { i: 2, mods: Mods::default() }]);
    run(&mut ui, 0.1, &[Event::MouseUp(Button::Left), Event::Key(Key::Down, Mods::default())], |ui| { evs = ui.virt(id, r, 50, None, 28.0).events; });
    assert_eq!(evs, vec![ListEvent::Nav(Nav::Down, Mods::default())]);
    // grid: 4 columns of 100px
    let gid = Id::new("g");
    run(&mut ui, 0.2, &[mv(150.0, 10.0), Event::MouseDown(Button::Left)], |ui| {
        let resp = ui.virt(gid, rect(0.0, 0.0, 410.0, 300.0), 40, Some(100.0), 100.0);
        assert_eq!(resp.cols, 4);
        assert_eq!(resp.events, vec![ListEvent::Click { i: 1, mods: Mods::default() }]);
    });
}

#[test]
fn reveal_scrolls_into_view() {
    let mut ui = Ui::new(Theme::dark());
    let id = Id::new("l");
    run(&mut ui, 0.0, &[], |ui| { ui.virt(id, rect(0.0, 0.0, 400.0, 280.0), 1000, None, 28.0); });
    ui.reveal(id, 500);
    let mut resp_off = 0.0;
    for f in 1..120 {
        run(&mut ui, f as f64 * 0.016, &[], |ui| { resp_off = ui.virt(id, rect(0.0, 0.0, 400.0, 280.0), 1000, None, 28.0).offset; });
    }
    assert!((resp_off - (500.0 * 28.0 + 28.0 - 280.0)).abs() < 1.0, "{resp_off}");
}

#[test]
fn split_divider_drags() {
    let mut ui = Ui::new(Theme::dark());
    let id = Id::new("s");
    let area = rect(0.0, 0.0, 800.0, 400.0);
    let mut a = rect(0.0, 0.0, 0.0, 0.0);
    run(&mut ui, 0.0, &[mv(400.0, 100.0)], |ui| { a = ui.split(id, area, Axis::Horizontal, 0.5, 100.0).0; });
    assert!((a.w - 399.5).abs() < 1.0);
    run(&mut ui, 0.1, &[Event::MouseDown(Button::Left)], |ui| { ui.split(id, area, Axis::Horizontal, 0.5, 100.0); });
    run(&mut ui, 0.2, &[mv(200.0, 100.0)], |ui| { a = ui.split(id, area, Axis::Horizontal, 0.5, 100.0).0; });
    run(&mut ui, 0.3, &[Event::MouseUp(Button::Left)], |ui| { a = ui.split(id, area, Axis::Horizontal, 0.5, 100.0).0; });
    assert!((a.w - 199.5).abs() < 1.5, "{}", a.w);
    // min clamp
    run(&mut ui, 0.4, &[mv(200.0, 100.0), Event::MouseDown(Button::Left)], |ui| { ui.split(id, area, Axis::Horizontal, 0.5, 100.0); });
    run(&mut ui, 0.5, &[mv(5.0, 100.0)], |ui| { a = ui.split(id, area, Axis::Horizontal, 0.5, 100.0).0; });
    assert!(a.w >= 99.0, "min width respected: {}", a.w);
}

#[test]
fn text_input_edits() {
    let mut ui = Ui::new(Theme::dark());
    let id = Id::new("t");
    let r = rect(10.0, 10.0, 300.0, 30.0);
    let mut s = String::from("ab");
    run(&mut ui, 0.0, &[mv(50.0, 20.0), Event::MouseDown(Button::Left)], |ui| { ui.text_input(id, r, &mut s, ""); });
    run(&mut ui, 0.1, &[Event::MouseUp(Button::Left), Event::Key(Key::End, Mods::default()), Event::Text("ż!".into())], |ui| { ui.text_input(id, r, &mut s, ""); });
    assert_eq!(s, "abż!");
    run(&mut ui, 0.2, &[Event::Key(Key::Backspace, Mods::default()), Event::Key(Key::Backspace, Mods::default())], |ui| { ui.text_input(id, r, &mut s, ""); });
    assert_eq!(s, "ab");
    let mut resp = TextResp::default();
    run(&mut ui, 0.3, &[Event::Key(Key::Enter, Mods::default())], |ui| { resp = ui.text_input(id, r, &mut s, ""); });
    assert!(resp.submitted);
    // click elsewhere drops focus
    run(&mut ui, 0.4, &[mv(700.0, 500.0), Event::MouseDown(Button::Left)], |_| {});
    assert!(ui.focus.is_none());
}

#[test]
fn tabs_select_close_new() {
    let mut ui = Ui::new(Theme::dark());
    let id = Id::new("tabs");
    let titles: Vec<String> = vec!["one".into(), "two".into(), "three".into()];
    let r = rect(0.0, 0.0, 800.0, 36.0);
    let mut resp = TabsResp::default();
    run(&mut ui, 0.0, &[mv(300.0, 20.0), Event::MouseDown(Button::Left)], |ui| { resp = ui.tabs(id, r, &titles, 0, true); });
    assert_eq!(resp.select, Some(1));
    run(&mut ui, 0.1, &[Event::MouseUp(Button::Left), mv(300.0, 20.0), Event::MouseDown(Button::Middle)], |ui| { resp = ui.tabs(id, r, &titles, 0, true); });
    assert_eq!(resp.close, Some(1));
    let plus_x = 4.0 + 3.0 * 200.0 + 14.0;
    run(&mut ui, 0.2, &[Event::MouseUp(Button::Middle), mv(plus_x, 20.0), Event::MouseDown(Button::Left)], |ui| { resp = ui.tabs(id, r, &titles, 0, true); });
    run(&mut ui, 0.3, &[Event::MouseUp(Button::Left)], |ui| { resp = ui.tabs(id, r, &titles, 0, true); });
    assert!(resp.new);
}

#[test]
fn menu_choose_and_dismiss() {
    let mut ui = Ui::new(Theme::dark());
    let id = Id::new("m");
    let items = vec![MenuItem::new("Open", "Enter"), MenuItem::sep(), MenuItem::new("Delete", "Del"), MenuItem::new("Nope", "").disabled()];
    let mut r = MenuResp::Open;
    run(&mut ui, 0.0, &[mv(50.0, 50.0)], |ui| { r = ui.menu(id, v2(40.0, 40.0), &items); });
    assert_eq!(r, MenuResp::Open);
    run(&mut ui, 0.1, &[Event::Key(Key::Down, Mods::default()), Event::Key(Key::Down, Mods::default())], |ui| { r = ui.menu(id, v2(40.0, 40.0), &items); });
    run(&mut ui, 0.2, &[Event::Key(Key::Enter, Mods::default())], |ui| { r = ui.menu(id, v2(40.0, 40.0), &items); });
    assert_eq!(r, MenuResp::Chosen(2));
    run(&mut ui, 0.3, &[mv(700.0, 500.0), Event::MouseDown(Button::Left)], |ui| { r = ui.menu(id, v2(40.0, 40.0), &items); });
    assert_eq!(r, MenuResp::Dismiss);
}

#[test]
fn popup_blocks_widgets_beneath() {
    let mut ui = Ui::new(Theme::dark());
    let items = vec![MenuItem::new("A", ""), MenuItem::new("B", "")];
    let btn = rect(40.0, 40.0, 100.0, 30.0);
    run(&mut ui, 0.0, &[mv(60.0, 50.0)], |ui| { ui.menu(Id::new("m"), v2(40.0, 40.0), &items); });
    let mut clicked = false;
    run(&mut ui, 0.1, &[Event::MouseDown(Button::Left)], |ui| {
        clicked = ui.button(Id::new("b"), btn, "x", BtnStyle::Normal);
        ui.menu(Id::new("m"), v2(40.0, 40.0), &items);
    });
    run(&mut ui, 0.2, &[Event::MouseUp(Button::Left)], |ui| { clicked |= ui.button(Id::new("b"), btn, "x", BtnStyle::Normal); });
    assert!(!clicked);
}

fn scene(ui: &mut Ui) {
    let t = ui.theme.clone();
    let full = rect(0.0, 0.0, ui.size.x, ui.size.y);
    let (bar, rest) = full.cut_top(44.0);
    ui.draw().rect(bar, t.bg_alt);
    ui.button(Id::new("p"), rect(10.0, 7.0, 90.0, 30.0), "Primary", BtnStyle::Primary);
    ui.button(Id::new("n"), rect(110.0, 7.0, 90.0, 30.0), "Normal", BtnStyle::Normal);
    ui.icon_button(Id::new("i"), rect(210.0, 7.0, 30.0, 30.0), Icon::Back, "", true, true);
    let mut s = String::from("/home/user/projekty");
    ui.text_input(Id::new("t"), rect(250.0, 7.0, 220.0, 30.0), &mut s, "");
    let (side, main) = rest.cut_left(160.0);
    ui.draw().rect(side, t.bg_alt);
    ui.label(rect(side.x + 12.0, side.y + 8.0, 130.0, 24.0), "Zażółć gęślą jaźń", t.fg_dim);
    let l = ui.virt(Id::new("list"), main, 1000, None, 30.0);
    for i in l.first..l.last {
        let r = l.cell_rect(i);
        if i == 2 { ui.draw().rounded(r.inset_xy(4.0, 1.0), t.selection_bg, 6.0); }
        ui.icon2(if i % 2 == 0 { Icon::Folder } else { Icon::File }, rect(r.x + 10.0, r.y + 5.0, 20.0, 20.0), t.accent, t.bg);
        ui.label(rect(r.x + 40.0, r.y, 300.0, r.h), &format!("Item number {i}.txt"), t.fg);
    }
}

#[test]
fn headless_golden_scene() {
    let mut ui = Ui::new(Theme::dark());
    let fb = render_frame(&mut ui, 720, 360, 1.0, 0.0, &[], scene);
    // semantic pixel checks
    assert_eq!(fb.pixel(55, 22), ui.theme.accent, "primary button fill");
    assert_eq!(fb.pixel(700, 340), ui.theme.bg, "empty area is bg");
    assert!(fb.pixel(300, 100) != ui.theme.bg_alt, "list drew content");
    // golden
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/scene.png");
    if std::env::var("UPDATE_GOLDEN").is_ok() || !path.exists() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        fb.save_png(&path).unwrap();
    }
    let gold = soft::Framebuffer::load_png(&path).unwrap();
    assert_eq!(fb.diff(&gold, 2), 0, "scene differs from golden (UPDATE_GOLDEN=1 to refresh)");
}

#[test]
fn dpi_scaling_doubles_pixels() {
    let mut ui = Ui::new(Theme::dark());
    let fb1 = render_frame(&mut ui, 100, 50, 1.0, 0.0, &[], |ui| { ui.draw().rect(rect(10.0, 10.0, 20.0, 20.0), rgb(255, 0, 0)); });
    let fb2 = render_frame(&mut ui, 200, 100, 2.0, 0.0, &[], |ui| { ui.draw().rect(rect(10.0, 10.0, 20.0, 20.0), rgb(255, 0, 0)); });
    assert_eq!(fb1.pixel(15, 15), rgb(255, 0, 0));
    assert_eq!(fb2.pixel(25, 25), rgb(255, 0, 0));
    assert_ne!(fb2.pixel(15, 15), rgb(255, 0, 0));
}

#[test]
fn ellipsize_keeps_extension() {
    let s = text::ellipsize("a_very_long_file_name_here.tar.gz", 14.0, false, 150.0);
    assert!(s.contains('…') && s.ends_with("r.gz") || s.ends_with(".tar.gz") || s.ends_with("ar.gz"), "{s}");
    assert!(text::measure(&s, 14.0, false) <= 150.0);
}
