//! Ordered list of systems run against a [`World`].
//!
//! A system added with [`Schedule::add`] gets the whole world and runs alone, on the calling
//! thread, after everything before it and before everything after it: structural changes and
//! other writes that must be ordered go there. A system added with [`Schedule::add_system`]
//! declares an [`Access`] and gets a [`View`] of those columns. When the world's parallel switch
//! is on, such systems run in batches: a system joins the earliest batch after every earlier
//! system it conflicts with (and after the last whole-world system), and the systems of a batch
//! run at once. Systems that conflict keep their order, and systems that do not conflict touch
//! different columns, so the world ends the same as when every system runs in turn.

use crate::{Access, View, World, pool};

type WorldSystem = Box<dyn FnMut(&mut World) + Send>;
type ViewSystem = Box<dyn FnMut(&mut View<'_>) + Send>;

enum System {
    World(WorldSystem),
    View(Access, ViewSystem),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Batch {
    World(usize),
    Views(Vec<usize>),
}

/// Systems in the order they run.
#[derive(Default)]
pub struct Schedule {
    systems: Vec<(String, System)>,
    batches: Vec<Batch>,
}

impl Schedule {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a system that gets the whole world. It runs alone, on the calling thread, after
    /// every system added before it.
    pub fn add(&mut self, name: impl Into<String>, system: impl FnMut(&mut World) + Send + 'static) {
        self.batches.push(Batch::World(self.systems.len()));
        self.systems.push((name.into(), System::World(Box::new(system))));
    }

    /// Appends a system that touches only the components `access` names. It runs after every
    /// earlier system it conflicts with, and may run at the same time as others.
    pub fn add_system(&mut self, name: impl Into<String>, access: Access, system: impl FnMut(&mut View<'_>) + Send + 'static) {
        let index = self.systems.len();
        let floor = self.batches.iter().rposition(|b| matches!(b, Batch::World(_))).map_or(0, |b| b + 1);
        let after = self
            .batches
            .iter()
            .enumerate()
            .skip(floor)
            .filter(|(_, b)| matches!(b, Batch::Views(ids) if ids.iter().any(|&i| self.access(i).is_some_and(|a| a.conflicts_with(&access)))))
            .map(|(b, _)| b + 1)
            .max()
            .unwrap_or(floor);
        match self.batches.get_mut(after) {
            Some(Batch::Views(ids)) => ids.push(index),
            _ => self.batches.push(Batch::Views(vec![index])),
        }
        self.systems.push((name.into(), System::View(access, Box::new(system))));
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.systems.iter().map(|(name, _)| name.as_str())
    }

    /// The systems' names in the batches they run in when parallel is on, in order.
    pub fn batches(&self) -> Vec<Vec<&str>> {
        let name = |i: &usize| self.systems.get(*i).map_or("", |(n, _)| n.as_str());
        self.batches
            .iter()
            .map(|b| match b {
                Batch::World(i) => vec![name(i)],
                Batch::Views(ids) => ids.iter().map(name).collect(),
            })
            .collect()
    }

    fn access(&self, index: usize) -> Option<&Access> {
        match self.systems.get(index) {
            Some((_, System::View(access, _))) => Some(access),
            _ => None,
        }
    }

    /// Runs every system once: in the order added, or in batches when the world's parallel
    /// switch is on.
    pub fn run(&mut self, world: &mut World) {
        if !world.parallel() {
            for (_, system) in &mut self.systems {
                match system {
                    System::World(run) => run(world),
                    System::View(access, run) => {
                        if let Some(mut view) = world.view(access) {
                            run(&mut view);
                        }
                    }
                }
            }
            return;
        }
        for batch in &self.batches {
            match batch {
                Batch::World(i) => {
                    if let Some((_, System::World(run))) = self.systems.get_mut(*i) {
                        run(world);
                    }
                }
                Batch::Views(ids) => {
                    let mut accesses = Vec::with_capacity(ids.len());
                    let mut runs = Vec::with_capacity(ids.len());
                    for (i, (_, system)) in self.systems.iter_mut().enumerate() {
                        if let System::View(access, run) = system
                            && ids.contains(&i)
                        {
                            accesses.push(&*access);
                            runs.push(run);
                        }
                    }
                    let views = View::split(world, &accesses);
                    pool::run(true, runs.into_iter().zip(views).collect(), |(run, mut view): (&mut ViewSystem, View<'_>)| run(&mut view));
                }
            }
        }
    }
}
