//! Editor for a component's JSON value. Every control takes the width it is given, so editing a
//! value never changes the width of the panel around it.

use crate::i18n::tl;
use bayleaf_engine::scene::components::{MAX_PROPERTY_ITEMS, MAX_PROPERTY_TEXT, PropertyType, PropertyValue};
use serde_json::{Number, Value};

use crate::drag_wrap::WrapDrag;
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

/// The least width a value gets beside its label; in a narrower panel it goes under the label.
const MIN_BESIDE: f32 = 150.0;

/// One labelled row. The closure gets the label's id and the width left for the value. In a
/// narrow panel the label takes a line of its own and the value the full width under it, so
/// nothing runs past the panel's edge.
pub(crate) fn row<R>(ui: &mut egui::Ui, t: &Tokens, label: &str, body: impl FnOnce(&mut egui::Ui, egui::Id, f32) -> R) -> R {
    let text = egui::RichText::new(crate::i18n::tl(label)).color(t.text_muted);
    if ui.available_width() < LABEL_WIDTH + GAP + MIN_BESIDE {
        return ui
            .vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let label = ui.add(egui::Label::new(text).truncate());
                ui.horizontal(|ui| {
                    ui.set_min_height(ROW_HEIGHT);
                    let width = (ui.available_width() - 1.0).max(16.0);
                    body(ui, label.id, width)
                })
                .inner
            })
            .inner;
    }
    ui.horizontal(|ui| {
        ui.set_min_height(ROW_HEIGHT);
        let label = ui.add_sized([LABEL_WIDTH, ROW_HEIGHT], egui::Label::new(text).truncate());
        let width = (ui.available_width() - 1.0).max(16.0);
        body(ui, label.id, width)
    })
    .inner
}

/// A text field with a × button after it that removes what it edits, in the row's room: the button
/// first, from the right, and the field in what is left, so the row is never wider than its room (a
/// wider row would widen a resizable panel a little more every frame). Returns whether the edit was
/// finished (the field left) and whether × was pressed.
pub(crate) fn removable_text(ui: &mut egui::Ui, value: &mut String, label: egui::Id, remove_hint: &str) -> (bool, bool) {
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let removed = ui.small_button("×").on_hover_text(remove_hint).clicked();
        let field = ui.add_sized([ui.available_width().max(16.0), ROW_HEIGHT], egui::TextEdit::singleline(value)).labelled_by(label);
        (field.lost_focus(), removed)
    })
    .inner
}

/// Lets a field take a file dragged from the Project panel. While a file that `accepts` takes is
/// over the field, the field is ringed; a file it does not take shows a no-drop pointer and is
/// left for whatever else may take it. Returns the dropped file's path (relative to the project).
pub(crate) fn file_drop(ui: &egui::Ui, response: &egui::Response, accepts: impl Fn(&str) -> bool) -> Option<String> {
    let asset = response.dnd_hover_payload::<crate::project::DraggedAsset>()?;
    if !accepts(&asset.relative) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::NoDrop);
        return None;
    }
    let t = theme::tokens(ui);
    ui.painter().rect_stroke(response.rect, theme::RADIUS, egui::Stroke::new(2.0, t.focus), egui::StrokeKind::Inside);
    if !ui.input(|i| i.pointer.any_released()) {
        return None;
    }
    egui::DragAndDrop::clear_payload(ui.ctx());
    Some(asset.relative.clone())
}

