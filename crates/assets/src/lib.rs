//! L2: reading asset files: images (PNG) and 3D models (glTF 2.0).
//!
//! Loaders turn files into plain data the renderer uploads. Each result carries an id that
//! changes when the file changes, so caches know when to load again. Limits keep a broken or
//! hostile file from using all memory. A stable asset database with sidecar metadata comes later.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// Largest image side, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 8192;
/// Most vertices in one model.
pub const MAX_VERTICES: usize = 2_000_000;
/// Largest model or image file, in bytes.
pub const MAX_FILE_BYTES: u64 = 256 << 20;

/// An image as 8-bit RGBA pixels, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// Changes when the file changes.
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A model's triangles, every mesh of its default scene merged, with node transforms applied.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshData {
    /// Changes when the file changes.
    pub id: u64,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// Smallest and largest corner of the box around every position.
    pub min: [f32; 3],
    pub max: [f32; 3],
}

fn file_id(path: &Path, modified: Option<SystemTime>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    modified.hash(&mut hasher);
    // 0 is kept for "no asset".
    hasher.finish() | 1
}

fn read_limited(path: &Path) -> Result<(Vec<u8>, Option<SystemTime>), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{} is larger than {} MiB", path.display(), MAX_FILE_BYTES >> 20));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok((bytes, meta.modified().ok()))
}

/// Reads a PNG file.
pub fn load_image(path: &Path) -> Result<Image, String> {
    let (bytes, modified) = read_limited(path)?;
    let mut image = decode_png(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    image.id = file_id(path, modified);
    Ok(image)
}

/// Decodes PNG bytes into RGBA. The id is left at 0.
pub fn decode_png(bytes: &[u8]) -> Result<Image, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // Palettes and low bit depths become 8-bit channels; 16-bit channels become 8-bit.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("not a PNG image: {e}"))?;
    let info = reader.info();
    if info.width == 0 || info.height == 0 || info.width > MAX_IMAGE_SIDE || info.height > MAX_IMAGE_SIDE {
        return Err(format!("images may be at most {MAX_IMAGE_SIDE} pixels on a side, not {} x {}", info.width, info.height));
    }
    let size = reader.output_buffer_size().ok_or("the image is too large")?;
    let mut buffer = vec![0; size];
    let frame = reader.next_frame(&mut buffer).map_err(|e| format!("broken PNG image: {e}"))?;
    let pixels = &buffer[..frame.buffer_size()];
    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        png::ColorType::Indexed => return Err("unexpected palette image after expansion".to_owned()),
    };
    Ok(Image { id: 0, width: frame.width, height: frame.height, rgba })
}

/// Reads a glTF 2.0 model: a `.glb` file, or a `.gltf` file with its buffers in files beside it.
pub fn load_model(path: &Path) -> Result<MeshData, String> {
    let (bytes, modified) = read_limited(path)?;
    let gltf = gltf::Gltf::from_slice(&bytes).map_err(|e| format!("{}: not a glTF 2.0 model: {e}", path.display()))?;
    let folder = path.parent().unwrap_or(Path::new("."));
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    for buffer in gltf.buffers() {
        let data = match buffer.source() {
            gltf::buffer::Source::Bin => gltf.blob.clone().ok_or_else(|| format!("{}: the binary chunk is missing", path.display()))?,
            gltf::buffer::Source::Uri(uri) if uri.starts_with("data:") => {
                return Err(format!("{}: buffers inside the file as text are not supported yet; save the model as .glb", path.display()));
            }
            gltf::buffer::Source::Uri(uri) => {
                let file = buffer_path(folder, uri).ok_or_else(|| format!("{}: buffer {uri} must be a file next to the model", path.display()))?;
                read_limited(&file)?.0
            }
        };
        if data.len() < buffer.length() {
            return Err(format!("{}: buffer {} is shorter than it says", path.display(), buffer.index()));
        }
        buffers.push(data);
    }
    let mut mesh = MeshData {
        id: file_id(path, modified),
        positions: Vec::new(),
        normals: Vec::new(),
        uvs: Vec::new(),
        indices: Vec::new(),
        min: [0.0; 3],
        max: [0.0; 3],
    };
    let scene = gltf.default_scene().or_else(|| gltf.scenes().next()).ok_or_else(|| format!("{}: the model has no scene", path.display()))?;
    for node in scene.nodes() {
        add_node(&node, glam::Mat4::IDENTITY, &buffers, &mut mesh, 0).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    if mesh.indices.is_empty() {
        return Err(format!("{}: the model has no triangles", path.display()));
    }
    let (mut min, mut max) = (glam::Vec3::splat(f32::INFINITY), glam::Vec3::splat(f32::NEG_INFINITY));
    for p in &mesh.positions {
        min = min.min(glam::Vec3::from(*p));
        max = max.max(glam::Vec3::from(*p));
    }
    mesh.min = min.to_array();
    mesh.max = max.to_array();
    Ok(mesh)
}

/// A buffer file named by `uri`, only directly beside the model or below it.
fn buffer_path(folder: &Path, uri: &str) -> Option<PathBuf> {
    let relative = Path::new(uri);
    if relative.is_absolute() || relative.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return None;
    }
    Some(folder.join(relative))
}

