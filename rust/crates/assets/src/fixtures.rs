//! A small skinned model written in code, for tests and examples (our own work; see
//! `ATTRIBUTION.md`): a wall two units wide whose left edge follows the joint `Root` and whose
//! right edge follows `Arm`, with a `Hand` joint at that edge. Its animations, a second long:
//! `Stretch` moves `Arm` 1.5 to the right, `Raise` moves it 1 up, `Wave` turns `Hand` a quarter
//! turn about Z.

use std::path::{Path, PathBuf};

/// Writes `arm.gltf` and `arm.bin` into `dir`; gives the path of `arm.gltf`.
pub fn skinned_arm(dir: &Path) -> std::io::Result<PathBuf> {
    let mut bin: Vec<u8> = Vec::new();
    let f32s = |values: &[f32], bin: &mut Vec<u8>| values.iter().for_each(|v| bin.extend(v.to_le_bytes()));
    // Positions 0..48: a wall from (-1, -1) to (1, 1), facing +Z.
    f32s(&[-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0, -1.0, 1.0, 0.0], &mut bin);
    // Indices 48..60.
    for i in [0u16, 1, 2, 0, 2, 3] {
        bin.extend(i.to_le_bytes());
    }
    // Joints 60..76: the left edge follows joint 0 (Root), the right edge joint 1 (Arm).
    for joint in [0u8, 1, 1, 0] {
        bin.extend([joint, 0, 0, 0]);
    }
    // Weights 76..140.
    for _ in 0..4 {
        f32s(&[1.0, 0.0, 0.0, 0.0], &mut bin);
    }
    // Key times 140..148, then Stretch 148..172, Raise 172..196 and Wave 196..228.
    f32s(&[0.0, 1.0], &mut bin);
    f32s(&[0.0, 0.0, 0.0, 1.5, 0.0, 0.0], &mut bin);
    f32s(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0], &mut bin);
    let half = std::f32::consts::FRAC_1_SQRT_2;
    f32s(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, half, half], &mut bin);
    std::fs::write(dir.join("arm.bin"), &bin)?;
    let view = |offset: usize, length: usize| serde_json::json!({ "buffer": 0, "byteOffset": offset, "byteLength": length });
    let channel = |node: usize, path: &str, output: usize| serde_json::json!({ "channels": [{ "sampler": 0, "target": { "node": node, "path": path } }], "samplers": [{ "input": 4, "output": output }] });
    let mut stretch = channel(1, "translation", 5);
    stretch["name"] = "Stretch".into();
    let mut raise = channel(1, "translation", 6);
    raise["name"] = "Raise".into();
    let mut wave = channel(2, "rotation", 7);
    wave["name"] = "Wave".into();
    let gltf = serde_json::json!({
        "asset": { "version": "2.0", "generator": "Bayleaf Engine test fixture" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [
            { "name": "Root", "mesh": 0, "skin": 0, "children": [1] },
            { "name": "Arm", "children": [2] },
            { "name": "Hand", "translation": [1.0, 0.0, 0.0] }
        ],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0, "JOINTS_0": 2, "WEIGHTS_0": 3 }, "indices": 1 }] }],
        "skins": [{ "joints": [0, 1, 2] }],
        "animations": [stretch, raise, wave],
        "buffers": [{ "uri": "arm.bin", "byteLength": bin.len() }],
        "bufferViews": [view(0, 48), view(48, 12), view(60, 16), view(76, 64), view(140, 8), view(148, 24), view(172, 24), view(196, 32)],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3", "min": [-1, -1, 0], "max": [1, 1, 0] },
            { "bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR" },
            { "bufferView": 2, "componentType": 5121, "count": 4, "type": "VEC4" },
            { "bufferView": 3, "componentType": 5126, "count": 4, "type": "VEC4" },
            { "bufferView": 4, "componentType": 5126, "count": 2, "type": "SCALAR", "min": [0], "max": [1] },
            { "bufferView": 5, "componentType": 5126, "count": 2, "type": "VEC3" },
            { "bufferView": 6, "componentType": 5126, "count": 2, "type": "VEC3" },
            { "bufferView": 7, "componentType": 5126, "count": 2, "type": "VEC4" }
        ]
    });
    let path = dir.join("arm.gltf");
    std::fs::write(&path, gltf.to_string())?;
    Ok(path)
}
