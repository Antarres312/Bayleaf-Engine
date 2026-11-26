//! Turns the object around its Y axis. Property `speed`: degrees per second (default 90).

use bayleaf_script_sdk::{Many, Property, Script, log, property, script};

#[derive(Default)]
struct Spinner {
    speed: f32,
}

impl Script for Spinner {
    const PROPERTIES: &'static [Property] = &[Property::Number("speed", 90.0)];

    // Read when the speed may have changed, not every step.
    fn read_properties(&mut self) {
        self.speed = property("speed").unwrap_or(90.0);
    }

    fn start(&mut self) {
        log("spinner: started");
    }

    // Every spinning object in one go: each one's script and transform.
    fn update_many(objects: &mut Many<'_, Self>, dt: f32) {
        for (spinner, transform) in objects.iter_mut() {
            transform.rotation[1] = (transform.rotation[1] + spinner.speed * dt) % 360.0;
        }
    }
}

script!(Spinner);