/// Edits `value` in place within `width`. `label` names the value for assistive technology.
pub(crate) fn edit(ui: &mut egui::Ui, value: &mut Value, salt: impl std::hash::Hash + std::fmt::Debug, label: egui::Id, width: f32) -> Outcome {
    let field = format!("{salt:?}");
    ui.push_id(salt, |ui| match value {
        Value::Bool(b) => {
            let response = ui.checkbox(b, "").labelled_by(label);
            Outcome { changed: response.changed(), ended: response.changed() }
        }
        Value::Number(n) => edit_number(ui, n, label, width),
        Value::String(s) => {
            let response = ui.add_sized([width, ROW_HEIGHT], egui::TextEdit::singleline(s).align(egui::Align2::LEFT_CENTER)).labelled_by(label);
            // Text fields also take a file dragged from the Project panel: its path.
            if let Some(path) = file_drop(ui, &response, |_| true) {
                *s = path;
                return Outcome { changed: true, ended: true };
            }
            Outcome::from_response(&response)
        }
        Value::Array(items) if (1..=AXES.len()).contains(&items.len()) && items.iter().all(Value::is_number) => edit_vector(ui, items, width),
        Value::Array(items) => edit_list(ui, items, label, width, &field),
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
            // Exactly `width` wide: egui's least control width and the button padding would
            // otherwise widen a narrow field past its row.
            ui.spacing_mut().interact_size.x = 0.0;
            ui.spacing_mut().button_padding.x = 3.0;
            ui.add_sized([width, ROW_HEIGHT], drag).wrap_drag()
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
    let field_width = ((width - count * AXIS_LABEL_WIDTH - (2.0 * count - 1.0) * spacing - 1.0) / count).max(16.0);
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
/// A new point for a curve field (a list of points, empty so far), by the field's name.
fn first_point(field: &str) -> Option<Value> {
    if field.contains("sizeKeys") {
        Some(serde_json::json!({ "at": 0.5, "size": 1.0 }))
    } else if field.contains("colorKeys") {
        Some(serde_json::json!({ "at": 0.5, "color": [1.0, 1.0, 1.0, 1.0] }))
    } else {
        None
    }
}

fn edit_list(ui: &mut egui::Ui, items: &mut Vec<Value>, label: egui::Id, width: f32, field: &str) -> Outcome {
    let mut outcome = Outcome::default();
    // Texts, and points of a curve, are added and removed one by one.
    let point_list = first_point(field).is_some() && items.iter().all(Value::is_object);
    let text_list = !point_list && items.iter().all(Value::is_string);
    let can_remove = text_list || point_list;
    let mut remove = None;
    ui.vertical(|ui| {
        ui.set_width(width);
        for (index, item) in items.iter_mut().enumerate() {
            // Remove sits at the right edge and the field takes what is left. The row keeps its own height.
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if can_remove
                        && ui
                            .add(egui::Button::new(tl("Remove")).min_size(egui::vec2(0.0, ROW_HEIGHT)))
                            .on_hover_text(crate::i18n::tf("Remove item {number}", &[("number", &(index + 1).to_string())]))
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
        if (text_list || point_list) && ui.add_sized([width, ROW_HEIGHT], egui::Button::new(tl("Add Item"))).clicked() {
            // A point copies the last one (or starts halfway), so it can be moved from there.
            let new = if point_list { items.last().cloned().or_else(|| first_point(field)).unwrap_or(Value::Null) } else { Value::String(String::new()) };
            items.push(new);
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
    let response = craft_ui::ComboBox::from_id_salt(("choice", salt)).width(width).truncate().selected_text(title_case(current)).show_ui(ui, |ui| {
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
                if ui
                    .add_sized([62.0, ROW_HEIGHT], egui::Button::new(tl("Remove")))
                    .on_hover_text(crate::i18n::tf("Remove {name}", &[("name", &name.to_string())]))
                    .clicked()
                {
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
                egui::TextEdit::singleline(&mut draft).hint_text(tl("New property")).align(egui::Align2::LEFT_CENTER),
            );
            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label("New property name"));
            let name = draft.trim().to_owned();
            let can_add = !name.is_empty() && !map.contains_key(&name);
            let submitted = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.add_enabled_ui(can_add, |ui| ui.add_sized([62.0, ROW_HEIGHT], egui::Button::new(tl("Add")))).inner.clicked() || submitted) && can_add {
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

/// A labelled row for a script property of type `ty`, with a reset button that is always there and
/// works only when the value is not its default. `objects` are the scene's object names, for an
/// object property (which also takes an object dragged from the hierarchy). Returns the edit outcome
/// and whether reset was clicked.
/// One script property's row: what it is, and how the script's layout shows it (if it says).
pub(crate) struct PropertyRow<'a> {
    pub(crate) name: &'a str,
    pub(crate) ty: PropertyType,
    /// One value, a list, or a map and its keys' type.
    pub(crate) shape: bayleaf_engine::scene::components::PropertyShape,
    /// The value differs from the script's default (a reset button shows).
    pub(crate) changed: bool,
    pub(crate) objects: &'a [(bayleaf_core::StableId, String)],
    pub(crate) layout: Option<&'a bayleaf_engine::scene::inspector::PropertyLayout>,
}

pub(crate) fn named_property(
    ui: &mut egui::Ui,
    property: &PropertyRow<'_>,
    value: &mut PropertyValue,
    salt: impl std::hash::Hash + std::fmt::Debug,
) -> (Outcome, bool) {
    let t = theme::tokens(ui);
    let mut reset = false;
    let layout = property.layout;
    let title = layout.and_then(|l| l.label.clone()).unwrap_or_else(|| title_case(property.name));
    let read_only = layout.is_some_and(|l| l.read_only);
    let hint = layout.map_or("", |l| l.hint.as_str());
    let shown = ui.push_id(salt, |ui| {
        row(ui, t, &title, |ui, label, _| {
            if !hint.is_empty() {
                ui.ctx().accesskit_node_builder(label, |node| node.set_description(hint.to_owned()));
            }
            // The reset button sits at the right edge and the field takes what is left, so the
            // row never grows past the panel. It is always laid out, so the field keeps its place
            // (and a drag on it keeps going) when the value first leaves its default.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if reset_button(ui, property.changed && !read_only, &title).clicked() {
                    reset = true;
                }
                let field_width = (ui.available_width() - 1.0).max(40.0);
                // A read-only value shows, greyed, and takes no edits.
                ui.add_enabled_ui(!read_only, |ui| property_field(ui, property.ty, value, property.objects, label, field_width, layout)).inner
            })
            .inner
        })
    });
    let outcome = shown.inner;
    if !hint.is_empty() {
        shown.response.on_hover_text(hint);
    }
    (outcome, reset)
}

/// A list or map script property: its title with a button that adds an item (or key) and the reset
/// button on one row, then a row per item (a list's by its place, a map's with its key in a field)
/// with the field of its type and a × that takes it out. Returns the edit outcome and whether reset
/// was clicked.
pub(crate) fn shaped_property(
    ui: &mut egui::Ui,
    property: &PropertyRow<'_>,
    value: &mut PropertyValue,
    salt: impl std::hash::Hash + std::fmt::Debug,
) -> (Outcome, bool) {
    let t = theme::tokens(ui);
    let mut reset = false;
    let mut outcome = Outcome::default();
    let layout = property.layout;
    let title = layout.and_then(|l| l.label.clone()).unwrap_or_else(|| title_case(property.name));
    let read_only = layout.is_some_and(|l| l.read_only);
    let ty = property.ty;
    // A map's keys are typed as text and kept only when they are keys of its keys' type.
    let key_type = match property.shape {
        bayleaf_engine::scene::components::PropertyShape::Map(key) => key,
        _ => PropertyType::Text,
    };
    ui.push_id(salt, |ui| {
        let count = match &*value {
            PropertyValue::List { list } => list.len(),
            PropertyValue::Map { map } => map.len(),
            _ => 0,
        };
        let is_map = matches!(value, PropertyValue::Map { .. });
        row(ui, t, &title, |ui, _, _| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if reset_button(ui, property.changed && !read_only, &title).clicked() {
                    reset = true;
                }
                let (word, hint) = if is_map { ("Add Key", "Add a key to {name}") } else { ("Add Item", "Add an item to {name}") };
                let add = ui.add_enabled(!read_only && count < MAX_PROPERTY_ITEMS, egui::Button::new(tl(word)).small());
                ui.ctx().accesskit_node_builder(add.id, |node| node.set_label(crate::i18n::tf(hint, &[("name", &title)])));
                if add.clicked() {
                    match value {
                        PropertyValue::List { list } => list.push(ty.zero()),
                        PropertyValue::Map { map } => {
                            let fresh = key_type.fresh_key(|k| map.contains_key(k));
                            if let Some(fresh) = fresh {
                                map.insert(fresh, ty.zero());
                            }
                        }
                        _ => {}
                    }
                    outcome = Outcome { changed: true, ended: true };
                }
            });
        });
        ui.add_enabled_ui(!read_only, |ui| {
            ui.indent("items", |ui| match value {
                PropertyValue::List { list } => {
                    let mut gone = None;
                    for (i, item) in list.iter_mut().enumerate() {
                        ui.push_id(i, |ui| {
                            row(ui, t, &i.to_string(), |ui, label, _| {
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let remove = ui.small_button("×").on_hover_text(tl("Remove"));
                                    let name = crate::i18n::tf("Remove item {index} of {name}", &[("index", &i.to_string()), ("name", &title)]);
                                    ui.ctx().accesskit_node_builder(remove.id, |node| node.set_label(name));
                                    if remove.clicked() {
                                        gone = Some(i);
                                    }
                                    let width = (ui.available_width() - 1.0).max(40.0);
                                    outcome.merge(property_field(ui, ty, item, property.objects, label, width, None));
                                });
                            });
                        });
                    }
                    if let Some(i) = gone {
                        list.remove(i);
                        outcome = Outcome { changed: true, ended: true };
                    }
                }
                PropertyValue::Map { map } => {
                    let (mut gone, mut renamed) = (None, None);
                    for (i, (key, item)) in map.iter_mut().enumerate() {
                        ui.push_id(i, |ui| {
                            ui.horizontal(|ui| {
                                ui.set_min_height(ROW_HEIGHT);
                                // The key as typed stays until the field is left, then the entry moves to it.
                                let id = ui.make_persistent_id("key");
                                let mut typed = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| key.clone());
                                let field = ui.add_sized([LABEL_WIDTH, ROW_HEIGHT], egui::TextEdit::singleline(&mut typed).char_limit(MAX_PROPERTY_TEXT));
                                let name = crate::i18n::tf("Key {index} of {name}", &[("index", &i.to_string()), ("name", &title)]);
                                ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(name));
                                if field.lost_focus() {
                                    if typed != *key {
                                        renamed = Some((key.clone(), typed));
                                    }
                                    ui.data_mut(|d| d.remove::<String>(id));
                                } else if field.has_focus() {
                                    ui.data_mut(|d| d.insert_temp(id, typed));
                                }
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let remove = ui.small_button("×").on_hover_text(tl("Remove"));
                                    let name = crate::i18n::tf("Remove {key} from {name}", &[("key", key), ("name", &title)]);
                                    ui.ctx().accesskit_node_builder(remove.id, |node| node.set_label(name));
                                    if remove.clicked() {
                                        gone = Some(key.clone());
                                    }
                                    let width = (ui.available_width() - 1.0).max(40.0);
                                    outcome.merge(property_field(ui, ty, item, property.objects, field.id, width, None));
                                });
                            });
                        });
                    }
                    if let Some(key) = gone {
                        map.remove(&key);
                        outcome = Outcome { changed: true, ended: true };
                    }
                    // A key renamed to one the map has already, or to no key of its type, stays as it was.
                    if let Some((old, new)) = renamed.filter(|(_, new)| key_type.fits_key(new) && !map.contains_key(new))
                        && let Some(item) = map.remove(&old)
                    {
                        map.insert(new, item);
                        outcome = Outcome { changed: true, ended: true };
                    }
                }
                _ => {}
            });
        });
    });
    (outcome, reset)
}

