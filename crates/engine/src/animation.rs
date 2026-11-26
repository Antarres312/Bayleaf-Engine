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

use bayleaf_animation::states::{self, Machine};
use bayleaf_animation::{ClipEvent, Clips, Motion, Number, Parameter, Player};
use bayleaf_core::StableId;
use bayleaf_core::math::Mat4;
use bayleaf_render::assets::skeleton::Skeleton;
use bayleaf_scene::components::{Mesh, ModelAnimation};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Level, Mutates, Prompt, Session, always, parse};

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
    /// Its joints' names, shared with what scripts read.
    names: Arc<[String]>,
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
    let marks: Vec<(String, ClipEvent)> =
        settings.iter().flat_map(|s| s.events.iter().map(|m| (m.clip.clone(), ClipEvent { time: m.time, name: m.event.clone() }))).collect();
    let clips = Clips::of_model(&skeleton, &marks);
    let mut player = Player::new();
    // Root motion moves the object only in play mode, where the scene is a copy.
    if let Some(s) = settings.as_ref().filter(|s| autoplay && s.root_motion) {
        player.root = if s.root_joint.is_empty() { bayleaf_animation::default_root(&skeleton) } else { skeleton.node_named(&s.root_joint) };
        if player.root.is_none() {
            session.log(Level::Error, format!("Root motion: the model has no joint named \"{}\"", s.root_joint));
        }
    }
    match settings.filter(|_| autoplay) {
        // Animation states, when the component names some that read and compile.
        Some(settings) if !settings.states.is_empty() => match load_states(session, &settings.states) {
            Ok(machine) => {
                let problems: Vec<String> = machine.states.iter().flat_map(|s| s.motion.problems(&skeleton, &clips)).collect();
                for problem in problems {
                    session.log(Level::Error, format!("{}: {problem}", settings.states));
                }
                player.run_states(machine);
            }
            Err(problem) => session.log(Level::Error, format!("{}: {problem}; the object plays no animation states", settings.states)),
        },
        Some(settings) if settings.playing => {
            let index = skeleton.animation(&settings.animation);
            if let Some(name) = index.and_then(|i| skeleton.animations.get(i)).map(|a| a.name.clone()) {
                let _ = player.play("", Motion::Clip { clip: name, speed: settings.speed, looping: settings.looping }, 0.0);
            }
        }
        _ => {}
    }
    let names: Arc<[String]> = Arc::from(skeleton.names.as_slice());
    Animated { model, skeleton, clips, player, paused: false, names }
}

/// Reads and compiles the animation states at `path` (relative to the project folder).
fn load_states(session: &Session, path: &str) -> Result<Arc<Machine>, String> {
    let full = session.allowed(&session.script_dir().join(path), Access::Read).map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&full).map_err(|e| format!("cannot be read: {e}"))?;
    let graph = bayleaf_graph::Graph::from_json(&text, &states::CATALOG).map_err(|e| e.to_string())?;
    states::compile(&graph).map(Arc::new).map_err(|problems| bayleaf_graph::summary(&problems))
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

/// One play-mode step: every player moves on by `dt` and its pose is kept for the views; root
/// motion moves its object, and the clip events it passed go to the object's scripts.
pub(crate) fn step(session: &mut Session, dt: f32) {
    sync(session);
    let ids: Vec<StableId> = session.animation.objects.keys().copied().collect();
    let mut events: Vec<(StableId, String)> = Vec::new();
    for id in ids {
        let Some(a) = session.animation.objects.get_mut(&id) else { continue };
        let step = a.player.advance(&a.skeleton, &a.clips, dt);
        let world = Arc::new(a.world());
        session.animation.poses.insert(id, world);
        events.extend(step.events.into_iter().map(|e| (id, e)));
        if step.root.length_squared() > 0.0 {
            move_by_root(session, id, step.root);
        }
    }
    let view = script_view(session);
    if let Some(play) = session.play.as_mut() {
        play.host.set_animation(view);
    }
    if events.is_empty() {
        return;
    }
    let input = Arc::clone(&session.input);
    if let Some(play) = session.play.as_mut() {
        play.host.trigger_events(&mut session.scene, &events, play.time, &input);
    }
    crate::play::after_message(session);
}

