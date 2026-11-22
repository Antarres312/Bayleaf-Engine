//! Models in formats craft-mesh reads (Wavefront OBJ with its MTL libraries), turned into the
//! engine's [`MeshData`] the way glTF models are: every placed mesh merged, node transforms
//! applied, triangles grouped by material. Files the model names (material libraries, images) are
//! read only beside it or below it; an image named by a full path elsewhere is looked for by its
//! file name beside the model, as exporters often write the artist's own folders.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::{Image, MAX_VERTICES, MeshData, MeshPart, ModelMaterial, buffer_path, decode_image, file_id, import, read_limited};

/// The formats read here, by file extension. FBX waits: craft-mesh reads it through craft-zip,
/// whose `zip` brings a dependency with a licence not on the craft-contrib list.
pub const EXTENSIONS: &[&str] = &["obj"];

/// Whether `extension` (lower case) is read here.
pub fn reads(extension: &str) -> bool {
    EXTENSIONS.contains(&extension)
}

fn limits() -> craft_mesh::Limits {
    craft_mesh::Limits { max_vertices: MAX_VERTICES, max_triangles: MAX_VERTICES, ..craft_mesh::Limits::default() }
}

/// The file a model names, beside it or below it; else, for a name with folders that are not
/// there, its file name beside the model.
fn named_file(folder: &Path, name: &str) -> Option<PathBuf> {
    let name = name.trim().replace('\\', "/");
    if let Some(path) = buffer_path(folder, &name).filter(|p| p.is_file()) {
        return Some(path);
    }
    let file = name.rsplit('/').next().filter(|f| !f.is_empty() && *f != ".." && *f != ".")?;
    Some(folder.join(file)).filter(|p| p.is_file())
}

/// Reads the model's own scene, asking for the files it names beside it.
fn read_scene(path: &Path, bytes: &[u8]) -> Result<craft_mesh::Scene, String> {
    let folder = path.parent().unwrap_or(Path::new("."));
    let extension = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let mut resolve = |name: &str| named_file(folder, name).and_then(|file| read_limited(&file).ok()).map(|(bytes, _)| bytes);
    let scene = match extension.as_str() {
        "obj" => craft_mesh::obj::read(bytes, &limits(), &mut resolve),
        other => return Err(format!("{other} models are not read here")),
    };
    let scene = scene.map_err(|e| e.to_string())?;
    scene.validate().map_err(|e| e.to_string())?;
    Ok(scene)
}

/// The files a model names beside it (material libraries and images that are there), each once.
pub fn model_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    let (bytes, _) = read_limited(path)?;
    let scene = read_scene(path, &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let folder = path.parent().unwrap_or(Path::new("."));
    let mut files: Vec<PathBuf> = Vec::new();
    for name in &scene.external {
        if let Some(file) = named_file(folder, name).and_then(|f| f.strip_prefix(folder).ok().map(Path::to_path_buf))
            && !files.contains(&file)
        {
            files.push(file);
        }
    }
    Ok(files)
}

/// Reads an OBJ model. Its import settings apply: the scale, and turning a Z-up model to Y up.
pub fn load_model(path: &Path) -> Result<MeshData, String> {
    let settings = import::ImportSettings::read_for(path)?.and_then(|s| s.model).unwrap_or_default();
    let (bytes, modified) = read_limited(path)?;
    let at = |e: String| format!("{}: {e}", path.display());
    let scene = read_scene(path, &bytes).map_err(at)?;
    let id = file_id(path, modified);
    let up = match settings.up_axis {
        import::UpAxis::Y => glam::DMat4::IDENTITY,
        import::UpAxis::Z => glam::DMat4::from_rotation_x(-std::f64::consts::FRAC_PI_2),
    };
    let root = glam::DMat4::from_scale(glam::DVec3::splat(f64::from(settings.scale))) * up;
    let mut mesh = MeshData {
        id,
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::new(),
        min: [0.0; 3],
        max: [0.0; 3],
        material: None,
        parts: Vec::new(),
        warnings: scene.warnings.clone(),
    };
    // Runs of indices and the material each uses.
    let mut runs: Vec<(u32, u32, Option<usize>)> = Vec::new();
    for (index, placed) in scene.instances(&limits()).map_err(|e| at(e.to_string()))? {
        let Some(source) = scene.meshes.get(index) else { continue };
        let world = root * glam::DMat4::from_cols_array(&placed);
        add_mesh(source, world, &mut mesh, &mut runs).map_err(at)?;
    }
    if mesh.indices.is_empty() {
        return Err(at("the model has no triangles".to_owned()));
    }
    let folder = path.parent().unwrap_or(Path::new("."));
    let mut images: HashMap<usize, Option<Arc<Image>>> = HashMap::new();
    let mut materials: HashMap<usize, ModelMaterial> = HashMap::new();
    for (first, count, index) in runs {
        let material = index
            .and_then(|i| scene.materials.get(i).map(|m| (i, m)))
            .map(|(i, m)| materials.entry(i).or_insert_with(|| material_of(m, &scene, folder, id, &mut images, &mut mesh.warnings)).clone());
        mesh.parts.push(MeshPart { first, count, material });
    }
    mesh.material = mesh.parts.iter().find_map(|p| p.material.clone());
    let (mut min, mut max) = (glam::Vec3::splat(f32::INFINITY), glam::Vec3::splat(f32::NEG_INFINITY));
    for p in &mesh.positions {
        min = min.min(glam::Vec3::from(*p));
        max = max.max(glam::Vec3::from(*p));
    }
    mesh.min = min.to_array();
    mesh.max = max.to_array();
    Ok(mesh)
}

