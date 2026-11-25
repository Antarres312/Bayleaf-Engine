//! A small game runtime for the comparison in bench/README.md ("The same runtime in C++ and Rust"):
//! the design a fast Bayleaf runtime would have, written the same way in both languages
//! (bench/runtime/cpp/runtime.hpp mirrors this file).
//!
//! - Components in packed arrays, an object is an index with a generation.
//! - Objects are found by a 128-bit id or a name through hash maps, at start, not per step.
//! - Writing a transform marks the object moved, once per step; world matrices are worked out for
//!   the moved objects only, and the list of them is what a renderer or an editor would read.
//! - Game scripts are plain structs kept in one array per script type; the runtime makes one dynamic
//!   call per type per step ("typed"). For comparison, the usual boxed object per script behind a
//!   dynamic update per object ("virtual") runs on the same world.

use std::any::{Any, TypeId};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Id {
    pub hi: u64,
    pub lo: u64,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct Transform {
    pub position: [f64; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}
impl Default for Transform {
    fn default() -> Self {
        Self { position: [0.0; 3], rotation: [0.0; 3], scale: [1.0; 3] }
    }
}
#[derive(Clone, Copy)]
pub struct Mesh {
    pub shape: i32,
    pub color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
}
impl Default for Mesh {
    fn default() -> Self {
        Self { shape: 0, color: [0.8, 0.8, 0.8, 1.0], metallic: 0.0, roughness: 0.5 }
    }
}
#[derive(Clone, Copy)]
pub struct Collider {
    pub shape: i32,
    pub size: [f32; 3],
    pub friction: f32,
    pub restitution: f32,
    pub trigger: bool,
}
impl Default for Collider {
    fn default() -> Self {
        Self { shape: 0, size: [1.0; 3], friction: 0.5, restitution: 0.0, trigger: false }
    }
}
#[derive(Clone, Copy)]
pub struct PointLight {
    pub color: [f32; 3],
    pub intensity: f32,
    pub range: f32,
}
impl Default for PointLight {
    fn default() -> Self {
        Self { color: [1.0; 3], intensity: 1.0, range: 10.0 }
    }
}

/// An object: its place in the arrays, and the generation that place had when the handle was made.
#[derive(Clone, Copy, Default)]
pub struct Entity {
    pub index: u32,
    pub generation: u32,
}

#[derive(Clone, Copy)]
pub struct Frame {
    pub dt: f32,
    pub time: f64,
}

/// Translation * rotation (X, then Y, then Z, degrees) * scale, column-major (as bench/cpp/bench.cpp).
fn world_matrix(t: &Transform, m: &mut [f64; 16]) {
    let rad = std::f64::consts::PI / 180.0;
    let (ax, ay, az) = (f64::from(t.rotation[0]) * rad, f64::from(t.rotation[1]) * rad, f64::from(t.rotation[2]) * rad);
    let (cx, sx, cy, sy, cz, sz) = (ax.cos(), ax.sin(), ay.cos(), ay.sin(), az.cos(), az.sin());
    let r = [cy * cz, cy * sz, -sy, sx * sy * cz - cx * sz, sx * sy * sz + cx * cz, sx * cy, cx * sy * cz + sx * sz, cx * sy * sz - sx * cz, cx * cy];
    for c in 0..3 {
        for row in 0..3 {
            m[c * 4 + row] = r[c * 3 + row] * f64::from(t.scale[c]);
        }
        m[c * 4 + 3] = 0.0;
    }
    m[12] = t.position[0];
    m[13] = t.position[1];
    m[14] = t.position[2];
    m[15] = 1.0;
}

#[derive(Default)]
pub struct World {
    ids: Vec<Id>,
    names: Vec<String>,
    transforms: Vec<Transform>,
    world: Vec<[f64; 16]>,
    generation: Vec<u32>,
    moved_flag: Vec<bool>,
    moved: Vec<u32>,
    meshes: Vec<(u32, Mesh)>,
    colliders: Vec<(u32, Collider)>,
    lights: Vec<(u32, PointLight)>,
    tags: Vec<(u32, String)>,
    by_id: HashMap<Id, u32>,
    by_name: HashMap<String, u32>,
}

impl World {
    pub fn create(&mut self, id: Id, name: String) -> Entity {
        let index = self.transforms.len() as u32;
        self.by_id.entry(id).or_insert(index);
        self.by_name.entry(name.clone()).or_insert(index);
        self.ids.push(id);
        self.names.push(name);
        self.transforms.push(Transform::default());
        self.world.push([0.0; 16]);
        self.generation.push(1);
        self.moved_flag.push(false);
        self.mark(index);
        Entity { index, generation: 1 }
    }

    pub fn len(&self) -> usize {
        self.transforms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.transforms.is_empty()
    }

    pub fn alive(&self, e: Entity) -> bool {
        self.generation.get(e.index as usize) == Some(&e.generation)
    }

    /// The first object of that name; generation 0 (never alive) when there is none.
    pub fn find(&self, name: &str) -> Entity {
        self.by_name.get(name).map_or(Entity::default(), |i| Entity { index: *i, generation: self.generation[*i as usize] })
    }

    pub fn find_id(&self, id: Id) -> Entity {
        self.by_id.get(&id).map_or(Entity::default(), |i| Entity { index: *i, generation: self.generation[*i as usize] })
    }

    pub fn transform(&self, e: Entity) -> &Transform {
        &self.transforms[e.index as usize]
    }

    /// Where the transforms start in memory (48 bytes each, in object order).
    pub fn transforms_at(&self) -> usize {
        self.transforms.as_ptr() as usize
    }

    /// For writing: the object counts as moved this step.
    pub fn transform_mut(&mut self, e: Entity) -> &mut Transform {
        self.mark(e.index);
        &mut self.transforms[e.index as usize]
    }

    pub fn world_of(&self, e: Entity) -> &[f64; 16] {
        &self.world[e.index as usize]
    }

    pub fn add_mesh(&mut self, e: Entity, c: Mesh) {
        self.meshes.push((e.index, c));
    }

    pub fn add_collider(&mut self, e: Entity, c: Collider) {
        self.colliders.push((e.index, c));
    }

    pub fn add_light(&mut self, e: Entity, c: PointLight) {
        self.lights.push((e.index, c));
    }

    pub fn tag(&mut self, e: Entity, tag: String) {
        self.tags.push((e.index, tag));
    }

    /// World matrices of the objects moved since the last call. Returns what a consumer of the moved
    /// list (a renderer, an editor's viewport) would take: here, the sum of their world x positions.
    pub fn finish_step(&mut self) -> f64 {
        let mut taken = 0.0;
        for &index in &self.moved {
            let i = index as usize;
            world_matrix(&self.transforms[i], &mut self.world[i]);
            taken += self.world[i][12];
            self.moved_flag[i] = false;
        }
        self.moved.clear();
        taken
    }

    /// Only forgets what moved (for timing the scripts alone).
    pub fn forget_moved(&mut self) {
        for &index in &self.moved {
            self.moved_flag[index as usize] = false;
        }
        self.moved.clear();
    }

    pub fn checksum(&self) -> f64 {
        self.transforms.iter().zip(&self.world).map(|(t, w)| t.position[0] + t.position[1] + t.position[2] + f64::from(t.rotation[1]) + w[0]).sum()
    }

    /// A sum over every component that only holds data, and the target found by its id: equal across
    /// languages when the scenes are the same.
    pub fn data_sum(&self) -> f64 {
        let mut sum = 0.0;
        for (_, m) in &self.meshes {
            sum += f64::from(m.shape) + f64::from(m.color.iter().sum::<f32>() + m.metallic + m.roughness);
        }
        for (_, c) in &self.colliders {
            sum += f64::from(c.shape) + f64::from(c.size.iter().sum::<f32>() + c.friction + c.restitution) + f64::from(u8::from(c.trigger));
        }
        for (_, l) in &self.lights {
            sum += f64::from(l.color.iter().sum::<f32>() + l.intensity + l.range);
        }
        for (_, tag) in &self.tags {
            sum += tag.len() as f64;
        }
        sum + self.world_of(self.find_id(Id { hi: 0, lo: 1 }))[15]
    }

    fn mark(&mut self, index: u32) {
        let flag = &mut self.moved_flag[index as usize];
        if !*flag {
            *flag = true;
            self.moved.push(index);
        }
    }
}

/// What a script works with: the world and its own object.
pub struct Me<'a> {
    pub world: &'a mut World,
    pub entity: Entity,
}

impl Me<'_> {
    pub fn transform_mut(&mut self) -> &mut Transform {
        self.world.transform_mut(self.entity)
    }
}