/// A list of choices: a whole number by its place in `choices`, or a text from them.
fn choice_field(ui: &mut egui::Ui, value: &mut PropertyValue, choices: &[String], label: egui::Id, width: f32) -> Outcome {
    let shown = match &*value {
        PropertyValue::Number(n) => choices.get(*n as usize).cloned().unwrap_or_else(|| format!("{n}")),
        PropertyValue::Text(text) => text.clone(),
        _ => String::new(),
    };
    let mut picked = None;
    let combo = craft_ui::ComboBox::from_id_salt("choices").selected_text(shown).width(width).truncate().show_ui(ui, |ui| {
        for (i, choice) in choices.iter().enumerate() {
            let current = match &*value {
                PropertyValue::Number(n) => *n as usize == i,
                PropertyValue::Text(text) => text == choice,
                _ => false,
            };
            if ui.selectable_label(current, choice).clicked() && !current {
                picked = Some(i);
            }
        }
    });
    combo.response.labelled_by(label);
    let Some(i) = picked else { return Outcome::default() };
    match value {
        PropertyValue::Number(n) => *n = i as f64,
        PropertyValue::Text(text) => *text = choices[i].clone(),
        _ => return Outcome::default(),
    }
    Outcome { changed: true, ended: true }
}

/// The field for one script property value of type `ty`.
fn property_field(
    ui: &mut egui::Ui,
    ty: PropertyType,
    value: &mut PropertyValue,
    objects: &[(bayleaf_core::StableId, String)],
    label: egui::Id,
    width: f32,
    layout: Option<&bayleaf_engine::scene::inspector::PropertyLayout>,
) -> Outcome {
    let range = layout.and_then(|l| l.range);
    let choices = layout.map_or(&[][..], |l| l.choices.as_slice());
    if !choices.is_empty() && matches!((ty, &*value), (PropertyType::WholeNumber, PropertyValue::Number(_)) | (PropertyType::Text, PropertyValue::Text(_))) {
        return choice_field(ui, value, choices, label, width);
    }
    match (ty, &mut *value) {
        (PropertyType::Number, PropertyValue::Number(n)) => {
            let response = match range {
                Some((low, high)) if layout.is_some_and(|l| l.slider) => {
                    ui.spacing_mut().slider_width = (width - 64.0).max(40.0);
                    ui.add(egui::Slider::new(n, low..=high).max_decimals(3))
                }
                Some((low, high)) => ui.add_sized([width, ROW_HEIGHT], egui::DragValue::new(n).speed(0.02).max_decimals(3).range(low..=high)).wrap_drag(),
                None => ui.add_sized([width, ROW_HEIGHT], egui::DragValue::new(n).speed(0.02).max_decimals(3)).wrap_drag(),
            }
            .labelled_by(label);
            Outcome::from_response(&response)
        }
        (PropertyType::WholeNumber, PropertyValue::Number(n)) => {
            let mut whole = *n as i32;
            let (low, high) = range.map_or((i32::MIN, i32::MAX), |(low, high)| (low.ceil() as i32, high.floor() as i32));
            let response = match range {
                Some(_) if layout.is_some_and(|l| l.slider) => {
                    ui.spacing_mut().slider_width = (width - 64.0).max(40.0);
                    ui.add(egui::Slider::new(&mut whole, low..=high))
                }
                _ => ui.add_sized([width, ROW_HEIGHT], egui::DragValue::new(&mut whole).speed(0.1).range(low..=high)).wrap_drag(),
            }
            .labelled_by(label);
            *n = f64::from(whole);
            Outcome::from_response(&response)
        }
        (PropertyType::TrueFalse, PropertyValue::Flag(b)) => {
            let response = ui.checkbox(b, "").labelled_by(label);
            Outcome { changed: response.changed(), ended: response.changed() }
        }
        (PropertyType::Vector, PropertyValue::Triple(v)) => {
            let mut items: Vec<Value> = v.iter().map(|x| Value::from(f64::from(*x))).collect();
            let outcome = edit_vector(ui, &mut items, width);
            for (slot, item) in v.iter_mut().zip(&items) {
                *slot = item.as_f64().unwrap_or(0.0) as f32;
            }
            outcome
        }
        (PropertyType::Color, PropertyValue::Triple(v)) => {
            let response = ui.color_edit_button_rgb(v).labelled_by(label);
            Outcome { changed: response.changed(), ended: response.changed() }
        }
        (PropertyType::Text, PropertyValue::Text(text)) => {
            let response = ui.add_sized([width, ROW_HEIGHT], egui::TextEdit::singleline(text).char_limit(MAX_PROPERTY_TEXT)).labelled_by(label);
            Outcome::from_response(&response)
        }
        (PropertyType::Object, PropertyValue::Text(chosen)) => {
            let shown = if chosen.is_empty() { tl("None").to_owned() } else { chosen.clone() };
            let mut picked = None;
            let combo = egui::ComboBox::from_id_salt("object").selected_text(shown).width(width).show_ui(ui, |ui| {
                if ui.selectable_label(chosen.is_empty(), tl("None")).clicked() {
                    picked = Some(String::new());
                }
                for (_, name) in objects {
                    if ui.selectable_label(chosen == name, name).clicked() {
                        picked = Some(name.clone());
                    }
                }
            });
            let response = combo.response.labelled_by(label);
            // An object dragged from the hierarchy and dropped on the field chooses it.
            if let Some(id) = crate::hierarchy::dragged_object(ui.ctx())
                && response.contains_pointer()
                && ui.input(|i| i.pointer.any_released())
                && let Some((_, name)) = objects.iter().find(|(o, _)| *o == id)
            {
                picked = Some(name.clone());
            }
            match picked {
                Some(name) if name != *chosen => {
                    *chosen = name;
                    Outcome { changed: true, ended: true }
                }
                _ => Outcome::default(),
            }
        }
        // A saved value of another shape than the script declares: start from the type's zero.
        _ => {
            if ui.add_sized([width, ROW_HEIGHT], egui::Button::new(tl("Not this type: reset"))).labelled_by(label).clicked() {
                *value = ty.zero();
                return Outcome { changed: true, ended: true };
            }
            Outcome::default()
        }
    }
}

