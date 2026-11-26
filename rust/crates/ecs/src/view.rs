//! A system's borrow of the columns its [`Access`] names. Views of systems that do not conflict
//! borrow disjoint columns of one world, so they can be used on different threads at once without
//! `unsafe`: each written column is lent to one view as `&mut`, each read-only column to any number
//! of views as `&`.

use std::any::TypeId;

use crate::{Access, Column, Component, Entity, Slot, TypedColumn, World, pool};

/// Fewest rows a job of a parallel query gets: below this, handing rows to another thread costs
/// more than it saves.
const MIN_ROWS_PER_JOB: usize = 256;

enum Col<'w> {
    Read(&'w dyn Column),
    Write(&'w mut dyn Column),
}

impl Col<'_> {
    fn read<T: Component>(&self) -> Option<&[T]> {
        let column: &dyn Column = match self {
            Col::Read(c) => *c,
            Col::Write(c) => &**c,
        };
        column.as_any().downcast_ref::<TypedColumn<T>>().map(|c| c.0.as_slice())
    }

    fn write<T: Component>(&mut self) -> Option<&mut [T]> {
        match self {
            Col::Read(_) => None,
            Col::Write(c) => c.as_any_mut().downcast_mut::<TypedColumn<T>>().map(|c| c.0.as_mut_slice()),
        }
    }
}

/// One archetype's entities and the view's columns of it.
struct Part<'w> {
    entities: &'w [Entity],
    columns: Vec<(TypeId, Col<'w>)>,
}

impl Part<'_> {
    fn read<T: Component>(&self) -> Option<&[T]> {
        self.columns.iter().find(|(t, _)| *t == TypeId::of::<T>()).and_then(|(_, c)| c.read())
    }

    fn write<T: Component>(&mut self) -> Option<&mut [T]> {
        self.columns.iter_mut().find(|(t, _)| *t == TypeId::of::<T>()).and_then(|(_, c)| c.write())
    }

    /// `W` to write and `R` to read, from two different columns.
    fn write_with<W: Component, R: Component>(&mut self) -> Option<(&mut [W], &[R])> {
        let (w_id, r_id) = (TypeId::of::<W>(), TypeId::of::<R>());
        if w_id == r_id {
            return None;
        }
        let (mut w, mut r) = (None, None);
        for (t, c) in &mut self.columns {
            if *t == w_id {
                w = Some(c);
            } else if *t == r_id {
                r = Some(&*c);
            }
        }
        Some((w?.write()?, r?.read()?))
    }
}

/// What one system may touch: the columns its [`Access`] names, in every archetype that has them.
/// Components it did not name are not there: queries skip them and lookups return `None`.
pub struct View<'w> {
    slots: &'w [Slot],
    /// By archetype index; `None` for archetypes with none of the view's types.
    parts: Vec<Option<Part<'w>>>,
    parallel: bool,
}

