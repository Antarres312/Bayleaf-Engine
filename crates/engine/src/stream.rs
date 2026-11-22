//! World streaming in play mode (the `streaming` feature, `crates/streaming`): Streamed Area
//! components add their scenes under themselves, read in the background, when a camera comes
//! near, and remove them again when every camera has gone; and `stream.split` cuts the open
//! scene into such areas on a grid. Builds without the feature say so instead.

use std::collections::BTreeMap;

use bayleaf_core::StableId;
use bayleaf_core::math::DVec3;
use bayleaf_scene::Scene;
use bayleaf_scene::components::{Camera, DirectionalLight, Environment, GameRules, PlayerControl, PlayerStart, StreamedArea, Transform};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::confine::Access;
use crate::play::{NoParams, Play, playing};
use crate::{CommandError, CommandSpec, Level, Mutates, Prompt, PromptField, Session, not_playing, parse};

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "stream.areas",
            label: "Streamed Areas",
            menu: "",
            shortcut: "",
            params: "none; returns {areas: [{id, name, scene, loadDistance, keepMargin, state, objects, problem?}]}: every Streamed Area in the playing scene; state is away, waiting, reading, here or failed",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| playing(s),
            run: areas_command,
        },
        CommandSpec {
            id: "stream.wait",
            label: "Wait for Streamed Areas",
            menu: "",
            shortcut: "",
            params: "{seconds?}: waits (at most seconds, default 10, up to 60) until no Streamed Area is being read, and adds those that arrived; returns {here, busy}",
            prompt: Prompt::None,
            mutates: Mutates::No,
            enabled: |s| playing(s),
            run: wait_command,
        },
        CommandSpec {
            id: "stream.split",
            label: "Split into Streamed Areas…",
            menu: "Object/Split into Streamed Areas…",
            shortcut: "",
            params: "{cellSize, folder, loadDistance?, keepMargin?}: moves the open scene's top objects into one .bscene per grid cell (cellSize wide, by X and Z) in folder (relative to the project; the files must not be there yet), and puts a Streamed Area for each cell under a new Streamed World object. Cameras, sun lights, environment, game rules, player starts and players stay. loadDistance defaults to 1.5 cells, keepMargin to a quarter cell; returns {world, areas: [{id, scene, objects}], moved}",
            prompt: Prompt::Fields(SPLIT_FIELDS),
            mutates: Mutates::Scene,
            enabled: not_playing,
            run: split_command,
        },
    ]
}

const SPLIT_FIELDS: &[PromptField] = &[
    PromptField { name: "cellSize", label: "Cell Size", default: "100", number: true },
    PromptField { name: "folder", label: "Folder", default: "cells", number: false },
];

/// The Streamed Areas and the cameras' places in `scene`.
fn gather(scene: &Scene) -> (Vec<(StableId, DVec3, StreamedArea)>, Vec<DVec3>) {
    let mut areas = Vec::new();
    let mut cameras = Vec::new();
    for (id, matrix) in scene.world_matrices() {
        let at = matrix.w_axis.truncate();
        if let Some(area) = scene.get::<StreamedArea>(id) {
            areas.push((id, at, area.clone()));
        }
        if scene.get::<Camera>(id).is_some() {
            cameras.push(at);
        }
    }
    (areas, cameras)
}