/// What scripts read of the animated objects after this step: each one's state and its joints'
/// places in the world.
fn script_view(session: &Session) -> bayleaf_script::AnimationView {
    let mut view = bayleaf_script::AnimationView::default();
    for (id, a) in &session.animation.objects {
        let (Some(world), Some(nodes)) = (session.scene.world_matrix(*id), session.animation.poses.get(id)) else { continue };
        let joints = nodes
            .iter()
            .map(|m| {
                let t = bayleaf_scene::components::Transform::from_matrix(world * (a.skeleton.root * *m).as_dmat4());
                let p = t.position.map(|v| v as f32);
                [p[0], p[1], p[2], t.rotation[0], t.rotation[1], t.rotation[2], t.scale[0], t.scale[1], t.scale[2]]
            })
            .collect();
        let state = a.player.state().map(str::to_owned);
        view.objects.insert(*id, bayleaf_script::AnimatedView { state, names: Arc::clone(&a.names), joints });
    }
    view
}

/// Does what a script asked of object `object`'s animation (`by` is the asking script's object);
/// what cannot be done goes to the console.
pub(crate) fn apply_script(session: &mut Session, by: StableId, object: StableId, action: bayleaf_script::AnimationAction) {
    use bayleaf_script::AnimationAction;
    let result = animated(session, object).and_then(|a| {
        match action {
            AnimationAction::Play { clip, fade, looping } => {
                let motion = Motion::Clip { clip, speed: 1.0, looping };
                if let Some(problem) = motion.problems(&a.skeleton, &a.clips).first() {
                    return Err(CommandError::Failed(problem.clone()));
                }
                a.player.play("", motion, fade).map_err(CommandError::Failed)?;
            }
            AnimationAction::Stop { fade } => a.player.stop("", fade),
            AnimationAction::Number { name, value } => a.player.set_parameter(&name, Parameter::Number(value)).map_err(CommandError::Failed)?,
            AnimationAction::Flag { name, value } => a.player.set_parameter(&name, Parameter::Flag(value)).map_err(CommandError::Failed)?,
            AnimationAction::Trigger { name } => a.player.set_parameter(&name, Parameter::Trigger(true)).map_err(CommandError::Failed)?,
        }
        Ok(())
    });
    if let Err(error) = result {
        let who = session.scene.name(by).unwrap_or("Object").to_owned();
        let target = session.scene.name(object).unwrap_or("Object").to_owned();
        session.log(Level::Error, format!("[{who}] cannot change the animation of {target}: {error}"));
    }
}

