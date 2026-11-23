//! Own process: PATH is emptied, so tool-backed formats must fail with an install hint, not crash.
use filem8::archive::{self, Limits, Prog};

#[test]
fn missing_tools_give_install_hints() {
    let root = std::env::temp_dir().join(format!("fm8-arch-notools-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("empty-path")).unwrap();
    std::env::set_var("PATH", root.join("empty-path"));
    let lim = Limits::default();
    for (name, magic, what) in [("a.rar", &b"Rar!\x1a\x07\x00 payload"[..], "unrar"), ("a.7z", b"7z\xbc\xaf\x27\x1c\0\x04", "7zip"), ("a.tar.xz", b"\xfd7zXZ\0\0\x04", "tar"), ("a.zst", b"\x28\xb5\x2f\xfd\0\0", "zstd")] {
        let f = root.join(name);
        std::fs::write(&f, magic).unwrap();
        let e = archive::extract(&f, &root.join("out"), &lim, &Prog::new()).unwrap_err();
        assert!(e.contains("Install") && e.contains(what), "{name}: {e}");
    }
    assert!(archive::compress(&[root.join("a.rar")], &root.join("o.7z"), archive::Format::SevenZ, &Prog::new()).unwrap_err().contains("Install"));
    assert!(!root.join("o.7z").exists());
    let _ = std::fs::remove_dir_all(&root);
}
