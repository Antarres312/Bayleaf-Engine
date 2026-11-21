//! Editor for a component's JSON value. Every control takes the width it is given, so editing a
//! value never changes the width of the panel around it.

use serde_json::{Number, Value};

use crate::theme::{self, GAP, LABEL_WIDTH, ROW_HEIGHT, Tokens};

/// What happened to a value this frame.
#[derive(Default, Clone, Copy)]
pub(crate) struct Outcome {
    /// The value changed and should be written back.
    pub changed: bool,
    /// A drag or text edit finished, so the next change starts a new undo step.
    pub ended: bool,
}

impl Outcome {
    fn merge(&mut self, other: Outcome) {
        self.changed |= other.changed;
        self.ended |= other.ended;
    }

    fn from_response(response: &egui::Response) -> Self {
        Self { changed: response.changed(), ended: response.drag_stopped() || response.lost_focus() }
    }
}

const AXES: [&str; 4] = ["X", "Y", "Z", "W"];
const AXIS_LABEL_WIDTH: f32 = 14.0;

/// Edits the fields of a component: one row per field, label column on the left.
/// Text fields with a fixed set of allowed values (`options`) get a choice list.
pub(crate) fn edit_fields(ui: &mut egui::Ui, value: &mut Value, salt: &str, options: &dyn Fn(&str) -> &'static [&'static str]) -> Outcome {
    let t = theme::tokens(ui);
    let mut outcome = Outcome::default();
    ui.push_id(salt, |ui| match value {
        Value::Object(fields) => {
            for (key, field) in fields.iter_mut() {
                outcome.merge(row(ui, t, &title_case(key), |ui, label, width| {
                    let choices = options(key);
                    match field {
                        Value::String(current) if !choices.is_empty() => choose(ui, current, choices, key, label, width),
                        _ if is_color(key, field) => edit_color(ui, field, label, width),
                        Value::Object(map) if key == "properties" => edit_named_numbers(ui, map, width),
                        _ => edit(ui, field, key.as_str(), label, width),
                    }
                }));
            }
        }
        other => {
            outcome.merge(row(ui, t, "Value", |ui, label, width| edit(ui, other, "value", label, width)));
        }
    });
    outcome
}

/// One labelled row. The closure gets the label's id and the width left for the value.
fn row(ui: &mut egui::Ui, t: &Tokens, label: &str, body: impl FnOnce(&mut egui::Ui, egui::Id, f32) -> Outcome) -> Outcome {
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        let label = ui.add_sized([LABEL_WIDTH, ROW_HEIGHT], egui::Label::new(egui::RichText::new(label).color(t.text_muted)).truncate());
        // Whatever is left after the label; the floor only applies to extremely narrow panels.
        let width = (ui.available_width() - 1.0).max(40.0);
        body(ui, label.id, width)
    })
    .inner
}

/// Edits `value` in place within `width`. `label` names the value for assistive technology.
pub(crate) fn edit(ui: &mut egui::Ui, value: &mut Value, salt: impl std::hash::Hash + std::fmt::Debug, label: egui::Id, width: f32) -> Outcome {
    ui.push_id(salt, |ui| match value {
        Value::Bool(b) => {
            let response = ui.checkbox(b, "").labelled_by(label);
            Outcome { changed: response.changed(), ended: response.changed() }
        }
        Value::Number(n) => edit_number(ui, n, label, width),
        Value::String(s) => {
            let response = ui.add_sized([width, ROW_HEIGHT], egui::TextEdit::singleline(s).align(egui::Align2::LEFT_CENTER)).labelled_by(label);
            // Text fields also take a file dragged from the Project panel: its path.
            if response.dnd_hover_payload::<crate::project::DraggedAsset>().is_some() {
                let t = theme::tokens(ui);
                ui.painter().rect_stroke(response.rect, theme::RADIUS, egui::Stroke::new(2.0, t.focus), egui::StrokeKind::Inside);
            }
            if let Some(asset) = response.dnd_release_payload::<crate::project::DraggedAsset>() {
                *s = asset.relative.clone();
                return Outcome { changed: true, ended: true };
            }
            Outcome::from_response(&response)
        }
        Value::Array(items) if (1..=AXES.len()).contains(&items.len()) && items.iter().all(Value::is_number) => edit_vector(ui, items, width),
        Value::Array(items) => edit_list(ui, items, label, width),
        Value::Object(fields) => {
            let t = theme::tokens(ui);
            let mut outcome = Outcome::default();
            ui.vertical(|ui| {
                ui.set_width(width);
                for (key, field) in fields.iter_mut() {
                    outcome.merge(row(ui, t, &title_case(key), |ui, label, width| edit(ui, field, key.as_str(), label, width)));
                }
            });
            outcome
        }
        Value::Null => {
            ui.weak("none");
            Outcome::default()
        }
    })
    .inner
}

