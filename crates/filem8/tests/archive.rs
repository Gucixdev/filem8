//! Archive engine with malicious and ordinary archives. Temp dirs only.
use filem8::archive::{self, Format, Limits, Prog};
use fm8_codec::inflate;
use std::path::{Path, PathBuf};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fm8-arch-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn lim() -> Limits { Limits { max_bytes: 64 << 20, max_files: 1000 } }

/// Hand-rolled zip with arbitrary member names. method 0 = stored, 8 = deflate; `fake_size` lies in the headers.
fn zip_of(members: &[(&str, &[u8], u16, Option<u32>)]) -> Vec<u8> {
    let (mut out, mut cd) = (vec![], vec![]);
    for (name, data, method, fake) in members {
        let body = if *method == 8 { inflate::deflate(data) } else { data.to_vec() };
        let crc = inflate::crc32(&[data]);
        let usize_ = fake.unwrap_or(data.len() as u32);
        let off = out.len() as u32;
        out.extend(b"PK\x03\x04"); out.extend(20u16.to_le_bytes()); out.extend(0x800u16.to_le_bytes()); out.extend(method.to_le_bytes());
        out.extend([0u8; 4]); out.extend(crc.to_le_bytes()); out.extend((body.len() as u32).to_le_bytes()); out.extend(usize_.to_le_bytes());
        out.extend((name.len() as u16).to_le_bytes()); out.extend(0u16.to_le_bytes()); out.extend(name.as_bytes()); out.extend(&body);
        cd.extend(b"PK\x01\x02"); cd.extend(20u16.to_le_bytes()); cd.extend(20u16.to_le_bytes()); cd.extend(0x800u16.to_le_bytes()); cd.extend(method.to_le_bytes());
        cd.extend([0u8; 4]); cd.extend(crc.to_le_bytes()); cd.extend((body.len() as u32).to_le_bytes()); cd.extend(usize_.to_le_bytes());
        cd.extend((name.len() as u16).to_le_bytes()); cd.extend([0u8; 12]); cd.extend(off.to_le_bytes()); cd.extend(name.as_bytes());
    }
    let cdo = out.len() as u32;
    out.extend(&cd);
    out.extend(b"PK\x05\x06"); out.extend([0u8; 4]); out.extend((members.len() as u16).to_le_bytes()); out.extend((members.len() as u16).to_le_bytes());
    out.extend((cd.len() as u32).to_le_bytes()); out.extend(cdo.to_le_bytes()); out.extend(0u16.to_le_bytes());
    out
}

/// Hand-rolled ustar: (name, type, link, data).
fn tar_of(members: &[(&str, u8, &str, &[u8])]) -> Vec<u8> {
    let mut t = vec![];
    for (name, ty, link, data) in members {
        let mut h = [0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..108].copy_from_slice(b"0000644\0");
        h[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        h[156] = *ty;
        h[157..157 + link.len()].copy_from_slice(link.as_bytes());
        h[257..263].copy_from_slice(b"ustar\0");
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|&b| b as u32).sum();
        h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        t.extend(h); t.extend(*data); t.extend(vec![0u8; (512 - data.len() % 512) % 512]);
    }
    t.extend(vec![0u8; 1024]);
    t
}
fn gz(d: &[u8]) -> Vec<u8> {
    let mut o = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    o.extend(inflate::deflate(d)); o.extend(inflate::crc32(&[d]).to_le_bytes()); o.extend((d.len() as u32).to_le_bytes());
    o
}
fn files_under(d: &Path) -> Vec<String> {
    let mut v = vec![];
    let mut st = vec![d.to_path_buf()];
    while let Some(p) = st.pop() { for e in std::fs::read_dir(&p).unwrap().flatten() { let q = e.path(); v.push(q.strip_prefix(d).unwrap().to_string_lossy().into_owned()); if q.is_dir() && !q.is_symlink() { st.push(q); } } }
    v.sort();
    v
}

