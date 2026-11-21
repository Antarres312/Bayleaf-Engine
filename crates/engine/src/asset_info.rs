//! `asset.info`: what a project file is and holds, for the Inspector and for agents. Reads only;
//! each kind of file says what matters about it (an image its size, a scene its objects, a graph
//! its nodes and problems, a text file its first lines).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::confine::Access;
use crate::{CommandError, CommandSpec, Mutates, Prompt, Session, always, parse};

/// Text shown from a text file at most.
pub const TEXT_PREVIEW_BYTES: usize = 4096;

pub(crate) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "asset.info",
        label: "File Info",
        menu: "",
        shortcut: "",
        params: "{path}: a file or folder, relative to the project folder or absolute; returns {path, kind, bytes, rows: [[label, value]], problems, text?}",
        prompt: Prompt::None,
        mutates: Mutates::No,
        enabled: always,
        run: asset_info,
    }]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Params {
    path: PathBuf,
}

fn asset_info(session: &mut Session, params: &Value) -> Result<Value, CommandError> {
    let p: Params = parse(params)?;
    let path = session.allowed(&session.script_dir().join(&p.path), Access::Read)?;
    Ok(describe(session, &path).to_json(&path))
}

/// What is known about one file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    /// What kind of file it is, e.g. "Image".
    pub kind: &'static str,
    pub bytes: u64,
    /// Label and value pairs, in the order to show them.
    pub rows: Vec<(String, String)>,
    /// What is wrong with the file, if anything.
    pub problems: Vec<String>,
    /// The start of a text file.
    pub text: Option<String>,
}

impl Info {
    fn row(&mut self, label: &str, value: impl ToString) {
        self.rows.push((label.to_owned(), value.to_string()));
    }

    fn to_json(&self, path: &Path) -> Value {
        json!({ "path": path, "kind": self.kind, "bytes": self.bytes, "rows": self.rows, "problems": self.problems, "text": self.text })
    }
}

/// File extensions shown as text.
const TEXT_EXTENSIONS: &[&str] = &["txt", "md", "rs", "toml", "json", "wgsl", "wat", "ron", "yaml", "yml", "csv", "gitignore"];

/// Reads `path` (already allowed) and says what it is.
pub fn describe(session: &Session, path: &Path) -> Info {
    let mut info = Info::default();
    let Ok(meta) = std::fs::metadata(path) else {
        info.kind = "Missing";
        info.problems.push(format!("{} does not exist", path.display()));
        return info;
    };
    info.bytes = meta.len();
    if meta.is_dir() {
        return folder(path);
    }
    let extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut result = match extension.as_str() {
        "png" | "jpg" | "jpeg" => image(path),
        "glb" | "gltf" => model(path),
        "bscene" => scene(path),
        "bprefab" => prefab(session, path),
        "bmaterial" | "bscript" => graph(path, &extension),
        "wasm" => script(path),
        "bproject" => project(path),
        "binput" => input_map(session, path),
        e if TEXT_EXTENSIONS.contains(&e) || name.starts_with('.') => text(path, "Text"),
        _ => Info { kind: "File", ..Info::default() },
    };
    result.bytes = info.bytes;
    result
}

fn folder(path: &Path) -> Info {
    let mut info = Info { kind: "Folder", ..Info::default() };
    if path.join("Cargo.toml").is_file() {
        info = text(&path.join("Cargo.toml"), "Script Source");
        info.row("Build", "Right-click the folder in the Project panel and pick Build Script");
        return info;
    }
    match std::fs::read_dir(path) {
        Ok(entries) => {
            let (mut files, mut folders) = (0, 0);
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir()) { folders += 1 } else { files += 1 }
            }
            info.row("Files", files);
            info.row("Folders", folders);
        }
        Err(e) => info.problems.push(format!("cannot read the folder: {e}")),
    }
    info
}

fn image(path: &Path) -> Info {
    let mut info = Info { kind: "Image", ..Info::default() };
    match bayleaf_render::assets::load_image(path) {
        Ok(image) => {
            info.row("Size", format!("{} × {} pixels", image.width, image.height));
            let opaque = image.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255);
            info.row("Transparency", if opaque { "None" } else { "Yes" });
        }
        Err(e) => info.problems.push(e),
    }
    info
}

fn model(path: &Path) -> Info {
    let mut info = Info { kind: "Model", ..Info::default() };
    match bayleaf_render::assets::load_model(path) {
        Ok(mesh) => {
            info.row("Vertices", mesh.positions.len());
            info.row("Triangles", mesh.indices.len() / 3);
            let size: Vec<String> = (0..3).map(|i| format!("{:.2}", mesh.max[i] - mesh.min[i])).collect();
            info.row("Size", format!("{} m", size.join(" × ")));
            info.row("Texture coordinates", if mesh.uvs.is_empty() { "No" } else { "Yes" });
            if mesh.parts.len() > 1 {
                info.row("Parts", format!("{}, each with its own material", mesh.parts.len()));
            }
            if let Some(m) = &mesh.material {
                let image = m.image.as_ref().map_or_else(|| "no image".to_owned(), |i| format!("a {} × {} image", i.width, i.height));
                let normal = if m.normal.is_some() { ", a normal map" } else { "" };
                info.row("Material", format!("its own colour, {image}{normal}, metallic {:.2}, roughness {:.2}", m.metallic, m.roughness));
            }
        }
        Err(e) => info.problems.push(e),
    }
    info
}

