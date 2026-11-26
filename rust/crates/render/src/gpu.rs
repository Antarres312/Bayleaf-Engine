//! GPU side: pipelines, buffers and an offscreen render target.

use std::collections::HashMap;

use bayleaf_core::math::{Mat4, Vec3};
use bayleaf_scene::components::Shape;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::extract::{DrawItem, Frame, LightKind, MAX_LIGHTS, normal_matrix};
use crate::material::CompiledMaterial;
use crate::mesh::{self, Vertex};

/// Colour format of [`RenderTarget`]. sRGB, so shaders write linear colour.
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// The scene's own colour, before post-processing makes the displayed picture: high range, linear.
pub(crate) const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Samples per pixel of a target made with [`RenderTarget::new`]; 1 draws without multisampling.
pub const DEFAULT_SAMPLES: u32 = 4;
/// The selection mask: one channel, set where a selected shape is.
const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightRaw {
    direction: [f32; 4],
    position: [f32; 4],
    radiance: [f32; 4],
    extra: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    ambient: [f32; 4],
    light_view_proj: [[f32; 4]; 4],
    // x: lights casting shadows, y: one shadow-map texel in uv.
    shadow_count: [f32; 4],
    ambient_ground: [f32; 4],
    inv_view_proj: [[f32; 4]; 4],
    sky_zenith: [f32; 4],
    sky_horizon: [f32; 4],
    sky_ground: [f32; 4],
    fog: [f32; 4],
    // Per light casting shadows: x: its index in `lights`, y: depth bias, z: how far to push tested
    // points along their normal, w: its first layer of the shadow map.
    shadow_lights: [[f32; 4]; MAX_SHADOW_LIGHTS],
    // Per light casting shadows: x: its layers, y: 0 one face, 1 six faces around a point light,
    // 2 sun cascades by view depth.
    shadow_modes: [[f32; 4]; MAX_SHADOW_LIGHTS],
    // Per light casting shadows: the view depths where its second, third and fourth cascades start.
    shadow_splits: [[f32; 4]; MAX_SHADOW_LIGHTS],
    // World to each layer of the shadow map.
    shadow_faces: [[[f32; 4]; 4]; MAX_SHADOW_LAYERS],
    // The ground grid in view coordinates: x, z: the world's position modulo 10 under the eye (the
    // grid's pattern), y: the height of the ground (world y = 0) from the eye.
    grid: [f32; 4],
    // x, y: where the world's X and Z axes (world z = 0 and x = 0) are from the eye, along z and x.
    grid_axes: [f32; 4],
    // The world point the frame's coordinates start from, for materials that read world positions.
    origin: [f32; 4],
    lights: [LightRaw; MAX_LIGHTS],
}

const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Faces of a point light's shadow: six around it.
const SHADOW_FACES: usize = 6;
/// Most lights that cast shadows at once, and most shadow-map layers they use together.
const MAX_SHADOW_LIGHTS: usize = 4;
const MAX_SHADOW_LAYERS: usize = MAX_SHADOW_LIGHTS * SHADOW_FACES;
/// Bytes between face matrices in the face buffer (the uniform offset alignment).
const FACE_STRIDE: u64 = 256;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InstanceRaw {
    model: [[f32; 4]; 4],
    normal: [[f32; 4]; 3],
    color: [f32; 4],
    params: [f32; 4],
    uv_rect: [f32; 4],
    // x: the first of its skin's matrices in the joint texture, y: 1 when skinned there.
    skin: [f32; 4],
}

impl InstanceRaw {
    fn new(item: &DrawItem, selected: bool, skin: Option<u32>) -> Self {
        let n = normal_matrix(item.model);
        Self {
            model: item.model.to_cols_array_2d(),
            normal: [n.x_axis.extend(0.0).to_array(), n.y_axis.extend(0.0).to_array(), n.z_axis.extend(0.0).to_array()],
            color: item.color.to_array(),
            params: [item.metallic, item.roughness, f32::from(u8::from(selected)), f32::from(u8::from(item.unlit))],
            uv_rect: item.uv_rect,
            skin: skin.map_or([0.0; 4], |first| [first as f32, 1.0, 0.0, 0.0]),
        }
    }
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// The model's version its vertices are of.
    version: u64,
}

/// What a frame drew and what the renderer keeps on the GPU.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RenderStats {
    pub draw_calls: u32,
    pub triangles: u64,
    pub mesh_bytes: u64,
    pub image_bytes: u64,
    /// The GPU's own time for the newest frame it has measured, in milliseconds.
    pub gpu_ms: Option<f32>,
    /// Whether the device measures GPU time at all (it needs timestamp queries).
    pub gpu_timed: bool,
}

/// Offscreen colour, multisample and depth textures of one size.
pub struct RenderTarget {
    width: u32,
    height: u32,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    /// The multisampled colour, drawn into and resolved into `hdr_view`; none with 1 sample.
    msaa_view: Option<wgpu::TextureView>,
    samples: u32,
    /// The scene's colour resolved from the multisampled one, which post-processing reads.
    hdr_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    /// Where the selected shapes are, for their outline.
    mask_view: wgpu::TextureView,
    /// Bloom's chain of half-size pictures.
    bloom_views: Vec<wgpu::TextureView>,
}

