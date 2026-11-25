use std::sync::Arc;

use bayleaf_assets::skeleton::Skeleton;
use bayleaf_core::math::Vec3;

use crate::{ClipEvent, Clips, Motion, Number, Parameter, Player};

/// The generated arm model (`bayleaf_assets::fixtures`): Root, Arm under it, Hand under Arm.
fn arm(events: &[(String, ClipEvent)]) -> (Arc<Skeleton>, Clips) {
    let dir = std::env::temp_dir().join(format!("bayleaf-animation-{}-{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(&dir).unwrap();
    let model = bayleaf_assets::load_model(&bayleaf_assets::fixtures::skinned_arm(&dir).unwrap()).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let skeleton = model.skeleton.unwrap();
    let clips = Clips::of_model(&skeleton, events);
    (skeleton, clips)
}

fn rand_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn once(clip: &str) -> Motion {
    Motion::Clip { clip: clip.to_owned(), speed: 1.0, looping: false }
}

/// Where the player puts the Arm joint now (its place against Root, which stays at rest).
fn arm_at(player: &Player, skeleton: &Skeleton, clips: &Clips) -> Vec3 {
    player.pose(skeleton, clips).locals[1].translation
}

#[test]
fn a_fade_carries_one_clip_into_the_next_over_its_time() {
    let (skeleton, clips) = arm(&[]);
    let mut player = Player::new();
    player.play("", once("Stretch"), 0.0).unwrap();
    player.advance(&skeleton, &clips, 1.0);
    let stretched = arm_at(&player, &skeleton, &clips);
    assert!(stretched.x > 1.4, "Stretch at its end: {stretched}");
    player.play("", once("Raise"), 1.0).unwrap();
    player.advance(&skeleton, &clips, 0.5);
    let halfway = arm_at(&player, &skeleton, &clips);
    assert!(halfway.x > 0.1 && halfway.x < stretched.x - 0.1, "half way through the fade, part stretched: {halfway}");
    assert!(halfway.y > 0.05 && halfway.y < 0.5, "and part raised: {halfway}");
    assert!(player.layers()[0].fading());
    player.advance(&skeleton, &clips, 0.6);
    let raised = arm_at(&player, &skeleton, &clips);
    assert!(raised.x.abs() < 1e-3 && raised.y > 0.99, "after the fade, Raise alone: {raised}");
    assert!(!player.layers()[0].fading(), "the finished fade lets the old clip go");
    player.stop("", 0.0);
    assert!(arm_at(&player, &skeleton, &clips).length() < 1e-4 && !player.is_playing(), "stopped, at rest");
}

#[test]
fn a_mix_weighs_its_clips_by_where_a_parameter_lies() {
    let (skeleton, clips) = arm(&[]);
    let mut player = Player::new();
    let mix = Motion::Mix { items: vec![(once("Stretch"), 0.0), (once("Raise"), 1.0)], by: Number::Parameter("aim".to_owned()) };
    player.play("", mix, 0.0).unwrap();
    player.advance(&skeleton, &clips, 1.0);
    let mut at = |aim: f32| {
        player.set_parameter("aim", Parameter::Number(aim)).unwrap();
        arm_at(&player, &skeleton, &clips)
    };
    let (low, high, past) = (at(0.25), at(0.75), at(5.0));
    assert!(low.x > high.x && low.y < high.y, "more Raise as aim rises: {low} then {high}");
    assert!(low.x > 0.0 && low.y > 0.0, "both clips show between their points: {low}");
    assert!(past.x.abs() < 1e-4 && past.y > 0.99, "past the last point, that clip alone: {past}");
}

#[test]
fn a_layer_plays_only_on_its_joint_and_the_nodes_under_it() {
    let (skeleton, clips) = arm(&[]);
    assert_eq!(skeleton.mask("Arm"), Some(vec![0.0, 1.0, 1.0]), "Arm and Hand, not Root");
    assert_eq!(skeleton.mask("Missing"), None);
    let mut player = Player::new();
    player.play("", once("Stretch"), 0.0).unwrap();
    // Raise moves Arm, which a layer on Hand does not cover.
    player.play("Hand", once("Raise"), 0.0).unwrap();
    player.advance(&skeleton, &clips, 1.0);
    let beside = arm_at(&player, &skeleton, &clips);
    assert!(beside.x > 1.4 && beside.y.abs() < 1e-4, "a layer on Hand leaves Arm to the base: {beside}");
    player.play("Arm", once("Raise"), 0.0).unwrap();
    player.advance(&skeleton, &clips, 1.0);
    let over = arm_at(&player, &skeleton, &clips);
    assert!(over.y > 0.99, "a layer on Arm moves it: {over}");
    player.stop("Arm", 1.0);
    player.advance(&skeleton, &clips, 0.5);
    let stopping = arm_at(&player, &skeleton, &clips);
    assert!(stopping.y > 0.1 && stopping.y < 0.9, "a stopping layer fades out: {stopping}");
    player.advance(&skeleton, &clips, 0.6);
    assert!(player.layer("Arm").is_none() && arm_at(&player, &skeleton, &clips).y.abs() < 1e-4, "and goes");
}

#[test]
fn clip_events_arrive_once_each_time_their_moment_passes() {
    let marks =
        [("Stretch".to_owned(), ClipEvent { time: 0.25, name: "step".to_owned() }), ("Stretch".to_owned(), ClipEvent { time: 0.75, name: "land".to_owned() })];
    let (skeleton, clips) = arm(&marks);
    let mut player = Player::new();
    player.play("", Motion::clip("Stretch"), 0.0).unwrap();
    let mut heard = Vec::new();
    for _ in 0..20 {
        heard.extend(player.advance(&skeleton, &clips, 0.1).events);
    }
    assert_eq!(heard, ["step", "land", "step", "land"], "two loops of the clip");
    // One long step passes both.
    let mut long = Player::new();
    long.play("", once("Stretch"), 0.0).unwrap();
    assert_eq!(long.advance(&skeleton, &clips, 5.0).events, ["step", "land"], "a clip that does not loop marks them once");
    assert!(long.advance(&skeleton, &clips, 5.0).events.is_empty());
}

#[test]
fn root_motion_moves_the_object_and_keeps_the_joint_in_place() {
    let (skeleton, clips) = arm(&[]);
    let mut player = Player::new();
    player.root = Some(1);
    player.play("", Motion::clip("Stretch"), 0.0).unwrap();
    let first = player.advance(&skeleton, &clips, 0.5).root;
    assert!((first.x - 0.75).abs() < 1e-3 && first.y == 0.0, "half the clip carries it half way: {first}");
    assert!(arm_at(&player, &skeleton, &clips).x.abs() < 1e-3, "the joint is drawn where it stood");
    // Across the clip's start over, the runs add up rather than jump back.
    let across = player.advance(&skeleton, &clips, 1.0).root;
    assert!((across.x - 1.5).abs() < 1e-3, "a whole clip's length further: {across}");
    // Raise moves up only: nothing along the ground.
    player.play("", Motion::clip("Raise"), 0.0).unwrap();
    assert!(player.advance(&skeleton, &clips, 0.5).root.length() < 1e-4, "a move up is not root motion");
    assert!(arm_at(&player, &skeleton, &clips).y > 0.4, "and stays in the pose");
}

#[test]
fn a_motion_names_what_the_model_lacks() {
    let (skeleton, clips) = arm(&[]);
    let layer =
        Motion::Layer { base: Box::new(Motion::clip("Walk")), top: Box::new(Motion::clip("Wave")), joint: "Elbow".to_owned(), weight: Number::Value(1.0) };
    let problems = layer.problems(&skeleton, &clips);
    assert!(problems.iter().any(|p| p.contains("\"Walk\"") && p.contains("Stretch")), "a missing clip, with the ones there are: {problems:?}");
    assert!(problems.iter().any(|p| p.contains("\"Elbow\"")), "{problems:?}");
    assert!(Motion::clip("Wave").problems(&skeleton, &clips).is_empty());
}
