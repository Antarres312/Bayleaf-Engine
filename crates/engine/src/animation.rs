//! Skeletal animation in the session (`docs/animation.md`): a player for each animated object, the
//! poses the views draw, and the `animation.*` commands.
//!
//! In play mode each object with a Model Animation gets a player when play starts (or when it is
//! spawned), which plays its clip; scripts and commands then play, fade, mix and stop clips on it.
//! Outside play mode a player is made only by an `animation.*` command (the Inspector's preview);
//! its clips move only while the editor calls [`Session::animate`], and the scene file is never
//! changed. Starting or stopping play mode drops every player.

use std::collections::HashMap;
use std::sync::Arc;

use bayleaf_animation::{Clips, Motion, Number, Parameter, Player};
use bayleaf_core::StableId;
use bayleaf_core::math::Mat4;
use bayleaf_render::assets::skeleton::Skeleton;
use bayleaf_scene::components::{Mesh, ModelAnimation};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

/// Longest clip or joint name, in bytes.
const MAX_NAME: usize = 256;
/// Most clips one mix holds.
const MAX_MIX: usize = 16;

/// The session's animated objects and their poses.
#[derive(Default)]
pub(crate) struct Animation {
    cache: bayleaf_render::assets::AssetCache,
    objects: HashMap<StableId, Animated>,
    /// Each animated object's nodes, as the views draw them (see [`bayleaf_render::AssetSource::pose`]).
    poses: HashMap<StableId, Arc<Vec<Mat4>>>,
}

/// One object's player and what it plays on.
pub(crate) struct Animated {
    /// The model file (the Mesh's `model`) the player was made for.
    model: String,
    pub(crate) skeleton: Arc<Skeleton>,
    pub(crate) clips: Clips,
    pub(crate) player: Player,
    /// Outside play mode: whether its time stands still (a preview paused or scrubbed).
    paused: bool,
}

impl Animated {
    fn world(&self) -> Vec<Mat4> {
        self.skeleton.world(&self.player.pose(&self.skeleton, &self.clips))
    }
}

impl Animation {
    /// Drops every player and pose (play mode starts or stops, or another scene opens).
    pub(crate) fn clear(&mut self) {
        self.objects.clear();
        self.poses.clear();
    }
}

impl Session {
    /// The nodes of each object an animation player poses, for the views to draw (wrap the view's
    /// assets in [`Posed`]).
    pub fn animation_poses(&self) -> &HashMap<StableId, Arc<Vec<Mat4>>> {
        &self.animation.poses
    }

    /// Whether an animation preview moves outside play mode (the editor then keeps calling
    /// [`Session::animate`]).
    pub fn animation_previewing(&self) -> bool {
        !self.is_playing() && self.animation.objects.values().any(|a| !a.paused && a.player.is_playing())
    }

    /// Moves the editor's animation previews on by `dt` seconds (outside play mode; play mode
    /// moves its own players as it steps).
    pub fn animate(&mut self, dt: f32) {
        if self.is_playing() {
            return;
        }
        let ids: Vec<StableId> = self.animation.objects.iter().filter(|(_, a)| !a.paused).map(|(id, _)| *id).collect();
        for id in ids {
            if let Some(a) = self.animation.objects.get_mut(&id) {
                a.player.advance(&a.skeleton, &a.clips, dt);
                let world = Arc::new(a.world());
                self.animation.poses.insert(id, world);
            }
        }
    }
}

/// A view's assets with the session's poses: what the views pass to `extract_with`, so the
/// objects animation players move are drawn as they pose them.
pub struct Posed<'a> {
    pub assets: &'a mut dyn bayleaf_render::AssetSource,
    pub poses: &'a HashMap<StableId, Arc<Vec<Mat4>>>,
}

impl bayleaf_render::AssetSource for Posed<'_> {
    fn material(&mut self, path: &str) -> Option<Arc<bayleaf_render::CompiledMaterial>> {
        self.assets.material(path)
    }
    fn image(&mut self, path: &str) -> Option<Arc<bayleaf_render::assets::Image>> {
        self.assets.image(path)
    }
    fn model(&mut self, path: &str) -> Option<Arc<bayleaf_render::assets::MeshData>> {
        self.assets.model(path)
    }
    fn lighting(&mut self, path: &str) -> Option<Arc<bayleaf_scene::lighting::Lighting>> {
        self.assets.lighting(path)
    }
    fn post(&mut self, path: &str) -> Option<Arc<bayleaf_scene::post::PostProcessing>> {
        self.assets.post(path)
    }
    fn pose(&mut self, object: StableId) -> Option<Arc<Vec<Mat4>>> {
        self.poses.get(&object).cloned()
    }
}

