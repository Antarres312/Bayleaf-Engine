//! The C# engine must read every model and image of the examples and of `tests/files` as this one
//! does (docs/csharp-redesign.md). This test writes a short summary of what this engine reads from
//! each (`refused` when it refuses the file) into `tests/files/expected/<name>.summary` when
//! `BAYLEAF_BLESS=1` is set, and compares with it otherwise; the C# test
//! (`tests/Bayleaf.Assets.Tests/SharedFilesTests.cs`) writes the same summary from its reader and
//! compares with the same file. Import settings files are read and written back, and compared
//! byte for byte, as the scene's files are.
//!
//! Summaries keep numbers to four decimals and compare them within a small tolerance, so the two
//! engines' float printing and last-bit rounding do not matter. Pixels are a checksum, except a
//! JPEG's or OpenEXR's: decoders round differently, so those are listed byte by byte and compared
//! within a tolerance named on the line (`rgba~N`).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::import::ImportSettings;
use crate::skeleton::{Property, Skeleton};
use crate::{Image, MeshData, MeshPart, ModelMaterial, load_image, load_model, model_files};

/// FNV-1a, 64 bits, as 16 hex digits.
pub(crate) fn fnv(bytes: &[u8]) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        hash = (hash ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn num(v: f32) -> String {
    if v.is_nan() {
        "NaN".to_owned()
    } else if v.is_infinite() {
        if v > 0.0 { "inf".to_owned() } else { "-inf".to_owned() }
    } else {
        format!("{v:.4}")
    }
}

fn nums(values: &[f32]) -> String {
    values.iter().map(|v| num(*v)).collect::<Vec<_>>().join(" ")
}

fn kind_of(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpeg"
    } else if bytes.starts_with(&[0x76, 0x2f, 0x31, 0x01]) {
        "exr"
    } else {
        "png"
    }
}

/// How an image of a model is shown: its size, whether it holds numbers, and its pixels' checksum.
fn picture(image: Option<&std::sync::Arc<Image>>) -> String {
    match image {
        None => "none".to_owned(),
        Some(i) => format!("{}x{} data={} {}", i.width, i.height, i.data, fnv(&i.rgba)),
    }
}

fn material(m: Option<&ModelMaterial>) -> String {
    match m {
        None => "none".to_owned(),
        Some(m) => format!(
            "base {} metallic {} roughness {} image {} metal-roughness {} normal {}",
            nums(&m.base_color),
            num(m.metallic),
            num(m.roughness),
            picture(m.image.as_ref()),
            picture(m.metal_roughness.as_ref()),
            picture(m.normal.as_ref())
        ),
    }
}

fn image_summary(path: &Path) -> String {
    let Ok(image) = load_image(path) else { return "refused\n".to_owned() };
    let bytes = std::fs::read(path).unwrap_or_default();
    let kind = kind_of(&bytes);
    let mut s = String::new();
    let _ = writeln!(s, "image\nformat {kind}\nsize {} {}\nmipmaps {}\ndata {}", image.width, image.height, image.mipmaps, image.data);
    match kind {
        "png" => {
            let _ = writeln!(s, "rgba {}", fnv(&image.rgba));
        }
        _ => {
            let tolerance = if kind == "jpeg" { 3 } else { 1 };
            let hex: String = image.rgba.iter().map(|b| format!("{b:02x}")).collect();
            let _ = writeln!(s, "rgba~{tolerance} {hex}");
        }
    }
    match &image.blocks {
        None => s.push_str("blocks none\n"),
        Some(b) => {
            let all: Vec<u8> = b.levels.concat();
            let _ = writeln!(s, "blocks {} {} {}", format!("{:?}", b.compression).to_lowercase(), b.levels.len(), fnv(&all));
        }
    }
    s
}

/// A rotation with the sign that makes w positive (q and -q are the same turn).
fn turn(q: glam::Quat) -> [f32; 4] {
    let q = if q.w < 0.0 || (q.w == 0.0 && [q.x, q.y, q.z].iter().find(|v| **v != 0.0).is_some_and(|v| *v < 0.0)) { -q } else { q };
    q.to_array()
}