/// Moves object `id` by `delta`, a move in its model (with the import scale and axes): through
/// the object's own turn and scale into the world, then into its parent's space.
fn move_by_root(session: &mut Session, id: StableId, delta: bayleaf_core::math::Vec3) {
    let Some(world) = session.scene.world_matrix(id) else { return };
    let in_world = world.transform_vector3(delta.as_dvec3());
    let in_parent = match session.scene.parent(id).and_then(|p| session.scene.world_matrix(p)) {
        Some(parent) => parent.inverse().transform_vector3(in_world),
        None => in_world,
    };
    let Some(mut transform) = session.scene.get::<bayleaf_scene::components::Transform>(id).cloned() else { return };
    for (axis, by) in transform.position.iter_mut().zip(in_parent.to_array()) {
        *axis += by;
    }
    if transform.position.iter().all(|p| p.is_finite()) {
        let _ = session.scene.set(id, transform);
    }
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    let spec =
        |id, label, params, run| CommandSpec { id, label, menu: "", shortcut: "", params, prompt: Prompt::None, mutates: Mutates::No, enabled: always, run };
    vec![
        spec(
            "animation.get",
            "Read Animation",
            "{id}: the object's animation: {clips: [{name, duration}], joints: [names], layers: [{joint, motion, time, fading}], parameters: {name: value}, playing, state (the animation state it is in, or null), states (its states' names, or null)}",
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
        spec(
            "animation.joint.get",
            "Read Joint",
            "{id, joint}: where the joint of the object's model is in the world now, as its animation poses it (at rest when nothing plays): {position, rotation (degrees), scale}",
            joint_get,
        ),
        spec(
            "animation.state.set",
            "Go to Animation State",
            "{id, state, fade?: seconds}: an object running animation states goes to that state now, fading over `fade` (default 0.25)",
            state_set,
        ),
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
        "state": a.player.state(),
        "states": a.player.machine().map(|m| m.states.iter().map(|s| s.name.clone()).collect::<Vec<_>>()),
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JointParams {
    id: StableId,
    joint: String,
}

/// Where joint `joint` of object `id`'s model is in the world now: by its player when it has one,
/// else at rest.
pub(crate) fn joint_world(session: &mut Session, id: StableId, joint: &str) -> Result<bayleaf_core::math::DMat4, CommandError> {
    let world = session.scene.world_matrix(id).ok_or_else(|| CommandError::Failed(format!("there is no object {id}")))?;
    let (skeleton, nodes) = match session.animation.objects.get(&id).filter(|a| session.scene.get::<Mesh>(id).is_some_and(|m| m.model == a.model)) {
        Some(a) => (Arc::clone(&a.skeleton), a.world()),
        None => {
            let (_, skeleton) = load_skeleton(session, id)?;
            let nodes = skeleton.pose(None, 0.0);
            (skeleton, nodes)
        }
    };
    let node = skeleton
        .node_named(joint)
        .ok_or_else(|| CommandError::Failed(format!("there is no joint named \"{joint}\"; the model has {}", skeleton.names.join(", "))))?;
    let in_model = skeleton.root * nodes.get(node).copied().unwrap_or(Mat4::IDENTITY);
    Ok(world * in_model.as_dmat4())
}

fn joint_get(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: JointParams = parse(params)?;
    check_name("joint", &p.joint)?;
    let t = bayleaf_scene::components::Transform::from_matrix(joint_world(session, p.id, &p.joint)?);
    Ok(json!({ "position": t.position, "rotation": t.rotation, "scale": t.scale }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateParams {
    id: StableId,
    state: String,
    #[serde(default = "default_fade")]
    fade: f32,
}

fn state_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: StateParams = parse(params)?;
    check_name("state", &p.state)?;
    check_fade(p.fade)?;
    let a = animated(session, p.id)?;
    a.player.go_to(&p.state, p.fade).map_err(CommandError::Failed)?;
    a.paused = false;
    let world = Arc::new(a.world());
    let out = describe(a);
    session.animation.poses.insert(p.id, world);
    Ok(out)
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
        let hand = run(&mut session, "animation.joint.get", json!({"id": id, "joint": "Hand"})).unwrap();
        let x = hand["position"][0].as_f64().unwrap();
        assert!(x > 2.4, "the Hand joint is a unit past the stretched Arm, in the world: {hand}");
        assert!(run(&mut session, "animation.joint.get", json!({"id": id, "joint": "Elbow"})).is_err());

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

    /// Animation states with Idle (Stretch) and Up (Raise), and a transition to Up on the trigger "up".
    fn write_states(dir: &std::path::Path) {
        use bayleaf_animation::states::CATALOG;
        use bayleaf_graph::{Graph, Link, PinRef};
        let mut graph = Graph::empty(&CATALOG);
        let node = |graph: &mut Graph, kind: &str, values: Value| {
            let id = graph.add_node(&CATALOG, kind, [0.0, 0.0]).unwrap();
            for (name, value) in values.as_object().unwrap() {
                graph.set_value(&CATALOG, id, name, value.clone()).unwrap();
            }
            id
        };
        let link = |graph: &mut Graph, from: u32, out: &str, to: u32, input: &str| {
            graph.connect(&CATALOG, Link { from: PinRef::new(from, out), to: PinRef::new(to, input) }).unwrap();
        };
        let idle = node(&mut graph, "state", json!({"name": "Idle"}));
        let up = node(&mut graph, "state", json!({"name": "Up"}));
        let (stretch, raise) = (node(&mut graph, "clip", json!({"clip": "Stretch"})), node(&mut graph, "clip", json!({"clip": "Raise", "play": "Once"})));
        link(&mut graph, stretch, "motion", idle, "motion");
        link(&mut graph, raise, "motion", up, "motion");
        let go = node(&mut graph, "transition", json!({"from": "Idle", "to": "Up", "fade": 0.0}));
        let trigger = node(&mut graph, "trigger", json!({"name": "up"}));
        link(&mut graph, trigger, "value", go, "when");
        std::fs::write(dir.join("arm.banim"), graph.to_json()).unwrap();
    }

    #[test]
    fn an_object_runs_the_animation_states_its_model_animation_names() {
        let (registry, mut session, id, dir) = arm_scene("states");
        write_states(&dir);
        let run = |session: &mut Session, command: &str, params: Value| registry.execute(session, command, &params);
        let animation = json!({"animation": "", "playing": true, "looping": true, "speed": 1, "states": "arm.banim"});
        run(&mut session, "scene.component.set", json!({"id": id, "component": "modelAnimation", "value": animation})).unwrap();
        run(&mut session, "play.start", Value::Null).unwrap();
        run(&mut session, "play.step", json!({"seconds": 0.1})).unwrap();
        let state = run(&mut session, "animation.get", json!({"id": id})).unwrap();
        assert_eq!(state["state"], "Idle");
        assert_eq!(state["states"], json!(["Idle", "Up"]));
        run(&mut session, "animation.trigger", json!({"id": id, "name": "up"})).unwrap();
        run(&mut session, "play.step", json!({"seconds": 1.0})).unwrap();
        assert_eq!(run(&mut session, "animation.get", json!({"id": id})).unwrap()["state"], "Up");
        assert!(arm_joint(&session, &id).y > 0.9, "Up plays Raise: {}", arm_joint(&session, &id));
        run(&mut session, "animation.state.set", json!({"id": id, "state": "Idle", "fade": 0})).unwrap();
        assert_eq!(run(&mut session, "animation.get", json!({"id": id})).unwrap()["state"], "Idle");
        let missing = run(&mut session, "animation.state.set", json!({"id": id, "state": "Run"})).unwrap_err().to_string();
        assert!(missing.contains("Idle, Up"), "{missing}");
        run(&mut session, "play.stop", Value::Null).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A script that logs the name of each message or event its object gets.
    fn listener_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (import "bayleaf" "log" (func $log (param i32 i32)))
                (import "bayleaf" "message_name" (func $name (param i32 i32) (result i32)))
                (memory (export "memory") 1)
                (func (export "bayleaf_abi") (result i32) i32.const 1)
                (func (export "bayleaf_message") (param $v f32)
                  (call $log (i32.const 100) (call $name (i32.const 100) (i32.const 64)))))"#,
        )
        .unwrap()
    }

    #[test]
    fn clip_events_reach_scripts_and_root_motion_moves_the_object() {
        let dir = std::env::temp_dir().join(format!("bayleaf-animation-events-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        bayleaf_render::assets::fixtures::skinned_arm(&dir).unwrap();
        std::fs::write(dir.join("listen.wasm"), listener_wasm()).unwrap();
        let registry = CommandRegistry::with_builtins();
        let mut session = Session::default();
        let run = |session: &mut Session, command: &str, params: Value| registry.execute(session, command, &params);
        let mesh = json!({"shape": "cube", "color": [1, 1, 1, 1], "metallic": 0, "roughness": 1, "model": "arm.gltf"});
        let animation = json!({"animation": "Stretch", "playing": true, "looping": true, "speed": 1, "rootMotion": true,
            "events": [{"clip": "Stretch", "time": 0.25, "event": "footstep"}]});
        let components = json!({"mesh": mesh, "modelAnimation": animation, "scripts": [{"module": "listen.wasm"}]});
        let created = run(&mut session, "scene.object.create", json!({"name": "Walker", "components": components})).unwrap();
        let id = created["id"].as_str().unwrap().to_owned();
        run(&mut session, "scene.saveAs", json!({"path": dir.join("walk.bscene")})).unwrap();
        run(&mut session, "play.start", Value::Null).unwrap();
        run(&mut session, "play.step", json!({"seconds": 0.1, "steps": 5})).unwrap();
        let heard = session.console().iter().filter(|l| l.text.contains("footstep")).count();
        assert_eq!(heard, 1, "the marked moment arrives once: {:?}", session.console().iter().map(|l| &l.text).collect::<Vec<_>>());
        let position = session.scene().component(bayleaf_core::StableId::parse_hex(&id).unwrap(), "transform").unwrap()["position"][0].as_f64().unwrap();
        assert!((position - 0.75).abs() < 0.01, "half of Stretch carries the object 0.75 along: {position}");
        assert!(arm_joint(&session, &id).x.abs() < 1e-3, "and its root joint is drawn in place");
        run(&mut session, "play.stop", Value::Null).unwrap();
        let back = session.scene().component(bayleaf_core::StableId::parse_hex(&id).unwrap(), "transform").unwrap()["position"][0].as_f64().unwrap();
        assert!(back.abs() < 1e-9, "stopping play puts it back");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A script that plays Raise on its object when it starts and logs "found" each step it reads
    /// the Hand joint.
    fn player_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (import "bayleaf" "log" (func $log (param i32 i32)))
                (import "bayleaf" "self_handle" (func $me (result i32)))
                (import "bayleaf" "animation_play" (func $play (param i32 i32 i32 f32 i32) (result i32)))
                (import "bayleaf" "joint_transform" (func $joint (param i32 i32 i32 i32) (result i32)))
                (memory (export "memory") 1)
                (data (i32.const 100) "Raise")
                (data (i32.const 110) "Hand")
                (data (i32.const 120) "found")
                (func (export "bayleaf_abi") (result i32) i32.const 1)
                (func (export "bayleaf_start") (drop (call $play (call $me) (i32.const 100) (i32.const 5) (f32.const 0) (i32.const 0))))
                (func (export "bayleaf_update") (param $dt f32)
                  (if (call $joint (call $me) (i32.const 110) (i32.const 4) (i32.const 200))
                    (then (if (f32.gt (f32.load (i32.const 204)) (f32.const 0.5)) (then (call $log (i32.const 120) (i32.const 5))))))))"#,
        )
        .unwrap()
    }

    #[test]
    fn a_script_plays_a_clip_and_reads_a_joint() {
        let (registry, mut session, id, dir) = arm_scene("script");
        std::fs::write(dir.join("player.wasm"), player_wasm()).unwrap();
        let run = |session: &mut Session, command: &str, params: Value| registry.execute(session, command, &params);
        run(&mut session, "scene.component.add", json!({"id": id, "component": "scripts"})).unwrap();
        run(&mut session, "scene.component.set", json!({"id": id, "component": "scripts", "value": [{"module": "player.wasm"}]})).unwrap();
        run(&mut session, "play.start", Value::Null).unwrap();
        run(&mut session, "play.step", json!({"seconds": 0.1, "steps": 12})).unwrap();
        assert!(
            arm_joint(&session, &id).y > 0.9 && arm_joint(&session, &id).x.abs() < 1e-3,
            "the script's Raise replaced Stretch: {}",
            arm_joint(&session, &id)
        );
        let found = session.console().iter().filter(|l| l.text.contains("found")).count();
        assert!(found > 0, "the script read the raised Hand joint: {:?}", session.console().iter().map(|l| &l.text).collect::<Vec<_>>());
        run(&mut session, "play.stop", Value::Null).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