fn edit_number(ui: &mut egui::Ui, n: &mut Number, label: egui::Id, width: f32) -> Outcome {
    let was_integer = n.is_i64() || n.is_u64();
    let mut x = n.as_f64().unwrap_or(0.0);
    let drag = if was_integer { egui::DragValue::new(&mut x).speed(1.0) } else { egui::DragValue::new(&mut x).speed(0.02).max_decimals(3) };
    // Number fields use the field shade, like text fields, not the button shade.
    let t = theme::tokens(ui);
    let response = ui
        .scope(|ui| {
            let w = &mut ui.visuals_mut().widgets;
            w.inactive.weak_bg_fill = t.field;
            w.hovered.weak_bg_fill = t.field_hover;
            w.active.weak_bg_fill = t.field;
            ui.add_sized([width, ROW_HEIGHT], drag)
        })
        .inner
        .labelled_by(label);
    let outcome = Outcome::from_response(&response);
    if outcome.changed {
        let replacement = if was_integer && x.fract() == 0.0 && x.abs() < 9.0e15 { Some(Number::from(x as i64)) } else { Number::from_f64(x) };
        if let Some(replacement) = replacement {
            *n = replacement;
        }
    }
    outcome
}

/// Short number lists (positions, rotations, scales) on one row with coloured axis letters.
fn edit_vector(ui: &mut egui::Ui, items: &mut [Value], width: f32) -> Outcome {
    let t = theme::tokens(ui);
    let colors = [t.axis_x, t.axis_y, t.axis_z, t.text_muted];
    let count = items.len() as f32;
    // Row: [label s field] s [label s field] ... — exactly `width` wide, never wider.
    ui.spacing_mut().item_spacing.x = GAP / 2.0;
    let spacing = GAP / 2.0;
    let field_width = ((width - count * AXIS_LABEL_WIDTH - (2.0 * count - 1.0) * spacing - 1.0) / count).max(24.0);
    let mut outcome = Outcome::default();
    for ((item, axis), color) in items.iter_mut().zip(AXES).zip(colors) {
        let axis_label = ui.add_sized([AXIS_LABEL_WIDTH, ROW_HEIGHT], egui::Label::new(egui::RichText::new(axis).color(color).strong()));
        if let Value::Number(n) = item {
            outcome.merge(ui.push_id(axis, |ui| edit_number(ui, n, axis_label.id, field_width)).inner);
        }
    }
    outcome
}

/// Lists: every item editable. Lists of text can grow and shrink.
fn edit_list(ui: &mut egui::Ui, items: &mut Vec<Value>, label: egui::Id, width: f32) -> Outcome {
    let mut outcome = Outcome::default();
    let text_list = items.iter().all(Value::is_string);
    let mut remove = None;
    ui.vertical(|ui| {
        ui.set_width(width);
        for (index, item) in items.iter_mut().enumerate() {
            // Remove sits at the right edge and the field takes what is left. The row keeps its own height.
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if text_list
                        && ui
                            .add(egui::Button::new("Remove").min_size(egui::vec2(0.0, ROW_HEIGHT)))
                            .on_hover_text(format!("Remove item {}", index + 1))
                            .clicked()
                    {
                        remove = Some(index);
                    }
                    let field_width = (ui.available_width() - 1.0).max(40.0);
                    outcome.merge(edit(ui, item, index, label, field_width));
                });
            });
        }
        if items.is_empty() {
            ui.weak("No items");
        }
        if text_list && ui.add_sized([width, ROW_HEIGHT], egui::Button::new("Add Item")).clicked() {
            items.push(Value::String(String::new()));
            outcome = Outcome { changed: true, ended: true };
        }
    });
    if let Some(index) = remove {
        items.remove(index);
        outcome = Outcome { changed: true, ended: true };
    }
    outcome
}