/// The skeleton of object `id`'s model, read through the session's confinement.
fn load_skeleton(session: &mut Session, id: StableId) -> Result<(String, Arc<Skeleton>), CommandError> {
    let mesh = session.scene.get::<Mesh>(id).ok_or_else(|| CommandError::Failed(format!("object {id} has no Mesh, so no model to animate")))?;
    if mesh.model.is_empty() {
        return Err(CommandError::Failed(format!("object {id}'s Mesh has no model to animate")));
    }
    let model = mesh.model.clone();
    let path = session.allowed(&session.script_dir().join(&model), Access::Read)?;
    let data = session.animation.cache.model(&path).map_err(|e| CommandError::Failed(format!("{model}: {e}")))?;
    let skeleton = data.skeleton.clone().ok_or_else(|| CommandError::Failed(format!("{model} has no skeleton or animations")))?;
    Ok((model, skeleton))
}

/// A new player for object `id` as its Model Animation sets it up, playing its clip when it is set
/// to play.
fn new_animated(session: &mut Session, id: StableId, model: String, skeleton: Arc<Skeleton>, autoplay: bool) -> Animated {
    let settings = session.scene.get::<ModelAnimation>(id).cloned();
    let clips = Clips::of_model(&skeleton, &[]);
    let mut player = Player::new();
    if let Some(settings) = settings.filter(|s| autoplay && s.playing) {
        let index = skeleton.animation(&settings.animation);
        if let Some(name) = index.and_then(|i| skeleton.animations.get(i)).map(|a| a.name.clone()) {
            let _ = player.play("", Motion::Clip { clip: name, speed: settings.speed, looping: settings.looping }, 0.0);
        }
    }
    Animated { model, skeleton, clips, player, paused: false }
}

/// Object `id`'s player, made when it has none (or its model changed).
fn animated(session: &mut Session, id: StableId) -> Result<&mut Animated, CommandError> {
    if !session.scene.contains(id) {
        return Err(CommandError::Failed(format!("there is no object {id}")));
    }
    let current = session.scene.get::<Mesh>(id).map(|m| m.model.clone());
    let fresh = session.animation.objects.get(&id).is_none_or(|a| Some(&a.model) != current.as_ref());
    if fresh {
        let (model, skeleton) = load_skeleton(session, id)?;
        let made = new_animated(session, id, model, skeleton, session.is_playing());
        session.animation.objects.insert(id, made);
    }
    session.animation.objects.get_mut(&id).ok_or_else(|| CommandError::Failed(format!("object {id} has no animation")))
}

/// Play mode started: every object with a Model Animation gets a player.
pub(crate) fn start(session: &mut Session) {
    session.animation.clear();
    sync(session);
}

/// Gives each object with a Model Animation and a model that has none a player (spawned ones too),
/// and drops the players of objects that went.
fn sync(session: &mut Session) {
    let scene = &session.scene;
    session.animation.objects.retain(|id, _| scene.contains(*id));
    session.animation.poses.retain(|id, _| scene.contains(*id));
    let wanted: Vec<StableId> = session
        .scene
        .world_matrices()
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| session.scene.get::<ModelAnimation>(*id).is_some() && !session.animation.objects.contains_key(id))
        .collect();
    for id in wanted {
        // A model that does not load or has no skeleton is drawn as it is; extract reports it.
        if let Ok((model, skeleton)) = load_skeleton(session, id) {
            let made = new_animated(session, id, model, skeleton, true);
            session.animation.objects.insert(id, made);
        }
    }
}

