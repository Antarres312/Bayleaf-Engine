//! The visual script's own panel, left of its graph: its variables (name, type, starting value,
//! buttons that place their nodes), its properties (the Property nodes the Inspector shows per
//! object) and where Add Component lists it. Everything changes through `graph.*` commands, so
//! it undoes like any other graph edit.

use std::collections::HashMap;

use bayleaf_engine::graph::{Graph, Node, VariableType};
use serde_json::{Value, json};

use crate::Editor;
use crate::theme::{self, ROW_HEIGHT, Tokens};

/// Names being typed, by the variable they rename, until the field is left.
#[derive(Default)]
pub(crate) struct ScriptPanel {
    renaming: HashMap<String, String>,
    /// The menu path as typed while its field has focus, and why it was refused, if it was.
    menu: Option<(String, Option<String>)>,
}

/// Draws the panel; `at` is where new nodes go on the canvas.
pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui, graph: &Graph, at: [f32; 2], panel: &mut ScriptPanel) {
    let t = theme::tokens(ui);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        variables(editor, ui, t, graph, at, panel);
        ui.add_space(theme::SECTION_GAP);
        properties(editor, ui, t, graph, at);
        ui.add_space(theme::SECTION_GAP);
        script(editor, ui, t, graph, panel);
    });
}

/// A name for a new variable or property that nothing uses yet: `stem`, `stem2`, …
fn free_name(stem: &str, taken: impl Fn(&str) -> bool) -> String {
    (1..).map(|n| if n == 1 { stem.to_owned() } else { format!("{stem}{n}") }).find(|name| !taken(name)).unwrap_or_else(|| stem.to_owned())
}

/// Adds a node of `kind` at `at` with its `name` setting, and selects nothing else.
fn place(editor: &mut Editor, kind: &str, at: [f32; 2], name: &str) {
    if let Some(added) = editor.dispatch("graph.node.add", &json!({ "kind": kind, "position": at }), None) {
        editor.dispatch("graph.node.set", &json!({ "node": added["node"], "name": "name", "value": name }), None);
    }
}

fn variables(editor: &mut Editor, ui: &mut egui::Ui, t: &Tokens, graph: &Graph, at: [f32; 2], panel: &mut ScriptPanel) {
    theme::headed(t, "Variables").show(ui, |ui| {
        if graph.variables.is_empty() {
            ui.add(
                egui::Label::new(egui::RichText::new("Numbers, vectors and true/false values this script keeps, per object.").small().color(t.text_muted))
                    .wrap(),
            );
        }
        for variable in &graph.variables {
            ui.push_id(("variable", &variable.name), |ui| {
                egui::Frame::new().fill(t.panel).corner_radius(theme::SECTION_RADIUS).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let width = ui.available_width();
                    ui.horizontal(|ui| {
                        let text = panel.renaming.entry(variable.name.clone()).or_insert_with(|| variable.name.clone());
                        let field = ui.add_sized([(width - 70.0).max(40.0), ROW_HEIGHT - 4.0], egui::TextEdit::singleline(text));
                        ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(format!("Name of variable {}", variable.name)));
                        if field.lost_focus() {
                            let typed = text.trim().to_owned();
                            if typed != variable.name {
                                editor.dispatch("graph.variable.set", &json!({ "name": variable.name, "rename": typed }), None);
                            }
                            panel.renaming.remove(&variable.name);
                        } else if !field.has_focus() {
                            *text = variable.name.clone();
                        }
                        ui.label(egui::RichText::new(variable.ty.label()).small().color(t.text_muted));
                    });
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Starts at").color(t.text_muted));
                        if let Some(start) = start_editor(ui, variable.ty, &variable.start, &variable.name) {
                            let key = format!("variable start {} {}", variable.name, editor.gesture);
                            editor.dispatch("graph.variable.set", &json!({ "name": variable.name, "start": start }), Some(&key));
                        }
                    });
                    ui.horizontal(|ui| {
                        let [get, set] = bayleaf_engine::script::visual::variable_kinds(variable.ty);
                        let read = ui.small_button("Get").on_hover_text("Place a node that reads it");
                        ui.ctx().accesskit_node_builder(read.id, |node| node.set_label(format!("Get {}", variable.name)));
                        if read.clicked() {
                            place(editor, get, at, &variable.name);
                        }
                        let write = ui.small_button("Set").on_hover_text("Place a node that sets it");
                        ui.ctx().accesskit_node_builder(write.id, |node| node.set_label(format!("Set {}", variable.name)));
                        if write.clicked() {
                            place(editor, set, at, &variable.name);
                        }
                        let remove = ui.small_button("Remove").on_hover_text("Its nodes keep working, starting from zero");
                        ui.ctx().accesskit_node_builder(remove.id, |node| node.set_label(format!("Remove variable {}", variable.name)));
                        if remove.clicked() {
                            editor.dispatch("graph.variable.remove", &json!({ "name": variable.name }), None);
                        }
                    });
                });
            });
            ui.add_space(theme::SECTION_GAP);
        }
        ui.menu_button("Add Variable", |ui| {
            for ty in VariableType::ALL {
                if ui.button(ty.label()).clicked() {
                    let stem = match ty {
                        VariableType::Number => "number",
                        VariableType::Vector => "vector",
                        VariableType::TrueFalse => "flag",
                    };
                    let name = free_name(stem, |n| graph.variable(n).is_some());
                    editor.dispatch("graph.variable.add", &json!({ "name": name, "type": ty }), None);
                    ui.close();
                }
            }
        });
    });
}

