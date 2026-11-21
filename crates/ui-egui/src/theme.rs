//! Theme tokens: every colour, size and spacing the editor uses comes from here (AA 14).
//!
//! There is a dark and a light set; the editor follows the system setting. Text pairs meet
//! WCAG 2.2 AA (4.5:1) and the keyboard focus ring 3:1, checked by the tests at the bottom.
//! Controls are told apart by fill shades instead of outlines; their labels identify them.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Theme, ThemePreference, Vec2};

/// Named colours and sizes of one theme.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tokens {
    /// Behind panels and between docked areas.
    pub backdrop: Color32,
    /// Panel background.
    pub panel: Color32,
    /// Cards inside panels, such as inspector components.
    pub surface: Color32,
    pub surface_hover: Color32,
    pub surface_active: Color32,
    /// Hairlines between areas; decorative only.
    pub divider: Color32,
    /// Recessed fill of text and number fields.
    pub field: Color32,
    pub field_hover: Color32,
    /// Fill of buttons, check boxes and choice lists, at rest, hovered and pressed.
    pub control: Color32,
    pub control_hover: Color32,
    pub control_active: Color32,
    /// Keyboard focus ring (3:1 against everything it surrounds).
    pub focus: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    /// Brand colour of the logo; decorative only, never for text.
    pub brand: Color32,
    /// Text on an accent fill.
    pub on_accent: Color32,
    pub axis_x: Color32,
    pub axis_y: Color32,
    pub axis_z: Color32,
    pub warning: Color32,
    pub error: Color32,
}

pub(crate) const DARK: Tokens = Tokens {
    backdrop: Color32::from_rgb(0x13, 0x15, 0x1A),
    panel: Color32::from_rgb(0x1B, 0x1E, 0x25),
    surface: Color32::from_rgb(0x23, 0x27, 0x31),
    surface_hover: Color32::from_rgb(0x2B, 0x30, 0x3B),
    surface_active: Color32::from_rgb(0x35, 0x3B, 0x48),
    divider: Color32::from_rgb(0x2C, 0x31, 0x3B),
    field: Color32::from_rgb(0x14, 0x17, 0x1C),
    field_hover: Color32::from_rgb(0x1C, 0x20, 0x27),
    control: Color32::from_rgb(0x2F, 0x35, 0x41),
    control_hover: Color32::from_rgb(0x39, 0x40, 0x4D),
    control_active: Color32::from_rgb(0x44, 0x4C, 0x5B),
    focus: Color32::from_rgb(0x7F, 0xAA, 0x8C),
    text: Color32::from_rgb(0xE6, 0xE8, 0xEE),
    text_muted: Color32::from_rgb(0xA6, 0xAC, 0xB9),
    accent: Color32::from_rgb(0x3E, 0x7A, 0x58),
    brand: Color32::from_rgb(0x7F, 0xAA, 0x8C),
    on_accent: Color32::WHITE,
    axis_x: Color32::from_rgb(0xF4, 0x70, 0x67),
    axis_y: Color32::from_rgb(0x6B, 0xC4, 0x6D),
    axis_z: Color32::from_rgb(0x6C, 0xB6, 0xFF),
    warning: Color32::from_rgb(0xE5, 0xB5, 0x4B),
    error: Color32::from_rgb(0xFF, 0x7B, 0x72),
};

pub(crate) const LIGHT: Tokens = Tokens {
    backdrop: Color32::from_rgb(0xE6, 0xE9, 0xEE),
    panel: Color32::from_rgb(0xF5, 0xF6, 0xF8),
    surface: Color32::WHITE,
    surface_hover: Color32::from_rgb(0xEE, 0xF0, 0xF4),
    surface_active: Color32::from_rgb(0xE1, 0xE5, 0xEB),
    divider: Color32::from_rgb(0xD9, 0xDD, 0xE3),
    field: Color32::from_rgb(0xED, 0xEF, 0xF3),
    field_hover: Color32::from_rgb(0xE4, 0xE8, 0xEE),
    control: Color32::from_rgb(0xE3, 0xE7, 0xED),
    control_hover: Color32::from_rgb(0xD8, 0xDD, 0xE5),
    control_active: Color32::from_rgb(0xCC, 0xD3, 0xDC),
    focus: Color32::from_rgb(0x2F, 0x6B, 0x4B),
    text: Color32::from_rgb(0x1B, 0x20, 0x2C),
    text_muted: Color32::from_rgb(0x55, 0x5D, 0x6C),
    accent: Color32::from_rgb(0x2F, 0x6B, 0x4B),
    brand: Color32::from_rgb(0x7F, 0xAA, 0x8C),
    on_accent: Color32::WHITE,
    axis_x: Color32::from_rgb(0xC0, 0x36, 0x31),
    axis_y: Color32::from_rgb(0x2E, 0x7D, 0x32),
    axis_z: Color32::from_rgb(0x1F, 0x63, 0xC6),
    warning: Color32::from_rgb(0x9A, 0x67, 0x00),
    error: Color32::from_rgb(0xC6, 0x28, 0x28),
};