fn skeleton_summary(s: &mut String, mesh: &MeshData, skeleton: &Skeleton) {
    let _ = writeln!(s, "skeleton nodes {} bindings {} animations {}", skeleton.nodes.len(), skeleton.bindings.len(), skeleton.animations.len());
    for (i, n) in skeleton.nodes.iter().enumerate() {
        let parent = n.parent.map_or_else(|| "-".to_owned(), |p| p.to_string());
        let _ = writeln!(
            s,
            "node {i} {} parent {parent} t {} r {} s {}",
            skeleton.names.get(i).map_or("", String::as_str),
            nums(&n.translation.to_array()),
            nums(&turn(n.rotation)),
            nums(&n.scale.to_array())
        );
    }
    let _ = writeln!(s, "order {}", skeleton.order.iter().map(ToString::to_string).collect::<Vec<_>>().join(" "));
    let _ = writeln!(s, "root {}", nums(&skeleton.root.to_cols_array()));
    for (i, b) in skeleton.bindings.iter().enumerate() {
        let _ = writeln!(s, "binding {i} node {} inverse {}", b.node, nums(&b.inverse_bind.to_cols_array()));
    }
    for (i, (lo, hi)) in skeleton.binding_bounds.iter().enumerate() {
        let _ = writeln!(s, "bounds {i} {} {}", nums(lo), nums(hi));
    }
    for (i, p) in skeleton.positions.iter().enumerate() {
        let b = skeleton.vertex_bindings.get(i).copied().unwrap_or_default();
        let w = skeleton.vertex_weights.get(i).copied().unwrap_or_default();
        let n = skeleton.normals.get(i).copied().unwrap_or_default();
        let _ = writeln!(s, "bound {i} {} {} {} {} {} {} {}", b[0], b[1], b[2], b[3], nums(&w), nums(p), nums(&n));
    }
    for (index, a) in skeleton.animations.iter().enumerate() {
        let _ = writeln!(s, "animation {} duration {} channels {}", a.name, num(a.duration), a.channels.len());
        for c in &a.channels {
            let property = match c.property {
                Property::Translation => "translation",
                Property::Rotation => "rotation",
                Property::Scale => "scale",
            };
            let _ = writeln!(s, "channel node {} {property} {} keys {}", c.node, if c.step { "step" } else { "linear" }, c.times.len());
            for (t, v) in c.times.iter().zip(&c.values) {
                let _ = writeln!(s, "key {} {}", num(*t), nums(v));
            }
        }
        let time = a.duration * 0.5;
        let posed = skeleton.posed(mesh, Some(index), time, false, mesh.id, 1);
        let _ = writeln!(s, "posed {} {} min {} max {}", a.name, num(time), nums(&posed.min), nums(&posed.max));
        for (i, p) in posed.positions.iter().enumerate() {
            let n = posed.normals.get(i).copied().unwrap_or_default();
            let _ = writeln!(s, "pv {i} {} {}", nums(p), nums(&n));
        }
    }
}

fn part(p: &MeshPart) -> String {
    format!("part {} {} {}", p.first, p.count, material(p.material.as_ref()))
}

fn model_summary(path: &Path) -> String {
    let Ok(mesh) = load_model(path) else { return "refused\n".to_owned() };
    let mut s = String::from("model\n");
    let files = model_files(path).unwrap_or_default();
    let _ = writeln!(s, "files {}", files.len());
    for f in &files {
        let _ = writeln!(s, "file {}", f.to_string_lossy().replace('\\', "/"));
    }
    let index_bytes: Vec<u8> = mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
    let _ = writeln!(s, "vertices {}\nindices {} {}", mesh.positions.len(), mesh.indices.len(), fnv(&index_bytes));
    let _ = writeln!(s, "min {}\nmax {}", nums(&mesh.min), nums(&mesh.max));
    for (i, p) in mesh.positions.iter().enumerate() {
        let n = mesh.normals.get(i).copied().unwrap_or_default();
        let uv = mesh.uvs.get(i).copied().unwrap_or_default();
        let _ = writeln!(s, "v {i} {} {} {}", nums(p), nums(&n), nums(&uv));
    }
    let _ = writeln!(s, "parts {}", mesh.parts.len());
    for p in &mesh.parts {
        let _ = writeln!(s, "{}", part(p));
    }
    let _ = writeln!(s, "material {}", material(mesh.material.as_ref()));
    let _ = writeln!(s, "warnings {}", mesh.warnings.len());
    for w in &mesh.warnings {
        let _ = writeln!(s, "warning {w}");
    }
    match &mesh.skeleton {
        None => s.push_str("skeleton none\n"),
        Some(skeleton) => skeleton_summary(&mut s, &mesh, skeleton),
    }
    s
}

/// What this engine reads from `path`: a summary for a model or image, the text written back for
/// import settings, or nothing for another kind of file.
fn written(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if extension == crate::import::SETTINGS_EXTENSION {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        return Some(ImportSettings::from_text(&text).map_or_else(|_| "refused\n".to_owned(), |s| s.to_text()));
    }
    if crate::MODEL_EXTENSIONS.contains(&extension.as_str()) {
        return Some(model_summary(path));
    }
    crate::IMAGE_EXTENSIONS.contains(&extension.as_str()).then(|| image_summary(path))
}

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

