//! GPU side: pipelines, buffers and an offscreen render target.

use std::collections::HashMap;

use bayleaf_core::math::{Mat4, Vec3};
use bayleaf_scene::components::Shape;
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::extract::{DrawItem, Frame, MAX_LIGHTS, normal_matrix};
use crate::material::CompiledMaterial;
use crate::mesh::{self, Vertex};

/// Colour format of [`RenderTarget`]. sRGB, so shaders write linear colour.
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightRaw {
    direction: [f32; 4],
    radiance: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    camera_pos: [f32; 4],
    ambient: [f32; 4],
    lights: [LightRaw; MAX_LIGHTS],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct InstanceRaw {
    model: [[f32; 4]; 4],
    normal: [[f32; 4]; 3],
    color: [f32; 4],
    params: [f32; 4],
}

impl InstanceRaw {
    fn new(item: &DrawItem, selected: bool) -> Self {
        let n = normal_matrix(item.model);
        Self {
            model: item.model.to_cols_array_2d(),
            normal: [n.x_axis.extend(0.0).to_array(), n.y_axis.extend(0.0).to_array(), n.z_axis.extend(0.0).to_array()],
            color: item.color.to_array(),
            params: [item.metallic, item.roughness, f32::from(u8::from(selected)), f32::from(u8::from(item.unlit))],
        }
    }
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

/// Offscreen colour, multisample and depth textures of one size.
pub struct RenderTarget {
    width: u32,
    height: u32,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    msaa_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
}

impl RenderTarget {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
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
        let msaa = texture("bayleaf msaa", COLOR_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let depth = texture("bayleaf depth", DEPTH_FORMAT, SAMPLES, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let msaa_view = msaa.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        Self { width, height, color, color_view, msaa_view, depth_view }
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The resolved colour texture, for display or copying.
    pub fn texture(&self) -> &wgpu::Texture {
        &self.color
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.color_view
    }
}

/// Draws [`Frame`]s into [`RenderTarget`]s.
pub struct Renderer {
    globals: wgpu::Buffer,
    globals_group: wgpu::BindGroup,
    layout: wgpu::PipelineLayout,
    /// Opaque and transparent pipelines by material key; 0 is the built-in surface, `None` a
    /// material whose shader the GPU refused (drawn with the built-in surface instead).
    pipelines: HashMap<u64, Option<(wgpu::RenderPipeline, wgpu::RenderPipeline)>>,
    /// Why the last refused material shader was refused.
    material_error: Option<String>,
    grid: wgpu::RenderPipeline,
    meshes: Vec<(Shape, GpuMesh)>,
    instances: wgpu::Buffer,
    instance_capacity: usize,
}

impl Renderer {
    pub fn new(device: &wgpu::Device) -> Self {
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bayleaf globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bayleaf globals layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let globals_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bayleaf globals group"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bayleaf pipeline layout"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });
        let grid_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bayleaf grid shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/grid.wgsl").into()),
        });

        let mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bayleaf mesh shader"),
            source: wgpu::ShaderSource::Wgsl(mesh_source(DEFAULT_SURFACE).into()),
        });
        let mut pipelines = HashMap::new();
        pipelines.insert(0, Some((mesh_pipeline(device, &layout, &mesh_shader, false), mesh_pipeline(device, &layout, &mesh_shader, true))));
        let grid = grid_pipeline(device, &layout, &grid_shader);

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
                (shape, GpuMesh { vertices, indices, index_count: data.indices.len() as u32 })
            })
            .collect();

        let instance_capacity = 256;
        let instances = instance_buffer(device, instance_capacity);
        Self { globals, globals_group, layout, pipelines, material_error: None, grid, meshes, instances, instance_capacity }
    }

    /// Why the GPU refused the last material shader it was given, if it refused one.
    pub fn material_error(&self) -> Option<&str> {
        self.material_error.as_deref()
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
        let pipelines = (mesh_pipeline(device, &self.layout, &shader, false), mesh_pipeline(device, &self.layout, &shader, true));
        let entry = match pollster::block_on(scope.pop()) {
            None => Some(pipelines),
            Some(error) => {
                self.material_error = Some(error_chain(&error));
                None
            }
        };
        self.pipelines.insert(material.key, entry);
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
        let globals = globals_for(frame);
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));
        for material in frame.items.iter().filter_map(|i| i.material.as_deref()) {
            self.prepare(device, material);
        }

        // Opaque items grouped by material and shape, then transparent items back to front.
        let mut opaque: Vec<&DrawItem> = frame.items.iter().filter(|i| !i.is_transparent()).collect();
        opaque.sort_by_key(|i| (material_key(i), shape_index(i.shape)));
        let mut transparent: Vec<&DrawItem> = frame.items.iter().filter(|i| i.is_transparent()).collect();
        let eye = frame.camera.position;
        transparent.sort_by(|a, b| distance(b.model, eye).total_cmp(&distance(a.model, eye)));
        let ordered: Vec<&DrawItem> = opaque.iter().chain(transparent.iter()).copied().collect();
        let raw: Vec<InstanceRaw> =
            ordered.iter().map(|i| InstanceRaw::new(i, frame.selected == Some(i.object) || frame.highlighted.contains(&i.object))).collect();
        if raw.len() > self.instance_capacity {
            self.instance_capacity = raw.len().next_power_of_two();
            self.instances = instance_buffer(device, self.instance_capacity);
        }
        if !raw.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&raw));
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("bayleaf frame") });
        {
            let c = frame.clear_color;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("bayleaf scene pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.msaa_view,
                    depth_slice: None,
                    resolve_target: Some(&target.color_view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: f64::from(c.x), g: f64::from(c.y), b: f64::from(c.z), a: f64::from(c.w) }),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.globals_group, &[]);
            pass.set_vertex_buffer(1, self.instances.slice(..));

            // Runs of opaque items that share a material and a shape are drawn as one instanced call.
            let mut start = 0usize;
            while start < opaque.len() {
                let first = opaque[start];
                let run = opaque[start..].iter().take_while(|i| material_key(i) == material_key(first) && i.shape == first.shape).count();
                if let (Some(pipeline), Some((_, mesh))) = (self.pipeline(material_key(first), false), self.meshes.iter().find(|(s, _)| *s == first.shape)) {
                    pass.set_pipeline(pipeline);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, start as u32..(start + run) as u32);
                }
                start += run;
            }

            if frame.show_grid {
                pass.set_pipeline(&self.grid);
                pass.draw(0..6, 0..1);
            }

            for (offset, item) in transparent.iter().enumerate() {
                if let (Some(pipeline), Some((_, mesh))) = (self.pipeline(material_key(item), true), self.meshes.iter().find(|(s, _)| *s == item.shape)) {
                    let index = (opaque.len() + offset) as u32;
                    pass.set_pipeline(pipeline);
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, index..index + 1);
                }
            }
        }
        queue.submit([encoder.finish()]);
    }
}