impl<'w> View<'w> {
    /// Splits `world` into one view per access. Accesses must not conflict with each other; where
    /// they do, a written column goes to the first access that writes it and no other.
    pub(crate) fn split(world: &'w mut World, accesses: &[&Access]) -> Vec<View<'w>> {
        let parallel = world.parallel();
        let World { slots, archetypes, .. } = world;
        let slots: &'w [Slot] = slots;
        let mut views: Vec<View<'w>> = accesses.iter().map(|_| View { slots, parts: Vec::new(), parallel }).collect();
        for archetype in archetypes.iter_mut() {
            let entities: &'w [Entity] = &archetype.entities;
            let mut parts: Vec<Part<'w>> = accesses.iter().map(|_| Part { entities, columns: Vec::new() }).collect();
            for (type_id, column) in archetype.columns.iter_mut() {
                if let Some(writer) = accesses.iter().position(|a| a.writes(*type_id)) {
                    if let Some(part) = parts.get_mut(writer) {
                        part.columns.push((*type_id, Col::Write(&mut **column)));
                    }
                    continue;
                }
                let column: &'w dyn Column = &**column;
                for (part, access) in parts.iter_mut().zip(accesses) {
                    if access.reads(*type_id) {
                        part.columns.push((*type_id, Col::Read(column)));
                    }
                }
            }
            for (view, part) in views.iter_mut().zip(parts) {
                view.parts.push((!part.columns.is_empty()).then_some(part));
            }
        }
        views
    }

    /// Whether queries of this view run on the thread pool.
    pub fn parallel(&self) -> bool {
        self.parallel
    }

    /// Every entity that has a `T`, if the view reads or writes `T`.
    pub fn query<T: Component>(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.parts.iter().flatten().filter_map(|p| p.read::<T>().map(|c| p.entities.iter().copied().zip(c.iter()))).flatten()
    }

    /// Every entity that has a `T`, with mutable access, if the view writes `T`.
    pub fn query_mut<T: Component>(&mut self) -> impl Iterator<Item = (Entity, &mut T)> {
        self.parts
            .iter_mut()
            .flatten()
            .filter_map(|p| {
                let entities = p.entities;
                p.write::<T>().map(|c| entities.iter().copied().zip(c.iter_mut()))
            })
            .flatten()
    }

    pub fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        let loc = World::location_in(self.slots, entity).ok()?;
        self.parts.get(loc.archetype)?.as_ref()?.read::<T>()?.get(loc.row)
    }

    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let loc = World::location_in(self.slots, entity).ok()?;
        self.parts.get_mut(loc.archetype)?.as_mut()?.write::<T>()?.get_mut(loc.row)
    }

    /// Calls `f` for every entity that has a `T`, on the thread pool when parallel is on. Each
    /// entity's call sees only its own component, so the result does not depend on the threads.
    pub fn for_each_mut<T: Component>(&mut self, f: impl Fn(Entity, &mut T) + Send + Sync) {
        let parallel = self.parallel;
        let rows = rows_per_job(parallel, self.parts.iter().flatten().filter(|p| p.read::<T>().is_some()).map(|p| p.entities.len()).sum());
        let mut jobs: Vec<(&[Entity], &mut [T])> = Vec::new();
        for part in self.parts.iter_mut().flatten() {
            let entities = part.entities;
            if let Some(column) = part.write::<T>() {
                jobs.extend(entities.chunks(rows).zip(column.chunks_mut(rows)));
            }
        }
        pool::run(parallel, jobs, |(entities, column)| {
            for (entity, value) in entities.iter().zip(column) {
                f(*entity, value);
            }
        });
    }

    /// Calls `f` for every entity that has both a `W` and an `R`, with the `W` to change and the
    /// `R` to read, on the thread pool when parallel is on.
    pub fn for_each_mut_with<W: Component, R: Component>(&mut self, f: impl Fn(Entity, &mut W, &R) + Send + Sync) {
        let parallel = self.parallel;
        let rows =
            rows_per_job(parallel, self.parts.iter().flatten().filter(|p| p.read::<W>().is_some() && p.read::<R>().is_some()).map(|p| p.entities.len()).sum());
        let mut jobs: Vec<(&[Entity], &mut [W], &[R])> = Vec::new();
        for part in self.parts.iter_mut().flatten() {
            let entities = part.entities;
            if let Some((written, read)) = part.write_with::<W, R>() {
                jobs.extend(entities.chunks(rows).zip(written.chunks_mut(rows)).zip(read.chunks(rows)).map(|((e, w), r)| (e, w, r)));
            }
        }
        pool::run(parallel, jobs, |(entities, written, read)| {
            for ((entity, w), r) in entities.iter().zip(written).zip(read) {
                f(*entity, w, r);
            }
        });
    }
}

/// Rows per job: one job per archetype when the query runs on one thread, else enough jobs to
/// keep every thread busy, but none under [`MIN_ROWS_PER_JOB`].
fn rows_per_job(parallel: bool, total: usize) -> usize {
    if !parallel {
        return usize::MAX;
    }
    total.div_ceil(pool::threads().saturating_mul(4).max(1)).max(MIN_ROWS_PER_JOB)
}