impl RenderTarget {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self::with_samples(device, width, height, DEFAULT_SAMPLES)
    }

    /// A target drawn with `samples` per pixel (multisampling): 1 for none, else 4 (the count every
    /// device supports).
    pub fn with_samples(device: &wgpu::Device, width: u32, height: u32, samples: u32) -> Self {
        let samples = if samples <= 1 { 1 } else { 4 };
        let (width, height) = (width.max(1), height.max(1));
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = |label, format, samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = texture(
            "bayleaf color",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let msaa = (samples > 1).then(|| texture("bayleaf msaa", HDR_FORMAT, samples, wgpu::TextureUsages::RENDER_ATTACHMENT));
        let hdr = texture("bayleaf scene colour", HDR_FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let depth = texture("bayleaf depth", DEPTH_FORMAT, samples, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let bloom_views = (0..crate::post::BLOOM_LEVELS)
            .map(|level| {
                let size = wgpu::Extent3d { width: (width >> (level + 1)).max(1), height: (height >> (level + 1)).max(1), depth_or_array_layers: 1 };
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("bayleaf bloom"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: HDR_FORMAT,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            })
            .collect();
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let msaa_view = msaa.map(|m| m.create_view(&wgpu::TextureViewDescriptor::default()));
        let hdr_view = hdr.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        let mask = texture("bayleaf selection mask", MASK_FORMAT, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let mask_view = mask.create_view(&wgpu::TextureViewDescriptor::default());
        Self { width, height, color, color_view, msaa_view, samples, hdr_view, depth_view, mask_view, bloom_views }
    }

    pub(crate) fn hdr_view(&self) -> &wgpu::TextureView {
        &self.hdr_view
    }

    pub(crate) fn depth_view(&self) -> &wgpu::TextureView {
        &self.depth_view
    }

    pub(crate) fn bloom_views(&self) -> &[wgpu::TextureView] {
        &self.bloom_views
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Samples per pixel: 1 without multisampling.
    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// The resolved colour texture, for display or copying.
    pub fn texture(&self) -> &wgpu::Texture {
        &self.color
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.color_view
    }

    /// A copy of what was last drawn, in a texture of its own: a picture kept without the target's
    /// multisampled and depth buffers, which the next drawing can reuse.
    pub fn snapshot(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
        let size = wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 };
        let copy = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bayleaf snapshot"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("bayleaf snapshot") });
        encoder.copy_texture_to_texture(self.color.as_image_copy(), copy.as_image_copy(), size);
        queue.submit([encoder.finish()]);
        copy.create_view(&wgpu::TextureViewDescriptor::default())
    }
}

/// Draws [`Frame`]s into [`RenderTarget`]s.
/// The GPU's answer on whether it takes a material's shader: no error, or why not.
type ShaderCheck = std::pin::Pin<Box<dyn std::future::Future<Output = Option<wgpu::Error>>>>;

pub struct Renderer {
    /// Samples per pixel the pipelines draw with; a target of another count makes them again.
    samples: u32,
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    layout: wgpu::PipelineLayout,
    /// Opaque and transparent pipelines by material key; 0 is the built-in surface, `None` a
    /// material whose shader the GPU refused (drawn with the built-in surface instead).
    pipelines: HashMap<u64, Option<(wgpu::RenderPipeline, wgpu::RenderPipeline)>>,
    /// Outline pipelines of the materials that have an outline, by material key.
    outlines: HashMap<u64, wgpu::RenderPipeline>,
    /// Additive pipelines by material key (0 the built-in surface).
    additive: HashMap<u64, wgpu::RenderPipeline>,
    post: crate::post::Post,
    /// Why the last refused material shader was refused.
    material_error: Option<String>,
    /// Materials whose shader the GPU has not yet said it takes or refuses (a browser answers a
    /// frame or more later); their pipelines draw meanwhile.
    checks: Vec<(u64, ShaderCheck)>,
    grid: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    meshes: Vec<(Shape, GpuMesh)>,
    instances: wgpu::Buffer,
    instance_capacity: usize,
    /// The shadow map's layers, to draw into, and its size per side.
    shadow_layers: Vec<wgpu::TextureView>,
    shadow_size: u32,
    shadow_layout: wgpu::BindGroupLayout,
    shadow_sampler: wgpu::Sampler,
    shadow_faces: wgpu::Buffer,
    shadow_face_group: wgpu::BindGroup,
    shadow_group: wgpu::BindGroup,
    shadow_pipeline: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    /// Selection outline: the selected shapes into the mask, then the band around it.
    mask_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    outline_layout: wgpu::BindGroupLayout,
    texture_sampler: wgpu::Sampler,
    /// Uploaded images by id; 0 is a white pixel.
    textures: HashMap<u64, GpuImage>,
    /// Texture bind groups by the images in their four slots and which slots read plain numbers.
    texture_groups: HashMap<TextureKey, wgpu::BindGroup>,
    /// Uploaded models by id.
    models: HashMap<u64, GpuMesh>,
    /// Every skinned item's matrices this frame, four texels a matrix, [`JOINTS_PER_ROW`] to a row;
    /// the vertex shader reads it (group 0, binding 1). Its rows, grown as needed.
    joints: wgpu::Texture,
    joint_rows: u32,
    globals_layout: wgpu::BindGroupLayout,
    /// Moves skinned models' vertices on the CPU and uploads them per item, instead of on the GPU:
    /// for devices without the vertex inputs GPU skinning needs, and for tests.
    cpu_skinning: bool,
    /// A version for each frame's CPU-posed models.
    posed_version: u64,
    /// Draw calls and triangles of the last frame's scene.
    drawn: (u32, u64),
    /// The GPU's time for each frame: made at the first frame (it needs the queue), `None` inside
    /// when the device cannot write timestamps.
    timer: Option<Option<crate::timing::GpuTimer>>,
}

impl Renderer {
    pub fn new(device: &wgpu::Device) -> Self {
        Self::with_samples(device, DEFAULT_SAMPLES)
    }

    /// A renderer for targets of `samples` per pixel. [`Self::render`] makes it again for a target
    /// of another count.
    pub fn with_samples(device: &wgpu::Device, samples: u32) -> Self {
        let samples = if samples <= 1 { 1 } else { 4 };
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bayleaf globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bayleaf globals layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let joints = joint_texture(device, 1);
        let globals_group = globals_group(device, &globals_layout, &globals, &joints);
        // Each layer's matrix, at its own offset, for the shadow pass.
        let shadow_faces = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bayleaf shadow faces"),
            size: FACE_STRIDE * MAX_SHADOW_LAYERS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let face_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bayleaf shadow face layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(64) },
                count: None,
            }],
        });
        let shadow_face_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bayleaf shadow face group"),
            layout: &face_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &shadow_faces, offset: 0, size: wgpu::BufferSize::new(64) }),
            }],
        });
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bayleaf shadow sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bayleaf shadow layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        // A small map for now; the first frame makes it the size the frame's settings ask for.
        let (shadow_group, shadow_layers) = shadow_map(device, &shadow_layout, &shadow_sampler, 1, 1);
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bayleaf texture layout"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let texture_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bayleaf texture sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bayleaf pipeline layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(&shadow_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let shadow_pass_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bayleaf shadow pass layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(&face_layout)],
            immediate_size: 0,
        });
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bayleaf shadow shader"),
            source: wgpu::ShaderSource::Wgsl(with_common(include_str!("shaders/shadow.wgsl")).into()),
        });
        let shadow_pipeline = shadow_pipeline(device, &shadow_pass_layout, &shadow_shader);
        let grid_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bayleaf grid shader"),
            source: wgpu::ShaderSource::Wgsl(with_common(include_str!("shaders/grid.wgsl")).into()),
        });

        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bayleaf mesh shader"),
            source: wgpu::ShaderSource::Wgsl(mesh_source(DEFAULT_SURFACE).into()),
        });
        let mut pipelines = HashMap::new();
        pipelines.insert(0, Some((mesh_pipeline(device, &layout, &mesh_shader, false, samples), mesh_pipeline(device, &layout, &mesh_shader, true, samples))));
        let grid = grid_pipeline(device, &layout, &grid_shader, samples);
        let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bayleaf sky shader"),
            source: wgpu::ShaderSource::Wgsl(with_common(include_str!("shaders/sky.wgsl")).into()),
        });
        let sky = sky_pipeline(device, &layout, &sky_shader, samples);
        let (mask_pipeline, outline_pipeline, outline_layout) = outline_pipelines(device, &globals_layout);
        let post = crate::post::Post::new(device, &globals_layout, with_common(include_str!("shaders/post.wgsl")), samples);
        let additive = HashMap::from([(0, additive_mesh_pipeline(device, &layout, &mesh_shader, samples))]);

        let meshes = Shape::ALL
            .iter()
            .map(|&shape| {
                let data = mesh::build(shape);
                let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("bayleaf vertices"),
                    contents: bytemuck::cast_slice(&data.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("bayleaf indices"),
                    contents: bytemuck::cast_slice(&data.indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
                (shape, GpuMesh { vertices, indices, index_count: data.indices.len() as u32, version: 0 })
            })
            .collect();

        let instance_capacity = 256;
        let instances = instance_buffer(device, instance_capacity);
        Self {
            globals,
            globals_group,
            layout,
            pipelines,
            outlines: HashMap::new(),
            additive,
            post,
            material_error: None,
            checks: Vec::new(),
            grid,
            sky,
            meshes,
            instances,
            instance_capacity,
            shadow_layers,
            shadow_size: 1,
            shadow_layout,
            shadow_sampler,
            shadow_faces,
            shadow_face_group,
            shadow_group,
            shadow_pipeline,
            mask_pipeline,
            outline_pipeline,
            outline_layout,
            texture_layout,
            texture_sampler,
            textures: HashMap::new(),
            texture_groups: HashMap::new(),
            models: HashMap::new(),
            joints,
            joint_rows: 1,
            globals_layout,
            // Sixteen vertex inputs: the vertex's five (with its joints and weights) and the item's eleven.
            cpu_skinning: device.limits().max_vertex_attributes < 16,
            posed_version: 0,
            drawn: (0, 0),
            timer: None,
            samples,
        }
    }

    /// Why the GPU refused the last material shader it was given, if it refused one.
    /// What the last frame drew and what the renderer keeps on the GPU, for the profiler.
    pub fn stats(&self) -> RenderStats {
        let mesh_bytes = self.meshes.iter().map(|(_, m)| m).chain(self.models.values()).map(|m| m.vertices.size() + m.indices.size()).sum();
        let image_bytes = self
            .textures
            .values()
            .map(|image| {
                let texture = &image.texture;
                // The first level's bytes (a quarter or an eighth of four a pixel when compressed),
                // and a third more for the smaller copies.
                let first = texture.format().theoretical_memory_footprint(texture.size());
                if texture.mip_level_count() > 1 { first * 4 / 3 } else { first }
            })
            .sum();
        let timer = self.timer.as_ref().and_then(Option::as_ref);
        RenderStats {
            draw_calls: self.drawn.0,
            triangles: self.drawn.1,
            mesh_bytes,
            image_bytes,
            gpu_ms: timer.and_then(crate::timing::GpuTimer::last),
            gpu_timed: timer.is_some_and(crate::timing::GpuTimer::works),
        }
    }

    pub fn material_error(&self) -> Option<&str> {
        self.material_error.as_deref()
    }

    /// Whether the GPU refused `material`'s shader, so its objects show the built-in surface.
    pub fn refused(&self, material: &CompiledMaterial) -> bool {
        matches!(self.pipelines.get(&material.key), Some(None))
    }

    /// Builds the pipelines of a material the first time it is drawn. A shader the GPU refuses
    /// is remembered as refused, so it is tried once, and its objects use the built-in surface.
    fn prepare(&mut self, device: &wgpu::Device, material: &CompiledMaterial) {
        if self.pipelines.contains_key(&material.key) {
            return;
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bayleaf material shader"),
            source: wgpu::ShaderSource::Wgsl(mesh_source(&material.surface).into()),
        });
        let samples = self.samples;
        let pipelines = (mesh_pipeline(device, &self.layout, &shader, false, samples), mesh_pipeline(device, &self.layout, &shader, true, samples));
        let outline = material.outline.is_some().then(|| outline_mesh_pipeline(device, &self.layout, &shader, samples));
        let additive = additive_mesh_pipeline(device, &self.layout, &shader, samples);
        if let Some(outline) = outline {
            self.outlines.insert(material.key, outline);
        }
        self.additive.insert(material.key, additive);
        self.pipelines.insert(material.key, Some(pipelines));
        // Never waits for the answer: a browser gives it only after this frame, and waiting here
        // would stop the page.
        self.checks.push((material.key, Box::pin(scope.pop())));
        self.settle_checks();
    }

    /// Takes the GPU's answers that have come: a refused shader's material goes back to the
    /// built-in surface.
    fn settle_checks(&mut self) {
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        let mut refused = Vec::new();
        self.checks.retain_mut(|(key, check)| match check.as_mut().poll(&mut context) {
            std::task::Poll::Pending => true,
            std::task::Poll::Ready(answer) => {
                if let Some(error) = answer {
                    refused.push((*key, error_chain(&error)));
                }
                false
            }
        });
        for (key, why) in refused {
            self.pipelines.insert(key, None);
            self.outlines.remove(&key);
            self.additive.remove(&key);
            self.material_error = Some(why);
        }
    }

    /// Draws the texture `id` for items whose image has that id, instead of uploading their
    /// image: a picture drawn on the GPU each frame (an interface standing in the world). Image
    /// groups made with an earlier texture of that id are made again.
    pub fn set_texture(&mut self, id: u64, texture: wgpu::Texture, view: wgpu::TextureView) {
        self.textures.insert(id, GpuImage { texture, color: view.clone(), raw: view });
        self.texture_groups.retain(|key, _| !key.0.contains(&id));
    }

    /// Puts the frame's images and models on the GPU the first time they are drawn, and makes
    /// the image groups its items need.
    fn upload_assets(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &Frame) {
        self.textures.entry(0).or_insert_with(|| upload_image(device, queue, &bayleaf_assets::Image::new(0, 1, 1, vec![255; 4])));
        // A normal straight out of the surface: (0.5, 0.5, 1) read as plain numbers.
        self.textures.entry(FLAT_NORMAL).or_insert_with(|| {
            upload_image(device, queue, &bayleaf_assets::Image { data: true, ..bayleaf_assets::Image::new(FLAT_NORMAL, 1, 1, vec![128, 128, 255, 255]) })
        });
        let mut keys = vec![([0u64; 4], 0u8)];
        for item in &frame.items {
            for image in item.textures.iter().flatten() {
                self.textures.entry(image.id).or_insert_with(|| upload_image(device, queue, image));
            }
            if let Some(mesh) = &item.mesh {
                match self.models.get_mut(&mesh.id) {
                    Some(gpu) if gpu.version == mesh.version => {}
                    // A posed model: the same vertices moved, written over the old ones.
                    Some(gpu) if gpu.vertices.size() == (mesh.positions.len() * std::mem::size_of::<Vertex>()) as u64 => {
                        queue.write_buffer(&gpu.vertices, 0, bytemuck::cast_slice(&model_vertices(mesh)));
                        gpu.version = mesh.version;
                    }
                    _ => {
                        self.models.insert(mesh.id, upload_model(device, mesh));
                    }
                }
            }
            keys.push(texture_key(item));
        }
        for key in keys {
            if self.texture_groups.contains_key(&key) {
                continue;
            }
            let view = |slot: usize| {
                let image = self.textures.get(&key.0[slot]).or_else(|| self.textures.get(&0))?;
                Some(if key.1 & (1 << slot) == 0 { &image.color } else { &image.raw })
            };
            let (Some(v0), Some(v1), Some(v2), Some(v3)) = (view(0), view(1), view(2), view(3)) else { continue };
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bayleaf images"),
                layout: &self.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(v0) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(v1) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(v2) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(v3) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&self.texture_sampler) },
                ],
            });
            self.texture_groups.insert(key, group);
        }
    }

    /// Moves skinned models' vertices on the CPU (and tests) instead of on the GPU, from the next
    /// frame. A device without the vertex inputs GPU skinning needs does so from the start.
    pub fn set_cpu_skinning(&mut self, on: bool) {
        self.cpu_skinning = on;
    }

    /// Whether skinned models are moved on the CPU (see [`Self::set_cpu_skinning`]).
    pub fn cpu_skinning(&self) -> bool {
        self.cpu_skinning
    }

    /// `frame` with each skinned model posed on the CPU: a mesh of its own per object, drawn as
    /// it is.
    fn pose_on_cpu(&mut self, frame: &Frame) -> Frame {
        self.posed_version += 1;
        let mut frame = frame.clone();
        let mut posed: HashMap<u64, std::sync::Arc<bayleaf_assets::MeshData>> = HashMap::new();
        for item in &mut frame.items {
            let Some(mesh) = item.mesh.clone() else { continue };
            let Some(skeleton) = mesh.skeleton.clone() else { continue };
            // Its own id, so each object's pose is its own mesh; its parts share it.
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(&(mesh.id, item.object), &mut hasher);
            let id = std::hash::Hasher::finish(&hasher) | 1;
            let version = self.posed_version;
            let skin = item.skin.take();
            let own = posed.entry(id).or_insert_with(|| {
                let matrices = skin.as_ref().map_or_else(|| skeleton.skin_matrices(&skeleton.pose(None, 0.0)), |s| s.matrices.clone());
                let mut moved = skeleton.skin_on_cpu(&mesh, &matrices, id, version);
                moved.skeleton = None;
                std::sync::Arc::new(moved)
            });
            item.mesh = Some(own.clone());
        }
        frame
    }

    /// Writes the matrices of every skinned item of `items` into the joint texture (each skin once,
    /// however many items share it), growing it as needed; gives each item the first of its
    /// matrices there, or `None` for an item not skinned on the GPU. A model without a skin of its
    /// own stands at rest.
    fn write_joints(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, items: &[&DrawItem]) -> Vec<Option<u32>> {
        let mut data: Vec<[f32; 4]> = Vec::new();
        let mut written: HashMap<*const crate::extract::Skin, u32> = HashMap::new();
        let mut at_rest: HashMap<u64, u32> = HashMap::new();
        let capacity = (MAX_JOINT_ROWS * JOINTS_PER_ROW) as usize;
        let mut firsts = Vec::with_capacity(items.len());
        for item in items {
            let Some((mesh, skeleton)) = item.mesh.as_ref().and_then(|m| gpu_skeleton(m).map(|s| (m, s))) else {
                firsts.push(None);
                continue;
            };
            let count = skeleton.bindings.len();
            let first = (data.len() / 4) as u32;
            let known = match &item.skin {
                Some(skin) => written.get(&std::sync::Arc::as_ptr(skin)).copied(),
                None => at_rest.get(&mesh.id).copied(),
            };
            if let Some(known) = known {
                firsts.push(Some(known));
                continue;
            }
            if data.len() / 4 + count > capacity {
                firsts.push(None);
                continue;
            }
            let matrices = match &item.skin {
                Some(skin) => {
                    written.insert(std::sync::Arc::as_ptr(skin), first);
                    skin.matrices.clone()
                }
                None => {
                    at_rest.insert(mesh.id, first);
                    skeleton.skin_matrices(&skeleton.pose(None, 0.0))
                }
            };
            // Exactly one matrix per binding, so no vertex reads another skin's.
            for i in 0..count {
                let m = matrices.get(i).copied().unwrap_or(skeleton.root);
                data.extend(m.to_cols_array_2d());
            }
            firsts.push(Some(first));
        }
        if data.is_empty() {
            return firsts;
        }
        let rows = (data.len() / 4).div_ceil(JOINTS_PER_ROW as usize) as u32;
        if rows > self.joint_rows {
            self.joint_rows = rows.next_power_of_two().min(MAX_JOINT_ROWS);
            self.joints = joint_texture(device, self.joint_rows);
            self.globals_group = globals_group(device, &self.globals_layout, &self.globals, &self.joints);
        }
        data.resize(rows as usize * JOINTS_PER_ROW as usize * 4, [0.0; 4]);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.joints, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&data),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(JOINTS_PER_ROW * 4 * 16), rows_per_image: Some(rows) },
            wgpu::Extent3d { width: JOINTS_PER_ROW * 4, height: rows, depth_or_array_layers: 1 },
        );
        firsts
    }

    /// The GPU mesh an item draws.
    fn gpu_mesh(&self, item: &DrawItem) -> Option<&GpuMesh> {
        match &item.mesh {
            Some(mesh) => self.models.get(&mesh.id),
            None => self.meshes.iter().find(|(s, _)| *s == item.shape).map(|(_, m)| m),
        }
    }

    fn pipeline(&self, key: u64, transparent: bool) -> Option<&wgpu::RenderPipeline> {
        let pair = match self.pipelines.get(&key) {
            Some(Some(pair)) => pair,
            _ => self.pipelines.get(&0)?.as_ref()?,
        };
        Some(if transparent { &pair.1 } else { &pair.0 })
    }

    /// Records and submits the commands that draw `frame` into `target`.
    pub fn render(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, target: &RenderTarget, frame: &Frame) {
        // Pipelines are made for one sample count: a target of another (multisampling turned on or
        // off) needs them made again, and the images and models are uploaded again with them.
        if target.samples() != self.samples {
            let cpu_skinning = self.cpu_skinning;
            *self = Self::with_samples(device, target.samples());
            self.cpu_skinning |= cpu_skinning;
        }
        let posed;
        let frame = if self.cpu_skinning {
            posed = self.pose_on_cpu(frame);
            &posed
        } else {
            frame
        };
        // The shadow map as large as the settings ask, with a layer for each face the frame's
        // shadow-casting lights use (a sun one per cascade, a point light six).
        let settings = frame.shadows.clamped();
        let shadows = shadow_views(frame, &settings);
        let layers = shadows.iter().map(|v| v.faces.len()).sum::<usize>().clamp(1, MAX_SHADOW_LAYERS);
        if self.shadow_size != settings.resolution || self.shadow_layers.len() != layers {
            let (group, views) = shadow_map(device, &self.shadow_layout, &self.shadow_sampler, settings.resolution, layers as u32);
            self.shadow_group = group;
            self.shadow_layers = views;
            self.shadow_size = settings.resolution;
        }
        let globals = globals_for(frame, &shadows, settings.resolution);
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        for material in frame.items.iter().filter_map(|i| i.material.as_deref()) {
            self.prepare(device, material);
        }
        self.settle_checks();
        self.upload_assets(device, queue, frame);

        // Opaque items grouped by material, mesh and images, then transparent items back to front.
        let mut opaque: Vec<&DrawItem> = frame.items.iter().filter(|i| !i.is_transparent()).collect();
        opaque.sort_by_key(|i| (material_key(i), mesh_key(i), i.part, texture_key(i)));
        let mut transparent: Vec<&DrawItem> = frame.items.iter().filter(|i| i.is_transparent()).collect();
        let eye = frame.camera.position;
        transparent.sort_by(|a, b| distance(b.model, eye).total_cmp(&distance(a.model, eye)));
        let ordered: Vec<&DrawItem> = opaque.iter().chain(transparent.iter()).copied().collect();
        let skins = self.write_joints(device, queue, &ordered);
        let raw: Vec<InstanceRaw> = ordered
            .iter()
            .zip(&skins)
            .map(|(i, skin)| InstanceRaw::new(i, frame.selected == Some(i.object) || frame.highlighted.contains(&i.object), *skin))
            .collect();
        if raw.len() > self.instance_capacity {
            self.instance_capacity = raw.len().next_power_of_two();
            self.instances = instance_buffer(device, self.instance_capacity);
        }
        if !raw.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&raw));
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("bayleaf frame") });
        let timer = self.timer.get_or_insert_with(|| crate::timing::GpuTimer::new(device, queue));
        if let Some(timer) = timer {
            timer.begin(device, &mut encoder);
        }
        let mut drawn = (0u32, 0u64);
        let faces: Vec<Mat4> = shadows.iter().flat_map(|v| v.faces.iter().copied()).collect();
        for (layer, matrix) in faces.iter().enumerate() {
            queue.write_buffer(&self.shadow_faces, layer as u64 * FACE_STRIDE, bytemuck::bytes_of(&matrix.to_cols_array_2d()));
        }
        // The opaque shapes, as seen from each light that casts shadows, into its layers.
        {
            for face in 0..faces.len() {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bayleaf shadow pass"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &self.shadow_layers[face],
                        depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.shadow_pipeline);
                pass.set_bind_group(0, &self.globals_group, &[]);
                pass.set_bind_group(1, &self.shadow_face_group, &[(face as u64 * FACE_STRIDE) as u32]);
                pass.set_vertex_buffer(1, self.instances.slice(..));
                let mut start = 0usize;
                while start < opaque.len() {
                    let first = opaque[start];
                    let run = opaque[start..].iter().take_while(|i| (mesh_key(i), i.part) == (mesh_key(first), first.part)).count();
                    if let Some(mesh) = self.gpu_mesh(first) {
                        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                        pass.draw_indexed(indices(first, mesh), 0, start as u32..(start + run) as u32);
                    }
                    start += run;
                }
            }
        }
        {
            let c = if frame.background { frame.clear_color } else { bayleaf_core::math::Vec4::ZERO };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bayleaf scene pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.msaa_view.as_ref().unwrap_or(&target.hdr_view),
                    depth_slice: None,
                    resolve_target: target.msaa_view.as_ref().map(|_| &target.hdr_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: f64::from(c.x), g: f64::from(c.y), b: f64::from(c.z), a: f64::from(c.w) }),
                        // The multisampled colour is resolved into the scene colour and then not needed;
                        // without multisampling the pass draws into the scene colour itself, which stays.
                        store: if target.msaa_view.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    // Kept for post-processing (depth fog, ambient occlusion).
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.globals_group, &[]);
            pass.set_bind_group(1, &self.shadow_group, &[]);
            pass.set_vertex_buffer(1, self.instances.slice(..));

            // Every pipeline takes the image group; start with white images.
            if let Some(group) = self.texture_groups.get(&([0; 4], 0)) {
                pass.set_bind_group(2, group, &[]);
            }
            if frame.sky.is_some() && frame.background {
                pass.set_pipeline(&self.sky);
                pass.draw(0..3, 0..1);
            }
            // Runs of opaque items that share a material, a mesh and images are drawn as one instanced call.
            let mut start = 0usize;
            while start < opaque.len() {
                let first = opaque[start];
                let key = (material_key(first), mesh_key(first), texture_key(first), first.part);
                let run = opaque[start..].iter().take_while(|i| (material_key(i), mesh_key(i), texture_key(i), i.part) == key).count();
                if let (Some(pipeline), Some(mesh), Some(group)) = (self.pipeline(key.0, false), self.gpu_mesh(first), self.texture_groups.get(&key.2)) {
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(2, group, &[]);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(indices(first, mesh), 0, start as u32..(start + run) as u32);
                    drawn.0 += 1;
                    drawn.1 += u64::from(indices(first, mesh).len() as u32 / 3) * run as u64;
                }
                start += run;
            }

            // Outlines of the opaque shapes whose material has one: their back faces, grown.
            let mut start = 0usize;
            while start < opaque.len() {
                let first = opaque[start];
                let key = (material_key(first), mesh_key(first), texture_key(first), first.part);
                let run = opaque[start..].iter().take_while(|i| (material_key(i), mesh_key(i), texture_key(i), i.part) == key).count();
                if let (Some(pipeline), Some(mesh), Some(group)) = (self.outlines.get(&key.0), self.gpu_mesh(first), self.texture_groups.get(&key.2)) {
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(2, group, &[]);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(indices(first, mesh), 0, start as u32..(start + run) as u32);
                    drawn.0 += 1;
                    drawn.1 += u64::from(indices(first, mesh).len() as u32 / 3) * run as u64;
                }
                start += run;
            }

            if frame.show_grid {
                pass.set_pipeline(&self.grid);
                pass.draw(0..6, 0..1);
            }

            for (offset, item) in transparent.iter().enumerate() {
                let pipeline = if item.additive {
                    self.additive.get(&material_key(item)).or_else(|| self.additive.get(&0))
                } else {
                    self.pipeline(material_key(item), true)
                };
                if let (Some(pipeline), Some(mesh), Some(group)) = (pipeline, self.gpu_mesh(item), self.texture_groups.get(&texture_key(item))) {
                    let index = (opaque.len() + offset) as u32;
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(2, group, &[]);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(indices(item, mesh), 0, index..index + 1);
                    drawn.0 += 1;
                    drawn.1 += u64::from(indices(item, mesh).len() as u32 / 3);
                }
            }
        }
        let selected: Vec<(u32, &DrawItem)> = ordered
            .iter()
            .enumerate()
            .filter(|(_, i)| frame.selected == Some(i.object) || frame.highlighted.contains(&i.object))
            .map(|(index, i)| (index as u32, *i))
            .collect();
        self.drawn = drawn;
        // The displayed picture, from the scene's colour.
        self.post.run(device, queue, &mut encoder, target, &self.globals_group, &frame.post);
        if !selected.is_empty() {
            self.outline(device, &mut encoder, target, &selected);
        }
        let timer = self.timer.as_mut().and_then(Option::as_mut);
        if let Some(timer) = timer {
            timer.end(&mut encoder);
        }
        queue.submit([encoder.finish()]);
        if let Some(timer) = self.timer.as_mut().and_then(Option::as_mut) {
            timer.after_submit();
        }
    }

    /// Outlines the selected shapes: draws them into the target's mask, then paints a band around
    /// the mask over the finished picture. `selected` holds each item's instance index.
    fn outline(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, target: &RenderTarget, selected: &[(u32, &DrawItem)]) {
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bayleaf selection mask pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.mask_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.mask_pipeline);
            pass.set_bind_group(0, &self.globals_group, &[]);
            pass.set_vertex_buffer(1, self.instances.slice(..));
            for (index, item) in selected {
                if let Some(mesh) = self.gpu_mesh(item) {
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(indices(item, mesh), 0, *index..index + 1);
                }
            }
        }
        let mask = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bayleaf selection mask group"),
            layout: &self.outline_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&target.mask_view) }],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("bayleaf outline pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.outline_pipeline);
        pass.set_bind_group(0, &self.globals_group, &[]);
        pass.set_bind_group(1, &mask, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The selection outline's two pipelines (the selected shapes into the mask; the band around it)
/// and the layout of the mask's bind group.
fn outline_pipelines(device: &wgpu::Device, globals_layout: &wgpu::BindGroupLayout) -> (wgpu::RenderPipeline, wgpu::RenderPipeline, wgpu::BindGroupLayout) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("bayleaf outline shader"),
        source: wgpu::ShaderSource::Wgsl(with_common(include_str!("shaders/outline.wgsl")).into()),
    });
    let mask_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("bayleaf selection mask layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        }],
    });
    let layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: groups, immediate_size: 0 })
    };
    let mask_pass_layout = layout("bayleaf selection mask pass layout", &[Some(globals_layout)]);
    let outline_pass_layout = layout("bayleaf outline pass layout", &[Some(globals_layout), Some(&mask_layout)]);
    let mask = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("bayleaf selection mask"),
        layout: Some(&mask_pass_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_mask"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &VERTEX_ATTRIBUTES,
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<InstanceRaw>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &INSTANCE_ATTRIBUTES,
                }),
            ],
        },
        primitive: wgpu::PrimitiveState { cull_mode: None, ..wgpu::PrimitiveState::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_mask"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState { format: MASK_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let outline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("bayleaf outline"),
        layout: Some(&outline_pass_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_screen"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_outline"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    (mask, outline, mask_layout)
}

/// The indices an item draws: its part of the mesh, or all of it.
fn indices(item: &DrawItem, mesh: &GpuMesh) -> std::ops::Range<u32> {
    match item.part {
        Some((first, count)) => first.min(mesh.index_count)..first.saturating_add(count).min(mesh.index_count),
        None => 0..mesh.index_count,
    }
}

/// A shadow map of `layers` layers, `size` texels a side, as the shapes read it (with the comparison
/// sampler), and a view of each layer to draw into.
fn shadow_map(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    size: u32,
    layers: u32,
) -> (wgpu::BindGroup, Vec<wgpu::TextureView>) {
    let layers = layers.max(1);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bayleaf shadow map"),
        size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: layers },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SHADOW_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let whole = texture.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
    let views = (0..layers)
        .map(|layer| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bayleaf shadow group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&whole) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    });
    (group, views)
}

