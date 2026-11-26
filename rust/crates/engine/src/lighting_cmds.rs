//! A scene's settings (`scene.settings`, `scene.settings.set`) and lighting assets (`lighting.get`,
//! `lighting.set`), and moving an older scene's Environment component into a lighting asset when
//! the scene opens.

use std::path::{Path, PathBuf};

use bayleaf_scene::components::Environment;
use bayleaf_scene::lighting::{self, Lighting, Sun};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Level, Mutates, Prompt, Session, always, parse};

pub(crate) fn commands() -> Vec<CommandSpec> {
    let spec = |id, label, params, mutates, run| CommandSpec { id, label, menu: "", shortcut: "", params, prompt: Prompt::None, mutates, enabled: always, run };
    vec![
        spec("scene.settings", "Scene Settings", "none; returns the open scene's own settings: {lighting}", Mutates::No, settings_get),
        spec(
            "scene.settings.set",
            "Change Scene Settings",
            "{lighting?, postProcessing?}: the lighting asset (.blight, relative to the project folder; empty for the engine's light) and the post-processing asset (.bpost; empty for none) the scene uses; returns the settings",
            Mutates::Scene,
            settings_set,
        ),
        spec(
            "lighting.get",
            "Read Lighting",
            "{path}: a lighting asset (.blight); returns {lighting: {skyColor, horizonColor, groundColor, ambient, showSky, fogDistance, sun: {enabled, rotation, color, intensity, shadows}}}",
            Mutates::No,
            lighting_get,
        ),
        spec(
            "lighting.set",
            "Change Lighting",
            "{path, lighting}: writes a lighting asset (the whole lighting, as lighting.get returns it); every scene that names it changes",
            Mutates::No,
            lighting_set,
        ),
        spec(
            "post.get",
            "Read Post-processing",
            "{path}: a post-processing asset (.bpost); returns {post: {toneMapping, bloom, color (with lookup, lookupStrength), vignette, depthFog, occlusion, antialiasing}}",
            Mutates::No,
            post_get,
        ),
        spec(
            "post.set",
            "Change Post-processing",
            "{path, post}: writes a post-processing asset (all of it, as post.get returns it); every scene and camera that names it changes. A colour lookup image (color.lookup) of the wrong size is refused; one not there yet is a warning",
            Mutates::No,
            post_set,
        ),
        spec(
            "post.lookup.neutral",
            "Make Neutral Lookup",
            "{path, steps?}: writes a colour lookup image (.png) that changes no colour, steps (2 to 64, default 16) squares of steps by steps side by side, to recolour in an image editor and name in color.lookup; returns {path, relative} (relative to the project). Refuses a file that is already there",
            Mutates::No,
            lookup_neutral,
        ),
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NeutralParams {
    path: PathBuf,
    #[serde(default = "default_steps")]
    steps: u32,
}

fn default_steps() -> u32 {
    16
}

/// The pixels of a lookup image that maps every colour to itself.
fn neutral_lookup(steps: u32) -> Vec<u8> {
    let level = |v: u32| ((v * 255 + (steps - 1) / 2) / (steps - 1)) as u8;
    let mut pixels = Vec::with_capacity((steps * steps * steps * 4) as usize);
    for green in 0..steps {
        for blue in 0..steps {
            for red in 0..steps {
                pixels.extend_from_slice(&[level(red), level(green), level(blue), 255]);
            }
        }
    }
    pixels
}

fn lookup_neutral(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: NeutralParams = parse(params)?;
    let max = bayleaf_scene::post::MAX_LOOKUP_STEPS;
    if !(2..=max).contains(&p.steps) {
        return Err(CommandError::BadParams(format!("steps must be between 2 and {max}")));
    }
    if !p.path.to_string_lossy().to_ascii_lowercase().ends_with(".png") {
        return Err(CommandError::BadParams("give a .png file".to_owned()));
    }
    let path = session.allowed(&session.script_dir().join(&p.path), Access::Write)?;
    if path.exists() {
        return Err(CommandError::BadParams(format!("{} is already there", path.display())));
    }
    let image = bayleaf_render::Image { width: p.steps * p.steps, height: p.steps, pixels: neutral_lookup(p.steps) };
    image.write_png(&path).map_err(|e| CommandError::Failed(e.to_string()))?;
    let relative = path.strip_prefix(session.script_dir()).unwrap_or(&path).to_string_lossy().replace('\\', "/");
    Ok(json!({ "path": path, "relative": relative }))
}

/// A post-processing asset's path, relative to the project folder or absolute, checked for `access`.
fn post_path(session: &Session, path: &Path, access: Access) -> Result<PathBuf, CommandError> {
    if !path.to_string_lossy().to_ascii_lowercase().ends_with(&format!(".{}", bayleaf_scene::post::EXTENSION)) {
        return Err(CommandError::BadParams(format!("give a .{} file", bayleaf_scene::post::EXTENSION)));
    }
    session.allowed(&session.script_dir().join(path), access)
}

fn post_get(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    let path = post_path(session, &p.path, Access::Read)?;
    let post = bayleaf_scene::post::PostProcessing::load(&path)?;
    Ok(json!({ "post": post }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PostSetParams {
    path: PathBuf,
    post: bayleaf_scene::post::PostProcessing,
}

fn post_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PostSetParams = parse(params)?;
    p.post.validate().map_err(CommandError::BadParams)?;
    let path = post_path(session, &p.path, Access::Write)?;
    let warnings = lookup_check(session, &p.post)?;
    p.post.save(&path)?;
    Ok(json!({ "path": path, "warnings": warnings }))
}

/// Refuses a colour lookup image of the wrong size; one that is not there yet is a warning.
fn lookup_check(session: &Session, post: &bayleaf_scene::post::PostProcessing) -> Result<Vec<String>, CommandError> {
    let name = &post.color.lookup;
    if name.is_empty() {
        return Ok(Vec::new());
    }
    let image_path = session.allowed(&session.script_dir().join(name), Access::Read)?;
    if !image_path.is_file() {
        return Ok(vec![format!("color.lookup {name} is not there yet; colours change once it is")]);
    }
    let image = bayleaf_render::assets::load_image(&image_path).map_err(|e| CommandError::BadParams(format!("color.lookup {name}: {e}")))?;
    if !bayleaf_scene::post::is_lookup_size(image.width, image.height) {
        return Err(CommandError::BadParams(format!(
            "color.lookup {name} is {} by {}; a lookup image is N squares of N by N side by side (N times N wide, N high, N from 2 to {})",
            image.width,
            image.height,
            bayleaf_scene::post::MAX_LOOKUP_STEPS
        )));
    }
    Ok(Vec::new())
}

fn settings_get(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    parse::<crate::play::NoParams>(params)?;
    serde_json::to_value(session.scene.settings()).map_err(|e| CommandError::Failed(e.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsParams {
    #[serde(default)]
    lighting: Option<String>,
    #[serde(default, rename = "postProcessing")]
    post_processing: Option<String>,
}

fn settings_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SettingsParams = parse(params)?;
    let mut settings = session.scene.settings().clone();
    if let Some(lighting) = p.lighting {
        settings.lighting = lighting.trim().replace('\\', "/");
    }
    if let Some(post) = p.post_processing {
        settings.post_processing = post.trim().replace('\\', "/");
    }
    settings.validate().map_err(CommandError::BadParams)?;
    session.scene.set_settings(settings)?;
    serde_json::to_value(session.scene.settings()).map_err(|e| CommandError::Failed(e.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    path: PathBuf,
}

/// A lighting asset's path, relative to the project folder or absolute, checked for `access`.
fn lighting_path(session: &Session, path: &Path, access: Access) -> Result<PathBuf, CommandError> {
    if !path.to_string_lossy().to_ascii_lowercase().ends_with(&format!(".{}", lighting::EXTENSION)) {
        return Err(CommandError::BadParams(format!("give a .{} file", lighting::EXTENSION)));
    }
    session.allowed(&session.script_dir().join(path), access)
}

fn lighting_get(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    let path = lighting_path(session, &p.path, Access::Read)?;
    let lighting = Lighting::load(&path)?;
    Ok(json!({ "lighting": lighting }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetParams {
    path: PathBuf,
    lighting: Lighting,
}

fn lighting_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SetParams = parse(params)?;
    p.lighting.validate().map_err(CommandError::BadParams)?;
    let path = lighting_path(session, &p.path, Access::Write)?;
    p.lighting.save(&path)?;
    Ok(json!({ "path": path }))
}

/// A file name in `dir` from `stem` that is not taken yet: `stem.blight`, `stem 2.blight`, …
fn free_path(dir: &Path, stem: &str) -> PathBuf {
    (1..)
        .map(|n| dir.join(if n == 1 { format!("{stem}.{}", lighting::EXTENSION) } else { format!("{stem} {n}.{}", lighting::EXTENSION) }))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dir.join(format!("{stem}.{}", lighting::EXTENSION)))
}

/// Moves an older scene's Environment component into a lighting asset beside the scene, which the
/// scene then names, and takes the component away; the scene is left changed, for saving. A scene
/// that already names a lighting asset only loses its Environment. Says what it did, or why it
/// left the scene as it was; `None` when the scene has no Environment.
pub(crate) fn move_environment(session: &mut Session) -> Option<(Level, String, bool)> {
    let found: Vec<_> = session.scene.world_matrices().into_iter().map(|(id, _)| id).filter(|id| session.scene.get::<Environment>(*id).is_some()).collect();
    let first = session.scene.get::<Environment>(*found.first()?)?.clone();
    if session.scene.settings().lighting.is_empty() {
        let scene_path = session.scene_path()?.to_path_buf();
        let dir = scene_path.parent()?.to_path_buf();
        let stem = scene_path.file_stem().map_or_else(|| "Scene".to_owned(), |s| s.to_string_lossy().into_owned());
        let target = free_path(&dir, &format!("{stem} lighting"));
        let target = match session.allowed(&target, Access::Write) {
            Ok(path) => path,
            Err(e) => return Some((Level::Info, format!("The scene's Environment stays as it is: its lighting asset cannot be written ({e})."), false)),
        };
        // The old scene's own lights stay its light: the asset's sun is off.
        let moved = Lighting::from_environment(&first, Sun { enabled: false, ..Sun::default() });
        if let Err(e) = moved.save(&target) {
            return Some((Level::Error, format!("The scene's Environment stays as it is: {e}"), false));
        }
        let relative = target.strip_prefix(session.script_dir()).unwrap_or(&target).to_string_lossy().replace('\\', "/");
        let mut settings = session.scene.settings().clone();
        settings.lighting = relative.clone();
        if session.scene.set_settings(settings).is_err() {
            return None;
        }
        for id in &found {
            let _ = session.scene.remove_component(*id, "environment");
        }
        return Some((Level::Info, format!("Moved the scene's Environment into the lighting asset {relative}; save the scene to keep it."), true));
    }
    for id in &found {
        let _ = session.scene.remove_component(*id, "environment");
    }
    Some((Level::Info, "The scene names a lighting asset, so its old Environment was taken away; save the scene to keep it.".to_owned(), true))
}
