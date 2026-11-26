//! Systems with declared access, views and the parallel schedule.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{Access, Component, Entity, Schedule, View, World};

#[derive(Debug, Clone)]
struct Position(f32);
#[derive(Debug, Clone)]
struct Velocity(f32);
#[derive(Debug, Clone)]
struct Angle(f32);
#[derive(Debug, Clone)]
struct Spin(f32);
#[derive(Debug, Clone)]
struct Health(f32);
#[derive(Debug, Clone)]
struct Regen(f32);
#[derive(Debug, Clone)]
struct Lit;

const DT: f32 = 1.0 / 60.0;

/// Entities spread over several archetypes, more of them than one job of a parallel query takes.
fn populate(world: &mut World, n: usize) {
    for i in 0..n {
        let e = world.spawn().unwrap();
        world.insert(e, Position(i as f32)).unwrap();
        world.insert(e, Velocity((i % 7) as f32 * 0.5)).unwrap();
        if i % 3 == 0 {
            world.insert(e, Angle(0.0)).unwrap();
            world.insert(e, Spin(i as f32 * 0.01)).unwrap();
        }
        if i % 2 == 0 {
            world.insert(e, Health(i as f32 % 90.0)).unwrap();
            world.insert(e, Regen(0.5)).unwrap();
        }
        if i % 10 == 0 {
            world.insert(e, Lit).unwrap();
        }
    }
}

/// The structural step both schedules share: one entity more, one light less.
fn restructure(w: &mut World) {
    if let Ok(e) = w.spawn() {
        let _ = w.insert(e, Position(-1.0));
        let _ = w.insert(e, Velocity(3.0));
    }
    if let Some(first) = w.query::<Lit>().map(|(e, _)| e).min() {
        let _ = w.remove::<Lit>(first);
    }
}

fn view_schedule() -> Schedule {
    let mut s = Schedule::new();
    s.add_system("move", Access::new().write::<Position>().read::<Velocity>(), |v: &mut View<'_>| {
        v.for_each_mut_with::<Position, Velocity>(|_, p, vel| p.0 += vel.0 * DT)
    });
    s.add_system("spin", Access::new().write::<Angle>().read::<Spin>(), |v: &mut View<'_>| {
        v.for_each_mut_with::<Angle, Spin>(|_, a, s| a.0 = (a.0 + s.0).sin())
    });
    s.add_system("heal", Access::new().write::<Health>().read::<Regen>(), |v: &mut View<'_>| {
        v.for_each_mut_with::<Health, Regen>(|_, h, r| h.0 = (h.0 + r.0).min(100.0))
    });
    s.add_system("drag", Access::new().write::<Velocity>(), |v: &mut View<'_>| v.for_each_mut::<Velocity>(|_, vel| vel.0 *= 0.99));
    s.add_system("follow", Access::new().write::<Angle>().read::<Position>(), |v: &mut View<'_>| {
        v.for_each_mut_with::<Angle, Position>(|_, a, p| a.0 = a.0 * 0.5 + p.0 * 0.001)
    });
    s.add("restructure", restructure);
    s.add_system("move back", Access::new().write::<Position>().read::<Velocity>(), |v: &mut View<'_>| {
        let moves: Vec<(Entity, f32)> = v.query::<Velocity>().map(|(e, vel)| (e, vel.0)).collect();
        for (e, dv) in moves {
            if let Some(p) = v.get_mut::<Position>(e) {
                p.0 -= dv * DT * 0.5;
            }
        }
    });
    s
}

/// `f` on every entity with an `A` and a `B`, through the whole world.
fn each<A: Component, B: Component + Clone>(w: &mut World, f: impl Fn(&mut A, &B)) {
    let rows: Vec<(Entity, B)> = w.query::<B>().map(|(e, b)| (e, b.clone())).collect();
    for (e, b) in rows {
        if let Some(a) = w.get_mut::<A>(e) {
            f(a, &b);
        }
    }
}

/// The same systems as [`view_schedule`] on the whole world, one after another.
fn reference_schedule() -> Schedule {
    let mut s = Schedule::new();
    s.add("move", |w: &mut World| each::<Position, Velocity>(w, |p, vel| p.0 += vel.0 * DT));
    s.add("spin", |w: &mut World| each::<Angle, Spin>(w, |a, s| a.0 = (a.0 + s.0).sin()));
    s.add("heal", |w: &mut World| each::<Health, Regen>(w, |h, r| h.0 = (h.0 + r.0).min(100.0)));
    s.add("drag", |w: &mut World| {
        for (_, vel) in w.query_mut::<Velocity>() {
            vel.0 *= 0.99;
        }
    });
    s.add("follow", |w: &mut World| each::<Angle, Position>(w, |a, p| a.0 = a.0 * 0.5 + p.0 * 0.001));
    s.add("restructure", restructure);
    s.add("move back", |w: &mut World| each::<Position, Velocity>(w, |p, vel| p.0 -= vel.0 * DT * 0.5));
    s
}

