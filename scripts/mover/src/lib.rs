//! Moves the object on the ground plane with W, A, S and D (or the arrow keys) and hops with Space.
//! Properties: `speed` in units per second (default 3), `jump` hop height (default 1).

use bayleaf_script_sdk::{Script, Transform, key_down, property, script};

#[derive(Default)]
struct Mover {
    ground: Option<f32>,
    vertical: f32,
}

impl Script for Mover {
    const PROPERTIES: &'static [(&'static str, f32)] = &[("speed", 3.0), ("jump", 1.0)];

    fn update(&mut self, dt: f32) {
        let speed = property("speed").unwrap_or(3.0);
        let jump = property("jump").unwrap_or(1.0);
        let axis = |plus: &[&str], minus: &[&str]| {
            let held = |keys: &[&str]| keys.iter().any(|k| key_down(k));
            f32::from(u8::from(held(plus))) - f32::from(u8::from(held(minus)))
        };
        let mut t = Transform::get();
        let ground = *self.ground.get_or_insert(t.position[1]);
        t.position[0] += axis(&["D", "ArrowRight"], &["A", "ArrowLeft"]) * speed * dt;
        t.position[2] += axis(&["S", "ArrowDown"], &["W", "ArrowUp"]) * speed * dt;

        // A simple hop: launch at the speed that reaches `jump` units, fall back with gravity.
        const GRAVITY: f32 = 9.81;
        if key_down("Space") && t.position[1] <= ground {
            self.vertical = (2.0 * GRAVITY * jump).sqrt();
        }
        self.vertical -= GRAVITY * dt;
        t.position[1] = (t.position[1] + self.vertical * dt).max(ground);
        if t.position[1] <= ground {
            self.vertical = 0.0;
        }
        t.set();
    }
}

script!(Mover);
