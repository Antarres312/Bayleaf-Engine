//! What the C# engine's commands must give back (docs/csharp-redesign.md, stage 2). Each batch of
//! steps in `tests/commands/*.json` (at the repository's root) runs in a fresh session, as
//! `bayleaf-cli batch` runs it, and what comes back is compared with
//! `tests/commands/expected/<name>.json`, which this test writes when `BAYLEAF_BLESS=1` is set. The C#
//! engine's test runs the same batches and compares with the same files. The registry's list is
//! written to `tests/commands/expected/commands.json` the same way.
//!
//! In a batch, `$root` in a string stands for the repository's folder and `$out` for a new empty
//! folder; `$N.field` is a field of step N's result, as the CLI has it. What comes back is made the
//! same on every machine: the two folders become `$root` and `$out` again (with `/`), ids become
//! `<id 0>`, `<id 1>`… in the order they first appear, and a params error from the JSON reader keeps
//! only its first words, since the two engines' readers word their errors differently.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{CommandRegistry, Session};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("..")
}

fn plain(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    text.strip_prefix("//?/").map(str::to_owned).unwrap_or(text)
}

/// `value` with `$root` and `$out` in its strings made the folders they stand for.
fn place(value: Value, root: &str, out: &str) -> Value {
    match value {
        Value::String(text) => Value::String(text.replace("$root", root).replace("$out", out)),
        Value::Array(items) => Value::Array(items.into_iter().map(|v| place(v, root, out)).collect()),
        Value::Object(fields) => Value::Object(fields.into_iter().map(|(k, v)| (k, place(v, root, out))).collect()),
        other => other,
    }
}

/// `$N.field` of step N's result, as the CLI's batch resolves it.
fn substitute(value: Value, results: &[Value]) -> Value {
    match value {
        Value::String(text) => {
            let found = text
                .strip_prefix('$')
                .and_then(|rest| rest.split_once('.'))
                .and_then(|(step, field)| results.get(step.parse::<usize>().ok()?)?.get(field).cloned());
            found.unwrap_or(Value::String(text))
        }
        Value::Array(items) => Value::Array(items.into_iter().map(|v| substitute(v, results)).collect()),
        Value::Object(fields) => Value::Object(fields.into_iter().map(|(k, v)| (k, substitute(v, results))).collect()),
        other => other,
    }
}

