//! The Inspector's editor for Input Map assets (`.binput`): a list of maps, each with its actions
//! (a name, a kind and bindings), and the map play starts with. Code and visual scripts name an
//! action `map/action`; here each action sits in its map. Edits are kept as typed and saved
//! through `input.asset.set` once everything reads right; until then the problem shows below.
//!
//! Each action is a card of two lines (name and kind; then its bindings and Remove), with widths
//! fixed from the card's width, so rows never push the panel wider.

use crate::i18n::tl;
use bayleaf_engine::input_cmds::{InputAction, InputAsset, InputMapDef};
use bayleaf_engine::script::actions::ActionKind;

use crate::theme::{self, ROW_HEIGHT, Tokens};

const KIND_WIDTH: f32 = 92.0;
const REMOVE_WIDTH: f32 = 76.0;
const REMOVE_MAP_WIDTH: f32 = 108.0;

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

struct MapDraft {
    name: String,
    actions: Vec<Row>,
}

/// An Input Map as being edited.
pub(crate) struct Draft {
    start: String,
    maps: Vec<MapDraft>,
    /// Why the typed maps are not saved.
    problem: Option<String>,
}

impl Draft {
    pub(crate) fn of(asset: &InputAsset) -> Self {
        let maps = asset
            .maps
            .iter()
            .map(|m| MapDraft {
                name: m.name.clone(),
                actions: m.actions.iter().map(|a| Row { name: a.name.clone(), kind: a.kind, bindings: a.bindings.join(", ") }).collect(),
            })
            .collect();
        Self { start: asset.start.clone(), maps, problem: None }
    }

    fn asset(&self) -> InputAsset {
        let maps = self
            .maps
            .iter()
            .map(|m| InputMapDef {
                name: m.name.trim().to_owned(),
                actions: m
                    .actions
                    .iter()
                    .map(|r| InputAction {
                        name: r.name.trim().to_owned(),
                        kind: r.kind,
                        bindings: r.bindings.split(',').map(str::trim).filter(|b| !b.is_empty()).map(str::to_owned).collect(),
                    })
                    .collect(),
            })
            .collect();
        InputAsset::new(&self.start, maps)
    }
}

/// What a click asks for, applied once the list is drawn.
enum Change {
    RemoveMap(usize),
    RemoveAction(usize, usize),
    AddAction(usize),
    AddMap,
}

