//! The game: the benchmark's three scripts and its scene, written against `runtime.rs` the way game
//! code would be (bench/runtime/cpp/game.cpp mirrors this file). Each script runs in both styles
//! with the same body: in an array of its type (typed) and boxed behind a dynamic call (virtual).

use crate::runtime::{Collider, Entity, Frame, Id, Me, Mesh, PointLight, Runtime, Script};

/// Rotation Y += speed x dt.
struct Spin {
    speed: f32,
}
impl Script for Spin {
    fn update(&mut self, me: &mut Me, f: &Frame) {
        let y = &mut me.transform_mut().rotation[1];
        *y = (*y + self.speed * f.dt) % 360.0;
    }
}

/// Position Y = base + height x sin(time x speed).
struct Bob {
    speed: f32,
    height: f32,
    base: f32,
}
impl Script for Bob {
    fn update(&mut self, me: &mut Me, f: &Frame) {
        // Positions are kept in double, set from single precision as Bayleaf's scripts set them.
        me.transform_mut().position[1] = f64::from(self.base + self.height * (f.time as f32 * self.speed).sin());
    }
}

/// Position X and Z ease towards the object named Target, found at start.
struct Follow {
    rate: f32,
    target: Entity,
}
impl Script for Follow {
    fn start(&mut self, me: &mut Me) {
        self.target = me.world.find("Target");
    }

    fn update(&mut self, me: &mut Me, f: &Frame) {
        if !me.world.alive(self.target) {
            return;
        }
        let k = (self.rate * f.dt).min(1.0);
        let goal = me.world.transform(self.target).position;
        let p = &mut me.transform_mut().position;
        for axis in [0, 2] {
            let here = p[axis] as f32;
            p[axis] = f64::from(here + (goal[axis] as f32 - here) * k);
        }
    }
}

// Per-object property values: the same formulas as bench/src/main.rs.
fn spin_speed(i: usize) -> f32 {
    30.0 + (i % 7) as f32 * 10.0
}
fn bob_speed(i: usize) -> f32 {
    1.0 + (i % 5) as f32 * 0.25
}
fn bob_height(i: usize) -> f32 {
    0.25 + (i % 3) as f32 * 0.1
}
fn follow_rate(i: usize) -> f32 {
    0.05 + (i % 4) as f32 * 0.05
}

fn attach<T: Script>(runtime: &mut Runtime, e: Entity, script: T, typed: bool) {
    if typed {
        runtime.add(e, script);
    } else {
        runtime.add_virtual(e, Box::new(script));
    }
}

/// The scene of bench/src/main.rs: the target first, then a grid of objects, each with a mesh, a
/// collider, a tag, a light on every tenth, and the three scripts.
pub fn build(runtime: &mut Runtime, n: usize, typed: bool) {
    runtime.world.create(Id { hi: 0, lo: 1 }, "Target".to_owned());
    let side = ((n as f64).sqrt().ceil() as usize).max(1);
    for i in 0..n {
        let id = Id { hi: 0x9e37_79b9_7f4a_7c15_u64.wrapping_mul(i as u64 + 1), lo: i as u64 + 2 };
        let e = runtime.world.create(id, format!("Object {i}"));
        let t = runtime.world.transform_mut(e);
        t.position[0] = (i % side) as f64 * 2.0 - side as f64;
        t.position[2] = (i / side) as f64 * 2.0 - side as f64;
        runtime.world.add_mesh(e, Mesh::default());
        runtime.world.add_collider(e, Collider::default());
        runtime.world.tag(e, "bench".to_owned());
        if i % 10 == 0 {
            runtime.world.add_light(e, PointLight::default());
        }
        attach(runtime, e, Spin { speed: spin_speed(i) }, typed);
        attach(runtime, e, Bob { speed: bob_speed(i), height: bob_height(i), base: 0.5 }, typed);
        attach(runtime, e, Follow { rate: follow_rate(i), target: Entity::default() }, typed);
    }
}