fn add_node(node: &gltf::Node<'_>, parent: glam::Mat4, buffers: &[Vec<u8>], mesh: &mut MeshData, depth: usize) -> Result<(), String> {
    if depth > 64 {
        return Err("the node tree is too deep".to_owned());
    }
    let world = parent * glam::Mat4::from_cols_array_2d(&node.transform().matrix());
    let normal_matrix = glam::Mat3::from_mat4(world).inverse().transpose();
    if let Some(gltf_mesh) = node.mesh() {
        for primitive in gltf_mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
            let Some(positions) = reader.read_positions() else { continue };
            let positions: Vec<[f32; 3]> = positions.collect();
            if mesh.positions.len() + positions.len() > MAX_VERTICES {
                return Err(format!("models may have at most {MAX_VERTICES} vertices"));
            }
            let base = mesh.positions.len() as u32;
            let count = positions.len();
            let indices: Vec<u32> = match reader.read_indices() {
                Some(indices) => indices.into_u32().collect(),
                None => (0..count as u32).collect(),
            };
            if indices.iter().any(|i| *i as usize >= count) {
                return Err("a triangle uses a vertex that does not exist".to_owned());
            }
            let normals: Vec<[f32; 3]> = match reader.read_normals() {
                Some(normals) => normals.collect(),
                None => flat_normals(&positions, &indices),
            };
            let uvs: Vec<[f32; 2]> = match reader.read_tex_coords(0) {
                Some(uvs) => uvs.into_f32().collect(),
                None => vec![[0.0, 0.0]; count],
            };
            if normals.len() != count || uvs.len() != count {
                return Err("a mesh part has a different number of normals or texture coordinates than positions".to_owned());
            }
            mesh.positions.extend(positions.iter().map(|p| world.transform_point3(glam::Vec3::from(*p)).to_array()));
            mesh.normals.extend(normals.iter().map(|n| (normal_matrix * glam::Vec3::from(*n)).normalize_or_zero().to_array()));
            mesh.uvs.extend(uvs);
            mesh.indices.extend(indices.into_iter().map(|i| base + i));
        }
    }
    for child in node.children() {
        add_node(&child, world, buffers, mesh, depth + 1)?;
    }
    Ok(())
}

/// Each vertex gets the normal of the last triangle that uses it; for models without normals.
fn flat_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    for tri in indices.as_chunks::<3>().0 {
        let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| glam::Vec3::from(positions[i as usize]));
        let n = (b - a).cross(c - a).normalize_or(glam::Vec3::Y).to_array();
        for i in tri {
            normals[*i as usize] = n;
        }
    }
    normals
}

/// A loaded file and the change time it was read at.
type Cached<T> = HashMap<PathBuf, (Option<SystemTime>, Result<T, String>)>;

/// Loaded images and models, read again when their file changes.
#[derive(Default)]
pub struct AssetCache {
    images: Cached<Arc<Image>>,
    models: Cached<Arc<MeshData>>,
}

impl AssetCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn image(&mut self, path: &Path) -> Result<Arc<Image>, String> {
        cached(&mut self.images, path, |p| load_image(p).map(Arc::new))
    }

    pub fn model(&mut self, path: &Path) -> Result<Arc<MeshData>, String> {
        cached(&mut self.models, path, |p| load_model(p).map(Arc::new))
    }
}

fn cached<T: Clone>(
    map: &mut HashMap<PathBuf, (Option<SystemTime>, Result<T, String>)>,
    path: &Path,
    load: impl Fn(&Path) -> Result<T, String>,
) -> Result<T, String> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let Some((seen, result)) = map.get(path)
        && *seen == modified
        && modified.is_some()
    {
        return result.clone();
    }
    let result = load(path);
    map.insert(path.to_path_buf(), (modified, result.clone()));
    result
}

#[cfg(test)]
mod tests;