fn areas_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<NoParams>(params)?;
    let (areas, _) = gather(&session.scene);
    let known = imp::states(session).map_err(CommandError::Failed)?;
    let listed: Vec<Value> = areas
        .into_iter()
        .map(|(id, _, area)| {
            let (state, objects, problem) = known.get(&id).cloned().unwrap_or(("away", Vec::new(), None));
            let mut entry = json!({
                "id": id,
                "name": session.scene.name(id).unwrap_or("Object"),
                "scene": area.scene,
                "loadDistance": area.load_distance,
                "keepMargin": area.keep_margin,
                "state": state,
                "objects": objects,
            });
            if let (Some(problem), Some(map)) = (problem, entry.as_object_mut()) {
                map.insert("problem".to_owned(), json!(problem));
            }
            entry
        })
        .collect();
    Ok(json!({ "areas": listed }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WaitParams {
    #[serde(default = "ten")]
    seconds: f64,
}

fn ten() -> f64 {
    10.0
}

fn wait_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: WaitParams = parse(params)?;
    if !(p.seconds.is_finite() && (0.0..=60.0).contains(&p.seconds)) {
        return Err(CommandError::BadParams("seconds must be between 0 and 60".to_owned()));
    }
    let (here, busy) = imp::wait(session, std::time::Duration::from_secs_f64(p.seconds)).map_err(CommandError::Failed)?;
    Ok(json!({ "here": here, "busy": busy }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SplitParams {
    cell_size: f64,
    folder: String,
    #[serde(default)]
    load_distance: Option<f64>,
    #[serde(default)]
    keep_margin: Option<f64>,
}

/// Whether a top object belongs everywhere rather than in one cell.
fn stays(scene: &Scene, id: StableId) -> bool {
    scene.get::<Camera>(id).is_some()
        || scene.get::<DirectionalLight>(id).is_some()
        || scene.get::<Environment>(id).is_some()
        || scene.get::<GameRules>(id).is_some()
        || scene.get::<PlayerStart>(id).is_some()
        || scene.get::<PlayerControl>(id).is_some()
        || scene.get::<StreamedArea>(id).is_some()
}

fn split_command(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SplitParams = parse(params)?;
    let size = p.cell_size;
    if !(size.is_finite() && (1.0..=1.0e6).contains(&size)) {
        return Err(CommandError::BadParams("cellSize must be between 1 and 1000000".to_owned()));
    }
    let area = StreamedArea { scene: String::new(), load_distance: p.load_distance.unwrap_or(size * 1.5), keep_margin: p.keep_margin.unwrap_or(size * 0.25) };
    let folder = p.folder.trim_matches(['/', '\\']).replace('\\', "/");
    if folder.is_empty() || folder.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(CommandError::BadParams("folder must be a folder inside the project".to_owned()));
    }
    let mut cells: BTreeMap<(i64, i64), Vec<StableId>> = BTreeMap::new();
    for &id in session.scene.children(None) {
        if stays(&session.scene, id) {
            continue;
        }
        let at = session.scene.get::<Transform>(id).map(|t| DVec3::from_array(t.position)).unwrap_or_default();
        cells.entry(((at.x / size).floor() as i64, (at.z / size).floor() as i64)).or_default().push(id);
    }
    if cells.is_empty() {
        return Err(CommandError::Failed(
            "nothing to split: the scene's top objects all belong everywhere (cameras, sun lights, environment, rules, players)".to_owned(),
        ));
    }
    // Check every file first, so a refusal leaves no files behind.
    let base = session.script_dir();
    let mut planned = Vec::new();
    for (&(x, z), objects) in &cells {
        let relative = format!("{folder}/cell_{x}_{z}.bscene");
        let path = session.allowed(&base.join(&relative), Access::Write)?;
        if path.exists() {
            return Err(CommandError::Failed(format!("{relative} is already there; choose another folder")));
        }
        let mut probe = area.clone();
        probe.scene = relative.clone();
        bayleaf_scene::components::SceneComponent::validate(&probe).map_err(CommandError::BadParams)?;
        let centre = DVec3::new((x as f64 + 0.5) * size, 0.0, (z as f64 + 0.5) * size);
        planned.push((relative, path, centre, objects.clone()));
    }
    for (relative, path, centre, objects) in &planned {
        let mut cell = Scene::new();
        let roots = cell.import(&session.scene.export(objects)?, None, None)?;
        for root in roots {
            let mut transform = cell.get::<Transform>(root).cloned().unwrap_or_default();
            transform.position = (DVec3::from_array(transform.position) - *centre).to_array();
            cell.set(root, transform)?;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| CommandError::Failed(format!("cannot make {}: {e}", parent.display())))?;
        }
        cell.save(path).map_err(|e| CommandError::Failed(format!("{relative}: {e}")))?;
    }
    let world = session.scene.create_object(None, "Streamed World")?;
    let mut listed = Vec::new();
    let mut moved = 0;
    for (relative, _, centre, objects) in planned {
        for id in &objects {
            moved += session.scene.delete_object(*id)?;
        }
        let (x, z) = (centre.x, centre.z);
        let id = session.scene.create_object(Some(world), &format!("Cell {}, {}", (x / size - 0.5).round(), (z / size - 0.5).round()))?;
        session.scene.set(id, Transform { position: centre.to_array(), ..Transform::default() })?;
        session.scene.set(id, StreamedArea { scene: relative.clone(), ..area.clone() })?;
        listed.push(json!({ "id": id, "scene": relative, "objects": objects.len() }));
    }
    session.log(Level::Info, format!("Split {moved} objects into {} Streamed Areas in {folder}.", listed.len()));
    Ok(json!({ "world": world, "areas": listed, "moved": moved }))
}

pub(crate) use imp::{clear, update};

#[cfg(feature = "streaming")]
mod imp {
    use std::collections::HashMap;
    use std::time::Duration;

    use bayleaf_core::StableId;
    use bayleaf_streaming::{Area, Arrival, State};

    use super::{Play, gather};
    use crate::confine::Access;
    use crate::{Level, Session};

    /// What `stream.areas` shows for an area that is not away: its state, objects and problem.
    pub(super) type Known = HashMap<StableId, (&'static str, Vec<StableId>, Option<String>)>;

    pub(crate) fn clear(play: &mut Play) {
        play.streamer.clear();
    }

    /// Adds the areas that arrived, then brings and takes areas as the cameras moved.
    pub(crate) fn update(session: &mut Session) {
        let Some(play) = session.play.as_mut() else { return };
        let mut streamer = std::mem::take(&mut play.streamer);
        let arrived = streamer.arrived();
        place(session, &mut streamer, arrived);
        let (found, cameras) = gather(&session.scene);
        if !found.is_empty()
            && cameras.is_empty()
            && let Some(play) = session.play.as_mut()
            && !std::mem::replace(&mut play.told_streaming, true)
        {
            session.log(Level::Info, "Streamed Areas wait for a camera: the scene has none.".to_owned());
        }
        let areas: Vec<Area> = found
            .into_iter()
            .map(|(id, at, area)| Area { id, at, scene: area.scene, load_distance: area.load_distance, keep_margin: area.keep_margin })
            .collect();
        let plan = streamer.plan(&areas, &cameras);
        for root in plan.remove.into_iter().flat_map(|(_, roots)| roots) {
            if session.scene.contains(root) {
                let _ = session.scene.delete_object(root);
            }
        }
        let base = session.script_dir();
        for (id, scene) in plan.read {
            match session.allowed(&base.join(&scene), Access::Read) {
                Ok(path) => streamer.read(id, path),
                Err(e) => {
                    log_failure(session, id, &scene, &e.to_string());
                    streamer.fail(id, e.to_string());
                }
            }
        }
        if let Some(play) = session.play.as_mut() {
            play.streamer = streamer;
        }
    }

    fn place(session: &mut Session, streamer: &mut bayleaf_streaming::Streamer, arrived: Vec<Arrival>) {
        let scenes: HashMap<StableId, String> = streamer.states().map(|(id, scene, _)| (id, scene.to_owned())).collect();
        for (id, result) in arrived {
            let scene = scenes.get(&id).cloned().unwrap_or_default();
            let added = result.and_then(|loaded| {
                if !session.scene.contains(id) {
                    return Err("its object is gone".to_owned());
                }
                crate::play::add_scene(session, &loaded, Some(id))
            });
            match added {
                Ok(roots) => streamer.placed(id, roots),
                Err(why) => {
                    log_failure(session, id, &scene, &why);
                    streamer.fail(id, why);
                }
            }
        }
    }

    fn log_failure(session: &mut Session, id: StableId, scene: &str, why: &str) {
        let who = session.scene.name(id).unwrap_or("Object").to_owned();
        session.log(Level::Error, format!("[{who} · Streamed Area] {scene} could not be added: {why}"));
    }

    pub(super) fn states(session: &Session) -> Result<Known, String> {
        let play = session.play.as_ref().ok_or("play mode is not running")?;
        Ok(play
            .streamer
            .states()
            .map(|(id, _, state)| {
                let objects = if let State::Here(roots) = state { roots.clone() } else { Vec::new() };
                let problem = if let State::Failed(why) = state { Some(why.clone()) } else { None };
                (id, (state.word(), objects, problem))
            })
            .collect())
    }

    pub(super) fn wait(session: &mut Session, timeout: Duration) -> Result<(usize, usize), String> {
        let play = session.play.as_mut().ok_or("play mode is not running")?;
        let mut streamer = std::mem::take(&mut play.streamer);
        let arrived = streamer.wait(timeout);
        place(session, &mut streamer, arrived);
        let here = streamer.states().filter(|(_, _, s)| matches!(s, State::Here(_))).count();
        let busy = streamer.busy();
        if let Some(play) = session.play.as_mut() {
            play.streamer = streamer;
        }
        Ok((here, busy))
    }
}

#[cfg(not(feature = "streaming"))]
mod imp {
    use std::collections::HashMap;
    use std::time::Duration;

    use bayleaf_core::StableId;
    use bayleaf_scene::components::StreamedArea;

    use super::Play;
    use crate::{Level, Session};

    pub(super) type Known = HashMap<StableId, (&'static str, Vec<StableId>, Option<String>)>;

    const NONE: &str = "this build has no world streaming";

    pub(crate) fn clear(_: &mut Play) {}

    /// Says once that Streamed Areas do nothing in this build.
    pub(crate) fn update(session: &mut Session) {
        let any = session.scene.world_matrices().into_iter().any(|(id, _)| session.scene.get::<StreamedArea>(id).is_some());
        if any
            && let Some(play) = session.play.as_mut()
            && !std::mem::replace(&mut play.told_streaming, true)
        {
            session.log(Level::Info, "This build has no world streaming, so Streamed Areas stay empty.".to_owned());
        }
    }

    pub(super) fn states(_: &Session) -> Result<Known, String> {
        Err(NONE.to_owned())
    }

    pub(super) fn wait(_: &mut Session, _: Duration) -> Result<(usize, usize), String> {
        Err(NONE.to_owned())
    }
}
