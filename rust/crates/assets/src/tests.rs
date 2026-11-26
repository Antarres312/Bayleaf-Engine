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
    let levels = mip_levels(&Image::new(1, 4, 3, rgba));
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

#[test]
fn import_settings_beside_a_model_scale_and_turn_it_and_reload_it_when_changed() {
    use super::AssetCache;
    use super::import::{ImportSettings, UpAxis, settings_path};
    let dir = temp_dir("import-settings");
    write_triangle(&dir, "tri.bin", 2);
    let model = dir.join("tri.gltf");
    let mut cache = AssetCache::new();
    let plain = cache.model(&model).unwrap();
    assert!(plain.max[1] > 1.9, "the node lifts it along Y");

    let mut settings = ImportSettings::defaults_for(&model).unwrap();
    let m = settings.model.as_mut().unwrap();
    m.scale = 3.0;
    m.up_axis = UpAxis::Z;
    std::fs::write(settings_path(&model), settings.to_text()).unwrap();
    let turned = cache.model(&model).unwrap();
    assert_ne!(turned.id, plain.id, "new settings make it a new model");
    // Up along Z becomes up along Y: what the node lifted along Y now points away from the viewer.
    assert!(turned.max[1] < 0.5 && turned.min[2] < -5.0, "scaled and turned: {:?} to {:?}", turned.min, turned.max);
    let size = |m: &super::MeshData| (0..3).map(|i| m.max[i] - m.min[i]).fold(0.0f32, f32::max);
    assert!(size(&turned) > size(&plain) * 2.0, "scaled up");

    std::fs::write(settings_path(&model), "{ broken").unwrap();
    let broken = cache.model(&model).unwrap_err();
    assert!(broken.contains("import settings"), "{broken}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_image_without_mipmaps_in_its_settings_says_so() {
    use super::import::{ImportSettings, settings_path};
    let dir = temp_dir("import-image");
    let path = dir.join("a.png");
    std::fs::write(&path, png_bytes()).unwrap();
    assert!(load_image(&path).unwrap().mipmaps, "on unless turned off");
    let mut settings = ImportSettings::defaults_for(&path).unwrap();
    settings.image.as_mut().unwrap().mipmaps = false;
    std::fs::write(settings_path(&path), settings.to_text()).unwrap();
    assert!(!load_image(&path).unwrap().mipmaps);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_exr_picture_s_linear_light_becomes_srgb_and_broken_ones_are_refused() {
    // 2 x 1: linear half grey, then black, as an uncompressed OpenEXR file.
    let linear: [f32; 256] = std::array::from_fn(|i| i as f32 / 255.0);
    let rgba = [128, 128, 128, 255, 0, 0, 0, 255];
    let exr = craft_exr::rgba8::write_rgba8(&rgba, 2, 1, true, craft_exr::rgba8::Compression::None, &linear).unwrap();
    let image = decode_image(&exr).unwrap();
    assert_eq!((image.width, image.height), (2, 1));
    let grey = image.rgba[0];
    assert!(grey > 170 && grey < 200, "linear half grey is stored as its sRGB number: {grey}");
    assert_eq!(&image.rgba[4..7], &[0, 0, 0]);
    assert!(decode_image(&exr[..exr.len() / 2]).is_err(), "a cut file is refused");
}

#[test]
fn an_obj_model_reads_its_material_library_and_image_beside_it() {
    let dir = temp_dir("obj");
    // A square of two triangles, its material red with an image, and a second part with none.
    std::fs::write(
        dir.join("square.obj"),
        "mtllib square.mtl\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl Red\nf 1/1 2/2 3/3 4/4\nusemtl Missing\nf 1/1 3/3 4/4\n",
    )
    .unwrap();
    std::fs::write(dir.join("square.mtl"), "newmtl Red\nKd 1 0 0\nmap_Kd red.png\nnewmtl Missing\nKd 0 0 1\nmap_Kd C:\\Artists\\gone.png\n").unwrap();
    std::fs::write(dir.join("red.png"), png_bytes()).unwrap();
    let model = load_model(&dir.join("square.obj")).unwrap();
    assert_eq!(model.indices.len(), 9, "a quad becomes two triangles, then one more");
    assert_eq!(model.parts.len(), 2, "one run per material");
    let red = model.parts[0].material.as_ref().unwrap();
    assert!(red.base_color[0] > 0.9 && red.base_color[2] < 0.1 && red.image.is_some(), "{red:?}");
    assert!(model.parts[1].material.as_ref().unwrap().image.is_none());
    assert!(model.warnings.iter().any(|w| w.contains("gone.png")), "an image that is not there is said: {:?}", model.warnings);
    let mut files = super::model_files(&dir.join("square.obj")).unwrap();
    files.sort();
    assert_eq!(files, [PathBuf::from("red.png"), PathBuf::from("square.mtl")], "what an export copies with it");
    // Broken and hostile models are refused with a reason.
    std::fs::write(dir.join("bad.obj"), "v 0 0 0\nf 1 2 3\n").unwrap();
    assert!(load_model(&dir.join("bad.obj")).is_err(), "a face naming vertices that are not there");
    std::fs::write(dir.join("empty.obj"), "# nothing\n").unwrap();
    assert!(load_model(&dir.join("empty.obj")).is_err(), "a model with nothing to draw");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_data_image_averages_its_numbers_and_compression_keeps_every_level_in_blocks() {
    let checker: Vec<u8> = (0..16).flat_map(|i| if (i % 4 + i / 4) % 2 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }).collect();
    let colour = mip_levels(&Image::new(1, 4, 4, checker.clone()))[0].2[0];
    let data = mip_levels(&Image { data: true, ..Image::new(1, 4, 4, checker.clone()) })[0].2[0];
    assert!(data.abs_diff(128) <= 1 && colour > data + 40, "numbers average as numbers ({data}), colours as light ({colour})");

    let image = Image::new(1, 8, 8, checker.repeat(4));
    let bc1 = super::compress(&image, super::import::Compression::Bc1).unwrap();
    let sizes: Vec<usize> = bc1.levels.iter().map(Vec::len).collect();
    assert_eq!(sizes, [32, 8, 8, 8], "8 x 8 down to 1 x 1, 8 bytes a 4 x 4 block");
    let bc3 = super::compress(&image, super::import::Compression::Bc3).unwrap();
    assert!(bc3.levels[0].len() == bc1.levels[0].len() * 2, "BC3 keeps twice the bytes, for smooth alpha");
    assert!(super::compress(&Image::new(1, 6, 4, vec![0; 96]), super::import::Compression::Bc1).is_err(), "sides must be multiples of 4");
}

/// A glTF triangle on node 1 (a child of node 0) and an animation "Slide" moving node 1 from x 0
/// to x 2 over a second. With `skinned`, the triangle's vertices instead follow joint node 1 of a
/// skin, from a mesh on node 0.
fn animated_triangle(dir: &Path, skinned: bool) -> PathBuf {
    let mut bin: Vec<u8> = Vec::new();
    let f32s = |values: &[f32], bin: &mut Vec<u8>| values.iter().for_each(|v| bin.extend(v.to_le_bytes()));
    f32s(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0], &mut bin); // positions: 0..36
    f32s(&[0.0, 1.0], &mut bin); // key times: 36..44
    f32s(&[0.0, 0.0, 0.0, 2.0, 0.0, 0.0], &mut bin); // translations: 44..68
    for _ in 0..3 {
        bin.extend([1u8, 0, 0, 0]); // joints: 68..80 (joint 1 of the skin is node 1)
    }
    f32s(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0], &mut bin); // weights: 80..128
    std::fs::write(dir.join("slide.bin"), &bin).unwrap();
    let mut attributes = serde_json::json!({ "POSITION": 0 });
    if skinned {
        attributes["JOINTS_0"] = serde_json::json!(3);
        attributes["WEIGHTS_0"] = serde_json::json!(4);
    }
    let nodes =
        if skinned { serde_json::json!([{ "children": [1], "mesh": 0, "skin": 0 }, {}]) } else { serde_json::json!([{ "children": [1] }, { "mesh": 0 }]) };
    let gltf = serde_json::json!({
        "asset": { "version": "2.0" }, "scene": 0, "scenes": [{ "nodes": [0] }], "nodes": nodes,
        "meshes": [{ "primitives": [{ "attributes": attributes }] }],
        "skins": if skinned { serde_json::json!([{ "joints": [0, 1] }]) } else { serde_json::json!([]) },
        "animations": [{ "name": "Slide", "channels": [{ "sampler": 0, "target": { "node": 1, "path": "translation" } }],
            "samplers": [{ "input": 1, "output": 2 }] }],
        "buffers": [{ "uri": "slide.bin", "byteLength": bin.len() }],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 36 }, { "buffer": 0, "byteOffset": 36, "byteLength": 8 },
            { "buffer": 0, "byteOffset": 44, "byteLength": 24 }, { "buffer": 0, "byteOffset": 68, "byteLength": 12 },
            { "buffer": 0, "byteOffset": 80, "byteLength": 48 }
        ],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] },
            { "bufferView": 1, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1] },
            { "bufferView": 2, "componentType": 5126, "count": 2, "type": "VEC3" },
            { "bufferView": 3, "componentType": 5121, "count": 3, "type": "VEC4" },
            { "bufferView": 4, "componentType": 5126, "count": 3, "type": "VEC4" }
        ]
    });
    let path = dir.join(if skinned { "skinned.gltf" } else { "slide.gltf" });
    std::fs::write(&path, gltf.to_string()).unwrap();
    path
}