/// A square button with a circular-arrow icon that puts `name` back to its default; greyed out and
/// inert while it already is.
fn reset_button(ui: &mut egui::Ui, enabled: bool, name: &str) -> egui::Response {
    let button = ui
        .add_enabled(enabled, egui::Button::new("").min_size(egui::vec2(ROW_HEIGHT, ROW_HEIGHT)))
        .on_hover_text(tl("Back to the script's default"))
        .on_disabled_hover_text(tl("This is the script's default"));
    ui.ctx().accesskit_node_builder(button.id, |node| node.set_label(crate::i18n::tf("Reset {name}", &[("name", name)])));
    let t = theme::tokens(ui);
    let color = if enabled { t.text } else { t.text_muted.gamma_multiply(0.6) };
    // An arc of most of a circle, open at the top right, with an arrowhead at its end.
    let center = button.rect.center();
    let radius = ROW_HEIGHT * 0.22;
    let (start, sweep) = (-0.25 * std::f32::consts::PI, 1.6 * std::f32::consts::PI);
    let at = |a: f32| center + radius * egui::vec2(a.cos(), a.sin());
    let points: Vec<egui::Pos2> = (0..=24).map(|i| at(start + sweep * i as f32 / 24.0)).collect();
    let stroke = egui::Stroke::new(1.5, color);
    ui.painter().add(egui::Shape::line(points, stroke));
    let tip = at(start);
    let along = egui::vec2(start.sin(), -start.cos()) * -1.0;
    let out = egui::vec2(start.cos(), start.sin());
    let size = ROW_HEIGHT * 0.14;
    ui.painter().add(egui::Shape::convex_polygon(vec![tip + along * size, tip - out * size, tip + out * size], color, egui::Stroke::NONE));
    button
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
