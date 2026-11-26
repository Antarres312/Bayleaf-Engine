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
