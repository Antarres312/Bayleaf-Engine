//! The Inspector's editors for data shapes (`.bshape`: fields, each with a name, a type, a default
//! and how it shows) and data files (`.bdata`: the shape chosen, then a row per field of it). The
//! Inspector writes what they return through `data.shape.set`, `data.set` and `data.field.set`.

use bayleaf_engine::scene::components::{PropertyShape, PropertyType, PropertyValue};
use bayleaf_engine::scene::data::{DataField, DataFile, DataShape, type_code};
use bayleaf_engine::scene::inspector::{InspectorItem, PropertyLayout};

use crate::i18n::tl;
use crate::theme::{self, ROW_HEIGHT, Tokens};
use crate::value_editor::{self, PropertyRow};

/// What a field holds, as the shape editor offers it.
const HOLDS: [(PropertyShape, &str); 4] = [
    (PropertyShape::Single, "One Value"),
    (PropertyShape::List, "List"),
    (PropertyShape::Map(PropertyType::Text), "Map by Text"),
    (PropertyShape::Map(PropertyType::WholeNumber), "Map by Whole Number"),
];

/// A change asked of a field's place in the shape.
enum Move {
    Up,
    Down,
    Remove,
}

/// Edits `shape` in place. Returns it when an edit has finished and the shape reads, to be written;
/// while it does not read (two fields of one name, say) it says why and nothing is written.
pub(crate) fn shape_editor(ui: &mut egui::Ui, shape: &mut DataShape, files: &[String]) -> Option<DataShape> {
    let t = theme::tokens(ui);
    let mut ended = false;
    let mut moved: Option<(usize, Move)> = None;
    ui.label(egui::RichText::new(tl("Fields")).small().color(t.text_muted));
    let count = shape.fields.len();
    for (i, field) in shape.fields.iter_mut().enumerate() {
        ui.push_id(("data field", i), |ui| {
            theme::section(t).inner_margin(egui::Margin::symmetric(6, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ended |= field_rows(ui, t, field, files);
                ui.horizontal(|ui| {
                    let shown = field.name.clone();
                    if ui.add_enabled(i > 0, egui::Button::new(tl("Up"))).on_hover_text(crate::i18n::tf("Moves {field} up", &[("field", &shown)])).clicked() {
                        moved = Some((i, Move::Up));
                    }
                    if ui.add_enabled(i + 1 < count, egui::Button::new(tl("Down"))).on_hover_text(crate::i18n::tf("Moves {field} down", &[("field", &shown)])).clicked() {
                        moved = Some((i, Move::Down));
                    }
                    if ui.button(tl("Remove")).on_hover_text(crate::i18n::tf("Removes the field {field}", &[("field", &shown)])).clicked() {
                        moved = Some((i, Move::Remove));
                    }
                });
            });
        });
        ui.add_space(theme::GAP);
    }
    if ui.button(tl("Add Field")).on_hover_text(tl("Adds a number field at the end")).clicked() {
        let taken = |name: &str| shape.fields.iter().any(|f| f.name == name);
        let name = (1..=bayleaf_engine::scene::data::MAX_FIELDS + 1)
            .map(|n| if n == 1 { "field".to_owned() } else { format!("field{n}") })
            .find(|n| !taken(n))
            .unwrap_or_default();
        shape.fields.push(DataField::new(&name, PropertyType::Number, PropertyShape::Single));
        ended = true;
    }
    match moved {
        Some((i, Move::Up)) if i > 0 => shape.fields.swap(i - 1, i),
        Some((i, Move::Down)) if i + 1 < shape.fields.len() => shape.fields.swap(i, i + 1),
        Some((i, Move::Remove)) => {
            shape.fields.remove(i);
        }
        _ => {}
    }
    ended |= moved.is_some();
    match shape.validate() {
        Ok(()) => ended.then(|| shape.clone()),
        Err(problem) => {
            theme::section(t).fill(t.warning).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tf("Not saved: {problem}", &[("problem", &problem)])).color(t.on_warning)).wrap());
            });
            None
        }
    }
}

