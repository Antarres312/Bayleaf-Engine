//! Rendering without a window: a scene to RGBA pixels or a PNG file.

use std::path::Path;

use crate::RenderError;
use crate::extract::Frame;
use crate::gpu::{RenderTarget, Renderer};

/// An RGBA8 image, rows top to bottom.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let start = (y as usize * self.width as usize + x as usize) * 4;
        self.pixels.get(start..start + 4).and_then(|p| p.try_into().ok())
    }

    pub fn write_png(&self, path: &Path) -> Result<(), RenderError> {
        let file = std::fs::File::create(path).map_err(|e| RenderError::Io(format!("cannot create {}: {e}", path.display())))?;
        self.encode_png(std::io::BufWriter::new(file))
    }

    /// The image as PNG file bytes.
    pub fn png_bytes(&self) -> Result<Vec<u8>, RenderError> {
        let mut bytes = Vec::new();
        self.encode_png(&mut bytes)?;
        Ok(bytes)
    }

    fn encode_png(&self, out: impl std::io::Write) -> Result<(), RenderError> {
        let mut encoder = png::Encoder::new(out, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().map_err(|e| RenderError::Io(e.to_string()))?;
        writer.write_image_data(&self.pixels).map_err(|e| RenderError::Io(e.to_string()))
    }
}

/// A device for rendering without a window.
pub struct HeadlessGpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl HeadlessGpu {
    /// Picks a GPU, falling back to a software adapter when there is none. `WGPU_BACKEND` and
    /// `WGPU_POWER_PREF` choose another, as they do for the editor's window.
    pub fn new() -> Result<Self, RenderError> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        if let Some(backends) = wgpu::Backends::from_env() {
            descriptor.backends = backends;
        }
        let instance = wgpu::Instance::new(descriptor);
        let chosen = wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::from_env().unwrap_or_default(), ..Default::default() };
        let adapter = pollster::block_on(instance.request_adapter(&chosen))
            .or_else(|_| pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { force_fallback_adapter: true, ..Default::default() })))
            .map_err(|e| RenderError::NoGpu(e.to_string()))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("bayleaf headless"),
            // GPU time for the profiler and block-compressed images, where the adapter has them.
            required_features: adapter.features() & (wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TEXTURE_COMPRESSION_BC),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| RenderError::NoGpu(e.to_string()))?;
        Ok(Self { device, queue })
    }

    /// Renders `frame` at `width` × `height` and reads the pixels back.
    pub fn render(&self, frame: &Frame, width: u32, height: u32) -> Result<Image, RenderError> {
        self.render_with_samples(frame, width, height, crate::gpu::DEFAULT_SAMPLES)
    }

    /// [`Self::render`] with `samples` per pixel: 1 without multisampling.
    pub fn render_with_samples(&self, frame: &Frame, width: u32, height: u32, samples: u32) -> Result<Image, RenderError> {
        let mut renderer = Renderer::with_samples(&self.device, samples);
        let target = RenderTarget::with_samples(&self.device, width, height, samples);
        renderer.render(&self.device, &self.queue, &target, frame);
        if let Some(error) = renderer.material_error() {
            return Err(RenderError::Gpu(format!("a material shader was refused: {error}")));
        }
        self.read_back(&target)
    }

    pub(crate) fn read_back(&self, target: &RenderTarget) -> Result<Image, RenderError> {
        let (width, height) = target.size();
        let unpadded = width * 4;
        let padded = unpadded.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bayleaf readback"),
            size: u64::from(padded) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("bayleaf readback") });
        encoder.copy_texture_to_buffer(
            target.texture().as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(height) },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| RenderError::Gpu(e.to_string()))?;
        receiver.recv().map_err(|e| RenderError::Gpu(e.to_string()))?.map_err(|e| RenderError::Gpu(e.to_string()))?;
        let mut pixels = Vec::with_capacity((unpadded * height) as usize);
        {
            let data = slice.get_mapped_range().map_err(|e| RenderError::Gpu(e.to_string()))?;
            for row in data.chunks(padded as usize).take(height as usize) {
                pixels.extend_from_slice(row.get(..unpadded as usize).unwrap_or(row));
            }
        }
        buffer.unmap();
        Ok(Image { width, height, pixels })
    }
}