#[test]
fn a_model_s_animation_moves_its_parts_and_its_skin() {
    let dir = temp_dir("animated");
    for skinned in [false, true] {
        let model = load_model(&animated_triangle(&dir, skinned)).unwrap();
        let skeleton = model.skeleton.as_ref().expect("an animated model keeps its skeleton");
        let slide = skeleton.animation("Slide").unwrap();
        assert_eq!(skeleton.animation(""), Some(slide), "no name picks the first");
        assert!((skeleton.animations[slide].duration - 1.0).abs() < 1e-6);
        let x = |mesh: &super::MeshData| mesh.positions[1][0];
        assert!((x(&model) - 1.0).abs() < 1e-5, "at rest where it was made (skinned: {skinned})");
        let half = skeleton.posed(&model, Some(slide), 0.5, false, 7, 1);
        assert!((x(&half) - 2.0).abs() < 1e-4, "half way, a unit along (skinned: {skinned}): {}", x(&half));
        let looped = skeleton.posed(&model, Some(slide), 1.25, true, 7, 2);
        assert!((x(&looped) - 1.5).abs() < 1e-4, "looping wraps round: {}", x(&looped));
        let held = skeleton.posed(&model, Some(slide), 3.0, false, 7, 3);
        assert!((x(&held) - 3.0).abs() < 1e-4, "not looping, it stays at the end: {}", x(&held));
        assert!(half.max[0] > model.max[0], "its box follows");
    }
    std::fs::remove_dir_all(&dir).ok();
}
