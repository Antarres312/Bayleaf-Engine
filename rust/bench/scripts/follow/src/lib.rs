//! Benchmark script: eases the object towards the object named `Target` on the ground plane.
//! Property `rate`: the part of the way covered per second.

use bayleaf_script_sdk::{Kept, Object, Property, Script, Transform, property, script};

#[derive(Default)]
struct Follow {
    target: Option<Kept>,
}

impl Script for Follow {
    const PROPERTIES: &'static [Property] = &[Property::Number("rate", 0.5)];

    fn start(&mut self) {
        self.target = Object::find("Target").and_then(Object::keep);
    }

    fn update(&mut self, dt: f32) {
        let Some(target) = self.target.and_then(Kept::object) else { return };
        let rate = property("rate").unwrap_or(0.5);
        let goal = target.transform().position;
        let mut t = Transform::get();
        let f = (rate * dt).min(1.0);
        t.position[0] += (goal[0] - t.position[0]) * f;
        t.position[2] += (goal[2] - t.position[2]) * f;
        t.set();
    }
}

script!(Follow);