fn choose(ui: &mut egui::Ui, current: &mut String, choices: &[&str], salt: &str, label: egui::Id, width: f32) -> Outcome {
    let mut changed = false;
    let response = egui::ComboBox::from_id_salt(("choice", salt)).width(width).selected_text(title_case(current)).show_ui(ui, |ui| {
        for choice in choices {
            if ui.selectable_label(current == choice, title_case(choice)).clicked() && current != choice {
                *current = (*choice).to_owned();
                changed = true;
            }
        }
    });
    response.response.labelled_by(label);
    Outcome { changed, ended: changed }
}

/// A field named `color` or `…Color` with 3 or 4 numbers between 0 and 1 (linear RGB or RGBA).
fn is_color(key: &str, value: &Value) -> bool {
    (key == "color" || key.ends_with("Color"))
        && matches!(value, Value::Array(items) if (3..=4).contains(&items.len()) && items.iter().all(|v| v.as_f64().is_some_and(|x| (0.0..=1.0).contains(&x))))
}

fn edit_color(ui: &mut egui::Ui, value: &mut Value, label: egui::Id, width: f32) -> Outcome {
    let Value::Array(items) = value else { return Outcome::default() };
    let channels: Vec<f32> = items.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect();
    let has_alpha = channels.len() == 4;
    let get = |i: usize| channels.get(i).copied().unwrap_or(1.0);
    let mut rgba = egui::Rgba::from_rgba_unmultiplied(get(0), get(1), get(2), if has_alpha { get(3) } else { 1.0 });
    let saved = ui.spacing().interact_size;
    ui.spacing_mut().interact_size = egui::vec2(width, ROW_HEIGHT);
    let alpha = if has_alpha { egui::color_picker::Alpha::OnlyBlend } else { egui::color_picker::Alpha::Opaque };
    let response = egui::color_picker::color_edit_button_rgba(ui, &mut rgba, alpha).labelled_by(label);
    ui.spacing_mut().interact_size = saved;
    let response = response.on_hover_text(format!(
        "R {:.2}  G {:.2}  B {:.2}{}",
        rgba.r(),
        rgba.g(),
        rgba.b(),
        if has_alpha { format!("  A {:.2}", rgba.a()) } else { String::new() }
    ));
    // egui round-trips the colour through HSV every frame, so compare nothing: only a real edit counts.
    if !response.changed() {
        return Outcome { changed: false, ended: response.clicked_elsewhere() };
    }
    let alpha_value = rgba.a();
    let unmultiplied = if alpha_value > 0.0 { [rgba.r() / alpha_value, rgba.g() / alpha_value, rgba.b() / alpha_value] } else { [0.0; 3] };
    let mut out: Vec<Value> = unmultiplied.iter().map(|c| number(*c)).collect();
    if has_alpha {
        out.push(number(alpha_value));
    }
    *items = out;
    Outcome { changed: true, ended: false }
}

fn number(x: f32) -> Value {
    let rounded = (f64::from(x.clamp(0.0, 1.0)) * 1000.0).round() / 1000.0;
    Number::from_f64(rounded).map_or(Value::Null, Value::Number)
}

pub(crate) fn title_case(key: &str) -> String {
    // camelCase keys become words: "directionalLight" -> "Directional Light".
    let mut out = String::with_capacity(key.len() + 4);
    for (i, c) in key.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            out.push(' ');
            out.push(c);
        } else {
            out.push(c);
        }
    }
    out
}