/// Every component's exact bits, by entity.
fn snapshot(w: &World) -> Vec<Vec<(Entity, u32)>> {
    fn bits<T: Component>(w: &World, f: impl Fn(&T) -> f32) -> Vec<(Entity, u32)> {
        let mut v: Vec<_> = w.query::<T>().map(|(e, c)| (e, f(c).to_bits())).collect();
        v.sort();
        v
    }
    vec![
        bits::<Position>(w, |c| c.0),
        bits::<Velocity>(w, |c| c.0),
        bits::<Angle>(w, |c| c.0),
        bits::<Spin>(w, |c| c.0),
        bits::<Health>(w, |c| c.0),
        bits::<Lit>(w, |_| 1.0),
    ]
}

#[test]
fn a_parallel_schedule_ends_with_the_same_world_as_one_thread() {
    let mut worlds: Vec<World> = (0..3).map(|_| World::new()).collect();
    for w in &mut worlds {
        populate(w, 3000);
    }
    assert_eq!(worlds[2].set_parallel(true), World::PARALLEL_AVAILABLE);
    let mut schedules = [reference_schedule(), view_schedule(), view_schedule()];
    for _ in 0..5 {
        for (s, w) in schedules.iter_mut().zip(&mut worlds) {
            s.run(w);
        }
    }
    let reference = snapshot(&worlds[0]);
    assert!(reference.iter().all(|c| !c.is_empty()));
    assert_eq!(snapshot(&worlds[1]), reference, "views on one thread");
    assert_eq!(snapshot(&worlds[2]), reference, "views in parallel");
}

#[test]
fn systems_join_the_earliest_batch_after_what_they_conflict_with() {
    let mut s = Schedule::new();
    let none = |_: &mut View<'_>| {};
    s.add_system("a", Access::new().write::<Position>().read::<Velocity>(), none);
    s.add_system("b", Access::new().write::<Angle>().read::<Spin>(), none);
    s.add_system("c", Access::new().read::<Position>().write::<Health>(), none);
    s.add("d", |_: &mut World| {});
    s.add_system("e", Access::new().write::<Velocity>(), none);
    s.add_system("f", Access::new().write::<Regen>(), none);
    s.add_system("g", Access::new().read::<Velocity>().write::<Spin>(), none);
    s.add_system("h", Access::new().read::<Velocity>().write::<Angle>(), none);
    assert_eq!(s.batches(), vec![vec!["a", "b"], vec!["c"], vec!["d"], vec!["e", "f"], vec!["g", "h"]]);
}

#[test]
fn conflicting_systems_never_run_together() {
    let mut world = World::new();
    populate(&mut world, 2000);
    world.set_parallel(true);
    let writers = Arc::new(AtomicUsize::new(0));
    let readers = Arc::new(AtomicUsize::new(0));
    let overlaps = Arc::new(AtomicUsize::new(0));
    let mut s = Schedule::new();
    // Each writer is followed by readers, which share a batch and so run at the same time.
    for k in 0..3 {
        let (w, r, o) = (writers.clone(), readers.clone(), overlaps.clone());
        s.add_system(format!("writer {k}"), Access::new().write::<Position>(), move |v: &mut View<'_>| {
            if w.fetch_add(1, Ordering::SeqCst) > 0 || r.load(Ordering::SeqCst) > 0 {
                o.fetch_add(1, Ordering::SeqCst);
            }
            v.for_each_mut::<Position>(|_, p| p.0 += 1.0);
            std::thread::sleep(std::time::Duration::from_millis(2));
            w.fetch_sub(1, Ordering::SeqCst);
        });
        for j in 0..3 {
            let (w, r, o) = (writers.clone(), readers.clone(), overlaps.clone());
            s.add_system(format!("reader {k}.{j}"), Access::new().read::<Position>(), move |v: &mut View<'_>| {
                r.fetch_add(1, Ordering::SeqCst);
                if w.load(Ordering::SeqCst) > 0 {
                    o.fetch_add(1, Ordering::SeqCst);
                }
                let _sum: f32 = v.query::<Position>().map(|(_, p)| p.0).sum();
                std::thread::sleep(std::time::Duration::from_millis(2));
                r.fetch_sub(1, Ordering::SeqCst);
            });
        }
    }
    assert_eq!(s.batches().len(), 6, "writer, readers, writer, readers, ...");
    for _ in 0..3 {
        s.run(&mut world);
    }
    assert_eq!(overlaps.load(Ordering::SeqCst), 0);
}

#[test]
fn a_view_sees_only_what_its_access_names() {
    let mut world = World::new();
    let e = world.spawn().unwrap();
    world.insert(e, Position(1.0)).unwrap();
    world.insert(e, Velocity(2.0)).unwrap();
    let mut view = world.view(&Access::new().read::<Position>()).unwrap();
    assert!(view.get::<Position>(e).is_some());
    assert!(view.get_mut::<Position>(e).is_none(), "read-only");
    assert!(view.get::<Velocity>(e).is_none(), "not named");
    assert_eq!(view.query_mut::<Position>().count(), 0);
}
