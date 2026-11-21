use std::path::{Path, PathBuf};

use super::{Image, decode_image, decode_png, load_image, load_model, mip_levels};

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

/// A 16 x 8 grey JPEG, written by hand: one channel, quantisation of 1, and Huffman tables with
/// one code each (DC difference 0, end of block), so every pixel is the middle grey 128.
fn grey_jpeg(width: u16) -> Vec<u8> {
    let mut b = vec![0xFF, 0xD8];
    b.extend([0xFF, 0xDB, 0x00, 0x43, 0x00]);
    b.extend([1u8; 64]);
    b.extend([0xFF, 0xC0, 0x00, 0x0B, 8, 0x00, 0x08]);
    b.extend(width.to_be_bytes());
    b.extend([1, 1, 0x11, 0]);
    for class in [0x00, 0x10] {
        b.extend([0xFF, 0xC4, 0x00, 0x14, class, 1]);
        b.extend([0u8; 15]);
        b.push(0x00);
    }
    b.extend([0xFF, 0xDA, 0x00, 0x08, 1, 1, 0x00, 0, 63, 0]);
    // Two blocks of "DC 0, end of block" (bits 00 00), padded with ones.
    b.extend([0x0F, 0xFF, 0xD9]);
    b
}

#[test]
fn jpeg_images_decode_by_their_bytes_and_oversized_ones_are_refused() {
    let image = decode_image(&grey_jpeg(16)).unwrap();
    assert_eq!((image.width, image.height), (16, 8));
    assert!(image.rgba.as_chunks::<4>().0.iter().all(|p| p[0].abs_diff(128) <= 2 && p[3] == 255), "grey and opaque");
    assert!(decode_image(&grey_jpeg(9000)).is_err(), "wider than the limit");

    let dir = temp_dir("jpeg");
    // The kind comes from the bytes: a JPEG named .png still loads.
    std::fs::write(dir.join("photo.png"), grey_jpeg(16)).unwrap();
    assert_eq!(load_image(&dir.join("photo.png")).unwrap().width, 16);
    std::fs::remove_dir_all(&dir).ok();
}
#[test]
fn smaller_copies_halve_down_to_one_pixel_and_average_as_light() {
    // A 4 x 3 black and white checkerboard.
    let rgba = (0..12).flat_map(|i| if (i % 4 + i / 4) % 2 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }).collect();
    let levels = mip_levels(&Image { id: 1, width: 4, height: 3, rgba });
    let sizes: Vec<(u32, u32)> = levels.iter().map(|(w, h, _)| (*w, *h)).collect();
    assert_eq!(sizes, [(2, 1), (1, 1)]);
    assert!(levels.iter().all(|(w, h, p)| p.len() == (*w * *h * 4) as usize));
    let grey = levels[0].2[0];
    assert!(grey > 160, "half black, half white is light grey as light, not 128: {grey}");
    assert_eq!(levels[0].2[3], 255, "opaque stays opaque");
}
#[test]
fn a_gltf_material_gives_its_colour_metal_roughness_and_image() {
    let dir = temp_dir("gltf-material");
    write_triangle(&dir, "tri.bin", 2);
    let mut gltf: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("tri.gltf")).unwrap()).unwrap();
    gltf["meshes"][0]["primitives"][0]["material"] = serde_json::json!(0);
    gltf["materials"] = serde_json::json!([{ "pbrMetallicRoughness": {
        "baseColorFactor": [0.2, 0.8, 0.1, 1.0], "metallicFactor": 0.25, "roughnessFactor": 0.75, "baseColorTexture": { "index": 0 },
        "metallicRoughnessTexture": { "index": 0 }
    }, "normalTexture": { "index": 0 } }]);
    gltf["textures"] = serde_json::json!([{ "source": 0 }]);
    gltf["images"] = serde_json::json!([{ "uri": "colour.png" }]);
    std::fs::write(dir.join("colour.png"), png_bytes()).unwrap();
    std::fs::write(dir.join("tri.gltf"), gltf.to_string()).unwrap();
    let mesh = load_model(&dir.join("tri.gltf")).unwrap();
    let material = mesh.material.unwrap();
    assert_eq!(material.base_color, [0.2, 0.8, 0.1, 1.0]);
    assert_eq!((material.metallic, material.roughness), (0.25, 0.75));
    let image = material.image.unwrap();
    assert_eq!(image.width, 2);
    assert_ne!(image.id, mesh.id, "the image has an id of its own for the GPU cache");
    assert!(material.metal_roughness.is_some() && material.normal.is_some(), "the metal/roughness and normal images load too");

    gltf["images"] = serde_json::json!([{ "uri": "../outside.png" }]);
    std::fs::write(dir.join("tri.gltf"), gltf.to_string()).unwrap();
    assert!(load_model(&dir.join("tri.gltf")).unwrap_err().contains("next to the model"), "images outside the model's folder");
    std::fs::remove_dir_all(&dir).ok();
}