/// A map of named numbers (script properties): edit, remove, and add by name.
fn edit_named_numbers(ui: &mut egui::Ui, map: &mut serde_json::Map<String, Value>, width: f32) -> Outcome {
    let t = theme::tokens(ui);
    let mut outcome = Outcome::default();
    let mut remove = None;
    ui.vertical(|ui| {
        ui.set_width(width);
        for (name, value) in map.iter_mut() {
            ui.horizontal(|ui| {
                let label = ui.add_sized([width * 0.4, ROW_HEIGHT], egui::Label::new(egui::RichText::new(name.as_str()).color(t.text_muted)).truncate());
                let field_width = (ui.available_width() - 70.0).max(40.0);
                if let Value::Number(n) = value {
                    outcome.merge(ui.push_id(name.as_str(), |ui| edit_number(ui, n, label.id, field_width)).inner);
                }
                if ui.add_sized([62.0, ROW_HEIGHT], egui::Button::new("Remove")).on_hover_text(format!("Remove {name}")).clicked() {
                    remove = Some(name.clone());
                }
            });
        }
        // The name being typed for a new property lives in egui's memory until it is added.
        let draft_id = ui.scope_id().with("new property");
        let mut draft = ui.data_mut(|d| d.get_temp::<String>(draft_id)).unwrap_or_default();
        ui.horizontal(|ui| {
            let field = ui.add_sized(
                [(ui.available_width() - 70.0).max(40.0), ROW_HEIGHT],
                egui::TextEdit::singleline(&mut draft).hint_text("New property").align(egui::Align2::LEFT_CENTER),
            );
            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label("New property name"));
            let name = draft.trim().to_owned();
            let can_add = !name.is_empty() && !map.contains_key(&name);
            let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.add_enabled_ui(can_add, |ui| ui.add_sized([62.0, ROW_HEIGHT], egui::Button::new("Add"))).inner.clicked() || submitted) && can_add {
                map.insert(name, Value::from(0.0));
                draft.clear();
                outcome = Outcome { changed: true, ended: true };
            }
        });
        ui.data_mut(|d| d.insert_temp(draft_id, draft));
    });
    if let Some(name) = remove {
        map.remove(&name);
        outcome = Outcome { changed: true, ended: true };
    }
    outcome
}

/// A labelled number row for a named value. Shows Reset when the value differs from its default.
/// Returns the edit outcome and whether Reset was clicked.
pub(crate) fn named_number(
    ui: &mut egui::Ui,
    name: &str,
    value: &mut Value,
    overridden: bool,
    salt: impl std::hash::Hash + std::fmt::Debug,
) -> (Outcome, bool) {
    let t = theme::tokens(ui);
    let mut reset = false;
    let outcome = ui
        .push_id(salt, |ui| {
            row(ui, t, &title_case(name), |ui, label, _| {
                // Reset sits at the right edge and the field takes what is left, so the row never
                // grows past the panel.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if overridden
                        && ui
                            .add(egui::Button::new("Reset").small().min_size(egui::vec2(0.0, ROW_HEIGHT)))
                            .on_hover_text("Go back to the script's default")
                            .clicked()
                    {
                        reset = true;
                    }
                    let field_width = (ui.available_width() - 1.0).max(40.0);
                    match value {
                        Value::Number(n) => edit_number(ui, n, label, field_width),
                        _ => Outcome::default(),
                    }
                })
                .inner
            })
        })
        .inner;
    (outcome, reset)
}

#[cfg(test)]
mod tests {
    use super::{is_color, title_case};
    use serde_json::json;

    #[test]
    fn camel_case_keys_become_words() {
        assert_eq!(title_case("directionalLight"), "Directional Light");
        assert_eq!(title_case("fov"), "Fov");
    }

    #[test]
    fn only_unit_range_color_fields_get_a_picker() {
        assert!(is_color("color", &json!([0.1, 0.2, 0.3, 1.0])));
        assert!(!is_color("color", &json!([0.1, 2.0, 0.3])));
        assert!(!is_color("position", &json!([0.1, 0.2, 0.3])));
    }
}