#[test]
fn zip_slip_is_rejected_and_nothing_escapes() {
    let root = tmp("slip");
    let dest = root.join("out/inner");
    for (i, bad) in ["../evil.txt", "a/../../evil.txt", "/tmp/fm8-abs-evil.txt", "..\\..\\evil.txt", "x/../../../evil.txt"].iter().enumerate() {
        let z = root.join(format!("bad{i}.zip"));
        std::fs::write(&z, zip_of(&[("ok.txt", b"fine", 0, None), (bad, b"pwn", 0, None)])).unwrap();
        let e = archive::extract(&z, &dest, &lim(), &Prog::new()).unwrap_err();
        assert!(e.contains("unsafe"), "{bad}: {e}");
    }
    assert!(!root.join("out/evil.txt").exists() && !root.join("evil.txt").exists() && !Path::new("/tmp/fm8-abs-evil.txt").exists());
    // atomic: the "ok.txt" member before the bad one must not be left behind either
    assert!(files_under(&dest).is_empty(), "{:?}", files_under(&dest));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tar_symlink_attacks() {
    let root = tmp("link");
    let dest = root.join("out");
    let cases: Vec<(&str, Vec<(&str, u8, &str, &[u8])>)> = vec![
        ("abs", vec![("l", b'2', "/etc", b"")]),
        ("up", vec![("d/l", b'2', "../../x", b"")]),
        ("through", vec![("l", b'2', "..", b""), ("l/pwn.txt", b'0', "", b"pwn")]),
        // chain: l3 looks fine lexically (l1/..) but physically escapes via l1 -> sub/l2 -> ..
        ("chain", vec![("sub/", b'5', "", b""), ("sub/l2", b'2', "..", b""), ("l1", b'2', "sub/l2", b""), ("l3", b'2', "l1/..", b""), ("l3/pwn.txt", b'0', "", b"pwn")]),
    ];
    for (n, m) in cases {
        let t = root.join(format!("{n}.tar"));
        std::fs::write(&t, tar_of(&m)).unwrap();
        let r = archive::extract(&t, &dest, &lim(), &Prog::new());
        assert!(r.is_err(), "{n}: {r:?}");
        assert!(!root.join("pwn.txt").exists(), "{n}: escaped");
        assert!(files_under(&dest).is_empty(), "{n}: leftovers {:?}", files_under(&dest));
    }
    // a harmless relative link survives
    let t = root.join("ok.tar");
    std::fs::write(&t, tar_of(&[("a.txt", b'0', "", b"A"), ("l", b'2', "a.txt", b"")])).unwrap();
    let r = archive::extract(&t, &dest, &lim(), &Prog::new()).unwrap();
    assert_eq!(std::fs::read_link(dest.join("l")).unwrap(), Path::new("a.txt"));
    assert_eq!(r.files, 2);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn bombs_hit_the_limits() {
    let root = tmp("bomb");
    let dest = root.join("out");
    let big = vec![0u8; 20 << 20];
    let z = root.join("big.zip");
    std::fs::write(&z, zip_of(&[("zeros.bin", &big, 8, None)])).unwrap();
    assert!(std::fs::metadata(&z).unwrap().len() < 200_000, "tiny archive, huge content");
    let small = Limits { max_bytes: 1 << 20, max_files: 1000 };
    let e = archive::extract(&z, &dest, &small, &Prog::new()).unwrap_err();
    assert!(e.contains("zip bomb"), "{e}");
    // header lies about the size: the decoder is capped anyway
    let z2 = root.join("lie.zip");
    std::fs::write(&z2, zip_of(&[("zeros.bin", &big, 8, Some(10))])).unwrap();
    assert!(archive::extract(&z2, &dest, &lim(), &Prog::new()).is_err());
    // file-count bomb
    let names: Vec<String> = (0..300).map(|i| format!("f{i}")).collect();
    let members: Vec<(&str, &[u8], u16, Option<u32>)> = names.iter().map(|n| (n.as_str(), &b"x"[..], 0u16, None)).collect();
    let z3 = root.join("many.zip");
    std::fs::write(&z3, zip_of(&members)).unwrap();
    let e = archive::extract(&z3, &dest, &Limits { max_bytes: 1 << 30, max_files: 100 }, &Prog::new()).unwrap_err();
    assert!(e.contains("zip bomb"), "{e}");
    // tar.gz bomb
    let t = root.join("b.tar.gz");
    std::fs::write(&t, gz(&tar_of(&[("zeros", b'0', "", &big)]))).unwrap();
    assert!(archive::extract(&t, &dest, &small, &Prog::new()).unwrap_err().contains("zip bomb"));
    assert!(files_under(&dest).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn never_overwrites() {
    let root = tmp("nooverwrite");
    let dest = root.join("out");
    std::fs::create_dir_all(dest.join("d")).unwrap();
    std::fs::write(dest.join("a.txt"), "KEEP").unwrap();
    std::fs::write(dest.join("d/x"), "KEEP-D").unwrap();
    let z = root.join("a.zip");
    std::fs::write(&z, zip_of(&[("a.txt", b"NEW", 0, None), ("d/y", b"NEW-Y", 0, None)])).unwrap();
    let r = archive::extract(&z, &dest, &lim(), &Prog::new()).unwrap();
    assert_eq!(std::fs::read_to_string(dest.join("a.txt")).unwrap(), "KEEP");
    assert_eq!(std::fs::read_to_string(dest.join("a (2).txt")).unwrap(), "NEW");
    assert_eq!(std::fs::read_to_string(dest.join("d/x")).unwrap(), "KEEP-D");
    assert_eq!(std::fs::read_to_string(dest.join("d (2)/y")).unwrap(), "NEW-Y");
    assert!(r.warnings.iter().any(|w| w.contains("new name")), "{:?}", r.warnings);
    assert!(!files_under(&dest).iter().any(|f| f.contains(".fm8-extract")), "staging cleaned");
    let _ = std::fs::remove_dir_all(&root);
}

fn sample(root: &Path) -> Vec<PathBuf> {
    let d = root.join("src");
    std::fs::create_dir_all(d.join("proj/empty")).unwrap();
    std::fs::create_dir_all(d.join("proj/deep/er")).unwrap();
    std::fs::write(d.join("proj/readme.txt"), "hello hello hello hello hello hello").unwrap();
    std::fs::write(d.join("proj/deep/er/data.bin"), (0..50_000u32).map(|i| (i % 251) as u8).collect::<Vec<u8>>()).unwrap();
    let long = format!("proj/deep/{}.txt", "n".repeat(130));
    std::fs::write(d.join(&long), "long name").unwrap();
    std::fs::write(d.join("solo.sh"), "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(d.join("solo.sh"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    vec![d.join("proj"), d.join("solo.sh")]
}

fn same_tree(a: &Path, b: &Path) {
    assert_eq!(files_under(a), files_under(b));
    for f in files_under(a) { let (x, y) = (a.join(&f), b.join(&f)); if x.is_file() { assert_eq!(std::fs::read(&x).unwrap(), std::fs::read(&y).unwrap(), "{f}"); } }
}

#[test]
fn own_zip_and_targz_roundtrip() {
    let root = tmp("rt");
    let sel = sample(&root);
    for (fmt, name) in [(Format::Zip, "o.zip"), (Format::TarGz, "o.tar.gz")] {
        let out = root.join(name);
        let p = Prog::new();
        archive::compress(&sel, &out, fmt, &p).unwrap();
        assert_eq!(p.fraction(), 1.0);
        let dest = root.join(format!("x-{name}"));
        let r = archive::extract(&out, &dest, &lim(), &Prog::new()).unwrap();
        assert!(r.files >= 6, "{r:?}");
        // proj/ and solo.sh at the top level, content identical
        assert!(dest.join("proj/deep/er/data.bin").exists() && dest.join("proj/empty").is_dir());
        same_tree(&root.join("src/proj"), &dest.join("proj"));
        assert_eq!(std::fs::metadata(dest.join("solo.sh")).map(|m| std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111).unwrap_or(0), if fmt == Format::TarGz { 0o111 } else { 0 }, "{name}: exec bit");
        // refuses to overwrite its own output
        assert!(archive::compress(&sel, &out, fmt, &Prog::new()).is_err());
    }
    // our zip is readable by the real tools too
    if let Some(z) = archive::which("7z") {
        let o = std::process::Command::new(z).arg("t").arg(root.join("o.zip")).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    }
    if let Some(t) = archive::which("bsdtar") {
        assert!(std::process::Command::new(t).arg("-tf").arg(root.join("o.tar.gz")).output().unwrap().status.success());
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cancel_leaves_nothing() {
    let root = tmp("cancel");
    let z = root.join("a.zip");
    std::fs::write(&z, zip_of(&[("a", b"1", 0, None), ("b", b"2", 0, None)])).unwrap();
    let p = Prog::new();
    p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    let dest = root.join("out");
    assert_eq!(archive::extract(&z, &dest, &lim(), &p).unwrap_err(), "Cancelled");
    assert!(files_under(&dest).is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn detect_by_magic_for_odd_extensions() {
    let root = tmp("magic");
    let f = root.join("thing.tpz");
    std::fs::write(&f, gz(&tar_of(&[("a", b'0', "", b"x")]))).unwrap();
    assert_eq!(archive::detect(&f), Some(archive::Kind::TarGz));
    let r = archive::extract(&f, &root.join("o"), &lim(), &Prog::new()).unwrap();
    assert_eq!(r.files, 1);
    std::fs::write(root.join("plain.tpz"), "just text").unwrap();
    assert_eq!(archive::detect(&root.join("plain.tpz")), None);
    assert_eq!(archive::kind_of_name("x.TAR.XZ"), Some(archive::Kind::TarXz));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn tools_extract_and_stay_safe() {
    let root = tmp("tools");
    let sel = sample(&root);
    // 7z and tar.zst created through the tools, extracted again (skipped when the tools are absent)
    for (fmt, name, tool) in [(Format::SevenZ, "o.7z", "7z"), (Format::TarZst, "o.tar.zst", "zstd")] {
        if archive::which(tool).is_none() && !(tool == "7z" && archive::which("7za").is_some()) { continue; }
        if fmt == Format::TarZst && archive::which("bsdtar").is_none() && archive::which("tar").is_none() { continue; }
        let out = root.join(name);
        archive::compress(&sel, &out, fmt, &Prog::new()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let dest = root.join(format!("x-{name}"));
        archive::extract(&out, &dest, &lim(), &Prog::new()).unwrap_or_else(|e| panic!("{name}: {e}"));
        same_tree(&root.join("src/proj"), &dest.join("proj"));
    }
    // single-file xz stream
    if let Some(xz) = archive::which("xz") {
        let f = root.join("note.txt");
        std::fs::write(&f, "xz content").unwrap();
        assert!(std::process::Command::new(xz).arg("-k").arg(&f).status().unwrap().success());
        let dest = root.join("xz-out");
        archive::extract(&root.join("note.txt.xz"), &dest, &lim(), &Prog::new()).unwrap();
        assert_eq!(std::fs::read_to_string(dest.join("note.txt")).unwrap(), "xz content");
        // tar.xz carrying a path-traversal member and an escaping symlink: whatever the tool does, nothing escapes
        let t = root.join("evil.tar");
        std::fs::write(&t, tar_of(&[("../escaped.txt", b'0', "", b"pwn"), ("ok.txt", b'0', "", b"ok"), ("l", b'2', "/etc", b"")])).unwrap();
        assert!(std::process::Command::new(archive::which("xz").unwrap()).arg("-k").arg(&t).status().unwrap().success());
        let dest2 = root.join("evil-out/inner");
        let _ = archive::extract(&root.join("evil.tar.xz"), &dest2, &lim(), &Prog::new());
        assert!(!root.join("evil-out/escaped.txt").exists() && !root.join("escaped.txt").exists());
        if let Ok(rd) = std::fs::read_dir(&dest2) { for e in rd.flatten() { assert!(!e.path().is_symlink() || std::fs::read_link(e.path()).unwrap().is_relative(), "escaping symlink survived: {:?}", e.path()); } }
    }
    let _ = std::fs::remove_dir_all(&root);
}