fn globals_for(frame: &Frame, shadows: &[ShadowView], shadow_size: u32) -> Globals {
    let mut lights = [LightRaw { direction: [0.0; 4], position: [0.0; 4], radiance: [0.0; 4], extra: [0.0; 4] }; MAX_LIGHTS];
    for (slot, light) in lights.iter_mut().zip(&frame.lights) {
        let kind = match light.kind {
            LightKind::Directional => 0.0,
            LightKind::Point => 1.0,
            LightKind::Spot => 2.0,
        };
        *slot = LightRaw {
            direction: light.direction.extend(kind).to_array(),
            position: light.position.extend(light.range).to_array(),
            radiance: light.radiance.extend(light.cos_outer).to_array(),
            extra: [light.cos_inner, 0.0, 0.0, 0.0],
        };
    }
    let count = frame.lights.len().min(MAX_LIGHTS) as f32;
    let sky = frame.sky.unwrap_or(crate::extract::Sky { zenith: Vec3::ZERO, horizon: Vec3::ZERO, ground: Vec3::ZERO });
    let light_view_proj = shadows.first().map_or(Mat4::IDENTITY, |v| v.faces[0]);
    let mut shadow_lights = [[-1.0, 0.0, 0.0, 0.0]; MAX_SHADOW_LIGHTS];
    let mut shadow_modes = [[0.0; 4]; MAX_SHADOW_LIGHTS];
    let mut shadow_splits = [[0.0; 4]; MAX_SHADOW_LIGHTS];
    let mut shadow_faces = [Mat4::IDENTITY.to_cols_array_2d(); MAX_SHADOW_LAYERS];
    let mut first = 0usize;
    for (slot, view) in shadows.iter().take(MAX_SHADOW_LIGHTS).enumerate() {
        shadow_lights[slot] = [view.index as f32, view.bias, view.normal_offset, first as f32];
        shadow_modes[slot] = [view.faces.len() as f32, view.mode, 0.0, 0.0];
        shadow_splits[slot] = view.splits;
        for (layer, face) in shadow_faces.iter_mut().skip(first).zip(&view.faces) {
            *layer = face.to_cols_array_2d();
        }
        first += view.faces.len();
    }
    Globals {
        view_proj: frame.camera.view_proj().to_cols_array_2d(),
        camera_pos: frame.camera.position.extend(frame.time).to_array(),
        ambient: frame.ambient.extend(count).to_array(),
        light_view_proj: light_view_proj.to_cols_array_2d(),
        shadow_count: [shadows.len().min(MAX_SHADOW_LIGHTS) as f32, 1.0 / shadow_size.max(1) as f32, 0.0, 0.0],
        ambient_ground: frame.ambient_ground.extend(0.0).to_array(),
        inv_view_proj: frame.camera.view_proj().inverse().to_cols_array_2d(),
        sky_zenith: sky.zenith.extend(0.0).to_array(),
        // w: 1 when there is a sky to reflect.
        sky_horizon: sky.horizon.extend(if frame.sky.is_some() { 1.0 } else { 0.0 }).to_array(),
        sky_ground: sky.ground.extend(0.0).to_array(),
        fog: frame.fog.0.extend(frame.fog.1).to_array(),
        shadow_lights,
        shadow_modes,
        shadow_splits,
        shadow_faces,
        grid: {
            let origin = frame.camera.origin;
            [origin.x.rem_euclid(10.0) as f32, (-origin.y) as f32, origin.z.rem_euclid(10.0) as f32, 0.0]
        },
        grid_axes: [(-frame.camera.origin.z) as f32, (-frame.camera.origin.x) as f32, 0.0, 0.0],
        origin: frame.camera.origin.as_vec3().extend(0.0).to_array(),
        lights,
    }
}