/// Text made the same on every machine (see the module's documentation).
fn normal_text(text: &str, folders: &[(String, &str)], ids: &mut Vec<String>) -> String {
    let mut out = text.to_owned();
    for (folder, name) in folders {
        for spelling in [folder.clone(), folder.replace('/', "\\")] {
            out = out.replace(&spelling, name);
        }
    }
    if out.starts_with("$root") || out.starts_with("$out") {
        out = out.replace('\\', "/");
    }
    let bytes: Vec<char> = out.chars().collect();
    let mut result = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let run = bytes[i..].iter().take_while(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()).count();
        let boundary = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let after = bytes.get(i + 32).is_none_or(|c| !c.is_ascii_alphanumeric());
        if run >= 32 && boundary && after {
            let id: String = bytes[i..i + 32].iter().collect();
            let n = ids.iter().position(|known| *known == id).unwrap_or_else(|| {
                ids.push(id.clone());
                ids.len() - 1
            });
            result.push_str(&format!("<id {n}>"));
            i += 32;
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    result
}

fn normal(value: Value, folders: &[(String, &str)], ids: &mut Vec<String>) -> Value {
    match value {
        Value::String(text) => Value::String(normal_text(&text, folders, ids)),
        Value::Array(items) => Value::Array(items.into_iter().map(|v| normal(v, folders, ids)).collect()),
        Value::Object(fields) => {
            let mut out = serde_json::Map::new();
            for (k, v) in fields {
                let key = normal_text(&k, folders, ids);
                out.insert(key, normal(v, folders, ids));
            }
            Value::Object(out)
        }
        other => other,
    }
}

/// A params error from the JSON reader, down to the words both engines share.
pub(crate) fn normal_error(error: &str) -> String {
    match error.strip_prefix("invalid params: ") {
        Some(rest)
            if ["invalid type", "invalid value", "invalid length", "missing field", "unknown field", "unknown variant", "duplicate field"]
                .iter()
                .any(|w| rest.starts_with(w)) =>
        {
            let words: Vec<&str> = rest.split_whitespace().take(3).collect();
            format!("invalid params: {}", words.join(" "))
        }
        _ => error.to_owned(),
    }
}

/// Runs one batch file as `bayleaf-cli batch` does and gives back what it prints, made the same on every machine.
fn run_batch(path: &Path) -> String {
    let root = root().canonicalize().unwrap();
    let out = std::env::temp_dir().join(format!("bayleaf-parity-{}-{}", path.file_stem().unwrap().to_string_lossy(), std::process::id()));
    std::fs::remove_dir_all(&out).ok();
    std::fs::create_dir_all(&out).unwrap();
    let out = out.canonicalize().unwrap();
    let (root_text, out_text) = (plain(&root), plain(&out));
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let registry = CommandRegistry::with_builtins();
    let mut session = Session::default();
    let mut results: Vec<Value> = Vec::new();
    let mut failure = None;
    for (index, step) in steps.into_iter().enumerate() {
        let cmd = step["cmd"].as_str().unwrap_or_default().to_owned();
        let params = substitute(place(step.get("params").cloned().unwrap_or(Value::Null), &root_text, &out_text), &results);
        match registry.execute(&mut session, &cmd, &params) {
            Ok(result) => results.push(result),
            Err(error) => {
                failure = Some((index, error.to_string()));
                break;
            }
        }
    }
    let folders = [(out_text.clone(), "$out"), (root_text.clone(), "$root")];
    let mut ids = Vec::new();
    let results = normal(Value::Array(results), &folders, &mut ids);
    let printed = match failure {
        Some((index, error)) => json!({ "error": normal_text(&normal_error(&error), &folders, &mut ids), "failedStep": index, "results": results }),
        None => json!({ "results": results }),
    };
    std::fs::remove_dir_all(&out).ok();
    format!("{}\n", serde_json::to_string_pretty(&printed).unwrap())
}

#[test]
fn the_command_batches_give_back_what_the_expected_files_say() {
    let dir = root().join("tests").join("commands");
    let expected_dir = dir.join("expected");
    let bless = std::env::var_os("BAYLEAF_BLESS").is_some_and(|v| v == "1");
    if bless {
        std::fs::create_dir_all(&expected_dir).unwrap();
        let list = serde_json::to_string_pretty(&CommandRegistry::with_builtins().list(&Session::default())).unwrap();
        std::fs::write(expected_dir.join("commands.json"), format!("{list}\n")).unwrap();
    }
    let mut batches: Vec<PathBuf> =
        std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
    batches.sort();
    assert!(!batches.is_empty(), "no batches in {}", dir.display());
    for batch in batches {
        let written = run_batch(&batch);
        let expected_path = expected_dir.join(batch.file_name().unwrap());
        if bless {
            std::fs::write(&expected_path, &written).unwrap();
        }
        let expected = std::fs::read_to_string(&expected_path).unwrap_or_default().replace("\r\n", "\n");
        assert_eq!(written, expected, "{} gives back something else than {}", batch.display(), expected_path.display());
    }
}

/// The scenes whose pictures both engines' `scene.render` must draw alike (stage 3 of
/// docs/csharp-redesign.md), each with the project it opens in: the examples, and test files that hold
/// every component, an old environment and the first format.
const RENDERED: [(&str, Option<&str>); 5] = [
    ("examples/demo/demo.bscene", None),
    ("examples/showcase/scenes/main.bscene", Some("examples/showcase")),
    ("tests/files/every-component.bscene", None),
    ("tests/files/old-environment.bscene", None),
    ("tests/files/version-1.bscene", None),
];

/// Pixels across and down of the compared pictures.
const RENDER_SIZE: (u32, u32) = (192, 108);

/// How far two pictures of the same scene may differ, per channel out of 255: on average over every
/// pixel, and in the pixel that differs most (why these, in docs/csharp-redesign.md, stage 3).
const MEAN_LIMIT: f64 = 1.0;
const MAX_LIMIT: u8 = 64;

/// The mean and the largest difference of each colour channel between two RGBA pictures of one size.
fn picture_difference(a: &[u8], b: &[u8]) -> ([f64; 3], [u8; 3]) {
    let (mut sum, mut max) = ([0u64; 3], [0u8; 3]);
    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
        for c in 0..3 {
            let d = p[c].abs_diff(q[c]);
            sum[c] += u64::from(d);
            max[c] = max[c].max(d);
        }
    }
    let pixels = (a.len() / 4).max(1) as f64;
    (sum.map(|s| s as f64 / pixels), max)
}

