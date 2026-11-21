//! Ordered list of systems run against a [`World`].

use crate::World;

type System = Box<dyn FnMut(&mut World) + Send>;

/// Systems in the order they run.
#[derive(Default)]
pub struct Schedule {
    systems: Vec<(String, System)>,
}

impl Schedule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a system. It runs after every system added before it.
    pub fn add(&mut self, name: impl Into<String>, system: impl FnMut(&mut World) + Send + 'static) {
        self.systems.push((name.into(), Box::new(system)));
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.systems.iter().map(|(name, _)| name.as_str())
    }

    /// Runs every system once, in order.
    pub fn run(&mut self, world: &mut World) {
        for (_, system) in &mut self.systems {
            system(world);
        }
    }
}