fn scene(path: &Path) -> Info {
    let mut info = Info { kind: "Scene", ..Info::default() };
    match bayleaf_scene::Scene::load(path) {
        Ok(scene) => {
            info.row("Objects", scene.object_count());
            let roots: Vec<&str> = scene.children(None).iter().filter_map(|id| scene.name(*id)).take(8).collect();
            info.row("Top level", roots.join(", "));
        }
        Err(e) => info.problems.push(e.to_string()),
    }
    info
}

fn prefab(session: &Session, path: &Path) -> Info {
    let mut info = Info { kind: "Prefab", ..Info::default() };
    match crate::prefab_cmds::read_prefab(session, &path.to_string_lossy()) {
        Ok(objects) => {
            let root = objects.first().and_then(|o| o.components.get("name")).and_then(Value::as_str).unwrap_or("Object");
            info.row("Root", root);
            info.row("Objects", objects.len());
        }
        Err(e) => info.problems.push(e.to_string()),
    }
    info
}

fn graph(path: &Path, extension: &str) -> Info {
    let kind = if extension == "bmaterial" { "Material" } else { "Visual Script" };
    let mut info = Info { kind, ..Info::default() };
    let Some(catalog) = crate::catalog_for_path(path) else { return info };
    let loaded = std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| bayleaf_graph::Graph::from_json(&t, catalog).map_err(|e| e.to_string()));
    match loaded {
        Ok(graph) => {
            info.row("Nodes", graph.nodes.len());
            info.row("Links", graph.links.len());
            info.problems = crate::problems(&graph).into_iter().map(|p| p.message).collect();
            if extension == "bscript" {
                properties(&mut info, path);
            }
        }
        Err(e) => info.problems.push(e),
    }
    info
}

fn script(path: &Path) -> Info {
    let mut info = Info { kind: "Script", ..Info::default() };
    properties(&mut info, path);
    info
}

/// The properties a script declares, from running its describe step.
fn properties(info: &mut Info, path: &Path) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let declared = bayleaf_script::module_bytes(path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| bayleaf_script::ScriptHost::new().describe(&name, &bytes).map_err(|e| e.to_string()));
    match declared {
        Ok(list) if list.is_empty() => info.row("Properties", "None"),
        Ok(list) => info.row("Properties", list.iter().map(|(n, v)| format!("{n} = {v}")).collect::<Vec<_>>().join(", ")),
        Err(e) => info.problems.push(e),
    }
}

fn project(path: &Path) -> Info {
    let mut info = text(path, "Project");
    if let Ok(file) = std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string())) {
        info.rows.push(("Name".to_owned(), file["name"].as_str().unwrap_or_default().to_owned()));
        info.rows.push(("Main scene".to_owned(), file["mainScene"].as_str().unwrap_or_default().to_owned()));
        info.text = None;
    }
    info
}

/// An Input Map: its maps, the one play starts with, and how many actions; or what is wrong.
fn input_map(session: &Session, path: &Path) -> Info {
    let mut info = Info { kind: "Input Map", ..Info::default() };
    match crate::input_cmds::read_asset(session, path).and_then(|asset| asset.maps().map(|maps| (asset, maps))) {
        Ok((asset, maps)) => {
            info.row("Maps", maps.maps().join(", "));
            info.row("Starts with", maps.start());
            info.row("Actions", asset.action_count());
        }
        Err(problem) => info.problems.push(problem),
    }
    info
}

/// The start of a text file, cut at a character boundary.
fn text(path: &Path, kind: &'static str) -> Info {
    let mut info = Info { kind, ..Info::default() };
    let read = std::fs::File::open(path).and_then(|file| {
        let mut bytes = Vec::with_capacity(TEXT_PREVIEW_BYTES);
        std::io::Read::read_to_end(&mut std::io::Read::take(file, TEXT_PREVIEW_BYTES as u64 + 1), &mut bytes)?;
        Ok(bytes)
    });
    match read {
        Ok(mut bytes) => {
            let cut = bytes.len() > TEXT_PREVIEW_BYTES;
            bytes.truncate(TEXT_PREVIEW_BYTES);
            let text = match String::from_utf8(bytes) {
                Ok(text) => text,
                // A cut in the middle of a character: keep the whole characters before it.
                Err(e) if e.utf8_error().error_len().is_none() => {
                    let valid = e.utf8_error().valid_up_to();
                    String::from_utf8_lossy(&e.into_bytes()[..valid]).into_owned()
                }
                Err(_) => {
                    info.problems.push("this is not a text file".to_owned());
                    return info;
                }
            };
            info.text = Some(if cut { format!("{text}\n…") } else { text });
        }
        Err(e) => info.problems.push(format!("cannot read the file: {e}")),
    }
    info
}
