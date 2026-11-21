//! Generic editor for a component's JSON value: numbers, text, switches, lists and records.

use serde_json::{Number, Value};

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

/// Edits `value` in place. `label` names the value for assistive technology.
pub(crate) fn edit(ui: &mut egui::Ui, value: &mut Value, salt: impl std::hash::Hash + std::fmt::Debug, label: Option<egui::Id>) -> Outcome {
    ui.push_id(salt, |ui| edit_inner(ui, value, label)).inner
}

fn edit_inner(ui: &mut egui::Ui, value: &mut Value, label: Option<egui::Id>) -> Outcome {
    match value {
        Value::Bool(b) => {
            let response = labelled(ui.checkbox(b, ""), label);
            Outcome { changed: response.changed(), ended: response.changed() }
        }
        Value::Number(n) => edit_number(ui, n, label),
        Value::String(s) => {
            let response = labelled(ui.add(egui::TextEdit::singleline(s).desired_width(f32::INFINITY)), label);
            Outcome::from_response(&response)
        }
        Value::Array(items) if items.len() <= AXES.len() && !items.is_empty() && items.iter().all(Value::is_number) => edit_vector(ui, items),
        Value::Array(items) => edit_list(ui, items),
        Value::Object(fields) => {
            let mut outcome = Outcome::default();
            egui::Grid::new("fields").num_columns(2).striped(true).show(ui, |ui| {
                for (key, field) in fields.iter_mut() {
                    let key_label = ui.label(title_case(key));
                    outcome.merge(edit(ui, field, key.as_str(), Some(key_label.id)));
                    ui.end_row();
                }
            });
            outcome
        }
        Value::Null => {
            ui.label("none");
            Outcome::default()
        }
    }
}

fn edit_number(ui: &mut egui::Ui, n: &mut Number, label: Option<egui::Id>) -> Outcome {
    let was_integer = n.is_i64() || n.is_u64();
    let mut x = n.as_f64().unwrap_or(0.0);
    let speed = if was_integer { 1.0 } else { 0.05 };
    let response = labelled(ui.add(egui::DragValue::new(&mut x).speed(speed)), label);
    let outcome = Outcome::from_response(&response);
    if outcome.changed {
        let replacement = if was_integer && x.fract() == 0.0 && x.abs() < 9.0e15 { Some(Number::from(x as i64)) } else { Number::from_f64(x) };
        if let Some(replacement) = replacement {
            *n = replacement;
        }
    }
    outcome
}

/// Short number lists (positions, rotations, scales) on one row with axis labels.
fn edit_vector(ui: &mut egui::Ui, items: &mut [Value]) -> Outcome {
    let mut outcome = Outcome::default();
    ui.horizontal(|ui| {
        for (item, axis) in items.iter_mut().zip(AXES) {
            let axis_label = ui.label(axis);
            if let Value::Number(n) = item {
                outcome.merge(ui.push_id(axis, |ui| edit_number(ui, n, Some(axis_label.id))).inner);
            }
        }
    });
    outcome
}

/// Lists: every item editable. Lists of text can grow and shrink.
fn edit_list(ui: &mut egui::Ui, items: &mut Vec<Value>) -> Outcome {
    let mut outcome = Outcome::default();
    let text_list = items.iter().all(Value::is_string);
    let mut remove = None;
    for (index, item) in items.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let item_label = ui.label(format!("Item {}", index + 1));
            outcome.merge(edit(ui, item, index, Some(item_label.id)));
            if text_list && ui.small_button("Remove").on_hover_text(format!("Remove item {}", index + 1)).clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        items.remove(index);
        outcome = Outcome { changed: true, ended: true };
    }
    if text_list && ui.small_button("Add Item").clicked() {
        items.push(Value::String(String::new()));
        outcome = Outcome { changed: true, ended: true };
    }
    if items.is_empty() && !text_list {
        ui.label("empty");
    }
    outcome
}

fn labelled(response: egui::Response, label: Option<egui::Id>) -> egui::Response {
    match label {
        Some(id) => response.labelled_by(id),
        None => response,
    }
}

fn title_case(key: &str) -> String {
    let mut chars = key.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}
