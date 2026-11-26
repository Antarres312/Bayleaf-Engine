//! The C# engine's material port (`src/Bayleaf.Render`) must read every material graph file as this
//! crate does and compile it to the same WGSL, byte for byte (docs/csharp-redesign.md). Both engines'
//! tests compare what they give with `tests/files/expected/`, which this test writes when
//! `BAYLEAF_BLESS=1` is set. The frame each shared scene extracts to is compared the same way, for
//! the C# port's extraction. Nothing here needs a GPU.

use std::path::{Path, PathBuf};

use bayleaf_graph::Graph;

use crate::material::{CATALOG, compile};

const REFUSED: &str = "refused\n";

fn files_below(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "expected") {
                files_below(&path, out);
            }
        } else {
            out.push(path);
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("..")
}

/// Compares `written` with the expected file `name`, first writing it there when blessing.
fn check(name: &str, written: &str, what: &Path) {
    let expected_dir = root().join("tests").join("files").join("expected");
    let expected_path = expected_dir.join(name);
    if std::env::var_os("BAYLEAF_BLESS").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(&expected_dir).unwrap();
        std::fs::write(&expected_path, written).unwrap();
    }
    let expected = std::fs::read_to_string(&expected_path).unwrap_or_default().replace("\r\n", "\n");
    assert_eq!(written, expected, "{} gives other text than {}", what.display(), expected_path.display());
}

/// The file as this crate writes it back after reading it with the material catalog, and the
/// `surface` function it compiles to; `refused` for either when it does not read or compile.
fn rewrite(path: &Path) -> (String, String) {
    let Ok(text) = std::fs::read_to_string(path) else { return (REFUSED.to_owned(), REFUSED.to_owned()) };
    let Ok(graph) = Graph::from_json(&text, &CATALOG) else { return (REFUSED.to_owned(), REFUSED.to_owned()) };
    let surface = compile(&graph).map_or_else(|_| REFUSED.to_owned(), |m| m.surface);
    (graph.to_json(), surface)
}

/// Every `.bmaterial` of the examples, the benchmark and `tests/files` reads, writes back and
/// compiles as the expected files say: `<name>` the file written back, `<name>.wgsl` its surface.
#[test]
fn the_shared_material_files_rewrite_and_compile_as_the_expected_files_say() {
    let root = root();
    let mut inputs = Vec::new();
    files_below(&root.join("examples"), &mut inputs);
    files_below(&root.join("rust").join("bench").join("graphs"), &mut inputs);
    files_below(&root.join("tests").join("files"), &mut inputs);
    let mut checked = 0;
    for input in inputs.iter().filter(|p| p.extension().is_some_and(|e| e == "bmaterial")) {
        let relative = input.strip_prefix(&root).unwrap_or(input).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>();
        let name = relative.iter().skip_while(|c| *c == "tests" || *c == "files").cloned().collect::<Vec<_>>().join("__");
        let (file, surface) = rewrite(input);
        check(&name, &file, input);
        check(&format!("{name}.wgsl"), &surface, input);
        checked += 1;
    }
    assert!(checked > 10, "only {checked} shared material files were found");
}

/// The material catalog in every detail, so the C# port's hand-written one is checked kind by kind.
#[test]
fn the_material_catalog_is_as_the_expected_file_says() {
    let written = serde_json::to_string_pretty(&CATALOG.spec()).unwrap() + "\n";
    check("material.catalog.json", &written, Path::new("the material catalog"));
}

/// The folder a scene's paths start from: the project folder above it (the one with a `.bproject`
/// file), else the scene's own folder, as `scene.render` takes it.
fn asset_base(scene: &Path) -> PathBuf {
    let own = scene.parent().map(Path::to_path_buf).unwrap_or_default();
    let project = |dir: &Path| std::fs::read_dir(dir).is_ok_and(|entries| entries.flatten().any(|e| e.path().extension().is_some_and(|x| x == "bproject")));
    own.ancestors().find(|dir| project(dir)).map_or_else(|| own.clone(), Path::to_path_buf)
}

