//! Benchmark script: moves the object up and down. Properties: `speed` (radians per second),
//! `height` (how far from `base`) and `base` (the middle height).

use bayleaf_script_sdk::{Property, Script, Transform, property, script, time};

#[derive(Default)]
struct Bob;

impl Script for Bob {
    const PROPERTIES: &'static [Property] = &[Property::Number("speed", 2.0), Property::Number("height", 0.5), Property::Number("base", 0.0)];

    fn update(&mut self, _dt: f32) {
        let speed = property("speed").unwrap_or(2.0);
        let height = property("height").unwrap_or(0.5);
        let base = property("base").unwrap_or(0.0);
        let mut t = Transform::get();
        t.position[1] = base + height * (time() as f32 * speed).sin();
        t.set();
    }
}

script!(Bob);