/// Whether two summaries say the same: line by line and word by word, numbers within a small
/// tolerance, and the bytes of an `rgba~N` line each within N.
pub(crate) fn same(written: &str, expected: &str) -> Result<(), String> {
    let (a, b): (Vec<&str>, Vec<&str>) = (written.lines().collect(), expected.lines().collect());
    if a.len() != b.len() {
        return Err(format!("{} lines, not {}", a.len(), b.len()));
    }
    for (n, (x, y)) in a.iter().zip(&b).enumerate() {
        let (xs, ys): (Vec<&str>, Vec<&str>) = (x.split(' ').collect(), y.split(' ').collect());
        let tolerance = xs.first().and_then(|w| w.strip_prefix("rgba~")).and_then(|t| t.parse::<i32>().ok());
        if let (Some(t), 2, 2) = (tolerance, xs.len(), ys.len())
            && xs[0] == ys[0]
        {
            let bytes = |h: &str| (0..h.len() / 2).map(|i| i32::from_str_radix(h.get(i * 2..i * 2 + 2).unwrap_or(""), 16).unwrap_or(-999)).collect::<Vec<_>>();
            let (p, q) = (bytes(xs[1]), bytes(ys[1]));
            let worst = p.iter().zip(&q).map(|(a, b)| (a - b).abs()).max().unwrap_or(0);
            if p.len() != q.len() || worst > t {
                return Err(format!("line {}: pixels differ by up to {worst} (or in number), more than {t}", n + 1));
            }
            continue;
        }
        let close = |p: &str, q: &str| {
            p == q
                || (p.contains('.') || q.contains('.'))
                    && match (p.parse::<f64>(), q.parse::<f64>()) {
                        (Ok(u), Ok(v)) => (u - v).abs() <= 2e-4 + 2e-5 * u.abs(),
                        _ => false,
                    }
        };
        if xs.len() != ys.len() || xs.iter().zip(&ys).any(|(p, q)| !close(p, q)) {
            return Err(format!("line {}: `{x}` is not `{y}`", n + 1));
        }
    }
    Ok(())
}

#[test]
fn the_shared_models_and_images_read_as_the_expected_files_say() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("..");
    let expected_dir = root.join("tests").join("files").join("expected");
    let bless = std::env::var_os("BAYLEAF_BLESS").is_some_and(|v| v == "1");
    let mut inputs = Vec::new();
    files_below(&root.join("examples"), &mut inputs);
    files_below(&root.join("tests").join("files"), &mut inputs);
    // The model the crate writes in code, and the default import settings of each kind of file.
    let fixture_dir = std::env::temp_dir().join(format!("bayleaf-assets-shared-{}", std::process::id()));
    std::fs::create_dir_all(&fixture_dir).unwrap();
    let arm = crate::fixtures::skinned_arm(&fixture_dir).unwrap();
    let mut extra: Vec<(String, String)> = vec![
        ("fixtures__arm.gltf".to_owned(), std::fs::read_to_string(&arm).unwrap()),
        ("fixtures__arm.gltf.summary".to_owned(), format!("bin {}\n{}", fnv(&std::fs::read(fixture_dir.join("arm.bin")).unwrap()), model_summary(&arm))),
    ];
    for (kind, file) in [("model", "a.glb"), ("image", "a.png"), ("sound", "a.ogg")] {
        let text = ImportSettings::defaults_for(Path::new(file)).map(|s| s.to_text()).unwrap_or_default();
        extra.push((format!("defaults__{kind}.bimport"), text));
    }
    std::fs::remove_dir_all(&fixture_dir).ok();
    let mut checked = 0;
    let mut compare = |name: String, text: String, exact: bool, from: &Path| {
        let expected_path = expected_dir.join(&name);
        if bless {
            std::fs::create_dir_all(&expected_dir).unwrap();
            std::fs::write(&expected_path, &text).unwrap();
        }
        let expected = std::fs::read_to_string(&expected_path).unwrap_or_default().replace("\r\n", "\n");
        if exact {
            assert_eq!(text, expected, "{} reads differently from {}", from.display(), expected_path.display());
        } else if let Err(e) = same(&text, &expected) {
            panic!("{} reads differently from {}: {e}", from.display(), expected_path.display());
        }
        checked += 1;
    };
    for input in inputs {
        let Some(text) = written(&input) else { continue };
        let relative = input.strip_prefix(&root).unwrap_or(&input).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>();
        let name = relative.iter().skip_while(|c| *c == "tests" || *c == "files").cloned().collect::<Vec<_>>().join("__");
        let settings = crate::import::is_settings_file(&input);
        compare(if settings { name } else { format!("{name}.summary") }, text, settings, &input);
    }
    for (name, text) in extra {
        let exact = !name.ends_with(".summary");
        compare(name, text, exact, &arm);
    }
    assert!(checked > 20, "only {checked} shared files were found");
}