/// One placed mesh's triangles, moved by `world`, onto the end of `mesh`.
fn add_mesh(source: &craft_mesh::Mesh, world: glam::DMat4, mesh: &mut MeshData, runs: &mut Vec<(u32, u32, Option<usize>)>) -> Result<(), String> {
    let unified = source.unified();
    if mesh.positions.len() + unified.positions.len() > MAX_VERTICES {
        return Err(format!("models may have at most {MAX_VERTICES} vertices"));
    }
    let base = mesh.positions.len() as u32;
    let count = unified.positions.len();
    let normal_matrix = glam::DMat3::from_mat4(world).inverse().transpose();
    // A mirroring placement turns the triangles inside out; their corners go the other way round.
    let mirrored = world.determinant() < 0.0;
    let mut triangles: Vec<(Vec<u32>, Option<usize>)> = Vec::new();
    for (part, corners) in source.parts.iter().zip(&unified.parts) {
        let mut indices = Vec::with_capacity(corners.len() * 3);
        for tri in corners {
            if tri.iter().any(|i| *i as usize >= count) {
                return Err("a triangle uses a vertex that does not exist".to_owned());
            }
            let [a, b, c] = *tri;
            indices.extend(if mirrored { [a, c, b] } else { [a, b, c] });
        }
        triangles.push((indices, part.material));
    }
    let all: Vec<u32> = triangles.iter().flat_map(|(i, _)| i.iter().copied()).collect();
    let positions: Vec<[f32; 3]> = unified.positions.iter().map(|p| world.transform_point3(glam::DVec3::from(*p)).as_vec3().to_array()).collect();
    let normals: Vec<[f32; 3]> = if unified.normals.len() == count {
        unified.normals.iter().map(|n| (normal_matrix * glam::DVec3::from(*n)).normalize_or_zero().as_vec3().to_array()).collect()
    } else {
        crate::flat_normals(&positions, &all)
    };
    let uvs = if unified.uvs.len() == count { unified.uvs.clone() } else { vec![[0.0, 0.0]; count] };
    mesh.positions.extend(positions);
    mesh.normals.extend(normals);
    mesh.uvs.extend(uvs);
    for (indices, material) in triangles {
        let (first, count) = (mesh.indices.len() as u32, indices.len() as u32);
        if count == 0 {
            continue;
        }
        mesh.indices.extend(indices.into_iter().map(|i| base + i));
        match runs.last_mut() {
            Some(last) if last.2 == material && last.0 + last.1 == first => last.1 += count,
            _ => runs.push((first, count, material)),
        }
    }
    Ok(())
}

/// The engine's material from a neutral one: base colour, metallic, roughness and the images it
/// can read (an image that cannot be read is left out).
fn material_of(
    m: &craft_mesh::Material,
    scene: &craft_mesh::Scene,
    folder: &Path,
    model_id: u64,
    images: &mut HashMap<usize, Option<Arc<Image>>>,
    warnings: &mut Vec<String>,
) -> ModelMaterial {
    let mut image = |slot: Option<craft_mesh::TextureRef>| {
        let index = slot.filter(|t| t.uv_set == 0)?.image;
        images
            .entry(index)
            .or_insert_with(|| {
                let source = scene.images.get(index)?;
                image_of(source, index, folder, model_id)
                    .map_err(|e| warnings.push(format!("material {}: image {} left out: {e}", m.name, source.uri.as_deref().unwrap_or(&source.name))))
                    .ok()
            })
            .clone()
    };
    let finite = |v: f32, fallback: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { fallback };
    ModelMaterial {
        base_color: m.base_color.map(|c| finite(c, 1.0)),
        metallic: finite(m.metallic, 0.0),
        roughness: finite(m.roughness, 1.0),
        image: image(m.base_color_texture),
        metal_roughness: image(m.metallic_roughness_texture).map(crate::as_data),
        normal: image(m.normal_texture).map(crate::as_data),
    }
}

fn image_of(source: &craft_mesh::Image, index: usize, folder: &Path, model_id: u64) -> Result<Arc<Image>, String> {
    let bytes = match (&source.data, &source.uri) {
        (Some(data), _) => data.clone(),
        (None, Some(uri)) => read_limited(&named_file(folder, uri).ok_or("it is not beside the model")?)?.0,
        (None, None) => return Err("it names no file".to_owned()),
    };
    let mut image = decode_image(&bytes)?;
    // Distinct from the model's id and from its other images.
    image.id = (model_id ^ (index as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)) | 1;
    Ok(Arc::new(image))
}