/// An editor for a starting value of `ty`. Returns the new value when it changed.
fn start_editor(ui: &mut egui::Ui, ty: VariableType, start: &Value, name: &str) -> Option<Value> {
    match ty {
        VariableType::Number => {
            let mut v = start.as_f64().unwrap_or(0.0);
            let field = ui.add(egui::DragValue::new(&mut v).speed(0.05).max_decimals(3));
            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(format!("Start of {name}")));
            field.changed().then(|| json!(v))
        }
        VariableType::Vector => {
            let mut v: Vec<f64> = (0..3).map(|i| start[i].as_f64().unwrap_or(0.0)).collect();
            let mut changed = false;
            for (i, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
                let field = ui.add(egui::DragValue::new(&mut v[i]).speed(0.05).max_decimals(2));
                ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(format!("Start of {name} {axis}")));
                changed |= field.changed();
            }
            changed.then(|| json!(v))
        }
        VariableType::TrueFalse => {
            let mut b = start.as_bool().unwrap_or(false);
            let field = ui.checkbox(&mut b, "");
            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(format!("Start of {name}")));
            field.changed().then(|| json!(b))
        }
    }
}

/// The script's properties: its Property nodes, whose values the Inspector sets per object.
fn properties(editor: &mut Editor, ui: &mut egui::Ui, t: &Tokens, graph: &Graph, at: [f32; 2]) {
    theme::headed(t, "Properties").show(ui, |ui| {
        let nodes: Vec<&Node> = graph.nodes.iter().filter(|n| n.kind == "property").collect();
        if nodes.is_empty() {
            ui.add(egui::Label::new(egui::RichText::new("Numbers set per object in the Inspector.").small().color(t.text_muted)).wrap());
        }
        for node in nodes {
            let name = node.values.get("name").and_then(Value::as_str).unwrap_or("speed").to_owned();
            let mut default = node.values.get("default").and_then(Value::as_f64).unwrap_or(1.0);
            ui.horizontal(|ui| {
                ui.label(&name);
                let field = ui.add(egui::DragValue::new(&mut default).speed(0.05).max_decimals(3));
                ui.ctx().accesskit_node_builder(field.id, |n| n.set_label(format!("Default of property {name}")));
                if field.changed() {
                    let key = format!("property default {} {}", node.id, editor.gesture);
                    editor.dispatch("graph.node.set", &json!({ "node": node.id, "name": "default", "value": default }), Some(&key));
                }
            });
        }
        if ui.button("Add Property").on_hover_text("Place a Property node: a number the Inspector sets per object").clicked() {
            let taken = |n: &str| graph.nodes.iter().any(|node| node.kind == "property" && node.values.get("name").and_then(Value::as_str) == Some(n));
            place(editor, "property", at, &free_name("value", taken));
        }
    });
}

/// Where Add Component lists the script: a path typed here (`graph.menu.set`). What is typed stays
/// while the field has focus, even when it does not read right yet (a trailing `/`).
fn script(editor: &mut Editor, ui: &mut egui::Ui, t: &Tokens, graph: &Graph, panel: &mut ScriptPanel) {
    theme::headed(t, "Script").show(ui, |ui| {
        ui.label(egui::RichText::new("Menu path").small().color(t.text_muted));
        let (text, problem) = panel.menu.get_or_insert_with(|| (graph.menu.clone(), None));
        let field = ui
            .add_sized([ui.available_width(), ROW_HEIGHT - 4.0], egui::TextEdit::singleline(text).hint_text("Scripts/My Script"))
            .on_hover_text("Folders, then the name: Light/Flicker. Empty: under Scripts.");
        ui.ctx().accesskit_node_builder(field.id, |n| n.set_label("Menu path in Add Component"));
        if field.changed() {
            let key = format!("menu path {}", editor.gesture);
            *problem = match editor.dispatch("graph.menu.set", &json!({ "path": text.as_str() }), Some(&key)) {
                Some(_) => None,
                None => Some(editor.status.clone()),
            };
        }
        if let Some(problem) = problem {
            ui.add(egui::Label::new(egui::RichText::new(format!("Not set: {problem}")).small().color(t.text)).wrap());
        }
        if !field.has_focus() && !field.changed() {
            panel.menu = None;
        }
    });
}