/// What the frame of a scene file holds, seen as `scene.render` sees it (its first camera, else a view
/// framing it, at 16:9, with its files): every number but the GPU's own keys and the materials' shader
/// text (which the material test compares). The C# port's extraction is compared with it. `refused`
/// for a file that does not load.
fn frame_summary(path: &Path) -> String {
    use bayleaf_core::math::DVec3;
    use serde_json::json;
    let Ok(scene) = bayleaf_scene::Scene::load(path) else { return REFUSED.to_owned() };
    let aspect = 16.0 / 9.0;
    let camera = crate::scene_camera(&scene, aspect).unwrap_or_else(|| {
        let (center, radius) = crate::bounds(&scene).unwrap_or((DVec3::ZERO, 2.0));
        crate::CameraView::framing(center, radius, aspect)
    });
    let mut materials = crate::MaterialLibrary::new();
    let mut cache = bayleaf_assets::AssetCache::new();
    let allow = |p: &Path| Ok(p.to_path_buf());
    let mut assets = crate::FileAssets { base: asset_base(path), materials: &mut materials, cache: &mut cache, allow: &allow, warnings: Vec::new() };
    let effects = crate::Effects { show: true, allowed: crate::EffectsAllowed::ALL, camera: crate::camera_object(&scene) };
    let options = crate::FrameOptions { show_grid: false, selected: None, time: 0.0, shadows: crate::ShadowSettings::DEFAULT, effects };
    let frame = crate::extract_with(&scene, camera, options, &mut assets);
    let items: Vec<serde_json::Value> = frame
        .items
        .iter()
        .map(|i| {
            json!({
                "object": i.object,
                "shape": i.shape,
                "model": i.model.to_cols_array(),
                "color": i.color.to_array(),
                "metallic": i.metallic,
                "roughness": i.roughness,
                "unlit": i.unlit,
                "transparent": i.is_transparent(),
                "material": i.material.is_some(),
                "mesh": i.mesh.as_ref().map(|m| json!({"vertices": m.positions.len(), "indices": m.indices.len(), "min": m.min, "max": m.max})),
                "part": i.part.map(|(first, count)| [first, count]),
                "textures": i.textures.iter().map(|t| t.as_ref().map(|image| [image.width, image.height])).collect::<Vec<_>>(),
                "additive": i.additive,
                "uvRect": i.uv_rect,
                "skin": i.skin.as_ref().map(|s| json!({"matrices": s.matrices.len(), "min": s.min.to_array(), "max": s.max.to_array()})),
            })
        })
        .collect();
    let lights: Vec<serde_json::Value> = frame
        .lights
        .iter()
        .map(|l| {
            json!({
                "kind": format!("{:?}", l.kind).to_lowercase(),
                "position": l.position.to_array(),
                "direction": l.direction.to_array(),
                "radiance": l.radiance.to_array(),
                "range": l.range,
                "cosOuter": l.cos_outer,
                "cosInner": l.cos_inner,
                "shadows": l.shadows,
            })
        })
        .collect();
    let p = &frame.post;
    let summary = json!({
        "camera": {"view": frame.camera.view.to_cols_array(), "proj": frame.camera.proj.to_cols_array(), "position": frame.camera.position.to_array(), "origin": frame.camera.origin.to_array()},
        "items": items,
        "lights": lights,
        "ambient": frame.ambient.to_array(),
        "ambientGround": frame.ambient_ground.to_array(),
        "clearColor": frame.clear_color.to_array(),
        "sky": frame.sky.map(|s| json!({"zenith": s.zenith.to_array(), "horizon": s.horizon.to_array(), "ground": s.ground.to_array()})),
        "background": frame.background,
        "fog": [frame.fog.0.x, frame.fog.0.y, frame.fog.0.z, frame.fog.1],
        "post": {
            "exposure": p.exposure, "toneMapping": p.tone_mapping, "bloom": p.bloom, "bloomThreshold": p.bloom_threshold,
            "contrast": p.contrast, "saturation": p.saturation, "lookup": p.lookup.as_ref().map(|i| [i.width, i.height]), "lookupStrength": p.lookup_strength,
            "vignette": p.vignette, "vignetteSmoothness": p.vignette_smoothness, "fog": p.fog.map(|(c, start, end)| [c[0], c[1], c[2], start, end]),
            "occlusion": p.occlusion, "occlusionRadius": p.occlusion_radius, "antialiasing": p.antialiasing,
        },
        "warnings": assets.warnings.len(),
    });
    serde_json::to_string_pretty(&summary).unwrap_or_default() + "\n"
}

/// Every scene of the examples and `tests/files` gives the frame the expected files say
/// (`<name>.frame.json`), for the C# port's extraction test, which compares the numbers within a tolerance.
#[test]
fn the_shared_scenes_give_the_frames_the_expected_files_say() {
    let root = root();
    let mut inputs = Vec::new();
    files_below(&root.join("examples"), &mut inputs);
    files_below(&root.join("tests").join("files"), &mut inputs);
    let mut checked = 0;
    for input in inputs.iter().filter(|p| p.extension().is_some_and(|e| e == "bscene")) {
        let relative = input.strip_prefix(&root).unwrap_or(input).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>();
        let name = relative.iter().skip_while(|c| *c == "tests" || *c == "files").cloned().collect::<Vec<_>>().join("__");
        check(&format!("{name}.frame.json"), &frame_summary(input), input);
        checked += 1;
    }
    assert!(checked > 5, "only {checked} shared scenes were found");
}