/// The light that casts shadows, the matrices from the world to its shadow map's faces (one, or six
/// for a point light), and how far to push tested points away from their surface so a lit surface
/// does not shadow itself.
#[derive(Clone, Debug)]
struct ShadowView {
    index: usize,
    faces: Vec<Mat4>,
    /// 0: one face; 1: six faces around a point light; 2: sun cascades, picked by view depth.
    mode: f32,
    /// View depths where the second, third and fourth cascades start (beyond the last cascade's
    /// end nothing is shadowed).
    splits: [f32; 4],
    /// In shadow-map depth units.
    bias: f32,
    /// Along the normal, in world units.
    normal_offset: f32,
}

/// The lights that cast shadows, as many as `settings` allow (sun-like lights come first, then the
/// nearest), with their faces: a directional light is seen orthographically so the map covers every
/// drawn shape (or in cascades by view depth), a spot light through its cone, a point light through
/// six faces around it.
fn shadow_views(frame: &Frame, settings: &crate::extract::ShadowSettings) -> Vec<ShadowView> {
    let size = settings.resolution.max(1) as f32;
    let mut views = Vec::new();
    for (index, light) in frame.lights.iter().enumerate().filter(|(_, l)| l.shadows).take(settings.lights as usize) {
        let view = match light.kind {
            LightKind::Directional => {
                let Some((center, radius)) = crate::extract::item_bounds(frame.items.iter().filter(|i| !i.unlit)) else { continue };
                let radius = radius.max(0.5);
                let dir = light.direction.normalize_or(Vec3::NEG_Y);
                // Cascades follow the camera in a large scene, or when the shadow reaches less far than the scene.
                let large = settings.cascades > 1 && radius > CASCADE_MIN_RADIUS;
                let limited = settings.reach > 0.0 && settings.reach < frame.camera.position.distance(center) + radius;
                if (large || limited) && frame.camera.proj.w_axis.w == 0.0 {
                    cascades(frame, index, dir, center, radius, settings)
                } else {
                    let eye = center - dir * radius * 2.0;
                    let view = Mat4::look_at_rh(eye, center, up_for(dir));
                    let matrix = Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.01, radius * 4.0) * view;
                    // One texel of world size: the box width 2r maps to 2 in x.
                    let texel = 2.0 / matrix.x_axis.truncate().length().max(1e-6) / size;
                    // Depth per world unit along the light: the length of the matrix's depth row.
                    let bias = texel * Vec3::new(matrix.x_axis.z, matrix.y_axis.z, matrix.z_axis.z).length() * 1.5;
                    ShadowView { index, faces: vec![matrix], mode: 0.0, splits: [0.0; 4], bias, normal_offset: texel * 1.5 }
                }
            }
            LightKind::Spot => {
                let half = light.cos_outer.clamp(-1.0, 1.0).acos().clamp(0.01, 1.55);
                let dir = light.direction.normalize_or(Vec3::NEG_Y);
                let (near, far) = (SPOT_SHADOW_NEAR, light.range.max(SPOT_SHADOW_NEAR * 2.0));
                let matrix = Mat4::perspective_rh(half * 2.0, 1.0, near, far) * Mat4::look_at_rh(light.position, light.position + dir, up_for(dir));
                // Texel size a third of the way out, where shadows usually fall.
                let texel = 2.0 * half.tan() * far / 3.0 / size;
                ShadowView { index, faces: vec![matrix], mode: 0.0, splits: [0.0; 4], bias: SPOT_SHADOW_BIAS, normal_offset: texel * 1.5 }
            }
            LightKind::Point => {
                let (near, far) = (SPOT_SHADOW_NEAR, light.range.max(SPOT_SHADOW_NEAR * 2.0));
                // A little wider than a quarter turn, so the filter's taps near a face's edge stay on it.
                let fov = 2.0 * POINT_FACE_SPREAD.atan();
                let proj = Mat4::perspective_rh(fov, 1.0, near, far);
                let faces = POINT_FACES.iter().map(|(dir, up)| proj * Mat4::look_at_rh(light.position, light.position + *dir, *up)).collect();
                let texel = 2.0 * POINT_FACE_SPREAD * far / 3.0 / size;
                ShadowView { index, faces, mode: 1.0, splits: [0.0; 4], bias: SPOT_SHADOW_BIAS, normal_offset: texel * 1.5 }
            }
        };
        views.push(view);
    }
    views
}

