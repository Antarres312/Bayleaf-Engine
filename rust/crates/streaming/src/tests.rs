use std::time::Duration;

use bayleaf_core::StableId;
use bayleaf_core::math::DVec3;
use bayleaf_scene::Scene;

use super::{Area, State, Streamer};

fn area(id: StableId, x: f64, scene: &str) -> Area {
    Area { id, at: DVec3::new(x, 0.0, 0.0), scene: scene.to_owned(), load_distance: 50.0, keep_margin: 10.0 }
}

#[test]
fn an_area_comes_within_its_distance_and_goes_only_past_the_margin() {
    let mut streamer = Streamer::new();
    let id = StableId::random().unwrap();
    let areas = [area(id, 100.0, "cell.bscene")];
    let at = |x| [DVec3::new(x, 0.0, 0.0)];

    assert!(streamer.plan(&areas, &at(0.0)).read.is_empty(), "too far to come");
    assert_eq!(streamer.plan(&areas, &at(60.0)).read.len(), 1, "within its distance");
    assert!(streamer.plan(&areas, &at(60.0)).read.is_empty(), "asked for once");
    streamer.placed(id, vec![StableId::random().unwrap()]);
    let edge = streamer.plan(&areas, &at(45.0));
    assert!(edge.remove.is_empty(), "inside the margin it stays");
    let away = streamer.plan(&areas, &at(30.0));
    assert_eq!(away.remove.len(), 1, "past the margin it goes");
    assert_eq!(away.remove[0].1.len(), 1, "with the objects it became");
    assert!(streamer.plan(&areas, &[]).read.is_empty(), "no camera, no change");
}

#[test]
fn a_changed_or_removed_area_goes() {
    let mut streamer = Streamer::new();
    let id = StableId::random().unwrap();
    let camera = [DVec3::ZERO];
    streamer.plan(&[area(id, 0.0, "a.bscene")], &camera);
    let changed = streamer.plan(&[area(id, 0.0, "b.bscene")], &camera);
    assert_eq!(changed.remove.len(), 1, "the old scene goes");
    assert_eq!(changed.read.len(), 1, "the new one comes");
    assert_eq!(streamer.plan(&[], &camera).remove.len(), 1, "the component went");
}

#[test]
fn scenes_are_read_in_the_background_and_failures_are_kept() {
    let dir = std::env::temp_dir().join(format!("bayleaf-streaming-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cell = Scene::new();
    cell.create_object(None, "Rock").unwrap();
    cell.save(&dir.join("cell.bscene")).unwrap();

    let mut streamer = Streamer::new();
    let (good, bad) = (StableId::random().unwrap(), StableId::random().unwrap());
    let areas = [area(good, 0.0, "cell.bscene"), area(bad, 0.0, "missing.bscene")];
    for (id, scene) in streamer.plan(&areas, &[DVec3::ZERO]).read {
        streamer.read(id, dir.join(scene));
    }
    let arrived = streamer.wait(Duration::from_secs(10));
    assert_eq!(arrived.len(), 2);
    let read = arrived.iter().find(|(id, _)| *id == good).and_then(|(_, r)| r.as_ref().ok()).expect("the cell arrived");
    assert_eq!(read.object_count(), 1);
    streamer.placed(good, read.children(None).to_vec());
    assert_eq!(streamer.busy(), 0);
    let failed = streamer.states().find(|(id, _, _)| *id == bad).map(|(_, _, s)| s.clone());
    assert!(matches!(failed, Some(State::Failed(_))), "the missing file is reported: {failed:?}");
    assert!(streamer.plan(&areas, &[DVec3::ZERO]).read.is_empty(), "not read again every step");
    std::fs::remove_dir_all(&dir).ok();
}
