//! The editor's fonts: the Craft apps' shared font stack (`craft_ui::fonts`), so text in any script
//! draws, such as a file name in Japanese or Arabic.

use craft_fonts_build::EmbeddedFace;

/// craft-fonts faces; empty unless built with `CRAFT_FONTS_DIR`.
static CRAFT_FONTS: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

/// Installs the font stack: egui's faces, then any embedded craft-fonts faces, then installed
/// system fonts for characters none of them has. The interface is in English.
pub fn install_fonts(ctx: &egui::Context) {
    let faces: Vec<craft_ui::fonts::CraftFace> = CRAFT_FONTS.iter().cloned().map(Into::into).collect();
    craft_ui::fonts::UiFonts::new(&craft_ui::fonts::I18nScripts).craft_faces(&faces).languages(&["en"]).ui_language("en").install(ctx);
}