/// Scenes with a bounding radius above this get sun shadows in cascades.
const CASCADE_MIN_RADIUS: f32 = 40.0;
/// Where 1, 2, 3 or 4 cascades split the view depth they cover, as fractions of it: nearer slices
/// smaller.
fn cascade_splits(count: u32) -> &'static [f32] {
    match count {
        0..=1 => &[0.0, 1.0],
        2 => &[0.0, 0.15, 1.0],
        3 => &[0.0, 0.06, 0.25, 1.0],
        _ => &[0.0, 0.04, 0.12, 0.35, 1.0],
    }
}

/// Shadow maps for a sun, one per cascade: each covers a slice of what the camera sees, nearer
/// slices smaller, so shadows close by stay sharp in a large world. They reach to the far side of
/// the scene, or as far as the settings say.
fn cascades(frame: &Frame, index: usize, dir: Vec3, scene_center: Vec3, scene_radius: f32, settings: &crate::extract::ShadowSettings) -> ShadowView {
    let camera = &frame.camera;
    let forward = camera.view.inverse().transform_vector3(Vec3::NEG_Z).normalize_or(Vec3::NEG_Z);
    let inverse = camera.view_proj().inverse();
    let rays: Vec<Vec3> =
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().map(|(x, y)| inverse.project_point3(Vec3::new(*x, *y, 1.0)) - camera.position).collect();
    let mut far = (camera.position.distance(scene_center) + scene_radius).max(1.0);
    if settings.reach > 0.0 {
        far = far.min(settings.reach);
    }
    let depths: Vec<f32> = cascade_splits(settings.cascades).iter().map(|s| (s * far).max(0.05)).collect();
    let size = settings.resolution.max(1) as f32;
    let mut faces = Vec::new();
    for slice in depths.windows(2) {
        let corners: Vec<Vec3> = slice.iter().flat_map(|d| rays.iter().map(move |ray| camera.position + *ray * (*d / ray.dot(forward).max(1e-4)))).collect();
        let center = corners.iter().copied().sum::<Vec3>() / corners.len() as f32;
        let radius = corners.iter().map(|c| c.distance(center)).fold(0.0f32, f32::max).max(0.5);
        // Far enough back along the light to catch every shape that can shadow the slice.
        let back = center.distance(scene_center) + scene_radius + radius;
        let up = up_for(dir);
        let mut eye = center - dir * back;
        // Moving in whole texels keeps shadow edges from crawling as the camera moves.
        let view = Mat4::look_at_rh(eye, center, up);
        let texel = 2.0 * radius / size;
        let local = view.transform_point3(center);
        let snapped = Vec3::new((local.x / texel).round() * texel, (local.y / texel).round() * texel, local.z);
        eye += view.inverse().transform_vector3(snapped - local);
        let view = Mat4::look_at_rh(eye, eye + dir, up);
        faces.push(Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.01, back * 2.0 + radius) * view);
    }
    // Where each later cascade starts, then where the last one ends.
    let mut splits = [f32::MAX; 4];
    for (slot, depth) in splits.iter_mut().zip(depths.iter().skip(1)) {
        *slot = *depth;
    }
    ShadowView { index, faces, mode: 2.0, splits, bias: 0.0, normal_offset: 0.0 }
}

