//! L1: archetype entity storage, queries and the system scheduler.
//!
//! Entities with the same set of component types share an archetype. Each archetype stores one
//! column per component type. Adding or removing a component moves the entity's row to the
//! matching archetype. No `unsafe`: columns are type-erased behind [`std::any::Any`].

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod schedule;

pub use schedule::Schedule;

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// Any `'static` thread-safe type can be a component.
pub trait Component: Any + Send + Sync {}
impl<T: Any + Send + Sync> Component for T {}

/// Handle to an entity. A despawned entity's handle stays invalid even after its slot is reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Entity {
    index: u32,
    generation: u32,
}

/// Errors from world operations.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EcsError {
    #[error("entity {0:?} does not exist")]
    NoSuchEntity(Entity),
    #[error("entity limit reached")]
    TooManyEntities,
}

#[derive(Clone, Copy, Debug)]
struct Location {
    archetype: usize,
    row: usize,
}

#[derive(Debug)]
struct Slot {
    generation: u32,
    location: Option<Location>,
}

trait Column: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn empty_like(&self) -> Box<dyn Column>;
    /// Moves the value at `row` to the end of `dst`, filling `row` with the last value.
    fn move_row(&mut self, row: usize, dst: &mut dyn Column);
    fn drop_row(&mut self, row: usize);
}

struct TypedColumn<T>(Vec<T>);

impl<T: Component> Column for TypedColumn<T> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn empty_like(&self) -> Box<dyn Column> {
        Box::new(TypedColumn::<T>(Vec::new()))
    }

    fn move_row(&mut self, row: usize, dst: &mut dyn Column) {
        if row < self.0.len() {
            let value = self.0.swap_remove(row);
            if let Some(dst) = dst.as_any_mut().downcast_mut::<TypedColumn<T>>() {
                dst.0.push(value);
            }
        }
    }

    fn drop_row(&mut self, row: usize) {
        if row < self.0.len() {
            self.0.swap_remove(row);
        }
    }
}

struct Archetype {
    columns: HashMap<TypeId, Box<dyn Column>>,
    entities: Vec<Entity>,
}

impl Archetype {
    fn column<T: Component>(&self) -> Option<&Vec<T>> {
        self.columns.get(&TypeId::of::<T>()).and_then(|c| c.as_any().downcast_ref::<TypedColumn<T>>()).map(|c| &c.0)
    }

    fn column_mut<T: Component>(&mut self) -> Option<&mut Vec<T>> {
        self.columns.get_mut(&TypeId::of::<T>()).and_then(|c| c.as_any_mut().downcast_mut::<TypedColumn<T>>()).map(|c| &mut c.0)
    }
}

/// Storage for every entity and component.
pub struct World {
    slots: Vec<Slot>,
    free: Vec<u32>,
    archetypes: Vec<Archetype>,
    by_types: HashMap<Vec<TypeId>, usize>,
}