/// Draws the editor. Returns the asset to save when an edit leaves everything reading right.
pub(crate) fn show(ui: &mut egui::Ui, draft: &mut Draft) -> Option<InputAsset> {
    let t = theme::tokens(ui);
    let mut changed = false;
    ui.label(egui::RichText::new(tl("Maps")).small().color(t.text_muted));
    ui.add(
        egui::Label::new(
            egui::RichText::new(tl("Each map holds its actions; one map is active at a time. Code and visual scripts name an action Map/Action (Player/Jump), or just Jump for the active map. Bindings: key:W, mouse:Left, pad:South, axis:LeftX+, separated by commas."))
                .small()
                .color(t.text_muted),
        )
        .wrap(),
    );
    ui.add_space(theme::GAP);
    changed |= start_row(ui, t, draft);
    ui.add_space(theme::GAP);

    let mut change = None;
    for (m, map) in draft.maps.iter_mut().enumerate() {
        ui.push_id(("input map", m), |ui| {
            theme::section(t).inner_margin(egui::Margin::symmetric(6, 6)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let width = ui.available_width();
                let gap = ui.spacing().item_spacing.x;
                let map_shown = if map.name.trim().is_empty() { format!("map {}", m + 1) } else { map.name.trim().to_owned() };
                ui.horizontal(|ui| {
                    let field = ui.add_sized(
                        [(width - REMOVE_MAP_WIDTH - gap).max(40.0), ROW_HEIGHT - 4.0],
                        egui::TextEdit::singleline(&mut map.name).hint_text(tl("Map name")).font(egui::TextStyle::Button),
                    );
                    ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(format!("Name of map {map_shown}")));
                    changed |= field.changed();
                    let remove = ui.add_sized([REMOVE_MAP_WIDTH, ROW_HEIGHT - 4.0], egui::Button::new(tl("Remove Map")));
                    ui.ctx().accesskit_node_builder(remove.id, |node| node.set_label(format!("Remove map {map_shown}")));
                    if remove.on_hover_text(tl("Remove this map and its actions")).clicked() {
                        change = Some(Change::RemoveMap(m));
                    }
                });
                ui.add_space(4.0);
                for (a, row) in map.actions.iter_mut().enumerate() {
                    ui.push_id(("action", a), |ui| {
                        if action_card(ui, t, &map_shown, a, row, &mut changed) {
                            change = Some(Change::RemoveAction(m, a));
                        }
                    });
                    ui.add_space(theme::SECTION_GAP);
                }
                let add = ui.button(tl("Add Action"));
                ui.ctx().accesskit_node_builder(add.id, |node| node.set_label(format!("Add action to {map_shown}")));
                if add.on_hover_text(tl("A new action in this map")).clicked() {
                    change = Some(Change::AddAction(m));
                }
            });
        });
        ui.add_space(theme::GAP);
    }
    if ui.button(tl("Add Map")).on_hover_text(tl("A new map, for another way to play (driving, a menu…)")).clicked() {
        change = Some(Change::AddMap);
    }
    match change {
        Some(Change::RemoveMap(m)) => {
            draft.maps.remove(m);
            changed = true;
        }
        Some(Change::RemoveAction(m, a)) => {
            draft.maps[m].actions.remove(a);
            changed = true;
        }
        Some(Change::AddAction(m)) => {
            let n = draft.maps[m].actions.len() + 1;
            draft.maps[m].actions.push(Row { name: format!("Action{n}"), kind: ActionKind::Held, bindings: String::new() });
            changed = true;
        }
        Some(Change::AddMap) => {
            let n = draft.maps.len() + 1;
            draft.maps.push(MapDraft { name: format!("Map{n}"), actions: Vec::new() });
            changed = true;
        }
        None => {}
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

/// One action of a map: name and kind, then bindings and Remove. Returns whether Remove was clicked.
fn action_card(ui: &mut egui::Ui, t: &Tokens, map: &str, index: usize, row: &mut Row, changed: &mut bool) -> bool {
    let mut remove = false;
    egui::Frame::new().fill(t.panel).corner_radius(theme::SECTION_RADIUS).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let width = ui.available_width();
        let gap = ui.spacing().item_spacing.x;
        let shown = if row.name.trim().is_empty() { format!("{map}/action {}", index + 1) } else { format!("{map}/{}", row.name.trim()) };
        ui.horizontal(|ui| {
            let name =
                ui.add_sized([(width - KIND_WIDTH - gap).max(40.0), ROW_HEIGHT - 4.0], egui::TextEdit::singleline(&mut row.name).hint_text(tl("Action name")));
            ui.ctx().accesskit_node_builder(name.id, |node| node.set_label(format!("Name of {shown}")));
            *changed |= name.changed();
            let current = KINDS.iter().find(|(k, ..)| *k == row.kind).map_or("Held", |(_, label, _)| label);
            let kind = craft_ui::ComboBox::from_id_salt("kind").width(KIND_WIDTH).selected_text(current).show_ui(ui, |ui| {
                for (kind, label, what) in KINDS {
                    if ui.selectable_label(row.kind == kind, label).on_hover_text(what).clicked() && row.kind != kind {
                        row.kind = kind;
                        *changed = true;
                    }
                }
            });
            ui.ctx().accesskit_node_builder(kind.response.id, |node| node.set_label(format!("Kind of {shown}: {current}")));
        });
        ui.horizontal(|ui| {
            let bindings = ui.add_sized(
                [(width - REMOVE_WIDTH - gap).max(40.0), ROW_HEIGHT - 4.0],
                egui::TextEdit::singleline(&mut row.bindings).hint_text(tl("key:Space, pad:South")),
            );
            ui.ctx().accesskit_node_builder(bindings.id, |node| node.set_label(format!("Bindings of {shown}")));
            *changed |= bindings.changed();
            let button = ui.add_sized([REMOVE_WIDTH, ROW_HEIGHT - 4.0], egui::Button::new(tl("Remove")));
            ui.ctx().accesskit_node_builder(button.id, |node| node.set_label(format!("Remove {shown}")));
            remove = button.on_hover_text(tl("Remove this action")).clicked();
        });
    });
    remove
}

/// The map play starts with.
fn start_row(ui: &mut egui::Ui, t: &Tokens, draft: &mut Draft) -> bool {
    let maps: Vec<String> = draft.maps.iter().map(|m| m.name.trim().to_owned()).filter(|n| !n.is_empty()).collect();
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
    changed
}