fn globals_for(frame: &Frame) -> Globals {
    let mut lights = [LightRaw { direction: [0.0; 4], radiance: [0.0; 4] }; MAX_LIGHTS];
    for (slot, light) in lights.iter_mut().zip(&frame.lights) {
        *slot = LightRaw { direction: light.direction.extend(0.0).to_array(), radiance: light.radiance.extend(0.0).to_array() };
    }
    let count = frame.lights.len().min(MAX_LIGHTS) as f32;
    Globals {
        view_proj: frame.camera.view_proj().to_cols_array_2d(),
        camera_pos: frame.camera.position.extend(frame.time).to_array(),
        ambient: frame.ambient.extend(count).to_array(),
        lights,
    }
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

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
const INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
    2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4,
    6 => Float32x4, 7 => Float32x4, 8 => Float32x4,
    9 => Float32x4, 10 => Float32x4,
];

fn mesh_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, transparent: bool) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(if transparent { "bayleaf transparent" } else { "bayleaf opaque" }),
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
        // No culling: quads and planes are visible from both sides; the shader flips back-face normals.
        primitive: wgpu::PrimitiveState { cull_mode: None, ..wgpu::PrimitiveState::default() },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(!transparent),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState { count: SAMPLES, ..wgpu::MultisampleState::default() },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: transparent.then_some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn grid_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule) -> wgpu::RenderPipeline {
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
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState { count: SAMPLES, ..wgpu::MultisampleState::default() },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: COLOR_FORMAT,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

/// The built-in surface: the mesh component's colour, metallic and roughness.
pub(crate) const DEFAULT_SURFACE: &str = "fn surface(s: SurfaceIn) -> Surface {
    return Surface(s.color.rgb, s.params.x, s.params.y, vec3<f32>(0.0), s.color.a);
}";

/// The mesh shader with `surface` filled in.
pub(crate) fn mesh_source(surface: &str) -> String {
    include_str!("shaders/mesh.wgsl").replace("// @surface", surface)
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