impl Default for World {
    fn default() -> Self {
        let empty = Archetype { columns: HashMap::new(), entities: Vec::new() };
        Self { slots: Vec::new(), free: Vec::new(), archetypes: vec![empty], by_types: HashMap::from([(Vec::new(), 0)]) }
    }
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an entity with no components.
    pub fn spawn(&mut self) -> Result<Entity, EcsError> {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                let index = u32::try_from(self.slots.len()).map_err(|_| EcsError::TooManyEntities)?;
                self.slots.push(Slot { generation: 0, location: None });
                index
            }
        };
        let generation = self.slots.get(index as usize).map_or(0, |s| s.generation);
        let entity = Entity { index, generation };
        let row = self.archetypes.first().map_or(0, |a| a.entities.len());
        if let Some(empty) = self.archetypes.first_mut() {
            empty.entities.push(entity);
        }
        self.set_location(entity, Some(Location { archetype: 0, row }));
        Ok(entity)
    }

    /// Removes an entity and all its components.
    pub fn despawn(&mut self, entity: Entity) -> Result<(), EcsError> {
        let loc = self.location(entity)?;
        if let Some(arch) = self.archetypes.get_mut(loc.archetype) {
            for column in arch.columns.values_mut() {
                column.drop_row(loc.row);
            }
        }
        self.remove_entity_row(loc);
        if let Some(slot) = self.slots.get_mut(entity.index as usize) {
            slot.location = None;
            slot.generation = slot.generation.wrapping_add(1);
        }
        self.free.push(entity.index);
        Ok(())
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        self.location(entity).is_ok()
    }

    /// Number of live entities.
    pub fn len(&self) -> usize {
        self.archetypes.iter().map(|a| a.entities.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Adds `value` to `entity`, replacing a component of the same type.
    pub fn insert<T: Component>(&mut self, entity: Entity, value: T) -> Result<(), EcsError> {
        let loc = self.location(entity)?;
        let type_id = TypeId::of::<T>();
        if let Some(column) = self.archetypes.get_mut(loc.archetype).and_then(|a| a.column_mut::<T>())
            && let Some(slot) = column.get_mut(loc.row)
        {
            *slot = value;
            return Ok(());
        }
        let mut types = self.types_of(loc.archetype);
        types.push(type_id);
        types.sort_unstable();
        let dst = self.archetype_for(types, loc.archetype, Some((type_id, Box::new(TypedColumn::<T>(Vec::new())))), None);
        self.move_entity(entity, loc, dst, None);
        if let Some(column) = self.archetypes.get_mut(dst).and_then(|a| a.column_mut::<T>()) {
            column.push(value);
        }
        Ok(())
    }

    /// Removes and returns the component of type `T`, if the entity has one.
    pub fn remove<T: Component>(&mut self, entity: Entity) -> Result<Option<T>, EcsError> {
        let loc = self.location(entity)?;
        let type_id = TypeId::of::<T>();
        let types: Vec<TypeId> = self.types_of(loc.archetype).into_iter().filter(|t| *t != type_id).collect();
        if types.len() == self.types_of(loc.archetype).len() {
            return Ok(None);
        }
        let dst = self.archetype_for(types, loc.archetype, None, Some(type_id));
        let taken = self.archetypes.get_mut(loc.archetype).and_then(|a| a.column_mut::<T>()).and_then(|c| (loc.row < c.len()).then(|| c.swap_remove(loc.row)));
        self.move_entity(entity, loc, dst, Some(type_id));
        Ok(taken)
    }

    pub fn has<T: Component>(&self, entity: Entity) -> bool {
        self.get::<T>(entity).is_some()
    }

    pub fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        let loc = self.location(entity).ok()?;
        self.archetypes.get(loc.archetype)?.column::<T>()?.get(loc.row)
    }

    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let loc = self.location(entity).ok()?;
        self.archetypes.get_mut(loc.archetype)?.column_mut::<T>()?.get_mut(loc.row)
    }

    /// Every entity that has a `T`, with the component.
    pub fn query<T: Component>(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.archetypes.iter().filter_map(|a| a.column::<T>().map(|c| a.entities.iter().copied().zip(c.iter()))).flatten()
    }

    /// Every entity that has a `T`, with mutable access to the component.
    pub fn query_mut<T: Component>(&mut self) -> impl Iterator<Item = (Entity, &mut T)> {
        self.archetypes
            .iter_mut()
            .filter_map(|a| {
                let entities = &a.entities;
                a.columns
                    .get_mut(&TypeId::of::<T>())
                    .and_then(|c| c.as_any_mut().downcast_mut::<TypedColumn<T>>())
                    .map(|c| entities.iter().copied().zip(c.0.iter_mut()))
            })
            .flatten()
    }

    fn location(&self, entity: Entity) -> Result<Location, EcsError> {
        self.slots.get(entity.index as usize).filter(|s| s.generation == entity.generation).and_then(|s| s.location).ok_or(EcsError::NoSuchEntity(entity))
    }

    fn set_location(&mut self, entity: Entity, location: Option<Location>) {
        if let Some(slot) = self.slots.get_mut(entity.index as usize) {
            slot.location = location;
        }
    }

    fn types_of(&self, archetype: usize) -> Vec<TypeId> {
        let mut types: Vec<TypeId> = self.archetypes.get(archetype).map(|a| a.columns.keys().copied().collect()).unwrap_or_default();
        types.sort_unstable();
        types
    }

    /// Finds or creates the archetype for `types`, building its columns from `template` plus `extra`,
    /// minus `without`.
    fn archetype_for(&mut self, types: Vec<TypeId>, template: usize, extra: Option<(TypeId, Box<dyn Column>)>, without: Option<TypeId>) -> usize {
        if let Some(&index) = self.by_types.get(&types) {
            return index;
        }
        let mut columns: HashMap<TypeId, Box<dyn Column>> = HashMap::new();
        if let Some(src) = self.archetypes.get(template) {
            for (type_id, column) in &src.columns {
                if Some(*type_id) != without {
                    columns.insert(*type_id, column.empty_like());
                }
            }
        }
        if let Some((type_id, column)) = extra {
            columns.insert(type_id, column);
        }
        self.archetypes.push(Archetype { columns, entities: Vec::new() });
        let index = self.archetypes.len() - 1;
        self.by_types.insert(types, index);
        index
    }

    /// Moves every column value of `entity` except `skip` from its archetype to `dst`.
    fn move_entity(&mut self, entity: Entity, loc: Location, dst: usize, skip: Option<TypeId>) {
        if let Some((src, dst_arch)) = two_mut(&mut self.archetypes, loc.archetype, dst) {
            for (type_id, column) in &mut src.columns {
                if Some(*type_id) == skip {
                    continue;
                }
                if let Some(dst_column) = dst_arch.columns.get_mut(type_id) {
                    column.move_row(loc.row, dst_column.as_mut());
                }
            }
            dst_arch.entities.push(entity);
        }
        let new_row = self.archetypes.get(dst).map_or(0, |a| a.entities.len().saturating_sub(1));
        self.remove_entity_row(loc);
        self.set_location(entity, Some(Location { archetype: dst, row: new_row }));
    }

    /// Removes the entity id at `loc` and fixes the location of the entity swapped into its row.
    fn remove_entity_row(&mut self, loc: Location) {
        let moved = self.archetypes.get_mut(loc.archetype).and_then(|a| {
            if loc.row < a.entities.len() {
                a.entities.swap_remove(loc.row);
            }
            a.entities.get(loc.row).copied()
        });
        if let Some(moved) = moved {
            self.set_location(moved, Some(loc));
        }
    }
}

fn two_mut<T>(items: &mut [T], a: usize, b: usize) -> Option<(&mut T, &mut T)> {
    if a == b {
        return None;
    }
    if a < b {
        let (left, right) = items.split_at_mut(b);
        Some((left.get_mut(a)?, right.first_mut()?))
    } else {
        let (left, right) = items.split_at_mut(a);
        let b_ref = left.get_mut(b)?;
        Some((right.first_mut()?, b_ref))
    }
}

#[cfg(test)]
mod tests;
