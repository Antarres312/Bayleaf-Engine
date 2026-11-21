//! Embeds the craft-fonts faces for the editor's interface when `CRAFT_FONTS_DIR` names a
//! craft-fonts checkout; without it the table is empty and the editor uses egui's faces and the
//! installed fonts.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if let Err(e) = craft_fonts_build::Embed::new().for_ui().write("craft_fonts.rs") {
        println!("cargo::error={e}");
    }
}