/// The directions and up vectors of a point light's faces, in the order the shader picks them:
/// +X, -X, +Y, -Y, +Z, -Z.
const POINT_FACES: [(Vec3, Vec3); 6] =
    [(Vec3::X, Vec3::NEG_Y), (Vec3::NEG_X, Vec3::NEG_Y), (Vec3::Y, Vec3::Z), (Vec3::NEG_Y, Vec3::NEG_Z), (Vec3::Z, Vec3::NEG_Y), (Vec3::NEG_Z, Vec3::NEG_Y)];
/// Tangent of half a point-light face's view angle.
const POINT_FACE_SPREAD: f32 = 1.1;
/// Closest a spot light's shadow map sees, in world units.
const SPOT_SHADOW_NEAR: f32 = 0.1;
/// Depth bias of spot light shadows, in perspective depth units.
const SPOT_SHADOW_BIAS: f32 = 0.0005;

/// An up vector for looking along `dir`.
fn up_for(dir: Vec3) -> Vec3 {
    if dir.cross(Vec3::Y).length_squared() < 1e-6 { Vec3::Z } else { Vec3::Y }
}
fn shape_index(shape: Shape) -> usize {
    Shape::ALL.iter().position(|s| *s == shape).unwrap_or(0)
}

fn distance(model: Mat4, eye: Vec3) -> f32 {
    model.w_axis.truncate().distance_squared(eye)
}

fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bayleaf instances"),
        size: (capacity * std::mem::size_of::<InstanceRaw>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 5] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 11 => Float32x2, 13 => Uint16x4, 14 => Float32x4];
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 11] = wgpu::vertex_attr_array![
    2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4,
    6 => Float32x4, 7 => Float32x4, 8 => Float32x4,
    9 => Float32x4, 10 => Float32x4, 12 => Float32x4, 15 => Float32x4,
];

/// Matrices in each row of the joint texture (four texels each).
const JOINTS_PER_ROW: u32 = 256;
/// Most rows the joint texture grows to: 2048, the smallest texture side every device takes.
const MAX_JOINT_ROWS: u32 = 2048;

/// A joint texture of `rows` rows.
fn joint_texture(device: &wgpu::Device, rows: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("bayleaf joints"),
        size: wgpu::Extent3d { width: JOINTS_PER_ROW * 4, height: rows.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// The per-frame group: the globals and the joint texture.
fn globals_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, globals: &wgpu::Buffer, joints: &wgpu::Texture) -> wgpu::BindGroup {
    let view = joints.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bayleaf globals group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
        ],
    })
}

fn mesh_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, transparent: bool, samples: u32) -> wgpu::RenderPipeline {
    mesh_pipeline_build(device, layout, shader, Kind { transparent, outline: false, additive: false }, samples)
}

/// Items that add their light (particles of fire and sparks): no depth written, colour added.
fn additive_mesh_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, samples: u32) -> wgpu::RenderPipeline {
    mesh_pipeline_build(device, layout, shader, Kind { transparent: true, outline: false, additive: true }, samples)
}

/// A material's outline: its back faces only (front faces culled), grown by `vs_outline`.
fn outline_mesh_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, samples: u32) -> wgpu::RenderPipeline {
    mesh_pipeline_build(device, layout, shader, Kind { transparent: false, outline: true, additive: false }, samples)
}

/// Which mesh pipeline: blended over what is behind, a material's outline, or added light.
#[derive(Clone, Copy)]
struct Kind {
    transparent: bool,
    outline: bool,
    additive: bool,
}

/// Light added to what is behind, weighted by the item's opacity.
const ADDITIVE: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
    alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
};

fn mesh_pipeline_build(device: &wgpu::Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, kind: Kind, samples: u32) -> wgpu::RenderPipeline {
    let Kind { transparent, outline, additive } = kind;
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(if outline {
            "bayleaf material outline"
        } else if transparent {
            "bayleaf transparent"
        } else {
            "bayleaf opaque"
        }),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(if outline { "vs_outline" } else { "vs_main" }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &VERTEX_ATTRIBUTES,
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<InstanceRaw>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &INSTANCE_ATTRIBUTES,
                }),
            ],
        },
        // No culling: quads and planes are visible from both sides; the shader flips back-face
        // normals. An outline draws only the back faces.
        primitive: wgpu::PrimitiveState { cull_mode: outline.then_some(wgpu::Face::Front), ..wgpu::PrimitiveState::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(!transparent),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState { count: samples, ..wgpu::MultisampleState::default() },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(if outline { "fs_outline" } else { "fs_main" }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: HDR_FORMAT,
                blend: if additive { Some(ADDITIVE) } else { transparent.then_some(wgpu::BlendState::ALPHA_BLENDING) },
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn grid_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, samples: u32) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("bayleaf grid"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState { cull_mode: None, ..wgpu::PrimitiveState::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            // Pushed back a little, so a surface lying on the ground plane always covers the grid
            // instead of flickering with it.
            bias: wgpu::DepthBiasState { constant: 64, slope_scale: 2.0, clamp: 0.0 },
        }),
        multisample: wgpu::MultisampleState { count: samples, ..wgpu::MultisampleState::default() },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// The built-in surface: the mesh component's colour, metallic and roughness.
pub(crate) const DEFAULT_SURFACE: &str = "fn surface(s: SurfaceIn) -> Surface {
    let image = textureSample(t0, material_sampler, s.uv);
    // glTF images: roughness in green and metallic in blue (t1), a tangent-space normal map (t2).
    // Both hold plain numbers, not colours, and are bound to be read as stored.
    let metal_rough = textureSample(t1, material_sampler, s.uv).rgb;
    let mapped = textureSample(t2, material_sampler, s.uv).rgb * 2.0 - vec3<f32>(1.0);
    let normal = bl_normal_map(s.normal, s.position, s.uv, mapped);
    return Surface(s.color.rgb * image.rgb, s.params.x * metal_rough.b, s.params.y * metal_rough.g, vec3<f32>(0.0), s.color.a * image.a, normal);
}
const USES_LIGHTS: bool = false;
const CUSTOM_LIGHTING: bool = false;
const OUTLINE: vec4<f32> = vec4<f32>(0.0);";

