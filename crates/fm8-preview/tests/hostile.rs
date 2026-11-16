//! glTF from an untrusted folder: no hang, no abort, no mesh that crashes the renderer.
use fm8_preview::model;
use std::path::PathBuf;

fn gltf(tag: &str, json: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fm8-hostile-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join(format!("{tag}.gltf"));
    std::fs::write(&p, json).unwrap();
    p
}

/// Run on a thread so a hang fails the test instead of blocking it.
fn load(p: PathBuf) -> Result<model::Mesh, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || { let _ = tx.send(model::load_mesh(&p)); });
    rx.recv_timeout(std::time::Duration::from_secs(10)).expect("glTF loader hung or died")
}

// 9 floats = one triangle, as a base64 data URI
const TRI: &str = "data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAA";

#[test]
fn node_cycle_is_walked_once() {
    // every node lists itself four times: 4^64 visits without a visited set
    let p = gltf("cycle", &format!(r#"{{"buffers":[{{"uri":"{TRI}"}}],"bufferViews":[{{"buffer":0}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":3}}],
        "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}],"nodes":[{{"mesh":0,"children":[0,0,0,0]}}],"scenes":[{{"nodes":[0]}}]}}"#));
    let m = load(p).unwrap();
    assert_eq!((m.positions.len(), m.triangles()), (3, 1));
    // without a mesh nothing stopped the walk at all
    assert_eq!(load(gltf("cycle2", r#"{"nodes":[{"children":[0,0,0,0]}],"scenes":[{"nodes":[0]}]}"#)).unwrap().triangles(), 0);
}

#[test]
fn accessor_count_larger_than_the_buffer_is_skipped() {
    // count 1e12 asked for a 24 TB vector (allocation failure aborts the process)
    let p = gltf("count", &format!(r#"{{"buffers":[{{"uri":"{TRI}"}}],"bufferViews":[{{"buffer":0}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":1000000000000}}],
        "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}],"nodes":[{{"mesh":0}}],"scenes":[{{"nodes":[0]}}]}}"#));
    assert_eq!(load(p).unwrap().triangles(), 0);
}

#[test]
fn indices_without_vertices_do_not_reach_the_renderer() {
    let p = gltf("novert", &format!(r#"{{"buffers":[{{"uri":"{TRI}"}}],"bufferViews":[{{"buffer":0}}],
        "accessors":[{{"bufferView":0,"componentType":5126,"count":0}},{{"bufferView":0,"componentType":5121,"count":3}}],
        "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"indices":1}}]}}],"nodes":[{{"mesh":0}}],"scenes":[{{"nodes":[0]}}]}}"#));
    let m = load(p).unwrap();
    assert!(m.indices.iter().all(|&i| (i as usize) < m.positions.len()), "index past the vertex list");
    model::render(&m, 0.3, 32);
}

fn file(name: &str, body: &[u8]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fm8-hostile-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

fn preview(p: PathBuf) -> fm8_preview::Preview {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || { let _ = tx.send(fm8_preview::load(&p, &fm8_preview::Opts::default())); });
    rx.recv_timeout(std::time::Duration::from_secs(10)).expect("preview hung or died")
}

#[test]
fn tiled_map_with_a_size_that_overflows_is_an_error() {
    let p = file("map.json", br#"{"width":65536,"height":65536,"tilewidth":1,"tileheight":1,"layers":[{"type":"tilelayer","data":[1]}]}"#);
    assert!(matches!(preview(p), fm8_preview::Preview::Error(_)));
}

#[test]
fn atlas_frames_are_clipped_and_the_image_stays_in_the_folder() {
    let png = fm8_codec::png::encode(&fm8_codec::Image::new(4, 4));
    file("sheet.png", &png);
    // a frame 10^15 pixels wide was outlined one pixel at a time
    let p = file("atlas.json", br#"{"frames":[{"filename":"a","frame":{"x":0,"y":0,"w":1000000000000000,"h":1000000000000000}}],"meta":{"image":"sheet.png"}}"#);
    assert!(matches!(preview(p), fm8_preview::Preview::Image(_)));
    // meta.image is a name next to the JSON, not a way to make the previewer read /dev/zero
    for img in ["/dev/zero", "../../../../../../dev/zero"] {
        let p = file("atlas2.json", format!(r#"{{"frames":[{{"filename":"a","frame":{{"x":0,"y":0,"w":1,"h":1}}}}],"meta":{{"image":"{img}"}}}}"#).as_bytes());
        assert!(matches!(preview(p), fm8_preview::Preview::Error(_)), "{img}");
    }
}

#[test]
fn a_fifo_with_an_image_name_is_not_opened() {
    let p = file("x", b"").with_file_name("pipe.png");
    let _ = std::fs::remove_file(&p);
    let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { mkfifo(c.as_ptr(), 0o600) }, 0);
    assert!(matches!(preview(p), fm8_preview::Preview::Info(_)));
}

extern "C" { fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32; }
