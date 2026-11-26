//! Embeds the craft-fonts faces for the editor's interface when `CRAFT_FONTS_DIR` names a
//! craft-fonts checkout, and the translation catalogs when `CRAFT_I18N_DIR` names a craft-i18n
//! checkout. Without them the tables are empty: egui's faces and the installed fonts, in English.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The app's own catalog in each craft-i18n locale folder, beside the shared `common.tsv`.
const APP_CATALOG: &str = "bayleaf.tsv";

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if let Err(e) = craft_fonts_build::Embed::new().for_ui().write("craft_fonts.rs") {
        println!("cargo::error={e}");
    }
    let Some(out) = std::env::var_os("OUT_DIR") else {
        println!("cargo::error=OUT_DIR is not set");
        return;
    };
    let table = catalogs();
    if let Err(e) = std::fs::write(Path::new(&out).join("craft_i18n.rs"), table) {
        println!("cargo::error=cannot write the translation table: {e}");
    }
}

/// `&[(locale, shared catalog, app catalog)]` for every locale folder in `CRAFT_I18N_DIR/locales`
/// that has either catalog, in code order; `&[]` when the variable is unset.
fn catalogs() -> String {
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_DIR");
    let mut table = String::from("&[");
    let Some(dir) = std::env::var_os("CRAFT_I18N_DIR").filter(|dir| !dir.is_empty()) else {
        table.push(']');
        return table;
    };
    let locales = PathBuf::from(dir).join("locales");
    println!("cargo::rerun-if-changed={}", locales.display());
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&locales).into_iter().flatten().flatten().map(|entry| entry.path()).filter(|p| p.is_dir()).collect();
    folders.sort();
    for folder in folders {
        let Some(code) = folder.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
        let [shared, app] = ["common.tsv", APP_CATALOG].map(|file| folder.join(file));
        if !shared.is_file() && !app.is_file() {
            continue;
        }
        let text = |path: &Path| {
            println!("cargo::rerun-if-changed={}", path.display());
            if path.is_file() { format!("include_str!({:?})", path.display().to_string()) } else { "\"\"".to_owned() }
        };
        let _ = write!(table, "({code:?}, {}, {}),", text(&shared), text(&app));
    }
    table.push(']');
    table
}