/// A field's rows: name, type, what it holds, default, label, hint, group and (for numbers) range.
/// Returns whether an edit finished.
fn field_rows(ui: &mut egui::Ui, t: &Tokens, field: &mut DataField, files: &[String]) -> bool {
    let mut ended = false;
    value_editor::row(ui, t, "Name", |ui, label, width| {
        let response = ui.add_sized([width, ROW_HEIGHT], egui::TextEdit::singleline(&mut field.name).char_limit(bayleaf_engine::scene::data::MAX_NAME)).labelled_by(label);
        ended |= response.lost_focus();
    });
    let (ty, shape) = field.kind().unwrap_or((PropertyType::Number, PropertyShape::Single));
    let mut chosen = (ty, shape);
    value_editor::row(ui, t, "Type", |ui, label, width| {
        let combo = craft_ui::ComboBox::from_id_salt("field type").width(width).selected_text(tl(ty.label())).show_ui(ui, |ui| {
            for option in PropertyType::ALL {
                if ui.selectable_label(option == ty, tl(option.label())).clicked() {
                    chosen.0 = option;
                }
            }
        });
        combo.response.labelled_by(label);
    });
    value_editor::row(ui, t, "Holds", |ui, label, width| {
        let shown = HOLDS.iter().find(|(s, _)| *s == shape).map_or("One Value", |(_, l)| l);
        let combo = craft_ui::ComboBox::from_id_salt("field holds").width(width).selected_text(tl(shown)).show_ui(ui, |ui| {
            for (option, text) in HOLDS {
                if ui.selectable_label(option == shape, tl(text)).clicked() {
                    chosen.1 = option;
                }
            }
        });
        combo.response.labelled_by(label);
    });
    if chosen != (ty, shape) {
        // A new type starts from its zero; a range only fits one number.
        field.code = type_code(chosen.0, chosen.1);
        field.default = chosen.0.zero_of(chosen.1);
        if !(matches!(chosen.0, PropertyType::Number | PropertyType::WholeNumber) && chosen.1 == PropertyShape::Single) {
            field.range = None;
            field.slider = false;
        }
        return true;
    }
    let row = PropertyRow { name: "default", ty, shape, changed: field.default != ty.zero_of(shape), objects: &[], files, layout: None };
    let (outcome, reset) = if shape == PropertyShape::Single {
        value_editor::named_property(ui, &row, &mut field.default, "field default")
    } else {
        value_editor::shaped_property(ui, &row, &mut field.default, "field default")
    };
    if reset {
        field.default = ty.zero_of(shape);
    }
    ended |= reset || (outcome.changed && outcome.ended);
    ended |= optional_text(ui, t, "Label", &mut field.label, bayleaf_engine::scene::data::MAX_NAME);
    let mut hint = Some(field.hint.clone()).filter(|h| !h.is_empty());
    if optional_text(ui, t, "Hint", &mut hint, bayleaf_engine::scene::data::MAX_HINT) {
        field.hint = hint.unwrap_or_default();
        ended = true;
    }
    ended |= optional_text(ui, t, "Group", &mut field.group, bayleaf_engine::scene::data::MAX_NAME);
    if matches!(ty, PropertyType::Number | PropertyType::WholeNumber) && shape == PropertyShape::Single {
        ended |= range_rows(ui, t, field);
    }
    ended
}

/// A text row for an optional text (empty for none); returns whether its edit finished.
fn optional_text(ui: &mut egui::Ui, t: &Tokens, title: &str, value: &mut Option<String>, max: usize) -> bool {
    let mut text = value.clone().unwrap_or_default();
    let finished = value_editor::row(ui, t, title, |ui, label, width| {
        ui.add_sized([width, ROW_HEIGHT], egui::TextEdit::singleline(&mut text).char_limit(max)).labelled_by(label).lost_focus()
    });
    let new = Some(text).filter(|t| !t.is_empty());
    if new != *value {
        *value = new;
    }
    finished
}

/// Whether a number field has a range, the range, and whether it shows as a slider.
fn range_rows(ui: &mut egui::Ui, t: &Tokens, field: &mut DataField) -> bool {
    let mut ended = false;
    value_editor::row(ui, t, "Range", |ui, label, _| {
        let mut on = field.range.is_some();
        if ui.checkbox(&mut on, "").labelled_by(label).changed() {
            field.range = on.then_some([0.0, 1.0]);
            field.slider &= on;
            ended = true;
        }
        if let Some([low, high]) = field.range.as_mut() {
            let from = ui.add(egui::DragValue::new(low).speed(0.05).prefix(format!("{} ", tl("From"))));
            let to = ui.add(egui::DragValue::new(high).speed(0.05).prefix(format!("{} ", tl("To"))));
            ended |= from.drag_stopped() || from.lost_focus() || to.drag_stopped() || to.lost_focus();
        }
    });
    if field.range.is_some() {
        value_editor::row(ui, t, "Slider", |ui, label, _| {
            ended |= ui.checkbox(&mut field.slider, "").labelled_by(label).changed();
        });
    }
    ended
}

