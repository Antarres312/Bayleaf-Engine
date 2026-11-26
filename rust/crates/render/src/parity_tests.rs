//! The C# engine's material port (`src/Bayleaf.Render`) must read every material graph file as this
//! crate does and compile it to the same WGSL, byte for byte (docs/csharp-redesign.md). Both engines'
//! tests compare what they give with `tests/files/expected/`, which this test writes when
//! `BAYLEAF_BLESS=1` is set. Only `crate::material` is used: nothing here needs a GPU.

use std::path::{Path, PathBuf};

use bayleaf_graph::Graph;

use crate::material::{CATALOG, compile};

const REFUSED: &str = "refused\n";

fn files_below(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "expected") {
                files_below(&path, out);
            }
        } else {
            out.push(path);
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("..")
}

/// Compares `written` with the expected file `name`, first writing it there when blessing.
fn check(name: &str, written: &str, what: &Path) {
    let expected_dir = root().join("tests").join("files").join("expected");
    let expected_path = expected_dir.join(name);
    if std::env::var_os("BAYLEAF_BLESS").is_some_and(|v| v == "1") {
        std::fs::create_dir_all(&expected_dir).unwrap();
        std::fs::write(&expected_path, written).unwrap();
    }
    let expected = std::fs::read_to_string(&expected_path).unwrap_or_default().replace("\r\n", "\n");
    assert_eq!(written, expected, "{} gives other text than {}", what.display(), expected_path.display());
}

/// The file as this crate writes it back after reading it with the material catalog, and the
/// `surface` function it compiles to; `refused` for either when it does not read or compile.
fn rewrite(path: &Path) -> (String, String) {
    let Ok(text) = std::fs::read_to_string(path) else { return (REFUSED.to_owned(), REFUSED.to_owned()) };
    let Ok(graph) = Graph::from_json(&text, &CATALOG) else { return (REFUSED.to_owned(), REFUSED.to_owned()) };
    let surface = compile(&graph).map_or_else(|_| REFUSED.to_owned(), |m| m.surface);
    (graph.to_json(), surface)
}

/// Every `.bmaterial` of the examples, the benchmark and `tests/files` reads, writes back and
/// compiles as the expected files say: `<name>` the file written back, `<name>.wgsl` its surface.
#[test]
fn the_shared_material_files_rewrite_and_compile_as_the_expected_files_say() {
    let root = root();
    let mut inputs = Vec::new();
    files_below(&root.join("examples"), &mut inputs);
    files_below(&root.join("rust").join("bench").join("graphs"), &mut inputs);
    files_below(&root.join("tests").join("files"), &mut inputs);
    let mut checked = 0;
    for input in inputs.iter().filter(|p| p.extension().is_some_and(|e| e == "bmaterial")) {
        let relative = input.strip_prefix(&root).unwrap_or(input).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>();
        let name = relative.iter().skip_while(|c| *c == "tests" || *c == "files").cloned().collect::<Vec<_>>().join("__");
        let (file, surface) = rewrite(input);
        check(&name, &file, input);
        check(&format!("{name}.wgsl"), &surface, input);
        checked += 1;
    }
    assert!(checked > 10, "only {checked} shared material files were found");
}

/// The material catalog in every detail, so the C# port's hand-written one is checked kind by kind.
#[test]
fn the_material_catalog_is_as_the_expected_file_says() {
    let written = serde_json::to_string_pretty(&CATALOG.spec()).unwrap() + "\n";
    check("material.catalog.json", &written, Path::new("the material catalog"));
}
