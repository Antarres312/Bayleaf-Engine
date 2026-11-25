//! Which component types a system reads and writes.

use std::any::TypeId;

use crate::Component;

/// The component types a system reads and writes. Two systems conflict when one writes a type the
/// other reads or writes; systems that do not conflict may run at the same time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Access {
    reads: Vec<(TypeId, &'static str)>,
    writes: Vec<(TypeId, &'static str)>,
}

impl Access {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds read access to `T`. A type that is also written counts as written.
    pub fn read<T: Component>(mut self) -> Self {
        let id = TypeId::of::<T>();
        if !self.reads.iter().any(|(t, _)| *t == id) {
            self.reads.push((id, std::any::type_name::<T>()));
        }
        self
    }

    /// Adds write access to `T`, which includes reading it.
    pub fn write<T: Component>(mut self) -> Self {
        let id = TypeId::of::<T>();
        if !self.writes.iter().any(|(t, _)| *t == id) {
            self.writes.push((id, std::any::type_name::<T>()));
        }
        self
    }

    pub fn reads(&self, type_id: TypeId) -> bool {
        self.reads.iter().any(|(t, _)| *t == type_id) || self.writes(type_id)
    }

    pub fn writes(&self, type_id: TypeId) -> bool {
        self.writes.iter().any(|(t, _)| *t == type_id)
    }

    /// Whether this and `other` must not run at the same time.
    pub fn conflicts_with(&self, other: &Access) -> bool {
        self.writes.iter().any(|(t, _)| other.reads(*t)) || other.writes.iter().any(|(t, _)| self.reads(*t))
    }
}
