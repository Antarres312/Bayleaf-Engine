//! The editor's fonts: the Craft apps' shared font stack (`craft_ui::fonts`), so text in any script
//! draws, such as a file name in Japanese or Arabic.

use craft_fonts_build::EmbeddedFace;

/// craft-fonts faces; empty unless built with `CRAFT_FONTS_DIR`.
static CRAFT_FONTS: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

/// Installs the font stack for the interface language in use: egui's faces, then any embedded
/// craft-fonts faces (the language's own CJK face first), then installed system fonts for
/// characters none of them has.
pub fn install_fonts(ctx: &egui::Context) {
    let faces: Vec<craft_ui::fonts::CraftFace> = CRAFT_FONTS.iter().cloned().map(Into::into).collect();
    craft_ui::fonts::UiFonts::new(&craft_ui::fonts::I18nScripts)
        .craft_faces(&faces)
        .languages(&crate::i18n::language_codes())
        .ui_language(crate::i18n::language())
        .install(ctx);
}
