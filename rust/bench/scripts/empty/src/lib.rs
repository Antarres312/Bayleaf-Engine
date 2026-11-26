//! Benchmark script: an update that does nothing, to time what running one script costs by itself.

use bayleaf_script_sdk::{Script, script};

#[derive(Default)]
struct Empty;

impl Script for Empty {
    fn update(&mut self, _dt: f32) {}
}

script!(Empty);
