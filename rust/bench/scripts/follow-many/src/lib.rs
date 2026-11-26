//! Benchmark script: `follow` written for many objects. Its property is read into a field when it
//! changes, and one update eases every object towards the object named `Target`.

use bayleaf_script_sdk::{Kept, Many, Object, Property, Script, property, script};

#[derive(Default)]
struct Follow {
    rate: f32,
    target: Option<Kept>,
}

impl Script for Follow {
    const PROPERTIES: &'static [Property] = &[Property::Number("rate", 0.5)];

    fn read_properties(&mut self) {
        self.rate = property("rate").unwrap_or(0.5);
    }

    fn start(&mut self) {
        self.target = Object::find("Target").and_then(Object::keep);
    }

    fn update_many(objects: &mut Many<'_, Self>, dt: f32) {
        for (follow, t) in objects.iter_mut() {
            let Some(target) = follow.target.and_then(Kept::object) else { continue };
            let goal = target.transform().position;
            let f = (follow.rate * dt).min(1.0);
            t.position[0] += (goal[0] - t.position[0]) * f;
            t.position[2] += (goal[2] - t.position[2]) * f;
        }
    }
}

script!(Follow);
