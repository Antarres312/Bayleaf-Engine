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

mod states {
    use bayleaf_graph::{Graph, Link, PinRef};
    use serde_json::json;

    use crate::states::{CATALOG, compile};
    use crate::{Parameter, Player};

    struct Build(Graph);

    impl Build {
        fn node(&mut self, kind: &str, values: serde_json::Value) -> u32 {
            let id = self.0.add_node(&CATALOG, kind, [0.0, 0.0]).unwrap();
            for (name, value) in values.as_object().unwrap() {
                self.0.set_value(&CATALOG, id, name, value.clone()).unwrap();
            }
            id
        }

        fn link(&mut self, from: u32, out: &str, to: u32, input: &str) {
            self.0.connect(&CATALOG, Link { from: PinRef::new(from, out), to: PinRef::new(to, input) }).unwrap();
        }
    }

    /// Idle mixes Stretch and Raise by "aim"; a "wave" trigger (unless "still") goes to Wave, a layer
    /// of Wave on Hand over Stretch; after one run of Wave, or with aim past 5, back to Idle. It
    /// uses every node kind.
    fn every_node() -> Graph {
        let mut b = Build(Graph::empty(&CATALOG));
        b.node("entry", json!({"state": "Idle"}));
        let idle = b.node("state", json!({"name": "Idle"}));
        let (stretch, raise) = (b.node("clip", json!({"clip": "Stretch", "play": "Once"})), b.node("clip", json!({"clip": "Raise", "play": "Once"})));
        let mix = b.node("mix", json!({"atA": 0.0, "atB": 1.0}));
        let aim = b.node("number", json!({"name": "aim"}));
        b.link(stretch, "motion", mix, "a");
        b.link(raise, "motion", mix, "b");
        b.link(aim, "value", mix, "by");
        b.link(mix, "motion", idle, "motion");
        let wave_state = b.node("state", json!({"name": "Wave"}));
        let (base, wave) = (b.node("clip", json!({"clip": "Stretch"})), b.node("clip", json!({"clip": "Wave"})));
        let layer = b.node("layer", json!({"joint": "Hand"}));
        b.link(base, "motion", layer, "base");
        b.link(wave, "motion", layer, "top");
        b.link(layer, "motion", wave_state, "motion");
        let go = b.node("transition", json!({"from": "Idle", "to": "Wave", "fade": 0.0}));
        let (trigger, still) = (b.node("trigger", json!({"name": "wave"})), b.node("flag", json!({"name": "still"})));
        let (and, not) = (b.node("and", json!({})), b.node("not", json!({})));
        b.link(trigger, "value", and, "a");
        b.link(still, "value", not, "in");
        b.link(not, "result", and, "b");
        b.link(and, "result", go, "when");
        let back = b.node("transition", json!({"from": "Wave", "to": "Idle", "fade": 0.0}));
        let time = b.node("stateTime", json!({}));
        let (ran, aimed) = (b.node("compare", json!({"test": "At Least", "b": 1.0})), b.node("compare", json!({"test": "Greater", "b": 5.0})));
        b.link(time, "runs", ran, "a");
        let aim_again = b.node("number", json!({"name": "aim"}));
        b.link(aim_again, "value", aimed, "a");
        let or = b.node("or", json!({}));
        b.link(ran, "result", or, "a");
        b.link(aimed, "result", or, "b");
        b.link(or, "result", back, "when");
        b.0
    }

    #[test]
    fn animation_states_with_every_node_kind_compile_and_run() {
        let graph = every_node();
        for def in CATALOG.nodes {
            assert!(graph.nodes.iter().any(|n| n.kind == def.kind), "the test graph lacks {}", def.kind);
        }
        let machine = std::sync::Arc::new(compile(&graph).unwrap());
        assert_eq!(machine.states.len(), 2);
        assert_eq!(machine.parameters.len(), 3, "aim, wave and still: {:?}", machine.parameters);
        let (skeleton, clips) = super::arm(&[]);
        let mut player = Player::new();
        player.run_states(machine);
        player.advance(&skeleton, &clips, 0.1);
        assert_eq!(player.state(), Some("Idle"));
        player.set_parameter("wave", Parameter::Trigger(true)).unwrap();
        player.advance(&skeleton, &clips, 0.1);
        assert_eq!(player.state(), Some("Wave"), "the trigger lets the transition through");
        assert_eq!(player.parameters.get("wave"), Some(&Parameter::Trigger(false)), "and is used up");
        player.advance(&skeleton, &clips, 1.2);
        player.advance(&skeleton, &clips, 0.01);
        assert_eq!(player.state(), Some("Idle"), "after one run of Wave, back");
        player.set_parameter("still", Parameter::Flag(true)).unwrap();
        player.set_parameter("wave", Parameter::Trigger(true)).unwrap();
        player.advance(&skeleton, &clips, 0.1);
        assert_eq!(player.state(), Some("Idle"), "Not still stops it");
        assert_eq!(player.parameters.get("wave"), Some(&Parameter::Trigger(true)), "a trigger no transition used stays set");
        player.go_to("Wave", 0.0).unwrap();
        assert_eq!(player.state(), Some("Wave"));
        assert!(player.go_to("Run", 0.0).unwrap_err().contains("Idle, Wave"));
    }

    #[test]
    fn broken_animation_states_name_their_problems_on_their_nodes() {
        let mut b = Build(Graph::empty(&CATALOG));
        let first = b.node("state", json!({"name": "Idle"}));
        let twin = b.node("state", json!({"name": "Idle"}));
        let nowhere = b.node("transition", json!({"from": "Idle", "to": "Run"}));
        let mix = b.node("mix", json!({}));
        b.link(mix, "motion", first, "motion");
        let speed = b.node("number", json!({"name": "speed"}));
        let flag = b.node("flag", json!({"name": "speed"}));
        let problems = compile(&b.0).unwrap_err();
        let on = |node: u32| problems.iter().filter(|p| p.node == Some(node)).map(|p| p.message.clone()).collect::<Vec<_>>();
        assert!(!on(twin).is_empty() && !on(nowhere).is_empty() && !on(mix).is_empty(), "{problems:?}");
        assert!(on(speed).len() + on(flag).len() >= 1, "one name, two kinds: {problems:?}");
        // A motion goes only into a motion input.
        let compare = b.node("compare", json!({}));
        assert!(b.0.connect(&CATALOG, Link { from: PinRef::new(mix, "motion"), to: PinRef::new(compare, "a") }).is_err());
        assert!(b.0.connect(&CATALOG, Link { from: PinRef::new(speed, "value"), to: PinRef::new(twin, "motion") }).is_err());
    }
}
