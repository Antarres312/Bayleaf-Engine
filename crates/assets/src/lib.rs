//! L2: reading asset files: images (PNG) and 3D models (glTF 2.0).
//!
//! Loaders turn files into plain data the renderer uploads. Each result carries an id that
//! changes when the file changes, so caches know when to load again. Limits keep a broken or
//! hostile file from using all memory. Import settings beside a file ([`import`]) say how to read
//! it, and are part of what makes its id change.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

pub mod import;
mod neutral;

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
    /// Drawn with smaller copies for far away (its import settings can turn them off).
    pub mipmaps: bool,
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
    /// The model's own material, from its first triangles that have one.
    pub material: Option<ModelMaterial>,
    /// Runs of triangles that share a material, in index order. A model with one material has one.
    pub parts: Vec<MeshPart>,
    /// What was left out or repaired while reading (an image that could not be read, for one).
    pub warnings: Vec<String>,
}

/// Triangles `indices[first..first + count]` and the material they use.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPart {
    pub first: u32,
    pub count: u32,
    pub material: Option<ModelMaterial>,
}

/// The parts of a glTF material the engine draws: base colour (linear RGBA, times the image when
/// there is one), metallic and roughness (times their image), and a normal map.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelMaterial {
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    /// The base colour image, read by the first texture coordinates.
    pub image: Option<Arc<Image>>,
    /// Roughness in green and metallic in blue, times the numbers above.
    pub metal_roughness: Option<Arc<Image>>,
    /// A tangent-space normal map.
    pub normal: Option<Arc<Image>>,
}

/// When the import settings beside `path` last changed, if it has them.
fn settings_modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(import::settings_path(path)).and_then(|m| m.modified()).ok()
}

fn file_id(path: &Path, modified: Option<SystemTime>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    modified.hash(&mut hasher);
    settings_modified(path).hash(&mut hasher);
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

/// File extensions of the images this crate reads.
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "exr"];

/// Reads a PNG, JPEG or OpenEXR file. The kind comes from the file's first bytes, not its name.
/// Its import settings apply.
pub fn load_image(path: &Path) -> Result<Image, String> {
    let settings = import::ImportSettings::read_for(path)?;
    let (bytes, modified) = read_limited(path)?;
    let mut image = decode_image(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    image.id = file_id(path, modified);
    image.mipmaps = settings.and_then(|s| s.image).unwrap_or_default().mipmaps;
    Ok(image)
}

/// One size of an image: width, height and RGBA pixels.
pub type MipLevel = (u32, u32, Vec<u8>);

/// The smaller copies of an image, each half the size of the one before (rounded down, at least
/// 1), down to 1 x 1, not counting the image itself. Colours are averaged as light (in linear
/// space, not as sRGB numbers), so a fine black and white pattern turns light grey, not dark grey.
pub fn mip_levels(image: &Image) -> Vec<MipLevel> {
    let to_linear: Vec<f32> = (0..=255u8).map(|v| srgb_to_linear(f32::from(v) / 255.0)).collect();
    let mut levels = Vec::new();
    let (mut w, mut h, mut src) = (image.width.max(1), image.height.max(1), image.rgba.clone());
    while w > 1 || h > 1 {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut out = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let mut sum = [0.0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = ((x * 2 + dx).min(w - 1), (y * 2 + dy).min(h - 1));
                    let at = ((sy * w + sx) * 4) as usize;
                    for c in 0..3 {
                        sum[c] += to_linear[usize::from(src[at + c])];
                    }
                    sum[3] += f32::from(src[at + 3]) / 255.0;
                }
                let at = ((y * nw + x) * 4) as usize;
                for c in 0..3 {
                    out[at + c] = (linear_to_srgb(sum[c] / 4.0) * 255.0).round() as u8;
                }
                out[at + 3] = (sum[3] / 4.0 * 255.0).round() as u8;
            }
        }
        levels.push((nw, nh, out.clone()));
        (w, h, src) = (nw, nh, out);
    }
    levels
}

fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// Decodes PNG, JPEG or OpenEXR bytes into RGBA. The id is left at 0.
pub fn decode_image(bytes: &[u8]) -> Result<Image, String> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        decode_jpeg(bytes)
    } else if bytes.starts_with(&EXR_MAGIC) {
        decode_exr(bytes)
    } else {
        decode_png(bytes)
    }
}

/// The first bytes of an OpenEXR file.
const EXR_MAGIC: [u8; 4] = [0x76, 0x2f, 0x31, 0x01];

/// Most pixels an OpenEXR picture may have (8192 × 8192).
const MAX_EXR_PIXELS: u64 = 1 << 26;

/// An OpenEXR picture's colour: the part with the most colour channels, its R, G, B and A (or Y
/// as grey). Its values are linear light; like every image here it is kept as 8-bit sRGB, so
/// values above 1 are clipped to white.
pub fn decode_exr(bytes: &[u8]) -> Result<Image, String> {
    use craft_exr::decode;
    let mut limits = decode::Limits::default();
    limits.max_pixels = MAX_EXR_PIXELS;
    limits.max_bytes = MAX_EXR_PIXELS * 20;
    let parts = decode::parts(bytes, &limits).map_err(|e| e.to_string())?;
    let index = decode::pick_part(&parts).ok_or("the picture has no colour channels")?;
    let part_name = parts.iter().find(|p| p.index == index).and_then(|p| p.name.clone());
    let named = |channel: &str| craft_exr::channels::part_channel_name(part_name.as_deref(), channel);
    let wanted = ["R", "G", "B", "A", "Y"].map(named);
    let keep = |name: &str| wanted.iter().any(|w| w == name);
    let fill = |name: &str| if name == wanted[3] { 1.0 } else { 0.0 };
    let part = decode::decode_display_part(bytes, index, &keep, &fill, &limits).map_err(|e| e.to_string())?;
    let (width, height) = (part.info.display_window.width, part.info.display_window.height);
    let pixels = width as usize * height as usize;
    let channel = |name: &str| part.channels.iter().find(|c| c.name == name).map(|c| c.samples.as_slice()).filter(|s| s.len() == pixels);
    let grey = channel(&wanted[4]);
    let [r, g, b] = [0, 1, 2].map(|i| channel(&wanted[i]).or(grey));
    let (Some(r), Some(g), Some(b)) = (r, g, b) else { return Err("the picture has no R, G and B (or Y) channels".to_owned()) };
    let alpha = channel(&wanted[3]);
    let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let mut rgba = Vec::with_capacity(pixels * 4);
    for i in 0..pixels {
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        rgba.extend([r[i], g[i], b[i]].map(|v| to_byte(linear_to_srgb(finite(v).clamp(0.0, 1.0)))));
        rgba.push(alpha.map_or(255, |a| to_byte(finite(a[i]))));
    }
    Ok(Image { id: 0, width, height, rgba, mipmaps: true })
}

/// Decodes JPEG bytes into RGBA (JPEG has no transparency, so alpha is 255). The id is left at 0.
pub fn decode_jpeg(bytes: &[u8]) -> Result<Image, String> {
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let side = MAX_IMAGE_SIDE as usize;
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA).set_max_width(side).set_max_height(side);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(zune_core::bytestream::ZCursor::new(bytes), options);
    decoder.decode_headers().map_err(|e| format!("not a JPEG image: {e:?}"))?;
    let (width, height) = decoder.dimensions().ok_or("not a JPEG image")?;
    if width == 0 || height == 0 || width > side || height > side {
        return Err(format!("images may be at most {MAX_IMAGE_SIDE} pixels on a side, not {width} x {height}"));
    }
    let rgba = decoder.decode().map_err(|e| format!("broken JPEG image: {e:?}"))?;
    if rgba.len() != width * height * 4 {
        return Err("broken JPEG image: wrong number of pixels".to_owned());
    }
    Ok(Image { id: 0, width: width as u32, height: height as u32, rgba, mipmaps: true })
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
    Ok(Image { id: 0, width: frame.width, height: frame.height, rgba, mipmaps: true })
}

