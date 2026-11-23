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
pub mod skeleton;

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
    /// Plain numbers rather than colours (a normal map, a mask): read as stored, not as sRGB.
    pub data: bool,
    /// The image in 4 x 4 blocks for the GPU, when its import settings ask for it: each level of
    /// [`mip_levels`] too when it has mipmaps.
    pub blocks: Option<Arc<Blocks>>,
}

/// An image block-compressed for the GPU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blocks {
    pub compression: import::Compression,
    /// The blocks of the image, then of each smaller copy.
    pub levels: Vec<Vec<u8>>,
}

impl Image {
    /// An image of `rgba` pixels: colours, with mipmaps, kept as it is.
    pub fn new(id: u64, width: u32, height: u32, rgba: Vec<u8>) -> Self {
        Self { id, width, height, rgba, mipmaps: true, data: false, blocks: None }
    }
}

/// Block-compresses `image` (and its smaller copies when it has mipmaps). Its sides must be
/// multiples of 4.
pub fn compress(image: &Image, compression: import::Compression) -> Result<Blocks, String> {
    let kind = match compression {
        import::Compression::None => return Err("no compression asked for".to_owned()),
        import::Compression::Bc1 => craft_dds::DdsCompression::Bc1,
        import::Compression::Bc3 => craft_dds::DdsCompression::Bc3,
    };
    if !image.width.is_multiple_of(4) || !image.height.is_multiple_of(4) {
        return Err(format!("block compression needs sides that are multiples of 4; this image is {} x {}", image.width, image.height));
    }
    let mut sizes = vec![(image.width, image.height, image.rgba.clone())];
    if image.mipmaps {
        sizes.extend(mip_levels(image));
    }
    let options = craft_dds::DdsOptions { compression: kind, mipmaps: false };
    let block_bytes = if kind == craft_dds::DdsCompression::Bc1 { 8 } else { 16 };
    let mut levels = Vec::with_capacity(sizes.len());
    for (width, height, pixels) in sizes {
        let file = craft_dds::encode(width, height, craft_dds::Layout::Rgba, &pixels, options).map_err(|e| format!("{e:?}"))?;
        // The blocks follow the 128-byte header ("DDS " and DDS_HEADER; BC1 and BC3 need no DX10 one).
        let blocks = file.get(128..).ok_or("the compressed image came out empty")?;
        let expected = width.div_ceil(4) as usize * height.div_ceil(4) as usize * block_bytes;
        if blocks.len() != expected {
            return Err(format!("the compressed image has {} bytes, not {expected}", blocks.len()));
        }
        levels.push(blocks.to_vec());
    }
    Ok(Blocks { compression, levels })
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
    /// The model's nodes and animations, when it has any: the vertices above are its rest pose.
    pub skeleton: Option<Arc<skeleton::Skeleton>>,
    /// Changes when the vertices change while the id stays (a model posed by its animation).
    pub version: u64,
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
    bayleaf_core::files::modified(import::settings_path(path))
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
    let size = bayleaf_core::files::len(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if size > MAX_FILE_BYTES {
        return Err(format!("{} is larger than {} MiB", path.display(), MAX_FILE_BYTES >> 20));
    }
    let bytes = bayleaf_core::files::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok((bytes, bayleaf_core::files::modified(path)))
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
    let settings = settings.and_then(|s| s.image).unwrap_or_default();
    image.mipmaps = settings.mipmaps;
    image.data = settings.color_space == import::ColorSpace::Data;
    if settings.compression != import::Compression::None {
        // asset.importSettings.set refuses sides that cannot be compressed; a file changed since
        // keeps its pixels as they are (its asset info says so).
        image.blocks = compress(&image, settings.compression).ok().map(Arc::new);
    }
    Ok(image)
}

/// One size of an image: width, height and RGBA pixels.
pub type MipLevel = (u32, u32, Vec<u8>);

/// The smaller copies of an image, each half the size of the one before (rounded down, at least
/// 1), down to 1 x 1, not counting the image itself. Colours are averaged as light (in linear
/// space, not as sRGB numbers), so a fine black and white pattern turns light grey, not dark grey;
/// a data image's numbers are averaged as they are.
pub fn mip_levels(image: &Image) -> Vec<MipLevel> {
    let to_linear: Vec<f32> = (0..=255u8).map(|v| if image.data { f32::from(v) / 255.0 } else { srgb_to_linear(f32::from(v) / 255.0) }).collect();
    let back = |v: f32| if image.data { v.clamp(0.0, 1.0) } else { linear_to_srgb(v) };
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
                    out[at + c] = (back(sum[c] / 4.0) * 255.0).round() as u8;
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
    Ok(Image::new(0, width, height, rgba))
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
    Ok(Image::new(0, width as u32, height as u32, rgba))
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
    Ok(Image::new(0, frame.width, frame.height, rgba))
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
        skeleton: None,
        version: 0,
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
    let mut rig = Rig::default();
    for node in scene.nodes() {
        let mut gathered = Gathered { buffers: &buffers, root, mesh: &mut mesh, runs: &mut runs, rig: &mut rig };
        add_node(&node, glam::Mat4::IDENTITY, &mut gathered, 0).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    if gltf.animations().len() > 0 || gltf.skins().len() > 0 {
        let skeleton = rig_skeleton(&gltf, &scene, &buffers, rig, root, &mut mesh.warnings).map_err(|e| format!("{}: {e}", path.display()))?;
        // Skinned parts stand as their joints hold them.
        let rest = skeleton.posed(&mesh, None, 0.0, false, mesh.id, 0);
        mesh.positions = rest.positions;
        mesh.normals = rest.normals;
        mesh.skeleton = Some(Arc::new(skeleton));
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
    let metal_roughness = read(pbr.metallic_roughness_texture().map(|i| (i.texture(), i.tex_coord())), "metallic and roughness")?.map(as_data);
    let normal = read(material.normal_texture().map(|i| (i.texture(), i.tex_coord())), "normal")?.map(as_data);
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

/// The same image read as plain numbers (a model's normal or metallic and roughness image).
fn as_data(image: Arc<Image>) -> Arc<Image> {
    if image.data {
        return image;
    }
    // Its own id: the same file may also be shown as a colour.
    Arc::new(Image { data: true, id: image.id ^ 0x2, ..(*image).clone() })
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
/// What a model's vertices are bound to, gathered as its parts are read.
#[derive(Default)]
struct Rig {
    bindings: Vec<skeleton::Binding>,
    /// Binding index by skin and joint, or (no skin) by node.
    binding_of: HashMap<(Option<usize>, usize), u16>,
    /// Each skin's inverse bind matrices.
    skins: HashMap<usize, Vec<glam::Mat4>>,
    vertex_bindings: Vec<[u16; 4]>,
    vertex_weights: Vec<[f32; 4]>,
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
}

impl Rig {
    fn bind(&mut self, key: (Option<usize>, usize), binding: impl FnOnce() -> Result<skeleton::Binding, String>) -> Result<u16, String> {
        if let Some(index) = self.binding_of.get(&key) {
            return Ok(*index);
        }
        let index = u16::try_from(self.bindings.len()).map_err(|_| "the model has more than 65535 joints and parts".to_owned())?;
        self.bindings.push(binding()?);
        self.binding_of.insert(key, index);
        Ok(index)
    }
}

/// A skinned part's skin, and its vertices' joints and weights.
type Skinned<'a, 'b> = (&'a gltf::Skin<'b>, &'a [[u16; 4]], &'a [[f32; 4]]);

/// A part's vertices as read, and its node's matrix into the model's space.
#[derive(Clone, Copy)]
struct Part<'a> {
    positions: &'a [[f32; 3]],
    normals: &'a [[f32; 3]],
    local: glam::Mat4,
    local_normals: glam::Mat3,
}

/// Where a model's parts are gathered.
struct Gathered<'a> {
    buffers: &'a [Vec<u8>],
    root: glam::Mat4,
    mesh: &'a mut MeshData,
    runs: &'a mut Vec<(u32, u32, Option<usize>)>,
    rig: &'a mut Rig,
}

/// The skeleton of a glTF model: its nodes at rest, the bindings its parts gathered, and its
/// animations.
fn rig_skeleton(
    gltf: &gltf::Gltf,
    scene: &gltf::Scene<'_>,
    buffers: &[Vec<u8>],
    rig: Rig,
    root: glam::Mat4,
    warnings: &mut Vec<String>,
) -> Result<skeleton::Skeleton, String> {
    let mut nodes: Vec<skeleton::Node> = gltf
        .nodes()
        .map(|n| {
            let (t, r, s) = n.transform().decomposed();
            skeleton::Node { parent: None, translation: glam::Vec3::from(t), rotation: glam::Quat::from_array(r).normalize(), scale: glam::Vec3::from(s) }
        })
        .collect();
    for node in gltf.nodes() {
        for child in node.children() {
            if let Some(slot) = nodes.get_mut(child.index()) {
                slot.parent = Some(node.index());
            }
        }
    }
    // Parents before children: down from the scene's roots, then any node left over.
    let mut order = Vec::with_capacity(nodes.len());
    let mut seen = vec![false; nodes.len()];
    let mut stack: Vec<usize> = scene.nodes().map(|n| n.index()).collect();
    stack.extend((0..nodes.len()).filter(|i| nodes[*i].parent.is_none()));
    stack.reverse();
    while let Some(i) = stack.pop() {
        if seen.get(i).copied().unwrap_or(true) {
            continue;
        }
        seen[i] = true;
        order.push(i);
        if let Some(node) = gltf.nodes().nth(i) {
            stack.extend(node.children().map(|c| c.index()).collect::<Vec<_>>().into_iter().rev());
        }
    }
    let mut animations = Vec::new();
    let mut keys = 0usize;
    for animation in gltf.animations() {
        let name = animation.name().map_or_else(|| format!("Animation {}", animation.index() + 1), str::to_owned);
        let mut channels = Vec::new();
        let mut duration = 0.0f32;
        for channel in animation.channels() {
            let reader = channel.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
            let property = match channel.target().property() {
                gltf::animation::Property::Translation => skeleton::Property::Translation,
                gltf::animation::Property::Rotation => skeleton::Property::Rotation,
                gltf::animation::Property::Scale => skeleton::Property::Scale,
                gltf::animation::Property::MorphTargetWeights => {
                    warnings.push(format!("{name}: shape blending (morph targets) is left out"));
                    continue;
                }
            };
            let times: Vec<f32> = reader.read_inputs().ok_or_else(|| format!("{name}: a channel has no key times"))?.collect();
            let mut values: Vec<[f32; 4]> = match reader.read_outputs() {
                Some(gltf::animation::util::ReadOutputs::Translations(v)) | Some(gltf::animation::util::ReadOutputs::Scales(v)) => {
                    v.map(|[x, y, z]| [x, y, z, 0.0]).collect()
                }
                Some(gltf::animation::util::ReadOutputs::Rotations(v)) => v.into_f32().collect(),
                _ => return Err(format!("{name}: a channel has no key values")),
            };
            let interpolation = channel.sampler().interpolation();
            if interpolation == gltf::animation::Interpolation::CubicSpline {
                // In-tangent, value, out-tangent for each key: the values.
                values = values.chunks(3).filter_map(|c| c.get(1).copied()).collect();
            }
            keys += times.len();
            if keys > skeleton::MAX_KEYS {
                return Err(format!("the animations have more than {} keys", skeleton::MAX_KEYS));
            }
            if values.len() != times.len() || times.windows(2).any(|w| w[0] > w[1]) || times.iter().any(|t| !t.is_finite()) {
                warnings.push(format!("{name}: a channel with keys out of order or unmatched is left out"));
                continue;
            }
            duration = duration.max(times.last().copied().unwrap_or(0.0));
            let step = interpolation == gltf::animation::Interpolation::Step;
            channels.push(skeleton::Channel { node: channel.target().node().index(), property, step, times, values });
        }
        animations.push(skeleton::Animation { name, duration, channels });
    }
    Ok(skeleton::Skeleton {
        nodes,
        order,
        bindings: rig.bindings,
        vertex_bindings: rig.vertex_bindings,
        vertex_weights: rig.vertex_weights,
        positions: rig.positions,
        normals: rig.normals,
        root,
        animations,
    })
}

fn add_node(node: &gltf::Node<'_>, parent: glam::Mat4, gathered: &mut Gathered<'_>, depth: usize) -> Result<(), String> {
    if depth > 64 {
        return Err("the node tree is too deep".to_owned());
    }
    let (buffers, root) = (gathered.buffers, gathered.root);
    let local = parent * glam::Mat4::from_cols_array_2d(&node.transform().matrix());
    let world = root * local;
    let normal_matrix = glam::Mat3::from_mat4(world).inverse().transpose();
    let local_normals = glam::Mat3::from_mat4(local).inverse().transpose();
    if let Some(gltf_mesh) = node.mesh() {
        for primitive in gltf_mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(Vec::as_slice));
            let Some(positions) = reader.read_positions() else { continue };
            let positions: Vec<[f32; 3]> = positions.collect();
            let mesh = &mut *gathered.mesh;
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
            let (first, index_count) = (mesh.indices.len() as u32, indices.len() as u32);
            mesh.indices.extend(indices.into_iter().map(|i| base + i));
            // Neighbouring parts with the same material draw as one.
            let material = primitive.material().index();
            match gathered.runs.last_mut() {
                Some(last) if last.2 == material && last.0 + last.1 == first => last.1 += index_count,
                _ => gathered.runs.push((first, index_count, material)),
            }
            let skin = node.skin().and_then(|skin| {
                let joints: Vec<[u16; 4]> = reader.read_joints(0)?.into_u16().collect();
                let weights: Vec<[f32; 4]> = reader.read_weights(0)?.into_f32().collect();
                Some((skin, joints, weights))
            });
            let part = Part { positions: &positions, normals: &normals, local, local_normals };
            bind_part(node.index(), skin.as_ref().map(|(s, j, w)| (s, j.as_slice(), w.as_slice())), &part, gathered)?;
        }
    }
    for child in node.children() {
        add_node(&child, local, gathered, depth + 1)?;
    }
    Ok(())
}

/// What a part's vertices follow when the model moves: its skin's joints by their weights, or
/// else its own node.
fn bind_part(node: usize, skinned: Option<Skinned<'_, '_>>, part: &Part<'_>, gathered: &mut Gathered<'_>) -> Result<(), String> {
    let buffers = gathered.buffers;
    let rig = &mut *gathered.rig;
    let Part { positions, normals, local, local_normals } = *part;
    let count = positions.len();
    if let Some((skin, joints, weights)) = skinned {
        if joints.len() != count || weights.len() != count {
            return Err("a skinned part has a different number of joints or weights than positions".to_owned());
        }
        rig.skins.entry(skin.index()).or_insert_with(|| {
            skin.reader(|b| buffers.get(b.index()).map(Vec::as_slice))
                .read_inverse_bind_matrices()
                .map(|m| m.map(|c| glam::Mat4::from_cols_array_2d(&c)).collect())
                .unwrap_or_default()
        });
        for (joint_set, weight_set) in joints.iter().zip(weights) {
            let mut bound = [0u16; 4];
            for (slot, joint) in bound.iter_mut().zip(joint_set) {
                let joint = usize::from(*joint);
                let inverse = rig.skins.get(&skin.index()).and_then(|m| m.get(joint)).copied().unwrap_or(glam::Mat4::IDENTITY);
                let node = skin.joints().nth(joint).map(|n| n.index()).ok_or("a vertex names a joint its skin does not have")?;
                *slot = rig.bind((Some(skin.index()), joint), || Ok(skeleton::Binding { node, inverse_bind: inverse }))?;
            }
            let total: f32 = weight_set.iter().filter(|w| w.is_finite() && **w > 0.0).sum();
            let weights = if total > 0.0 { weight_set.map(|w| if w.is_finite() && w > 0.0 { w / total } else { 0.0 }) } else { [1.0, 0.0, 0.0, 0.0] };
            rig.vertex_bindings.push(bound);
            rig.vertex_weights.push(weights);
        }
        // A skinned part's vertices are in its skin's space: its own node does not move them.
        rig.positions.extend_from_slice(positions);
        rig.normals.extend_from_slice(normals);
    } else {
        let inverse = local.inverse();
        let bound = rig.bind((None, node), || Ok(skeleton::Binding { node, inverse_bind: inverse }))?;
        rig.vertex_bindings.extend(std::iter::repeat_n([bound, 0, 0, 0], count));
        rig.vertex_weights.extend(std::iter::repeat_n([1.0, 0.0, 0.0, 0.0], count));
        rig.positions.extend(positions.iter().map(|p| local.transform_point3(glam::Vec3::from(*p)).to_array()));
        rig.normals.extend(normals.iter().map(|n| (local_normals * glam::Vec3::from(*n)).normalize_or_zero().to_array()));
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
    let modified = (bayleaf_core::files::modified(path), settings_modified(path));
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
