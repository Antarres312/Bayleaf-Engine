//! Benchmark script: an update that does nothing, like `empty`, in a module that imports the host's
//! component functions (as every visual script and the native bridge do). Its start reads a field
//! only when the object has a property no object sets, so the call never happens; the import alone
//! makes the host gather every object's component fields each step.

use bayleaf_script_sdk::{Object, Script, property, script};

#[derive(Default)]
struct Fields;

impl Script for Fields {
    fn start(&mut self) {
        if property("never").is_some() {
            let _ = Object::this().and_then(|o| o.field("pointLight", "intensity"));
        }
    }

    fn update(&mut self, _dt: f32) {}
}

script!(Fields);
