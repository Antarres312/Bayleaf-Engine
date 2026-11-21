//! The Inspector's editor for Input Map assets (`.binput`): actions named `map/action`, each with a
//! kind and its bindings, and the map play starts with. Edits are kept as typed and saved through
//! `input.asset.set` once every action reads right; until then the problem shows under the list.
//!
//! Each action is a card of two lines (name and kind; then its bindings and Remove),
//! with widths fixed from the card's width, so rows never push the panel wider.

use bayleaf_engine::input_cmds::{InputAction, InputAsset};
use bayleaf_engine::script::actions::ActionKind;

use crate::theme::{self, ROW_HEIGHT, Tokens};

const KIND_WIDTH: f32 = 92.0;
const REMOVE_WIDTH: f32 = 76.0;

/// The kinds, with what each means.
const KINDS: [(ActionKind, &str, &str); 3] = [
    (ActionKind::Trigger, "Trigger", "On only in the step it is pressed: a jump, a shot"),
    (ActionKind::Held, "Held", "On from the press for as long as it is held"),
    (ActionKind::Value, "Value", "How far it is pressed, 0 to 1: walking, steering"),
];

struct Row {
    name: String,
    kind: ActionKind,
    /// Bindings as typed, separated by commas.
    bindings: String,
}

/// An Input Map as being edited.
pub(crate) struct Draft {
    start: String,
    rows: Vec<Row>,
    /// Why the typed actions are not saved.
    problem: Option<String>,
}

impl Draft {
    pub(crate) fn of(asset: &InputAsset) -> Self {
        let rows = asset.actions.iter().map(|a| Row { name: a.name.clone(), kind: a.kind, bindings: a.bindings.join(", ") }).collect();
        Self { start: asset.start.clone(), rows, problem: None }
    }

    fn asset(&self) -> InputAsset {
        let actions = self
            .rows
            .iter()
            .map(|r| InputAction {
                name: r.name.trim().to_owned(),
                kind: r.kind,
                bindings: r.bindings.split(',').map(str::trim).filter(|b| !b.is_empty()).map(str::to_owned).collect(),
            })
            .collect();
        InputAsset::new(&self.start, actions)
    }

    /// The maps the typed names make, in order.
    fn maps(&self) -> Vec<String> {
        let mut maps: Vec<String> = Vec::new();
        for row in &self.rows {
            if let Some((map, _)) = row.name.split_once('/')
                && !map.is_empty()
                && !maps.iter().any(|m| m == map)
            {
                maps.push(map.to_owned());
            }
        }
        maps
    }
}

/// Draws the editor. Returns the asset to save when an edit leaves every action reading right.
pub(crate) fn show(ui: &mut egui::Ui, draft: &mut Draft) -> Option<InputAsset> {
    let t = theme::tokens(ui);
    let mut changed = false;
    ui.label(egui::RichText::new("Actions").small().color(t.text_muted));
    ui.add(
        egui::Label::new(
            egui::RichText::new("Name each map/action (Player/Jump). One map is active at a time; scripts ask for Jump in the active map or Player/Jump. Bindings: key:W, mouse:Left, pad:South, axis:LeftX+, separated by commas.")
                .small()
                .color(t.text_muted),
        )
        .wrap(),
    );
    ui.add_space(theme::GAP);
    changed |= start_row(ui, t, draft);
    ui.add_space(theme::GAP);

    let mut remove = None;
    for (index, row) in draft.rows.iter_mut().enumerate() {
        ui.push_id(("input action", index), |ui| {
            theme::section(t).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let width = ui.available_width();
                let gap = ui.spacing().item_spacing.x;
                let shown = if row.name.trim().is_empty() { format!("action {}", index + 1) } else { row.name.trim().to_owned() };
                ui.horizontal(|ui| {
                    let name_width = (width - KIND_WIDTH - gap).max(40.0);
                    let name = ui.add_sized([name_width, ROW_HEIGHT - 4.0], egui::TextEdit::singleline(&mut row.name).hint_text("Map/Action"));
                    ui.ctx().accesskit_node_builder(name.id, |node| node.set_label(format!("Name of {shown}")));
                    changed |= name.changed();
                    let current = KINDS.iter().find(|(k, ..)| *k == row.kind).map_or("Held", |(_, label, _)| label);
                    let kind = craft_ui::ComboBox::from_id_salt("kind").width(KIND_WIDTH).selected_text(current).show_ui(ui, |ui| {
                        for (kind, label, what) in KINDS {
                            if ui.selectable_label(row.kind == kind, label).on_hover_text(what).clicked() && row.kind != kind {
                                row.kind = kind;
                                changed = true;
                            }
                        }
                    });
                    ui.ctx().accesskit_node_builder(kind.response.id, |node| node.set_label(format!("Kind of {shown}: {current}")));
                });
                ui.horizontal(|ui| {
                    let bindings_width = (width - REMOVE_WIDTH - gap).max(40.0);
                    let bindings =
                        ui.add_sized([bindings_width, ROW_HEIGHT - 4.0], egui::TextEdit::singleline(&mut row.bindings).hint_text("key:Space, pad:South"));
                    ui.ctx().accesskit_node_builder(bindings.id, |node| node.set_label(format!("Bindings of {shown}")));
                    changed |= bindings.changed();
                    let button = ui.add_sized([REMOVE_WIDTH, ROW_HEIGHT - 4.0], egui::Button::new("Remove"));
                    ui.ctx().accesskit_node_builder(button.id, |node| node.set_label(format!("Remove {shown}")));
                    if button.on_hover_text("Remove this action").clicked() {
                        remove = Some(index);
                    }
                });
            });
        });
        ui.add_space(theme::SECTION_GAP);
    }
    if let Some(index) = remove {
        draft.rows.remove(index);
        changed = true;
    }
    if ui.button("Add Action").on_hover_text("A new action in the starting map").clicked() {
        let map = if draft.start.is_empty() { draft.maps().into_iter().next().unwrap_or_else(|| "Player".to_owned()) } else { draft.start.clone() };
        draft.rows.push(Row { name: format!("{map}/Action{}", draft.rows.len() + 1), kind: ActionKind::Held, bindings: String::new() });
        changed = true;
    }
    if let Some(problem) = &draft.problem {
        ui.add(egui::Label::new(egui::RichText::new(format!("Not saved: {problem}")).color(t.text)).wrap());
    }
    if !changed {
        return None;
    }
    let asset = draft.asset();
    match asset.maps() {
        Ok(_) => {
            draft.problem = None;
            Some(asset)
        }
        Err(problem) => {
            draft.problem = Some(problem);
            None
        }
    }
}

/// The map play starts with: one of the maps the names make.
fn start_row(ui: &mut egui::Ui, t: &Tokens, draft: &mut Draft) -> bool {
    let maps = draft.maps();
    let mut changed = false;
    crate::value_editor::row(ui, t, "Starts with", |ui, label, width| {
        let shown = if draft.start.is_empty() { maps.first().cloned().unwrap_or_else(|| "No maps yet".to_owned()) } else { draft.start.clone() };
        craft_ui::ComboBox::from_id_salt("input start map")
            .width(width)
            .truncate()
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for map in &maps {
                    if ui.selectable_label(draft.start == *map, map).clicked() && draft.start != *map {
                        draft.start = map.clone();
                        changed = true;
                    }
                }
            })
            .response
            .labelled_by(label);
    });
    let maps_text = if maps.is_empty() { "No maps yet".to_owned() } else { format!("Maps: {}", maps.join(", ")) };
    ui.label(egui::RichText::new(maps_text).small().color(t.text_muted));
    changed
}
