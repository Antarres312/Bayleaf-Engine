//! The GPU's own time for a frame, where the device writes timestamps (`TIMESTAMP_QUERY`): an empty
//! pass marks the frame's start and another its end. The two times are read back a frame or more
//! later, never waiting for the GPU; while a reading is on its way, frames go untimed.

use std::sync::{Arc, Mutex};

/// Bytes of the two timestamps.
const BYTES: u64 = 16;

pub(crate) struct GpuTimer {
    queries: wgpu::QuerySet,
    resolved: wgpu::Buffer,
    readback: wgpu::Buffer,
    /// Nanoseconds a timestamp tick stands for.
    period: f32,
    /// Set by the read-back once the readback buffer can be read: whether it mapped.
    pending: Option<Arc<Mutex<Option<bool>>>>,
    /// Whether this frame writes timestamps.
    timing: bool,
    /// The newest frame measured, in milliseconds.
    last: Option<f32>,
    /// Set when the timestamps read back were all zero.
    silent: bool,
}

impl GpuTimer {
    /// A timer, when the device can write timestamps.
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let queries = device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("bayleaf frame time"), ty: wgpu::QueryType::Timestamp, count: 2 });
        let buffer = |label, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: BYTES, usage, mapped_at_creation: false });
        Some(Self {
            queries,
            resolved: buffer("bayleaf frame time resolved", wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC),
            readback: buffer("bayleaf frame time readback", wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST),
            period: queue.get_timestamp_period(),
            pending: None,
            timing: false,
            last: None,
            silent: false,
        })
    }

    /// The newest frame's GPU time, in milliseconds, once one has been read.
    pub(crate) fn last(&self) -> Option<f32> {
        self.last
    }

    /// Reads a finished measurement, if there is one, then marks this frame's start unless one is
    /// still on its way.
    pub(crate) fn begin(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        if self.silent {
            self.timing = false;
            return;
        }
        if let Some(done) = &self.pending {
            // Lets the read-back finish without waiting for the GPU.
            if device.poll(wgpu::PollType::Poll).is_err() {
                self.timing = false;
                return;
            }
            let state = done.lock().ok().and_then(|s| *s);
            match state {
                None => {
                    self.timing = false;
                    return;
                }
                Some(mapped) => {
                    if mapped {
                        self.last = self.read();
                        self.readback.unmap();
                    }
                    self.pending = None;
                }
            }
        }
        self.timing = true;
        self.mark(encoder, Some(0), None);
    }

    /// Marks the frame's end and copies both times out, when this frame is timed.
    pub(crate) fn end(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if !self.timing {
            return;
        }
        self.mark(encoder, None, Some(1));
        encoder.resolve_query_set(&self.queries, 0..2, &self.resolved, 0);
        encoder.copy_buffer_to_buffer(&self.resolved, 0, &self.readback, 0, BYTES);
    }

    /// Asks for the times once the frame has been sent.
    pub(crate) fn after_submit(&mut self) {
        if !std::mem::take(&mut self.timing) {
            return;
        }
        let done = Arc::new(Mutex::new(None));
        let set = Arc::clone(&done);
        self.readback.map_async(wgpu::MapMode::Read, .., move |result| {
            if let Ok(mut state) = set.lock() {
                *state = Some(result.is_ok());
            }
        });
        self.pending = Some(done);
    }

    fn mark(&self, encoder: &mut wgpu::CommandEncoder, beginning: Option<u32>, end: Option<u32>) {
        let writes = wgpu::ComputePassTimestampWrites { query_set: &self.queries, beginning_of_pass_write_index: beginning, end_of_pass_write_index: end };
        drop(encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("bayleaf frame time"), timestamp_writes: Some(writes) }));
    }

    /// Whether the device's timestamps turned out to measure anything.
    pub(crate) fn works(&self) -> bool {
        !self.silent
    }

    fn read(&mut self) -> Option<f32> {
        let view = self.readback.get_mapped_range(..).ok()?;
        let ticks: [u64; 2] = bytemuck::pod_read_unaligned(view.get(..BYTES as usize)?);
        drop(view);
        // Some drivers offer timestamps and write only zeros (seen with Vulkan on an integrated
        // AMD GPU): no time is measured there.
        if ticks == [0, 0] {
            self.silent = true;
            return None;
        }
        // A clock that went backwards measures nothing.
        let elapsed = ticks[1].checked_sub(ticks[0])?;
        Some(elapsed as f32 * self.period / 1_000_000.0)
    }
}