/// A change to a data file the Inspector writes.
pub(crate) enum DataEdit {
    /// Another shape (its path; empty for none), keeping the values set.
    Shape(String),
    /// One field's value; `None` returns it to the shape's default.
    Field(String, Option<PropertyValue>),
}

/// A data file's shape and, when it reads, a row per field. `shapes` are the project's shapes, and
/// `files` the files file fields may name.
pub(crate) fn data_editor(
    ui: &mut egui::Ui,
    data: &DataFile,
    shape: Option<&Result<DataShape, String>>,
    shapes: &[String],
    files: &[String],
) -> Option<DataEdit> {
    let t = theme::tokens(ui);
    let mut edit = None;
    value_editor::row(ui, t, "Shape", |ui, label, width| {
        let shown = if data.shape.is_empty() { tl("None").to_owned() } else { data.shape.clone() };
        let combo = craft_ui::ComboBox::from_id_salt("data shape").width(width).selected_text(shown).show_ui(ui, |ui| {
            if ui.selectable_label(data.shape.is_empty(), tl("None")).clicked() && !data.shape.is_empty() {
                edit = Some(DataEdit::Shape(String::new()));
            }
            for path in shapes {
                if ui.selectable_label(data.shape == *path, path).clicked() && data.shape != *path {
                    edit = Some(DataEdit::Shape(path.clone()));
                }
            }
        });
        let response = combo.response.labelled_by(label);
        let is_shape = |p: &str| p.to_ascii_lowercase().ends_with(&format!(".{}", bayleaf_engine::scene::data::SHAPE_EXTENSION));
        if let Some(dropped) = value_editor::file_drop(ui, &response, is_shape) {
            edit = Some(DataEdit::Shape(dropped));
        }
    });
    ui.add_space(theme::GAP);
    match shape {
        None => {
            ui.add(egui::Label::new(egui::RichText::new(tl("Choose the shape that says which fields this data file has.")).color(t.text_muted)).wrap());
        }
        Some(Err(problem)) => {
            ui.add(egui::Label::new(crate::i18n::tf("Its shape does not read: {problem}", &[("problem", problem)])).wrap());
        }
        Some(Ok(shape)) => {
            for item in shape.layout() {
                if let Some(change) = layout_item(ui, data, shape, &item, files) {
                    edit = Some(change);
                }
            }
        }
    }
    edit
}

/// One item of a shape's layout: a field's row, or a group of them under its title.
fn layout_item(ui: &mut egui::Ui, data: &DataFile, shape: &DataShape, item: &InspectorItem, files: &[String]) -> Option<DataEdit> {
    match item {
        InspectorItem::Property(layout) => field_row(ui, data, shape, layout, files),
        InspectorItem::Group { title, items, .. } => {
            let mut edit = None;
            egui::CollapsingHeader::new(title).id_salt(("data group", title)).default_open(true).show(ui, |ui| {
                for item in items {
                    if let Some(change) = layout_item(ui, data, shape, item, files) {
                        edit = Some(change);
                    }
                }
            });
            edit
        }
        InspectorItem::Button { .. } | InspectorItem::Note { .. } => None,
    }
}

/// A field's row: its value in the file when it fits, else the shape's default (and a reset).
fn field_row(ui: &mut egui::Ui, data: &DataFile, shape: &DataShape, layout: &PropertyLayout, files: &[String]) -> Option<DataEdit> {
    let declared = shape.field(&layout.name)?.declared()?;
    let set = data.values.get(&declared.name).filter(|v| declared.accepts(v));
    let mut value = set.unwrap_or(&declared.default).clone();
    let row = PropertyRow {
        name: &declared.name,
        ty: declared.ty,
        shape: declared.shape,
        changed: set.is_some_and(|v| *v != declared.default),
        objects: &[],
        files,
        layout: Some(layout),
    };
    let salt = ("data value", declared.name.as_str());
    let (outcome, reset) = if declared.shape == PropertyShape::Single {
        value_editor::named_property(ui, &row, &mut value, salt)
    } else {
        value_editor::shaped_property(ui, &row, &mut value, salt)
    };
    if reset {
        return Some(DataEdit::Field(declared.name, None));
    }
    outcome.changed.then(|| DataEdit::Field(declared.name.clone(), Some(value)))
}
