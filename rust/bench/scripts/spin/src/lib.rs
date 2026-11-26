//! Benchmark script: turns the object about its Y axis. Property `speed`: degrees per second.

use bayleaf_script_sdk::{Property, Script, Transform, property, script};

#[derive(Default)]
struct Spin;

impl Script for Spin {
    const PROPERTIES: &'static [Property] = &[Property::Number("speed", 90.0)];

    fn update(&mut self, dt: f32) {
        let speed = property("speed").unwrap_or(90.0);
        let mut t = Transform::get();
        t.rotation[1] = (t.rotation[1] + speed * dt) % 360.0;
        t.set();
    }
}

script!(Spin);