/// The files a model names beside it (glTF buffers and images, OBJ material libraries and
/// images), relative to its folder, each once, in the order the model names them. A `.glb` with
/// everything inside names none.
pub fn model_files(path: &Path) -> Result<Vec<PathBuf>, String> {
    if neutral::reads(&extension_of(path)) {
        return neutral::model_files(path);
    }
    let (bytes, _) = read_limited(path)?;
    let gltf = gltf::Gltf::from_slice(&bytes).map_err(|e| format!("{}: not a glTF 2.0 model: {e}", path.display()))?;
    let mut uris: Vec<&str> = Vec::new();
    for buffer in gltf.buffers() {
        if let gltf::buffer::Source::Uri(uri) = buffer.source() {
            uris.push(uri);
        }
    }
    for image in gltf.images() {
        if let gltf::image::Source::Uri { uri, .. } = image.source() {
            uris.push(uri);
        }
    }
    let mut files: Vec<PathBuf> = Vec::new();
    for uri in uris.into_iter().filter(|u| !u.starts_with("data:")) {
        let file = buffer_path(Path::new(""), uri).ok_or_else(|| format!("{}: {uri} must be a file next to the model", path.display()))?;
        if !files.contains(&file) {
            files.push(file);
        }
    }
    Ok(files)
}

/// File extensions of the models this crate reads.
pub const MODEL_EXTENSIONS: &[&str] = &["glb", "gltf", "obj"];

fn extension_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Reads a model: glTF 2.0 (a `.glb` file, or a `.gltf` file with its buffers in files beside it),
/// or Wavefront OBJ with its material libraries. Its import settings apply: the scale, and
/// turning a Z-up model to Y up.
pub fn load_model(path: &Path) -> Result<MeshData, String> {
    if neutral::reads(&extension_of(path)) {
        return neutral::load_model(path);
    }
    load_gltf(path)
}

