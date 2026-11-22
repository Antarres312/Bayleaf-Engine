//! Bringing files into a project: `asset.import` copies models, images and sounds from outside
//! (a drop from the system, or an agent) into a folder of the project, checking first that each
//! can be read, so nothing half-imported stays behind. Models and images get import settings beside
//! them (`asset.importSettings.get`/`.set`). `scene.file.place` puts a model or sound in the scene
//! as an object, and `scene.image.apply` makes an image an object's look.

use std::path::{Path, PathBuf};

use bayleaf_core::StableId;
use bayleaf_render::assets::import::{ImportSettings, settings_path};
use bayleaf_scene::components::SOUND_EXTENSIONS;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

/// Most files one `asset.import` brings in.
pub const MAX_IMPORTED: usize = 256;

/// What a file brought in is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Model,
    Image,
    Sound,
}

impl Kind {
    pub fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "glb" | "gltf" => Some(Kind::Model),
            e if bayleaf_render::assets::IMAGE_EXTENSIONS.contains(&e) => Some(Kind::Image),
            e if SOUND_EXTENSIONS.contains(&e) => Some(Kind::Sound),
            _ => None,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Kind::Model => "model",
            Kind::Image => "image",
            Kind::Sound => "sound",
        }
    }
}

/// Why a file cannot be brought in, by its extension.
fn not_importable(path: &Path) -> String {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    match extension.as_str() {
        "obj" | "fbx" | "dae" | "blend" | "3ds" | "stl" | "ply" => {
            format!("{name}: .{extension} models are not read yet; export the model as glTF (.glb) and bring that in")
        }
        "ktx2" | "exr" | "hdr" | "webp" | "tga" | "bmp" | "tif" | "tiff" | "gif" | "dds" | "psd" => {
            format!("{name}: .{extension} images are not read yet; save it as PNG or JPEG")
        }
        "flac" => format!("{name}: FLAC sounds are not read yet; save it as WAV or Ogg Vorbis"),
        "" => format!("{name}: a file without an extension cannot be brought in"),
        _ => format!(
            "{name}: .{extension} files cannot be brought in; models are glTF (.glb, .gltf), images PNG or JPEG, sounds {}",
            SOUND_EXTENSIONS.iter().map(|e| e.to_ascii_uppercase()).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// Reads the file as the engine would, so a broken one is refused before anything is copied.
fn check_readable(path: &Path, kind: Kind) -> Result<(), String> {
    match kind {
        Kind::Model => bayleaf_render::assets::load_model(path).map(|_| ()),
        Kind::Image => bayleaf_render::assets::load_image(path).map(|_| ()),
        Kind::Sound => check_sound(path),
    }
}

#[cfg(feature = "audio")]
fn check_sound(path: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if meta.len() > bayleaf_render::assets::MAX_FILE_BYTES {
        return Err(format!("{} is larger than {} MiB", path.display(), bayleaf_render::assets::MAX_FILE_BYTES >> 20));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    bayleaf_audio::Clip::decode(&bytes, &extension, 48_000).map(|_| ()).map_err(|e| format!("{}: {e}", path.display()))
}

/// Without sound built in, a sound is only checked for being a readable file of a fitting size.
#[cfg(not(feature = "audio"))]
fn check_sound(path: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if meta.len() > bayleaf_render::assets::MAX_FILE_BYTES {
        return Err(format!("{} is larger than {} MiB", path.display(), bayleaf_render::assets::MAX_FILE_BYTES >> 20));
    }
    Ok(())
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    let spec = |id, label, params, mutates, run| CommandSpec { id, label, menu: "", shortcut: "", params, prompt: Prompt::None, mutates, enabled: always, run };
    vec![
        spec(
            "asset.import",
            "Import Files",
            "{files: [path], folder?}: copies models (.glb, .gltf with the files it names), images (.png, .jpg) and sounds into a folder of the project (relative to the project folder; the project folder when left out), each read first, so a file that cannot be read says why and nothing is copied. A name already taken gets a number. Models and images get import settings beside them. A file already in the project stays where it is. Returns {imported: [{path, kind, source}]}, paths relative to the project folder",
            Mutates::No,
            asset_import,
        ),
        spec(
            "asset.importSettings.get",
            "Import Settings",
            "{path}: how a model or image of the project is read: {settings: {model?: {scale, upAxis: \"y\" | \"z\"}, image?: {mipmaps}}, saved}; saved is false while it has only the defaults",
            Mutates::No,
            import_settings_get,
        ),
        spec(
            "asset.importSettings.set",
            "Set Import Settings",
            "{path, settings: {model?: {scale, upAxis}, image?: {mipmaps}}}: saves how a model or image is read, beside it; scenes using it read it again. Returns {path} of the settings file",
            Mutates::No,
            import_settings_set,
        ),
        spec(
            "scene.file.place",
            "Place File",
            "{path, parent?, index?, position?: [x, y, z]}: a model or a sound of the project as a new object named after it (a Mesh drawing the model, or an Audio Source playing the sound), at position (world units; the origin when left out). Returns {id}",
            Mutates::Scene,
            file_place,
        ),
        spec(
            "scene.image.apply",
            "Apply Image",
            "{id, image}: an image of the project becomes the object's look: a sprite shows it; a mesh gets a new material beside the image that shows it (named after it). Returns {material?}: the material made",
            Mutates::Scene,
            image_apply,
        ),
    ]
}

/// `path` as scenes name it: relative to the project folder, with `/`.
fn relative(session: &Session, path: &Path) -> String {
    let base = session.script_dir();
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let (base, path) = (canonical(&base), canonical(path));
    path.strip_prefix(&base).unwrap_or(&path).to_string_lossy().replace('\\', "/").trim_start_matches("//?/").to_owned()
}

/// Whether `path` lies inside the project folder.
fn in_project(session: &Session, path: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    session.project_dir().is_some_and(|project| canonical(path).starts_with(canonical(project)))
}

/// A path in `folder` named `stem` (and `extension`, if any) that is not taken: "Crate", then
/// "Crate 2", "Crate 3"...
fn free_path(folder: &Path, stem: &str, extension: Option<&str>) -> Result<PathBuf, CommandError> {
    let name = |n: usize| {
        let stem = if n == 1 { stem.to_owned() } else { format!("{stem} {n}") };
        match extension {
            Some(e) => format!("{stem}.{e}"),
            None => stem,
        }
    };
    (1..1000)
        .map(|n| folder.join(name(n)))
        .find(|p| !p.exists() && !settings_path(p).exists())
        .ok_or_else(|| CommandError::Failed(format!("{} has too many files named {stem}", folder.display())))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportParams {
    files: Vec<PathBuf>,
    #[serde(default)]
    folder: PathBuf,
}

/// One file to bring in, once checked: where from, the files beside it it needs, and its kind.
struct Planned {
    source: PathBuf,
    kind: Kind,
    /// For a model: the files it names, relative to its folder.
    beside: Vec<PathBuf>,
}

fn asset_import(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ImportParams = parse(params)?;
    if p.files.is_empty() {
        return Err(CommandError::BadParams("name at least one file".to_owned()));
    }
    if p.files.len() > MAX_IMPORTED {
        return Err(CommandError::BadParams(format!("at most {MAX_IMPORTED} files at once")));
    }
    let folder = session.allowed(&session.script_dir().join(&p.folder), Access::Write)?;
    if !folder.is_dir() {
        return Err(CommandError::BadParams(format!("{} is not a folder", folder.display())));
    }
    // Everything is checked before anything is copied.
    let mut planned = Vec::new();
    for file in &p.files {
        // A relative path starts at the project folder, like every other path agents give.
        let source = session.allowed(&session.script_dir().join(file), Access::Read)?;
        if !source.is_file() {
            return Err(CommandError::BadParams(format!("{} is not a file", source.display())));
        }
        let kind = Kind::of(&source).ok_or_else(|| CommandError::Failed(not_importable(&source)))?;
        check_readable(&source, kind).map_err(CommandError::Failed)?;
        let beside = if kind == Kind::Model { bayleaf_render::assets::model_files(&source).map_err(CommandError::Failed)? } else { Vec::new() };
        let source_folder = source.parent().unwrap_or(Path::new("."));
        for extra in &beside {
            session.allowed(&source_folder.join(extra), Access::Read)?;
        }
        planned.push(Planned { source, kind, beside });
    }
    let mut created: Vec<PathBuf> = Vec::new();
    let mut imported = Vec::new();
    let result = (|| -> Result<(), CommandError> {
        for plan in &planned {
            let path = if in_project(session, &plan.source) {
                // Already in the project: it stays, and gains settings if it has none.
                plan.source.clone()
            } else {
                copy_in(session, plan, &folder, &mut created)?
            };
            if let Some(settings) = ImportSettings::defaults_for(&path)
                && !settings_path(&path).exists()
            {
                let at = session.allowed(&settings_path(&path), Access::Write)?;
                std::fs::write(&at, settings.to_text()).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", at.display())))?;
                created.push(at);
            }
            imported.push(json!({ "path": relative(session, &path), "kind": plan.kind.word(), "source": plan.source }));
        }
        Ok(())
    })();
    if let Err(problem) = result {
        // Nothing half-imported stays: newest first, so folders are empty when their turn comes.
        for path in created.iter().rev() {
            if path.is_dir() {
                let _ = std::fs::remove_dir(path);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }
        return Err(problem);
    }
    Ok(json!({ "imported": imported }))
}

/// Copies one checked file (and what it names beside it) into `folder`; a model with files beside
/// it gets a folder of its own, so their names cannot clash with others.
fn copy_in(session: &Session, plan: &Planned, folder: &Path, created: &mut Vec<PathBuf>) -> Result<PathBuf, CommandError> {
    let failed = |what: &Path, e: std::io::Error| CommandError::Failed(format!("cannot copy {}: {e}", what.display()));
    let stem = plan.source.file_stem().map_or_else(|| "Imported".to_owned(), |s| s.to_string_lossy().into_owned());
    let file_name = plan.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let extension = plan.source.extension().map(|e| e.to_string_lossy().into_owned());
    if plan.beside.is_empty() {
        let to = session.allowed(&free_path(folder, &stem, extension.as_deref())?, Access::Write)?;
        std::fs::copy(&plan.source, &to).map_err(|e| failed(&plan.source, e))?;
        created.push(to.clone());
        return Ok(to);
    }
    let own = session.allowed(&free_path(folder, &stem, None)?, Access::Write)?;
    std::fs::create_dir(&own).map_err(|e| failed(&own, e))?;
    created.push(own.clone());
    let to = own.join(&file_name);
    std::fs::copy(&plan.source, &to).map_err(|e| failed(&plan.source, e))?;
    created.push(to.clone());
    let source_folder = plan.source.parent().unwrap_or(Path::new("."));
    for extra in &plan.beside {
        let target = session.allowed(&own.join(extra), Access::Write)?;
        // Folders the model names its files in, made as needed.
        let mut missing: Vec<PathBuf> = target.parent().into_iter().flat_map(Path::ancestors).take_while(|d| !d.exists()).map(Path::to_path_buf).collect();
        missing.reverse();
        for dir in missing {
            std::fs::create_dir(&dir).map_err(|e| failed(&dir, e))?;
            created.push(dir);
        }
        let from = source_folder.join(extra);
        std::fs::copy(&from, &target).map_err(|e| failed(&from, e))?;
        created.push(target);
    }
    Ok(to)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    path: PathBuf,
}

/// A model or image of the project, checked for reading (or writing its settings).
fn settings_target(session: &Session, path: &Path, access: Access) -> Result<PathBuf, CommandError> {
    let file = session.allowed(&session.script_dir().join(path), Access::Read)?;
    if !file.is_file() {
        return Err(CommandError::BadParams(format!("{} is not a file", file.display())));
    }
    if ImportSettings::defaults_for(&file).is_none() {
        return Err(CommandError::BadParams(format!("{} has no import settings: only models and images do", file.display())));
    }
    session.allowed(&settings_path(&file), access)?;
    Ok(file)
}

fn import_settings_get(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PathParams = parse(params)?;
    let file = settings_target(session, &p.path, Access::Read)?;
    let settings = ImportSettings::read_for(&file).map_err(CommandError::Failed)?;
    Ok(json!({ "settings": settings, "saved": settings_path(&file).is_file() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetParams {
    path: PathBuf,
    settings: Value,
}

fn import_settings_set(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: SetParams = parse(params)?;
    let file = settings_target(session, &p.path, Access::Write)?;
    let mut settings = ImportSettings::defaults_for(&file).ok_or_else(|| CommandError::Failed("no import settings for this file".to_owned()))?;
    let given = p.settings.as_object().ok_or_else(|| CommandError::BadParams("settings must be an object".to_owned()))?;
    // Only the section for the file's kind; it may leave out values, which keep the defaults.
    for (key, value) in given {
        match key.as_str() {
            "model" if settings.model.is_some() => {
                let mut current = serde_json::to_value(&settings.model).unwrap_or_default();
                merge(&mut current, value)?;
                settings.model = Some(serde_json::from_value(current).map_err(|e| CommandError::BadParams(format!("model: {e}")))?);
            }
            "image" if settings.image.is_some() => {
                let mut current = serde_json::to_value(&settings.image).unwrap_or_default();
                merge(&mut current, value)?;
                settings.image = Some(serde_json::from_value(current).map_err(|e| CommandError::BadParams(format!("image: {e}")))?);
            }
            other => return Err(CommandError::BadParams(format!("{} has no {other:?} import settings", file.display()))),
        }
    }
    settings.validate().map_err(CommandError::BadParams)?;
    let path = settings_path(&file);
    std::fs::write(&path, settings.to_text()).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", path.display())))?;
    Ok(json!({ "path": path }))
}

/// Sets each field of `from` on `into` (both objects).
fn merge(into: &mut Value, from: &Value) -> Result<(), CommandError> {
    let (Some(into), Some(from)) = (into.as_object_mut(), from.as_object()) else {
        return Err(CommandError::BadParams("each part of the settings must be an object".to_owned()));
    };
    for (key, value) in from {
        into.insert(key.clone(), value.clone());
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceParams {
    path: PathBuf,
    #[serde(default)]
    parent: Option<StableId>,
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    position: Option<[f64; 3]>,
}

fn file_place(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: PlaceParams = parse(params)?;
    let file = session.allowed(&session.script_dir().join(&p.path), Access::Read)?;
    if !file.is_file() {
        return Err(CommandError::BadParams(format!("{} is not a file", file.display())));
    }
    if p.position.is_some_and(|v| !v.iter().all(|c| c.is_finite() && c.abs() <= 1e12)) {
        return Err(CommandError::BadParams("position must be three finite numbers".to_owned()));
    }
    let name = file.file_stem().map_or_else(|| "Object".to_owned(), |s| s.to_string_lossy().into_owned());
    let named = relative(session, &file);
    let (component, value) = match Kind::of(&file) {
        Some(Kind::Model) => ("mesh", json!({ "model": named })),
        Some(Kind::Sound) => ("audioSource", json!({ "clip": named })),
        Some(Kind::Image) => return Err(CommandError::BadParams("an image is not placed on its own: apply it to an object with scene.image.apply".to_owned())),
        None => return Err(CommandError::BadParams(format!("{} is not a model or a sound", file.display()))),
    };
    let id = session.scene.create_object(p.parent, &name)?;
    let made = (|| -> Result<(), CommandError> {
        if p.index.is_some() {
            session.scene.reparent(id, p.parent, p.index)?;
        }
        session.scene.add_component(id, component)?;
        let mut current = session.scene.component(id, component)?;
        merge(&mut current, &value)?;
        session.scene.set_component(id, component, current)?;
        if let Some(position) = p.position {
            let mut transform = session.scene.component(id, "transform")?;
            transform["position"] = json!(position);
            session.scene.set_component(id, "transform", transform)?;
        }
        Ok(())
    })();
    if let Err(problem) = made {
        let _ = session.scene.delete_object(id);
        return Err(problem);
    }
    Ok(json!({ "id": id }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyParams {
    id: StableId,
    image: PathBuf,
}

fn image_apply(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: ApplyParams = parse(params)?;
    let image = session.allowed(&session.script_dir().join(&p.image), Access::Read)?;
    if Kind::of(&image) != Some(Kind::Image) || !image.is_file() {
        return Err(CommandError::BadParams(format!("{} is not an image of the project (.png or .jpg)", image.display())));
    }
    let named = relative(session, &image);
    if session.scene.component(p.id, "sprite").is_ok() {
        let mut sprite = session.scene.component(p.id, "sprite")?;
        sprite["image"] = json!(named);
        session.scene.set_component(p.id, "sprite", sprite)?;
        return Ok(json!({}));
    }
    let mut mesh = session
        .scene
        .component(p.id, "mesh")
        .map_err(|_| CommandError::BadParams("the object has neither a mesh nor a sprite to show the image".to_owned()))?;
    let stem = image.file_stem().map_or_else(|| "Image".to_owned(), |s| s.to_string_lossy().into_owned());
    let folder = image.parent().unwrap_or(Path::new("."));
    let material = session.allowed(&free_path(folder, &stem, Some(bayleaf_render::material::CATALOG.extension))?, Access::Write)?;
    std::fs::write(&material, image_material(&named)?).map_err(|e| CommandError::Failed(format!("cannot write {}: {e}", material.display())))?;
    let material_named = relative(session, &material);
    mesh["material"] = json!(material_named);
    if let Err(problem) = session.scene.set_component(p.id, "mesh", mesh) {
        let _ = std::fs::remove_file(&material);
        return Err(problem.into());
    }
    Ok(json!({ "material": material_named }))
}

/// A material graph showing `image` (as scenes name it) as the surface's colour.
fn image_material(image: &str) -> Result<String, CommandError> {
    use bayleaf_graph::{Graph, Link, PinRef};
    let catalog = &bayleaf_render::material::CATALOG;
    let broken = |e: bayleaf_graph::GraphError| CommandError::Failed(format!("cannot make the material: {e}"));
    let mut graph = Graph::new(catalog);
    let output = graph.nodes.iter().find(|n| n.kind == "output").map(|n| n.id).ok_or_else(|| CommandError::Failed("the material has no output".to_owned()))?;
    let texture = graph.add_node(catalog, "texture", [80.0, 40.0]).map_err(broken)?;
    graph.set_value(catalog, texture, "image", json!(image)).map_err(broken)?;
    graph.connect(catalog, Link { from: PinRef::new(texture, "color"), to: PinRef::new(output, "baseColor") }).map_err(broken)?;
    Ok(graph.to_json())
}