/// Corner radius of buttons and fields.
pub(crate) const RADIUS: u8 = 6;
/// Corner radius of cards and popups.
pub(crate) const CARD_RADIUS: u8 = 8;
/// Height of rows and controls; at least the 24-point minimum target size.
pub(crate) const ROW_HEIGHT: f32 = 26.0;
/// Width of the label column in the inspector.
pub(crate) const LABEL_WIDTH: f32 = 88.0;
/// Space between related controls.
pub(crate) const GAP: f32 = 6.0;

/// The tokens for the theme `ui` is drawn with.
pub(crate) fn tokens(ui: &egui::Ui) -> &'static Tokens {
    if ui.visuals().dark_mode { &DARK } else { &LIGHT }
}

/// Installs both themes and follows the system's light or dark setting.
pub(crate) fn install(ctx: &egui::Context) {
    ctx.set_style_of(Theme::Dark, style(&DARK, true));
    ctx.set_style_of(Theme::Light, style(&LIGHT, false));
    ctx.set_theme(ThemePreference::System);
}

/// A rounded card for a group of fields.
pub(crate) fn card(t: &Tokens) -> egui::Frame {
    egui::Frame::new().fill(t.surface).corner_radius(CARD_RADIUS).inner_margin(Margin::same(10))
}

fn style(t: &Tokens, dark: bool) -> egui::Style {
    let mut style = egui::Style::default();
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };

    v.override_text_color = Some(t.text);
    v.weak_text_color = Some(t.text_muted);
    v.panel_fill = t.panel;
    v.window_fill = t.surface;
    v.window_stroke = Stroke::new(1.0, t.divider);
    v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.menu_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.extreme_bg_color = t.field;
    v.text_edit_bg_color = Some(t.field);
    v.faint_bg_color = t.surface_hover;
    v.hyperlink_color = t.accent;
    v.warn_fg_color = t.warning;
    v.error_fg_color = t.error;
    v.selection.bg_fill = t.accent;
    v.selection.stroke = Stroke::new(1.5, t.on_accent);
    v.striped = false;
    v.indent_has_left_vline = false;

    let radius = CornerRadius::same(RADIUS);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = t.panel;
    w.noninteractive.weak_bg_fill = t.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, t.divider);
    w.noninteractive.fg_stroke = Stroke::new(1.0, t.text);
    w.noninteractive.corner_radius = radius;
    // Controls are told apart by fill, not outline. The only outline is the focus ring.
    for (state, fill) in [(&mut w.inactive, t.control), (&mut w.hovered, t.control_hover), (&mut w.active, t.control_active), (&mut w.open, t.control_active)] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
        state.fg_stroke = Stroke::new(1.5, t.text);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    // egui draws focused widgets with the active style, so this is the keyboard focus ring.
    w.active.bg_stroke = Stroke::new(1.5, t.focus);
    style.visuals = v;

    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(8.0, GAP);
    s.button_padding = Vec2::new(10.0, 4.0);
    s.interact_size = Vec2::new(40.0, ROW_HEIGHT);
    s.window_margin = Margin::same(10);
    s.menu_margin = Margin::same(6);
    s.indent = 14.0;
    s.icon_width = 14.0;
    s.icon_spacing = 6.0;
    s.combo_height = 240.0;

    style.text_styles = [
        (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
    ]
    .into();
    style
}

#[cfg(test)]
mod tests {
    use super::{DARK, LIGHT, Tokens};
    use egui::Color32;

    fn luminance(c: Color32) -> f64 {
        let channel = |v: u8| {
            let s = f64::from(v) / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
    }

    fn contrast(a: Color32, b: Color32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    fn check(name: &str, t: &Tokens) {
        let text_pairs = [
            ("text on panel", t.text, t.panel),
            ("text on surface", t.text, t.surface),
            ("text on hover", t.text, t.surface_hover),
            ("text on active", t.text, t.surface_active),
            ("text on field", t.text, t.field),
            ("text on control", t.text, t.control),
            ("text on hovered control", t.text, t.control_hover),
            ("text on pressed control", t.text, t.control_active),
            ("muted on field", t.text_muted, t.field),
            ("muted on panel", t.text_muted, t.panel),
            ("muted on surface", t.text_muted, t.surface),
            ("on-accent on accent", t.on_accent, t.accent),
            ("x axis on surface", t.axis_x, t.surface),
            ("y axis on surface", t.axis_y, t.surface),
            ("z axis on surface", t.axis_z, t.surface),
            ("error on panel", t.error, t.panel),
        ];
        for (what, fg, bg) in text_pairs {
            assert!(contrast(fg, bg) >= 4.5, "{name}: {what} is {:.2}:1", contrast(fg, bg));
        }
        for (what, fg, bg) in [
            ("focus ring on panel", t.focus, t.panel),
            ("focus ring on surface", t.focus, t.surface),
            ("focus ring on field", t.focus, t.field),
            ("focus ring on pressed control", t.focus, t.control_active),
            ("accent on panel", t.accent, t.panel),
        ] {
            assert!(contrast(fg, bg) >= 3.0, "{name}: {what} is {:.2}:1", contrast(fg, bg));
        }
    }

    #[test]
    fn both_themes_meet_wcag_contrast() {
        check("dark", &DARK);
        check("light", &LIGHT);
    }
}