fn load_gltf(path: &Path) -> Result<MeshData, String> {
    let settings = import::ImportSettings::read_for(path)?.and_then(|s| s.model).unwrap_or_default();
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
        material: None,
        parts: Vec::new(),
        warnings: Vec::new(),
    };
    let scene = gltf.default_scene().or_else(|| gltf.scenes().next()).ok_or_else(|| format!("{}: the model has no scene", path.display()))?;
    // Runs of indices and the glTF material each uses.
    let mut runs: Vec<(u32, u32, Option<usize>)> = Vec::new();
    let up = match settings.up_axis {
        import::UpAxis::Y => glam::Mat4::IDENTITY,
        // Z up, Y forward becomes Y up, Z toward the viewer.
        import::UpAxis::Z => glam::Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2),
    };
    let root = glam::Mat4::from_scale(glam::Vec3::splat(settings.scale)) * up;
    for node in scene.nodes() {
        add_node(&node, root, &buffers, &mut mesh, &mut runs, 0).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let mut materials: HashMap<usize, ModelMaterial> = HashMap::new();
    for (first, count, index) in runs {
        let material = match index.and_then(|i| gltf.materials().nth(i).map(|m| (i, m))) {
            Some((i, m)) => {
                if let std::collections::hash_map::Entry::Vacant(slot) = materials.entry(i) {
                    slot.insert(model_material(&m, &buffers, folder, mesh.id).map_err(|e| format!("{}: {e}", path.display()))?);
                }
                materials.get(&i).cloned()
            }
            None => None,
        };
        mesh.parts.push(MeshPart { first, count, material });
    }
    mesh.material = mesh.parts.iter().find_map(|p| p.material.clone());
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

/// The base colour, metallic, roughness and images of a glTF material.
fn model_material(material: &gltf::Material<'_>, buffers: &[Vec<u8>], folder: &Path, model_id: u64) -> Result<ModelMaterial, String> {
    let pbr = material.pbr_metallic_roughness();
    let read = |texture: Option<(gltf::Texture<'_>, u32)>, what: &str| match texture {
        Some((texture, 0)) => texture_image(&texture, buffers, folder, model_id).map(Some).map_err(|e| format!("{what} image: {e}")),
        // Images read by a second set of texture coordinates are left out.
        _ => Ok(None),
    };
    let image = read(pbr.base_color_texture().map(|i| (i.texture(), i.tex_coord())), "base colour")?;
    let metal_roughness = read(pbr.metallic_roughness_texture().map(|i| (i.texture(), i.tex_coord())), "metallic and roughness")?;
    let normal = read(material.normal_texture().map(|i| (i.texture(), i.tex_coord())), "normal")?;
    let finite = |v: f32, fallback: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { fallback };
    Ok(ModelMaterial {
        base_color: pbr.base_color_factor().map(|c| finite(c, 1.0)),
        metallic: finite(pbr.metallic_factor(), 1.0),
        roughness: finite(pbr.roughness_factor(), 1.0),
        image,
        metal_roughness,
        normal,
    })
}

/// The image a glTF texture shows: inside the binary buffer, or a file beside the model.
fn texture_image(texture: &gltf::Texture<'_>, buffers: &[Vec<u8>], folder: &Path, model_id: u64) -> Result<Arc<Image>, String> {
    let source = texture.source();
    let bytes = match source.source() {
        gltf::image::Source::View { view, .. } => {
            let buffer = buffers.get(view.buffer().index()).ok_or("its buffer is missing")?;
            let end = view.offset().checked_add(view.length()).ok_or("it lies outside its buffer")?;
            buffer.get(view.offset()..end).ok_or("it lies outside its buffer")?.to_vec()
        }
        gltf::image::Source::Uri { uri, .. } if uri.starts_with("data:") => {
            return Err("images inside the file as text are not supported yet; save the model as .glb".to_owned());
        }
        gltf::image::Source::Uri { uri, .. } => {
            let file = buffer_path(folder, uri).ok_or_else(|| format!("{uri} must be a file next to the model"))?;
            read_limited(&file)?.0
        }
    };
    let mut image = decode_image(&bytes)?;
    // Distinct from the model's id and from its other images.
    image.id = (model_id ^ (source.index() as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)) | 1;
    Ok(Arc::new(image))
}
fn add_node(
    node: &gltf::Node<'_>,
    parent: glam::Mat4,
    buffers: &[Vec<u8>],
    mesh: &mut MeshData,
    runs: &mut Vec<(u32, u32, Option<usize>)>,
    depth: usize,
) -> Result<(), String> {
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
            let (first, count) = (mesh.indices.len() as u32, indices.len() as u32);
            mesh.indices.extend(indices.into_iter().map(|i| base + i));
            // Neighbouring parts with the same material draw as one.
            let material = primitive.material().index();
            match runs.last_mut() {
                Some(last) if last.2 == material && last.0 + last.1 == first => last.1 += count,
                _ => runs.push((first, count, material)),
            }
        }
    }
    for child in node.children() {
        add_node(&child, world, buffers, mesh, runs, depth + 1)?;
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

/// Each file's result, by when the file and its import settings last changed.
type Cached<T> = HashMap<PathBuf, ((Option<SystemTime>, Option<SystemTime>), Result<T, String>)>;

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

fn cached<T: Clone>(map: &mut Cached<T>, path: &Path, load: impl Fn(&Path) -> Result<T, String>) -> Result<T, String> {
    // The file and its import settings: either changing loads it again.
    let modified = (std::fs::metadata(path).and_then(|m| m.modified()).ok(), settings_modified(path));
    if let Some((seen, result)) = map.get(path)
        && *seen == modified
        && modified.0.is_some()
    {
        return result.clone();
    }
    let result = load(path);
    map.insert(path.to_path_buf(), (modified, result.clone()));
    result
}

#[cfg(test)]
mod tests;
