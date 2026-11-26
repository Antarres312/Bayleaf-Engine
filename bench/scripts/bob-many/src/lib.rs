//! Benchmark script: `bob` written for many objects. Its properties are read into fields when they
//! change, and one update moves every object.

use bayleaf_script_sdk::{Many, Property, Script, property, script, time};

#[derive(Default)]
struct Bob {
    speed: f32,
    height: f32,
    base: f32,
}

impl Script for Bob {
    const PROPERTIES: &'static [Property] = &[Property::Number("speed", 2.0), Property::Number("height", 0.5), Property::Number("base", 0.0)];

    fn read_properties(&mut self) {
        self.speed = property("speed").unwrap_or(2.0);
        self.height = property("height").unwrap_or(0.5);
        self.base = property("base").unwrap_or(0.0);
    }

    fn update_many(objects: &mut Many<'_, Self>, _dt: f32) {
        let now = time() as f32;
        for (bob, t) in objects.iter_mut() {
            t.position[1] = bob.base + bob.height * (now * bob.speed).sin();
        }
    }
}

script!(Bob);
