//! Turns the object around its Y axis. Property `speed`: degrees per second (default 90).

use bayleaf_script_sdk::{Property, Script, Transform, log, property, script};

#[derive(Default)]
struct Spinner;

impl Script for Spinner {
    const PROPERTIES: &'static [Property] = &[Property::Number("speed", 90.0)];

    fn start(&mut self) {
        log("spinner: started");
    }

    fn update(&mut self, dt: f32) {
        let speed = property("speed").unwrap_or(90.0);
        let mut transform = Transform::get();
        transform.rotation[1] = (transform.rotation[1] + speed * dt) % 360.0;
        transform.set();
    }
}

script!(Spinner);
