use fm8_codec::{inflate, tar, zip};
use std::io::Write;

fn sample() -> Vec<(String, Vec<u8>)> {
    vec![
        ("a.txt".into(), b"hello hello hello hello".to_vec()),
        ("dir/b.bin".into(), (0..5000u32).map(|i| (i * 7) as u8).collect()),
        ("dir/sub/łódź long name ".repeat(6) + ".txt", b"x".to_vec()),
        ("empty".into(), vec![]),
    ]
}

#[test]
fn zip_lists_and_extracts_stored_and_deflated() {
    let mut buf = std::io::Cursor::new(vec![]);
    {
        let mut w = ::zip::ZipWriter::new(&mut buf);
        for (i, (n, d)) in sample().iter().enumerate() {
            let m = if i % 2 == 0 { ::zip::CompressionMethod::Deflated } else { ::zip::CompressionMethod::Stored };
            w.start_file(n.as_str(), ::zip::write::SimpleFileOptions::default().compression_method(m)).unwrap();
            w.write_all(d).unwrap();
        }
        w.add_directory("dir/", ::zip::write::SimpleFileOptions::default()).unwrap();
        w.finish().unwrap();
    }
    let b = buf.into_inner();
    let (es, more) = zip::list(&b, 100).unwrap();
    assert!(!more);
    assert_eq!(es.len(), 5); // 4 files + dir entry
    for (n, d) in sample() {
        let e = es.iter().find(|e| e.name == n).unwrap_or_else(|| panic!("missing {n}"));
        assert_eq!(zip::read(&b, e, 1 << 20).unwrap(), d, "{n}");
    }
    assert!(es.iter().any(|e| e.is_dir));
    let (few, more) = zip::list(&b, 2).unwrap();
    assert!(more && few.len() == 2);
    // a.txt is deflated: flip a byte of its payload -> error (crc or inflate)
    let e = es.iter().find(|e| e.name == "a.txt").unwrap();
    let mut bad = b.clone();
    bad[30 + e.name.len() + 2] ^= 0x55;
    assert!(zip::read(&bad, e, 1 << 20).map(|x| x != sample()[0].1).unwrap_or(true));
    assert!(zip::list(b"not a zip at all, definitely", 10).is_err());
}

#[test]
fn tar_and_targz_listing_with_long_names() {
    let mut raw = vec![];
    {
        let mut t = ::tar::Builder::new(&mut raw);
        for (n, d) in sample() {
            let mut h = ::tar::Header::new_gnu();
            h.set_size(d.len() as u64);
            h.set_mode(0o644);
            t.append_data(&mut h, &n, &d[..]).unwrap();
        }
        t.finish().unwrap();
    }
    let (es, more) = tar::list(&raw, 100);
    assert!(!more);
    let names: Vec<_> = es.iter().map(|e| e.name.clone()).collect();
    for (n, d) in sample() {
        let e = es.iter().find(|e| e.name == n).unwrap_or_else(|| panic!("missing {n}; have {names:?}"));
        assert_eq!(tar::read(&raw, e).unwrap(), &d[..]);
    }
    let mut gz = flate2::write::GzEncoder::new(vec![], flate2::Compression::default());
    gz.write_all(&raw).unwrap();
    let gz = gz.finish().unwrap();
    let un = inflate::gunzip_prefix(&gz, 1 << 20).unwrap();
    assert_eq!(un, raw);
    assert_eq!(tar::list(&un, 2).0.len(), 2);
    assert!(inflate::gunzip_prefix(&gz, 600).unwrap().len() < raw.len());
}
