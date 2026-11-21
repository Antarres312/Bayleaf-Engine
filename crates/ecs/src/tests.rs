use crate::{Schedule, World};

#[derive(Debug, PartialEq, Clone)]
struct Position(f32);
#[derive(Debug, PartialEq, Clone)]
struct Velocity(f32);
#[derive(Debug, PartialEq, Clone)]
struct Label(&'static str);

#[test]
fn adding_a_component_keeps_the_others() {
    let mut world = World::new();
    let e = world.spawn().unwrap();
    world.insert(e, Position(1.0)).unwrap();
    world.insert(e, Velocity(2.0)).unwrap();
    world.insert(e, Label("a")).unwrap();
    assert_eq!(world.get::<Position>(e), Some(&Position(1.0)));
    assert_eq!(world.get::<Velocity>(e), Some(&Velocity(2.0)));
    assert_eq!(world.get::<Label>(e), Some(&Label("a")));
}

#[test]
fn removing_a_component_returns_it_and_keeps_the_others() {
    let mut world = World::new();
    let e = world.spawn().unwrap();
    world.insert(e, Position(1.0)).unwrap();
    world.insert(e, Velocity(2.0)).unwrap();
    assert_eq!(world.remove::<Position>(e).unwrap(), Some(Position(1.0)));
    assert!(!world.has::<Position>(e));
    assert_eq!(world.get::<Velocity>(e), Some(&Velocity(2.0)));
    assert_eq!(world.remove::<Position>(e).unwrap(), None);
}

#[test]
fn moving_one_entity_does_not_disturb_its_neighbours() {
    let mut world = World::new();
    let entities: Vec<_> = (0..4).map(|_| world.spawn().unwrap()).collect();
    for (i, e) in entities.iter().enumerate() {
        world.insert(*e, Position(i as f32)).unwrap();
    }
    // Moving the first entity to another archetype swaps the last row into its place.
    world.insert(entities[0], Velocity(9.0)).unwrap();
    world.despawn(entities[1]).unwrap();
    for (i, e) in entities.iter().enumerate().filter(|(i, _)| *i != 1) {
        assert_eq!(world.get::<Position>(*e), Some(&Position(i as f32)), "entity {i}");
    }
}

#[test]
fn a_despawned_handle_stays_dead_after_its_slot_is_reused() {
    let mut world = World::new();
    let old = world.spawn().unwrap();
    world.insert(old, Position(1.0)).unwrap();
    world.despawn(old).unwrap();
    let new = world.spawn().unwrap();
    assert!(!world.is_alive(old));
    assert!(world.is_alive(new));
    assert!(world.get::<Position>(old).is_none());
    assert!(world.insert(old, Position(2.0)).is_err());
}

#[test]
fn query_visits_exactly_the_matching_entities() {
    let mut world = World::new();
    let a = world.spawn().unwrap();
    let b = world.spawn().unwrap();
    let c = world.spawn().unwrap();
    world.insert(a, Position(1.0)).unwrap();
    world.insert(b, Position(2.0)).unwrap();
    world.insert(b, Velocity(1.0)).unwrap();
    world.insert(c, Velocity(1.0)).unwrap();
    let mut found: Vec<_> = world.query::<Position>().map(|(e, _)| e).collect();
    found.sort();
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(found, expected);
}

#[test]
fn schedule_runs_systems_in_order() {
    let mut world = World::new();
    let e = world.spawn().unwrap();
    world.insert(e, Position(0.0)).unwrap();
    world.insert(e, Velocity(2.0)).unwrap();
    let mut schedule = Schedule::new();
    schedule.add("double velocity", |w: &mut World| {
        for (_, v) in w.query_mut::<Velocity>() {
            v.0 *= 2.0;
        }
    });
    schedule.add("integrate", |w: &mut World| {
        let moves: Vec<_> = w.query::<Velocity>().map(|(e, v)| (e, v.0)).collect();
        for (e, dv) in moves {
            if let Some(p) = w.get_mut::<Position>(e) {
                p.0 += dv;
            }
        }
    });
    schedule.run(&mut world);
    // Integration sees the doubled velocity only if it runs second.
    assert!(world.get::<Position>(e).unwrap().0 > 2.0);
}