/// `shader` with the shared per-frame values in front.
fn with_common(shader: &str) -> String {
    format!(
        "{}
{shader}",
        include_str!("shaders/common.wgsl")
    )
}

/// The mesh shader with `surface` filled in.
pub(crate) fn mesh_source(surface: &str) -> String {
    with_common(&include_str!("shaders/mesh.wgsl").replace("// @surface", surface))
}

fn material_key(item: &DrawItem) -> u64 {
    item.material.as_ref().map_or(0, |m| m.key)
}

/// An error and its causes, for messages.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// Depth-only pipeline for the shadow map. Back faces are kept: a closed shape's far side is
/// what keeps light from leaking under it.
fn shadow_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("bayleaf shadow"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &VERTEX_ATTRIBUTES,
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<InstanceRaw>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &INSTANCE_ATTRIBUTES,
                }),
            ],
        },
        primitive: wgpu::PrimitiveState { cull_mode: None, ..wgpu::PrimitiveState::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: SHADOW_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
        }),
        multisample: wgpu::MultisampleState::default(),
        fragment: None,
        multiview_mask: None,
        cache: None,
    })
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// An image on the GPU, with its smaller copies so it does not shimmer far away (unless its import
/// settings turn them off): its 4 x 4 blocks when it has them and the device reads them, else its
/// pixels. Its colour view reads the stored numbers as sRGB colour, its raw view as they are.
fn upload_image(device: &wgpu::Device, queue: &wgpu::Queue, image: &bayleaf_assets::Image) -> GpuImage {
    use bayleaf_assets::import::Compression;
    let blocks = image.blocks.as_deref().filter(|_| device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC));
    let (formats, levels, data) = match blocks {
        Some(blocks) => {
            let formats = match blocks.compression {
                Compression::Bc1 | Compression::None => (wgpu::TextureFormat::Bc1RgbaUnorm, wgpu::TextureFormat::Bc1RgbaUnormSrgb),
                Compression::Bc3 => (wgpu::TextureFormat::Bc3RgbaUnorm, wgpu::TextureFormat::Bc3RgbaUnormSrgb),
            };
            (formats, blocks.levels.len() as u32, blocks.levels.concat())
        }
        None => {
            let levels = if image.mipmaps { bayleaf_assets::mip_levels(image) } else { Vec::new() };
            let mut data = image.rgba.clone();
            for (_, _, pixels) in &levels {
                data.extend_from_slice(pixels);
            }
            ((wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Rgba8UnormSrgb), levels.len() as u32 + 1, data)
        }
    };
    let (raw_format, color_format) = formats;
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("bayleaf image"),
            size: wgpu::Extent3d { width: image.width.max(1), height: image.height.max(1), depth_or_array_layers: 1 },
            mip_level_count: levels.max(1),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: raw_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[color_format],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &data,
    );
    let view = |format| texture.create_view(&wgpu::TextureViewDescriptor { format: Some(format), ..Default::default() });
    let (color, raw) = (view(color_format), view(raw_format));
    GpuImage { texture, color, raw }
}

/// A model on the GPU.
fn upload_model(device: &wgpu::Device, mesh: &bayleaf_assets::MeshData) -> GpuMesh {
    let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("bayleaf model vertices"),
        contents: bytemuck::cast_slice(&model_vertices(mesh)),
        // Written again when an animation poses the model.
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    });
    let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("bayleaf model indices"),
        contents: bytemuck::cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    GpuMesh { vertices, indices, index_count: mesh.indices.len() as u32, version: mesh.version }
}

/// A model's vertices: for one skinned on the GPU, in the bound pose with their joints and weights;
/// else as the model holds them.
fn model_vertices(mesh: &bayleaf_assets::MeshData) -> Vec<Vertex> {
    let uv = |i: usize| mesh.uvs.get(i).copied().unwrap_or([0.0; 2]);
    if let Some(skeleton) = gpu_skeleton(mesh) {
        return (0..mesh.positions.len())
            .map(|i| Vertex {
                position: skeleton.positions[i],
                normal: skeleton.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]),
                uv: uv(i),
                joints: skeleton.vertex_bindings.get(i).copied().unwrap_or_default(),
                weights: skeleton.vertex_weights.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0]),
            })
            .collect();
    }
    (0..mesh.positions.len()).map(|i| Vertex::new(mesh.positions[i], mesh.normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]), uv(i))).collect()
}

/// The skeleton a model is skinned by on the GPU: one whose bound-pose vertices match the model's.
fn gpu_skeleton(mesh: &bayleaf_assets::MeshData) -> Option<&bayleaf_assets::skeleton::Skeleton> {
    mesh.skeleton.as_deref().filter(|s| s.positions.len() == mesh.positions.len() && !s.bindings.is_empty())
}

/// Which mesh an item draws: a built-in shape (its index) or a model (its id, with the top bit set).
fn mesh_key(item: &DrawItem) -> u64 {
    match &item.mesh {
        Some(mesh) => mesh.id | (1 << 63),
        None => shape_index(item.shape) as u64,
    }
}

/// The images in an item's four texture slots; 0 for an empty or missing slot.
fn texture_key(item: &DrawItem) -> TextureKey {
    // The built-in surface reads a normal map in slot 2, so an empty one there is flat, not white;
    // its slots 1 and 2 (metallic and roughness, normals) hold plain numbers whatever the image says.
    let built_in = item.material.is_none();
    let (mut ids, mut raw) = if built_in { ([0, 0, FLAT_NORMAL, 0], 0b0110) } else { ([0; 4], 0) };
    for (slot, (id, image)) in ids.iter_mut().zip(&item.textures).enumerate() {
        if let Some(image) = image {
            *id = image.id;
            if image.data {
                raw |= 1 << slot;
            }
        }
    }
    (ids, raw)
}

/// The images in a bind group's four slots, and a bit for each slot read as plain numbers.
type TextureKey = ([u64; 4], u8);

/// Texture id of the one-pixel flat normal map (image ids from files are odd, 0 is white).
const FLAT_NORMAL: u64 = 2;

/// An image on the GPU: one texture, read through two views, as colour and as plain numbers.
struct GpuImage {
    texture: wgpu::Texture,
    color: wgpu::TextureView,
    raw: wgpu::TextureView,
}

/// The sky: drawn first, behind everything, without touching the depth buffer.
fn sky_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, samples: u32) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("bayleaf sky"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState { cull_mode: None, ..wgpu::PrimitiveState::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState { count: samples, ..wgpu::MultisampleState::default() },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState { format: HDR_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
