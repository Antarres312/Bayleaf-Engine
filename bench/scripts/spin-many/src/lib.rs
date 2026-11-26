//! Benchmark script: `spin` written for many objects. Its property is read into a field when it
//! changes, and one update turns every object.

use bayleaf_script_sdk::{Many, Property, Script, property, script};

#[derive(Default)]
struct Spin {
    speed: f32,
}

impl Script for Spin {
    const PROPERTIES: &'static [Property] = &[Property::Number("speed", 90.0)];

    fn read_properties(&mut self) {
        self.speed = property("speed").unwrap_or(90.0);
    }

    fn update_many(objects: &mut Many<'_, Self>, dt: f32) {
        for (spin, t) in objects.iter_mut() {
            t.rotation[1] = (t.rotation[1] + spin.speed * dt) % 360.0;
        }
    }
}

script!(Spin);
