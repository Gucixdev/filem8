use std::io::Cursor;

#[test]
fn get_icons_shows_plan_and_does_nothing_without_yes() {
    let home = std::env::temp_dir().join(format!("fm8-geticons-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    for answer in ["\n", "n\n", ""] {
        let mut out = vec![];
        let code = filem8::geticons::run(&mut Cursor::new(answer.as_bytes().to_vec()), &mut out, &home, true);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(code, 0);
        assert!(text.contains("Would download") && text.contains("GPL-3.0") && text.contains("curl -fL"), "{text}");
        assert!(text.contains("Nothing was downloaded"));
        assert_eq!(std::fs::read_dir(&home).unwrap().count(), 0, "must not create anything without consent");
    }
    // already installed -> no prompt
    std::fs::create_dir_all(home.join("icons/Papirus")).unwrap();
    let mut out = vec![];
    filem8::geticons::run(&mut Cursor::new(b"y\n".to_vec()), &mut out, &home, true);
    assert!(String::from_utf8(out).unwrap().contains("already installed"));
    std::fs::remove_dir_all(&home).ok();
}