/// One play-mode step: every player moves on by `dt`, and its pose is kept for the views.
pub(crate) fn step(session: &mut Session, dt: f32) {
    sync(session);
    let ids: Vec<StableId> = session.animation.objects.keys().copied().collect();
    for id in ids {
        let Some(a) = session.animation.objects.get_mut(&id) else { continue };
        a.player.advance(&a.skeleton, &a.clips, dt);
        let world = Arc::new(a.world());
        session.animation.poses.insert(id, world);
    }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    let spec =
        |id, label, params, run| CommandSpec { id, label, menu: "", shortcut: "", params, prompt: Prompt::None, mutates: Mutates::No, enabled: always, run };
    vec![
        spec(
            "animation.get",
            "Read Animation",
            "{id}: the object's animation: {clips: [{name, duration}], joints: [names], layers: [{joint, motion, time, fading}], parameters: {name: value}, playing}",
            get,
        ),
        spec(
            "animation.play",
            "Play Clip",
            "{id, clip? | mix?: [{clip, at}], by?: number or parameter name, layer?: joint, fade?: seconds, loop?: true, speed?: 1}: plays a clip (or a mix of clips by where `by` lies among their points) on the object, fading from what played over `fade` (0 to 60, default 0.25); `layer` plays it on that joint and the ones under it only, over the rest",
            play,
        ),
        spec("animation.stop", "Stop Clip", "{id, layer?: joint, fade?: seconds}: the whole skeleton fades to rest; a layer fades out and goes", stop),
        spec(
            "animation.parameter.set",
            "Set Animation Parameter",
            "{id, name, value: number | true/false}: a parameter mixes and animation states read",
            parameter_set,
        ),
        spec("animation.trigger", "Trigger Animation Parameter", "{id, name}: sets a trigger, which the first transition that uses it uses up", trigger),
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdParams {
    id: StableId,
}

fn motion_json(motion: &Motion) -> Value {
    match motion {
        Motion::Rest => json!("rest"),
        Motion::Clip { clip, speed, looping } => json!({ "clip": clip, "speed": speed, "loop": looping }),
        Motion::Mix { items, by } => json!({
            "mix": items.iter().map(|(m, at)| json!({ "motion": motion_json(m), "at": at })).collect::<Vec<_>>(),
            "by": number_json(by),
        }),
        Motion::Layer { base, top, joint, weight } => {
            json!({ "base": motion_json(base), "top": motion_json(top), "joint": joint, "weight": number_json(weight) })
        }
    }
}

fn number_json(number: &Number) -> Value {
    match number {
        Number::Value(v) => json!(v),
        Number::Parameter(name) => json!(name),
    }
}

fn parameter_json(value: &Parameter) -> Value {
    match value {
        Parameter::Number(n) => json!(n),
        Parameter::Flag(b) => json!(b),
        Parameter::Trigger(b) => json!({ "trigger": b }),
    }
}

/// What object `id`'s animation is now, as `animation.get` gives it.
pub(crate) fn describe(a: &Animated) -> Value {
    let clips: Vec<Value> = a.clips.names().map(|n| json!({ "name": n, "duration": a.clips.get(n).map_or(0.0, |c| c.duration()) })).collect();
    let layers: Vec<Value> = a
        .player
        .layers()
        .iter()
        .map(|l| {
            let (motion, time) = l.current().map_or((json!("rest"), 0.0), |(m, t)| (motion_json(m), t));
            json!({ "joint": l.joint, "motion": motion, "time": time, "fading": l.fading() })
        })
        .collect();
    let parameters: serde_json::Map<String, Value> = a.player.parameters.iter().map(|(k, v)| (k.clone(), parameter_json(v))).collect();
    json!({
        "clips": clips,
        "joints": a.skeleton.names,
        "layers": layers,
        "parameters": parameters,
        "playing": a.player.is_playing(),
    })
}

fn get(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: IdParams = parse(params)?;
    Ok(describe(animated(session, p.id)?))
}

fn default_fade() -> f32 {
    0.25
}

fn default_true() -> bool {
    true
}

fn default_speed() -> f32 {
    1.0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MixItem {
    clip: String,
    at: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayParams {
    id: StableId,
    #[serde(default)]
    clip: Option<String>,
    #[serde(default)]
    mix: Option<Vec<MixItem>>,
    #[serde(default)]
    by: Option<Value>,
    #[serde(default)]
    layer: String,
    #[serde(default = "default_fade")]
    fade: f32,
    #[serde(default = "default_true", rename = "loop")]
    looping: bool,
    #[serde(default = "default_speed")]
    speed: f32,
}

fn check_name(what: &str, name: &str) -> Result<(), CommandError> {
    if name.len() > MAX_NAME {
        return Err(CommandError::BadParams(format!("{what} is longer than {MAX_NAME} bytes")));
    }
    Ok(())
}

fn check_fade(fade: f32) -> Result<(), CommandError> {
    if !(fade.is_finite() && (0.0..=bayleaf_animation::MAX_FADE).contains(&fade)) {
        return Err(CommandError::BadParams(format!("fade must be from 0 to {} seconds", bayleaf_animation::MAX_FADE)));
    }
    Ok(())
}

/// A number a motion reads: a number as it is, or text naming a parameter.
fn number_param(value: &Value) -> Result<Number, CommandError> {
    match value {
        Value::Number(n) => n.as_f64().map(|v| Number::Value(v as f32)).filter(|n| matches!(n, Number::Value(v) if v.is_finite())),
        Value::String(name) if !name.is_empty() && name.len() <= MAX_NAME => Some(Number::Parameter(name.clone())),
        _ => None,
    }
    .ok_or_else(|| CommandError::BadParams("by must be a number or a parameter's name".to_owned()))
}

fn play(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PlayParams = parse(params)?;
    check_name("layer", &p.layer)?;
    check_fade(p.fade)?;
    if !(p.speed.is_finite() && (0.0..=10.0).contains(&p.speed)) {
        return Err(CommandError::BadParams("speed must be from 0 to 10".to_owned()));
    }
    let clip = |name: String| Motion::Clip { clip: name, speed: p.speed, looping: p.looping };
    let motion = match (p.clip, p.mix) {
        (Some(name), None) => {
            check_name("clip", &name)?;
            clip(name)
        }
        (None, Some(items)) => {
            if items.is_empty() || items.len() > MAX_MIX {
                return Err(CommandError::BadParams(format!("a mix holds 1 to {MAX_MIX} clips")));
            }
            let mut mixed = Vec::with_capacity(items.len());
            for item in items {
                check_name("clip", &item.clip)?;
                if !item.at.is_finite() {
                    return Err(CommandError::BadParams("each clip's point must be a number".to_owned()));
                }
                mixed.push((clip(item.clip), item.at));
            }
            let by = p.by.as_ref().map_or(Ok(Number::Value(0.0)), number_param)?;
            Motion::Mix { items: mixed, by }
        }
        _ => return Err(CommandError::BadParams("give a clip or a mix".to_owned())),
    };
    let a = animated(session, p.id)?;
    let problems = motion.problems(&a.skeleton, &a.clips);
    if !p.layer.is_empty() && a.skeleton.mask(&p.layer).is_none() {
        return Err(CommandError::Failed(format!("there is no joint named \"{}\"; the model has {}", p.layer, a.skeleton.names.join(", "))));
    }
    if let Some(problem) = problems.first() {
        return Err(CommandError::Failed(problem.clone()));
    }
    a.player.play(&p.layer, motion, p.fade).map_err(CommandError::Failed)?;
    a.paused = false;
    let world = Arc::new(a.world());
    let out = describe(a);
    session.animation.poses.insert(p.id, world);
    Ok(out)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StopParams {
    id: StableId,
    #[serde(default)]
    layer: String,
    #[serde(default = "default_fade")]
    fade: f32,
}

fn stop(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: StopParams = parse(params)?;
    check_name("layer", &p.layer)?;
    check_fade(p.fade)?;
    let a = animated(session, p.id)?;
    if !p.layer.is_empty() && a.player.layer(&p.layer).is_none() {
        return Err(CommandError::Failed(format!("no layer plays on \"{}\"", p.layer)));
    }
    a.player.stop(&p.layer, p.fade);
    let world = Arc::new(a.world());
    let out = describe(a);
    session.animation.poses.insert(p.id, world);
    Ok(out)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ParameterParams {
    id: StableId,
    name: String,
    value: Value,
}

fn check_parameter_name(name: &str) -> Result<(), CommandError> {
    if name.is_empty() || name.len() > MAX_NAME {
        return Err(CommandError::BadParams(format!("a parameter's name is 1 to {MAX_NAME} bytes")));
    }
    Ok(())
}

fn parameter_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ParameterParams = parse(params)?;
    check_parameter_name(&p.name)?;
    let value = match &p.value {
        Value::Bool(b) => Parameter::Flag(*b),
        Value::Number(n) => match n.as_f64() {
            Some(v) if v.is_finite() && v.abs() <= 1.0e9 => Parameter::Number(v as f32),
            _ => return Err(CommandError::BadParams("value must be a number of at most a billion, or true/false".to_owned())),
        },
        _ => return Err(CommandError::BadParams("value must be a number or true/false".to_owned())),
    };
    set_parameter(session, p.id, &p.name, value)
}

fn set_parameter(session: &mut Session, id: StableId, name: &str, value: Parameter) -> Result<Value, CommandError> {
    let a = animated(session, id)?;
    a.player.set_parameter(name, value).map_err(CommandError::Failed)?;
    let world = Arc::new(a.world());
    let out = describe(a);
    session.animation.poses.insert(id, world);
    Ok(out)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TriggerParams {
    id: StableId,
    name: String,
}

fn trigger(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: TriggerParams = parse(params)?;
    check_parameter_name(&p.name)?;
    set_parameter(session, p.id, &p.name, Parameter::Trigger(true))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use crate::{CommandRegistry, Session};

    /// A saved scene with one object showing the generated arm model with a Model Animation
    /// playing Stretch once; gives its id.
    pub(crate) fn arm_scene(name: &str) -> (CommandRegistry, Session, String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("bayleaf-animation-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        bayleaf_render::assets::fixtures::skinned_arm(&dir).unwrap();
        let registry = CommandRegistry::with_builtins();
        let mut session = Session::default();
        let mesh = json!({"shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 1, "model": "arm.gltf"});
        let animation = json!({"animation": "Stretch", "playing": true, "looping": false, "speed": 1});
        let created =
            registry.execute(&mut session, "scene.object.create", &json!({"name": "Arm", "components": {"mesh": mesh, "modelAnimation": animation}})).unwrap();
        let id = created["id"].as_str().unwrap().to_owned();
        registry.execute(&mut session, "scene.saveAs", &json!({"path": dir.join("arm.bscene")})).unwrap();
        (registry, session, id, dir)
    }

    /// Where the session's pose puts the Arm joint in the model.
    pub(crate) fn arm_joint(session: &Session, id: &str) -> bayleaf_core::math::Vec3 {
        let id = bayleaf_core::StableId::parse_hex(id).unwrap();
        session.animation_poses().get(&id).expect("the object is posed")[1].w_axis.truncate()
    }

    #[test]
    fn clips_fade_and_mix_on_an_object_and_stopping_play_drops_them() {
        let (registry, mut session, id, dir) = arm_scene("fade");
        let run = |session: &mut Session, command: &str, params: Value| registry.execute(session, command, &params);
        run(&mut session, "play.start", Value::Null).unwrap();
        run(&mut session, "play.step", json!({"seconds": 1.0})).unwrap();
        assert!(arm_joint(&session, &id).x > 1.4, "its Model Animation plays Stretch: {}", arm_joint(&session, &id));

        run(&mut session, "animation.play", json!({"id": id, "clip": "Raise", "fade": 1.0, "loop": false})).unwrap();
        run(&mut session, "play.step", json!({"seconds": 0.5})).unwrap();
        let halfway = arm_joint(&session, &id);
        assert!(halfway.x > 0.1 && halfway.x < 1.4 && halfway.y > 0.05 && halfway.y < 0.5, "half way through the fade: {halfway}");
        let state = run(&mut session, "animation.get", json!({"id": id})).unwrap();
        assert_eq!(state["layers"][0]["fading"], true);
        assert_eq!(state["layers"][0]["motion"]["clip"], "Raise");
        assert!(state["joints"].as_array().unwrap().iter().any(|j| j == "Hand"));
        assert_eq!(state["clips"].as_array().unwrap().len(), 3);

        let mix = json!({"id": id, "mix": [{"clip": "Stretch", "at": 0}, {"clip": "Raise", "at": 1}], "by": "aim", "fade": 0, "loop": false});
        run(&mut session, "animation.play", mix).unwrap();
        run(&mut session, "animation.parameter.set", json!({"id": id, "name": "aim", "value": 0.75})).unwrap();
        run(&mut session, "play.step", json!({"seconds": 1.0})).unwrap();
        let mixed = arm_joint(&session, &id);
        assert!(mixed.y > mixed.x && mixed.x > 0.1, "aim 0.75 is mostly Raise: {mixed}");

        let missing = run(&mut session, "animation.play", json!({"id": id, "clip": "Walk"})).unwrap_err().to_string();
        assert!(missing.contains("Stretch"), "the clips there are: {missing}");
        let elbow = run(&mut session, "animation.play", json!({"id": id, "clip": "Wave", "layer": "Elbow"})).unwrap_err().to_string();
        assert!(elbow.contains("Hand"), "the joints there are: {elbow}");
        assert!(run(&mut session, "animation.play", json!({"id": id, "clip": "Wave", "fade": -1})).is_err());

        run(&mut session, "play.stop", Value::Null).unwrap();
        assert!(session.animation_poses().is_empty(), "stopping play mode drops its players");
        std::fs::remove_dir_all(&dir).ok();
    }
}
