use std::path::{Path, PathBuf};

use super::{decode_png, load_image, load_model};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bayleaf-assets-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A 2 x 1 RGB PNG: red, then blue.
fn png_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(&[255, 0, 0, 0, 0, 255]).unwrap();
    writer.finish().unwrap();
    bytes
}

#[test]
fn png_images_become_rgba_and_broken_files_are_refused() {
    let image = decode_png(&png_bytes()).unwrap();
    assert_eq!((image.width, image.height), (2, 1));
    assert_eq!(image.rgba, [255, 0, 0, 255, 0, 0, 255, 255], "RGB gains an opaque alpha");
    assert!(decode_png(b"not a png").is_err());
    let mut cut = png_bytes();
    cut.truncate(cut.len() / 2);
    assert!(decode_png(&cut).is_err());

    let dir = temp_dir("png");
    std::fs::write(dir.join("a.png"), png_bytes()).unwrap();
    let first = load_image(&dir.join("a.png")).unwrap();
    assert_ne!(first.id, 0);
    std::fs::remove_dir_all(&dir).ok();
}

/// A one-triangle glTF model with its buffer in `tri.bin`, moved up by 2 by its node.
fn write_triangle(dir: &Path, buffer_uri: &str, index: u16) {
    let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]];
    let mut bin: Vec<u8> = positions.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
    for i in [0u16, 1, index] {
        bin.extend(i.to_le_bytes());
    }
    bin.extend([0, 0]);
    std::fs::write(dir.join("tri.bin"), &bin).unwrap();
    let gltf = serde_json::json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "translation": [0.0, 2.0, 0.0]}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
        "buffers": [{"uri": buffer_uri, "byteLength": bin.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 36},
            {"buffer": 0, "byteOffset": 36, "byteLength": 6}
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, -1], "max": [1, 0, 0]},
            {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
        ]
    });
    std::fs::write(dir.join("tri.gltf"), gltf.to_string()).unwrap();
}

#[test]
fn a_gltf_model_loads_with_its_node_transform_and_made_up_normals() {
    let dir = temp_dir("gltf");
    write_triangle(&dir, "tri.bin", 2);
    let mesh = load_model(&dir.join("tri.gltf")).unwrap();
    assert_eq!((mesh.positions.len(), mesh.indices.len()), (3, 3));
    assert!(mesh.positions.iter().all(|p| (p[1] - 2.0).abs() < 1e-6), "the node moves it up by 2");
    assert!(mesh.normals.iter().all(|n| n[1] > 0.99), "a flat triangle facing up: {:?}", mesh.normals);
    assert_eq!(mesh.min, [0.0, 2.0, -1.0]);
    assert_eq!(mesh.max, [1.0, 2.0, 0.0]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn hostile_models_are_refused_with_a_reason() {
    let dir = temp_dir("gltf-hostile");
    write_triangle(&dir, "tri.bin", 7);
    assert!(load_model(&dir.join("tri.gltf")).unwrap_err().contains("does not exist"), "an index past the last vertex");
    write_triangle(&dir, "../outside.bin", 2);
    assert!(load_model(&dir.join("tri.gltf")).unwrap_err().contains("next to the model"), "buffers outside the model's folder");
    write_triangle(&dir, "data:application/octet-stream;base64,AAAA", 2);
    assert!(load_model(&dir.join("tri.gltf")).unwrap_err().contains(".glb"));
    std::fs::write(dir.join("junk.glb"), b"glTF junk").unwrap();
    assert!(load_model(&dir.join("junk.glb")).is_err());
    assert!(load_model(&dir.join("missing.glb")).is_err());
    std::fs::remove_dir_all(&dir).ok();
}