/// A game script. As `Box<dyn Script>` it is the "virtual" style; in a [`Typed`] array, the "typed".
pub trait Script: 'static {
    fn start(&mut self, _me: &mut Me) {}
    fn update(&mut self, me: &mut Me, frame: &Frame);
}

/// "typed": every script of one type in one array; one dynamic call per type per step.
trait Store {
    fn start_all(&mut self, world: &mut World);
    fn update_all(&mut self, world: &mut World, frame: &Frame);
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

struct Typed<T> {
    owners: Vec<Entity>,
    items: Vec<T>,
}

impl<T: Script> Store for Typed<T> {
    fn start_all(&mut self, world: &mut World) {
        for (owner, item) in self.owners.iter().zip(&mut self.items) {
            item.start(&mut Me { world, entity: *owner });
        }
    }

    fn update_all(&mut self, world: &mut World, frame: &Frame) {
        for (owner, item) in self.owners.iter().zip(&mut self.items) {
            item.update(&mut Me { world, entity: *owner }, frame);
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Default)]
pub struct Runtime {
    pub world: World,
    stores: Vec<Box<dyn Store>>,
    slots: HashMap<TypeId, usize>,
    owned: Vec<Vec<Box<dyn Script>>>,
}

impl Runtime {
    /// Typed: the script joins the array of its type.
    pub fn add<T: Script>(&mut self, owner: Entity, script: T) {
        let slot = *self.slots.entry(TypeId::of::<T>()).or_insert_with(|| {
            self.stores.push(Box::new(Typed::<T> { owners: Vec::new(), items: Vec::new() }));
            self.stores.len() - 1
        });
        if let Some(store) = self.stores[slot].as_any_mut().downcast_mut::<Typed<T>>() {
            store.owners.push(owner);
            store.items.push(script);
        }
    }

    /// Virtual: the object owns its scripts.
    pub fn add_virtual(&mut self, owner: Entity, script: Box<dyn Script>) {
        let index = owner.index as usize;
        if index >= self.owned.len() {
            self.owned.resize_with(index + 1, Vec::new);
        }
        self.owned[index].push(script);
    }

    pub fn start(&mut self) {
        for store in &mut self.stores {
            store.start_all(&mut self.world);
        }
        for (i, scripts) in self.owned.iter_mut().enumerate() {
            let mut me = Me { world: &mut self.world, entity: Entity { index: i as u32, generation: 1 } };
            for script in scripts {
                script.start(&mut me);
            }
        }
    }

    pub fn update(&mut self, frame: &Frame) {
        for store in &mut self.stores {
            store.update_all(&mut self.world, frame);
        }
        for (i, scripts) in self.owned.iter_mut().enumerate() {
            let mut me = Me { world: &mut self.world, entity: Entity { index: i as u32, generation: 1 } };
            for script in scripts {
                script.update(&mut me, frame);
            }
        }
    }
}
