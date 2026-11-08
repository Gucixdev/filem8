//! 3D loaders (own glTF/GLB, OBJ, STL) and the software renderer.
use fm8_preview::model::{load_mesh, render};
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fm8-model-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn b64(b: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 { if i <= c.len() { s.push(T[(n >> (18 - 6 * i)) as usize & 63] as char) } else { s.push('=') } }
    }
    s
}

/// One triangle (indices u16, 3 floats positions), node translated by (10,0,0).
fn tri_bin() -> Vec<u8> {
    let mut b = vec![];
    for p in [[0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] { for v in p { b.extend(v.to_le_bytes()); } }
    for i in [0u16, 1, 2] { b.extend(i.to_le_bytes()); }
    b.extend([0, 0]); // pad to 4
    b
}
fn tri_json(uri: &str) -> String {
    let buf = if uri.is_empty() { r#"{"byteLength":44}"#.to_string() } else { format!(r#"{{"byteLength":44,"uri":"{uri}"}}"#) };
    format!(r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0,"translation":[10,0,0]}}],
    "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}},"indices":1,"material":0}}]}}],
    "materials":[{{"pbrMetallicRoughness":{{"baseColorFactor":[1,0,0,1]}}}}],
    "accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}},{{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"}}],
    "bufferViews":[{{"buffer":0,"byteOffset":0,"byteLength":36}},{{"buffer":0,"byteOffset":36,"byteLength":6}}],"buffers":[{buf}]}}"#)
}

#[test]
fn gltf_embedded_and_glb() {
    let d = tmp("gltf");
    let bin = tri_bin();
    std::fs::write(d.join("a.gltf"), tri_json(&format!("data:application/octet-stream;base64,{}", b64(&bin)))).unwrap();
    let m = load_mesh(&d.join("a.gltf")).unwrap();
    assert_eq!(m.triangles(), 1);
    assert_eq!(m.indices, vec![0, 1, 2]);
    assert_eq!(m.positions[1], [11.0, 0.0, 0.0], "node translation applied");
    assert_eq!(m.color, [1.0, 0.0, 0.0]);
    // GLB container
    let mut js = tri_json("").into_bytes();
    while js.len() % 4 != 0 { js.push(b' '); }
    let mut g = b"glTF".to_vec();
    g.extend(2u32.to_le_bytes());
    g.extend(((12 + 8 + js.len() + 8 + bin.len()) as u32).to_le_bytes());
    g.extend((js.len() as u32).to_le_bytes()); g.extend(b"JSON"); g.extend(&js);
    g.extend((bin.len() as u32).to_le_bytes()); g.extend(b"BIN\0"); g.extend(&bin);
    std::fs::write(d.join("b.glb"), g).unwrap();
    assert_eq!(load_mesh(&d.join("b.glb")).unwrap().triangles(), 1);
    // external buffer next to the file, and a refused escape
    std::fs::write(d.join("t.bin"), &bin).unwrap();
    std::fs::write(d.join("c.gltf"), tri_json("t.bin")).unwrap();
    assert_eq!(load_mesh(&d.join("c.gltf")).unwrap().triangles(), 1);
    std::fs::write(d.join("e.gltf"), tri_json("../t.bin")).unwrap();
    assert!(load_mesh(&d.join("e.gltf")).is_err());
    std::fs::write(d.join("bad.gltf"), "{ nope").unwrap();
    assert!(load_mesh(&d.join("bad.gltf")).is_err());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn render_is_shaded_and_transparent_outside() {
    let d = tmp("render");
    // tetrahedron (ASCII STL, 4 faces)
    let v = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut s = String::from("solid t\n");
    for f in [[0, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]] {
        s.push_str("facet normal 0 0 0\nouter loop\n");
        for i in f { s.push_str(&format!("vertex {} {} {}\n", v[i][0], v[i][1], v[i][2])); }
        s.push_str("endloop\nendfacet\n");
    }
    std::fs::write(d.join("t.stl"), s).unwrap();
    let m = load_mesh(&d.join("t.stl")).unwrap();
    assert_eq!(m.triangles(), 4);
    let img = render(&m, 0.6, 64);
    assert_eq!(img.len(), 64 * 64 * 4);
    assert_eq!(img[3], 0, "corner is transparent");
    let opaque: Vec<&[u8]> = img.chunks(4).filter(|p| p[3] == 255).collect();
    assert!(opaque.len() > 200, "model covers pixels: {}", opaque.len());
    let lums: std::collections::BTreeSet<u8> = opaque.iter().map(|p| p[0]).collect();
    assert!(lums.len() >= 2, "flat faces get different light levels");
    assert_ne!(render(&m, 0.0, 64), render(&m, 1.5, 64), "yaw changes the picture");
    let _ = std::fs::remove_dir_all(&d);
}