/// Each scene of `RENDERED`, opened and drawn by `scene.render`, looks as `tests/files/expected/renders`
/// says: this test writes the pictures with `BAYLEAF_BLESS=1` (on a machine with a GPU) and otherwise
/// checks that this engine still draws them within the limits the C# engine's test is held to. It skips
/// without a GPU, unless `BAYLEAF_GPU_REQUIRED=1`.
#[test]
fn the_shared_scenes_render_as_the_expected_pictures() {
    let bless = std::env::var_os("BAYLEAF_BLESS").is_some_and(|v| v == "1");
    let expected_dir = root().join("tests").join("files").join("expected").join("renders");
    let out = std::env::temp_dir().join(format!("bayleaf-render-parity-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    for (scene, project) in RENDERED {
        let registry = CommandRegistry::with_builtins();
        let mut session = Session::default();
        if let Some(project) = project {
            registry.execute(&mut session, "project.open", &json!({"path": root().join(project)})).unwrap();
        }
        let name = format!("{}.png", scene.replace('/', "__"));
        // A test file is opened from a copy: opening an old one moves its environment into a lighting
        // file beside it.
        let opened = if scene.starts_with("tests/") {
            let copy = out.join(Path::new(scene).file_name().unwrap());
            std::fs::copy(root().join(scene), &copy).unwrap();
            copy
        } else {
            root().join(scene)
        };
        registry.execute(&mut session, "scene.open", &json!({"path": opened})).unwrap();
        let drawn = out.join(&name);
        let params = json!({"path": drawn, "width": RENDER_SIZE.0, "height": RENDER_SIZE.1});
        match registry.execute(&mut session, "scene.render", &params) {
            Ok(_) => {}
            Err(e) if std::env::var_os("BAYLEAF_GPU_REQUIRED").is_none() && e.to_string().contains("no usable graphics adapter") => {
                eprintln!("skipped: {e}");
                return;
            }
            Err(e) => panic!("{scene}: {e}"),
        }
        if bless {
            std::fs::create_dir_all(&expected_dir).unwrap();
            std::fs::copy(&drawn, expected_dir.join(&name)).unwrap();
        }
        let (now, expected) = (bayleaf_render::assets::load_image(&drawn).unwrap(), bayleaf_render::assets::load_image(&expected_dir.join(&name)).unwrap());
        assert_eq!((now.width, now.height), (expected.width, expected.height), "{scene}");
        let (mean, max) = picture_difference(&now.rgba, &expected.rgba);
        assert!(mean.iter().all(|m| *m <= MEAN_LIMIT) && max.iter().all(|m| *m <= MAX_LIMIT), "{scene} differs from {name}: mean {mean:?}, largest {max:?}");
    }
    std::fs::remove_dir_all(&out).ok();
}
