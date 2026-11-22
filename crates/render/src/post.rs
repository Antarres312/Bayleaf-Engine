//! Post-processing: the passes after the scene that make the displayed picture from its high-range
//! colour (`shaders/post.wgsl`): bloom down and up a chain of half-size pictures, then one pass with
//! exposure, tone mapping, colour adjustments, vignette, depth fog, ambient occlusion and
//! antialiasing. With [`PostSettings::default`] the picture is the scene's colour, clamped.

use crate::gpu::{COLOR_FORMAT, HDR_FORMAT, RenderTarget};

/// What post-processing a frame gets. The default changes nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostSettings {
    /// Brightness before tone mapping; 1 as drawn.
    pub exposure: f32,
    /// A filmic curve from high-range colour to the screen; off, colours above 1 clip.
    pub tone_mapping: bool,
    /// How strongly bright parts glow; 0 for no bloom.
    pub bloom: f32,
    /// Brightness from which parts glow.
    pub bloom_threshold: f32,
    /// 1 as drawn; more stretches colours away from mid grey.
    pub contrast: f32,
    /// 1 as drawn; 0 grey.
    pub saturation: f32,
    /// How dark the corners go, 0 to 1; 0 for none.
    pub vignette: f32,
    /// How gradually the vignette sets in, 0 to 1.
    pub vignette_smoothness: f32,
    /// Fog by distance from the camera: linear RGB colour, the distance where it starts and where it
    /// hides everything.
    pub fog: Option<([f32; 3], f32, f32)>,
    /// How dark creases and corners go (ambient occlusion), 0 to 1; 0 for none.
    pub occlusion: f32,
    /// How far around a point occlusion looks, in world units.
    pub occlusion_radius: f32,
    /// Smooths the remaining jagged edges of the finished picture.
    pub antialiasing: bool,
}

impl Default for PostSettings {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            tone_mapping: false,
            bloom: 0.0,
            bloom_threshold: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            vignette: 0.0,
            vignette_smoothness: 0.5,
            fog: None,
            occlusion: 0.0,
            occlusion_radius: 0.5,
            antialiasing: false,
        }
    }
}

/// Steps of the bloom chain, each half the size of the one before.
pub(crate) const BLOOM_LEVELS: usize = 5;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostRaw {
    a: [f32; 4],
    b: [f32; 4],
    fog: [f32; 4],
    c: [f32; 4],
    d: [f32; 4],
}

impl PostRaw {
    fn of(s: &PostSettings, width: u32, height: u32) -> Self {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        let (fog_color, start, end, on) = match s.fog {
            Some((color, start, end)) => (color, start, end, 1.0),
            None => ([0.0; 3], 0.0, 1.0, 0.0),
        };
        Self {
            a: [finite(s.exposure, 1.0).max(0.0), f32::from(u8::from(s.tone_mapping)), finite(s.bloom, 0.0).max(0.0), finite(s.bloom_threshold, 1.0).max(0.0)],
            b: [
                finite(s.contrast, 1.0),
                finite(s.saturation, 1.0),
                finite(s.vignette, 0.0).clamp(0.0, 1.0),
                finite(s.vignette_smoothness, 0.5).clamp(0.0, 1.0),
            ],
            fog: [fog_color[0], fog_color[1], fog_color[2], on],
            c: [finite(start, 0.0), finite(end, 1.0), finite(s.occlusion, 0.0).clamp(0.0, 1.0), finite(s.occlusion_radius, 0.5).max(0.01)],
            d: [f32::from(u8::from(s.antialiasing)), 1.0 / width.max(1) as f32, 1.0 / height.max(1) as f32, f32::from(u8::from(s.bloom > 0.0))],
        }
    }
}

/// The pipelines and buffers of post-processing.
pub(crate) struct Post {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    finish: wgpu::RenderPipeline,
    bright: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    up: wgpu::RenderPipeline,
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType, multisampled: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled },
        count: None,
    }
}

impl Post {
    pub(crate) fn new(device: &wgpu::Device, globals_layout: &wgpu::BindGroupLayout, shader_source: String) -> Self {
        let float = wgpu::TextureSampleType::Float { filterable: true };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bayleaf post layout"),
            entries: &[
                texture_entry(0, float, false),
                texture_entry(1, float, false),
                texture_entry(2, wgpu::TextureSampleType::Depth, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bayleaf post sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bayleaf post params"),
            size: std::mem::size_of::<PostRaw>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shader = device
            .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("bayleaf post shader"), source: wgpu::ShaderSource::Wgsl(shader_source.into()) });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bayleaf post pipeline layout"),
            bind_group_layouts: &[Some(globals_layout), Some(&layout)],
            immediate_size: 0,
        });
        let make = |entry: &str, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_full"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        Self {
            finish: make("fs_post", COLOR_FORMAT, None),
            bright: make("fs_bright", HDR_FORMAT, None),
            down: make("fs_down", HDR_FORMAT, None),
            up: make("fs_up", HDR_FORMAT, Some(add)),
            layout,
            sampler,
            params,
        }
    }

    fn group(&self, device: &wgpu::Device, source: &wgpu::TextureView, bloom: &wgpu::TextureView, depth: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bayleaf post group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(bloom) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: self.params.as_entire_binding() },
            ],
        })
    }

    fn pass(
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        clear: bool,
        pipeline: &wgpu::RenderPipeline,
        globals: &wgpu::BindGroup,
        group: &wgpu::BindGroup,
    ) {
        let load = if clear { wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT) } else { wgpu::LoadOp::Load };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("bayleaf post pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, globals, &[]);
        pass.set_bind_group(1, group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Makes the target's displayed picture from its high-range colour, as `settings` say.
    pub(crate) fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &RenderTarget,
        globals: &wgpu::BindGroup,
        settings: &PostSettings,
    ) {
        let (width, height) = target.size();
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&PostRaw::of(settings, width, height)));
        let bloom = target.bloom_views();
        if settings.bloom > 0.0 {
            // Bright parts into the first half-size picture, down the chain, then back up, adding.
            Self::pass(encoder, &bloom[0], true, &self.bright, globals, &self.group(device, target.hdr_view(), &bloom[1], target.depth_view()));
            for level in 1..bloom.len() {
                Self::pass(encoder, &bloom[level], true, &self.down, globals, &self.group(device, &bloom[level - 1], target.hdr_view(), target.depth_view()));
            }
            for level in (0..bloom.len() - 1).rev() {
                Self::pass(encoder, &bloom[level], false, &self.up, globals, &self.group(device, &bloom[level + 1], target.hdr_view(), target.depth_view()));
            }
        }
        Self::pass(encoder, target.view(), true, &self.finish, globals, &self.group(device, target.hdr_view(), &bloom[0], target.depth_view()));
    }
}
