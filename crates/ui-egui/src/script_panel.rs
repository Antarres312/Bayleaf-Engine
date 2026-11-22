//! The visual script's own panel, left of its graph: its variables (name, type, starting value,
//! buttons that place their nodes), its properties (the Property nodes the Inspector shows per
//! object), its functions (inputs and outputs, and buttons that place their nodes) and where Add
//! Component lists it. Everything changes through `graph.*` commands, so
//! it undoes like any other graph edit.

use crate::i18n::tl;
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
        functions(editor, ui, t, graph, at, panel);
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
                egui::Label::new(
                    egui::RichText::new(tl("Numbers, whole numbers, vectors, true/false, text, objects and colours this script keeps, per object."))
                        .small()
                        .color(t.text_muted),
                )
                .wrap(),
            );
        }
        for variable in &graph.variables {
            ui.push_id(("variable", &variable.name), |ui| {
                egui::Frame::new().fill(t.panel).corner_radius(theme::SECTION_RADIUS).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    // The type on the right, the name field in what is left, so a long type name never
                    // makes the row (and the panel) wider.
                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(tl(variable.ty.label())).small().color(t.text_muted));
                        let text = panel.renaming.entry(variable.name.clone()).or_insert_with(|| variable.name.clone());
                        let field = ui.add_sized([ui.available_width().max(40.0), ROW_HEIGHT - 4.0], egui::TextEdit::singleline(text));
                        ui.ctx().accesskit_node_builder(field.id, |node| {
                            node.set_label(crate::i18n::tf("Name of variable {name}", &[("name", &variable.name.to_string())]))
                        });
                        if field.lost_focus() {
                            let typed = text.trim().to_owned();
                            if typed != variable.name {
                                editor.dispatch("graph.variable.set", &json!({ "name": variable.name, "rename": typed }), None);
                            }
                            panel.renaming.remove(&variable.name);
                        } else if !field.has_focus() {
                            *text = variable.name.clone();
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(tl("Starts at")).color(t.text_muted));
                        if let Some(start) = start_editor(ui, variable.ty, &variable.start, &variable.name) {
                            let key = format!("variable start {} {}", variable.name, editor.gesture);
                            editor.dispatch("graph.variable.set", &json!({ "name": variable.name, "start": start }), Some(&key));
                        }
                    });
                    ui.horizontal(|ui| {
                        let [get, set] = bayleaf_engine::script::visual::variable_kinds(variable.ty);
                        let read = ui.small_button(tl("Get")).on_hover_text(tl("Place a node that reads it"));
                        ui.ctx().accesskit_node_builder(read.id, |node| node.set_label(crate::i18n::tf("Get {name}", &[("name", &variable.name.to_string())])));
                        if read.clicked() {
                            place(editor, get, at, &variable.name);
                        }
                        let write = ui.small_button(tl("Set")).on_hover_text(tl("Place a node that sets it"));
                        ui.ctx()
                            .accesskit_node_builder(write.id, |node| node.set_label(crate::i18n::tf("Set {name}", &[("name", &variable.name.to_string())])));
                        if write.clicked() {
                            place(editor, set, at, &variable.name);
                        }
                        let remove = ui.small_button(tl("Remove")).on_hover_text(tl("Its nodes keep working, starting from zero"));
                        ui.ctx().accesskit_node_builder(remove.id, |node| {
                            node.set_label(crate::i18n::tf("Remove variable {name}", &[("name", &variable.name.to_string())]))
                        });
                        if remove.clicked() {
                            editor.dispatch("graph.variable.remove", &json!({ "name": variable.name }), None);
                        }
                    });
                });
            });
            ui.add_space(theme::SECTION_GAP);
        }
        ui.menu_button(tl("Add Variable"), |ui| {
            for ty in VariableType::ALL {
                if ui.button(tl(ty.label())).clicked() {
                    let stem = match ty {
                        VariableType::Number => "number",
                        VariableType::Vector => "vector",
                        VariableType::TrueFalse => "flag",
                        VariableType::WholeNumber => "whole",
                        VariableType::Text => "text",
                        VariableType::Object => "object",
                        VariableType::Color => "color",
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
            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(crate::i18n::tf("Start of {name}", &[("name", name)])));
            field.changed().then(|| json!(v))
        }
        VariableType::Vector => {
            let mut v: Vec<f64> = (0..3).map(|i| start[i].as_f64().unwrap_or(0.0)).collect();
            let mut changed = false;
            for (i, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
                let field = ui.add(egui::DragValue::new(&mut v[i]).speed(0.05).max_decimals(2));
                ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(crate::i18n::tf("Start of {name} {axis}", &[("name", name), ("axis", axis)])));
                changed |= field.changed();
            }
            changed.then(|| json!(v))
        }
        VariableType::TrueFalse => {
            let mut b = start.as_bool().unwrap_or(false);
            let field = ui.checkbox(&mut b, "");
            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(crate::i18n::tf("Start of {name}", &[("name", name)])));
            field.changed().then(|| json!(b))
        }
        VariableType::WholeNumber => {
            let mut v = start.as_i64().and_then(|n| i32::try_from(n).ok()).unwrap_or(0);
            let field = ui.add(egui::DragValue::new(&mut v).speed(0.1));
            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(crate::i18n::tf("Start of {name}", &[("name", name)])));
            field.changed().then(|| json!(v))
        }
        VariableType::Text => {
            let mut text = start.as_str().unwrap_or_default().to_owned();
            let field = ui.add(egui::TextEdit::singleline(&mut text).char_limit(bayleaf_engine::graph::MAX_TEXT).desired_width(ui.available_width()));
            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(crate::i18n::tf("Start of {name}", &[("name", name)])));
            field.changed().then(|| json!(text))
        }
        // An object variable starts as none: it is set while the game plays.
        VariableType::Object => {
            ui.label(egui::RichText::new(tl("none, until set in play")).color(theme::tokens(ui).text_muted));
            None
        }
        VariableType::Color => {
            let mut rgb: [f32; 3] = std::array::from_fn(|i| start[i].as_f64().unwrap_or(1.0) as f32);
            let field = ui.color_edit_button_rgb(&mut rgb);
            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(crate::i18n::tf("Start of {name}", &[("name", name)])));
            field.changed().then(|| json!(rgb))
        }
    }
}

/// The script's properties: its Property nodes, whose values the Inspector sets per object. Each has
/// a type, chosen when it is added, and a default edited here.
fn properties(editor: &mut Editor, ui: &mut egui::Ui, t: &Tokens, graph: &Graph, at: [f32; 2]) {
    use bayleaf_engine::scene::components::PropertyType;
    use bayleaf_engine::script::visual::{property_kind, property_type};
    theme::headed(t, "Properties").show(ui, |ui| {
        let nodes: Vec<(&Node, PropertyType)> = graph.nodes.iter().filter_map(|n| property_type(&n.kind).map(|ty| (n, ty))).collect();
        if nodes.is_empty() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(tl("Values set per object in the Inspector: numbers, text, objects and more.")).small().color(t.text_muted),
                )
                .wrap(),
            );
        }
        for (node, ty) in nodes {
            let name = node.values.get("name").and_then(Value::as_str).unwrap_or("speed").to_owned();
            ui.push_id(("property", node.id), |ui| {
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(tl(ty.label())).small().color(t.text_muted));
                    ui.add(egui::Label::new(name.as_str()).truncate());
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(tl("Default")).color(t.text_muted));
                    if let Some(value) = default_editor(ui, ty, node.values.get("default"), &name) {
                        let key = format!("property default {} {}", node.id, editor.gesture);
                        editor.dispatch("graph.node.set", &json!({ "node": node.id, "name": "default", "value": value }), Some(&key));
                    }
                });
            });
            ui.add_space(4.0);
        }
        ui.menu_button(tl("Add Property"), |ui| {
            for ty in PropertyType::ALL {
                if ui.button(tl(ty.label())).clicked() {
                    let stem = match ty {
                        PropertyType::Number => "value",
                        PropertyType::WholeNumber => "count",
                        PropertyType::TrueFalse => "enabled",
                        PropertyType::Vector => "offset",
                        PropertyType::Color => "tint",
                        PropertyType::Text => "label",
                        PropertyType::Object => "target",
                    };
                    let taken = |n: &str| {
                        graph.nodes.iter().any(|node| property_type(&node.kind).is_some() && node.values.get("name").and_then(Value::as_str) == Some(n))
                    };
                    place(editor, property_kind(ty), at, &free_name(stem, taken));
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(tl("Place a Property node of a type: a value the Inspector sets per object"));
    });
}

/// An editor for a Property node's default. Returns the new setting value when it changed.
fn default_editor(ui: &mut egui::Ui, ty: bayleaf_engine::scene::components::PropertyType, current: Option<&Value>, name: &str) -> Option<Value> {
    use bayleaf_engine::scene::components::PropertyType;
    let variable = match ty {
        PropertyType::Number => VariableType::Number,
        PropertyType::WholeNumber => VariableType::WholeNumber,
        PropertyType::Vector => VariableType::Vector,
        PropertyType::Color => VariableType::Color,
        PropertyType::Text => VariableType::Text,
        PropertyType::TrueFalse => {
            // The node keeps True or False as a word.
            let mut on = current.and_then(Value::as_str) == Some("True");
            let field = ui.checkbox(&mut on, "");
            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(crate::i18n::tf("Default of property {name}", &[("name", name)])));
            return field.changed().then(|| json!(if on { "True" } else { "False" }));
        }
        PropertyType::Object => {
            ui.label(egui::RichText::new(tl("none; chosen per object in the Inspector")).color(theme::tokens(ui).text_muted));
            return None;
        }
    };
    let fallback = if ty == PropertyType::Color { json!([1.0, 1.0, 1.0]) } else { variable.zero() };
    start_editor(ui, variable, current.unwrap_or(&fallback), name)
}

/// Places a Function, Return or Call Function node of `function` at `at`, with its pins.
fn place_function(editor: &mut Editor, kind: &str, at: [f32; 2], function: &bayleaf_engine::graph::Function) {
    if let Some(added) = editor.dispatch("graph.node.add", &json!({ "kind": kind, "position": at }), None) {
        let node = added["node"].clone();
        editor.dispatch("graph.node.set", &json!({ "node": node, "name": "inputs", "value": bayleaf_engine::graph::param_pins(&function.inputs) }), None);
        editor.dispatch("graph.node.set", &json!({ "node": node, "name": "outputs", "value": bayleaf_engine::graph::param_pins(&function.outputs) }), None);
        editor.dispatch("graph.node.set", &json!({ "node": node, "name": "function", "value": function.name }), None);
    }
}

/// The script's functions: each one's name, its inputs and outputs with their types, and buttons
/// that place a Call Function node or remove it. Add Function declares one and places its Function
/// and Return nodes.
fn functions(editor: &mut Editor, ui: &mut egui::Ui, t: &Tokens, graph: &Graph, at: [f32; 2], panel: &mut ScriptPanel) {
    use bayleaf_engine::graph::{Function, Param};
    theme::headed(t, "Functions").show(ui, |ui| {
        if graph.functions.is_empty() {
            ui.add(
                egui::Label::new(egui::RichText::new(tl("Actions this script runs from several places, with inputs and outputs.")).small().color(t.text_muted))
                    .wrap(),
            );
        }
        for function in &graph.functions {
            ui.push_id(("function", &function.name), |ui| {
                egui::Frame::new().fill(t.panel).corner_radius(theme::SECTION_RADIUS).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let key = format!("function {}", function.name);
                    if let Some(typed) = name_field(ui, panel, &key, &function.name, &crate::i18n::tf("Name of function {name}", &[("name", &function.name)])) {
                        editor.dispatch("graph.function.set", &json!({ "name": function.name, "rename": typed }), None);
                    }
                    for (side, params) in [("inputs", &function.inputs), ("outputs", &function.outputs)] {
                        ui.label(egui::RichText::new(tl(if side == "inputs" { "Inputs" } else { "Outputs" })).small().color(t.text_muted));
                        let mut changed: Option<Vec<Param>> = None;
                        for (index, param) in params.iter().enumerate() {
                            ui.push_id((side, index), |ui| {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(ui.available_width(), ROW_HEIGHT),
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let remove = ui.small_button("×").on_hover_text(tl("Remove"));
                                        ui.ctx().accesskit_node_builder(remove.id, |node| {
                                            node.set_label(crate::i18n::tf(
                                                "Remove {name} from {function}",
                                                &[("name", &param.name), ("function", &function.name)],
                                            ))
                                        });
                                        if remove.clicked() {
                                            let mut kept = params.clone();
                                            kept.remove(index);
                                            changed = Some(kept);
                                        }
                                        ui.label(egui::RichText::new(tl(param.ty.label())).small().color(t.text_muted));
                                        let key = format!("function {} {side} {index}", function.name);
                                        let label = crate::i18n::tf("Name of {name} of {function}", &[("name", &param.name), ("function", &function.name)]);
                                        if let Some(typed) = name_field(ui, panel, &key, &param.name, &label) {
                                            let mut renamed = params.clone();
                                            renamed[index].name = typed;
                                            changed = Some(renamed);
                                        }
                                    },
                                );
                            });
                        }
                        let add = if side == "inputs" { tl("Add Input") } else { tl("Add Output") };
                        ui.menu_button(add, |ui| {
                            for ty in VariableType::ALL {
                                if ui.button(tl(ty.label())).clicked() {
                                    let stem = if side == "inputs" { "input" } else { "output" };
                                    let mut more = params.clone();
                                    more.push(Param { name: free_name(stem, |n| params.iter().any(|p| p.name == n)), ty });
                                    changed = Some(more);
                                    ui.close();
                                }
                            }
                        });
                        if let Some(list) = changed {
                            editor.dispatch("graph.function.set", &json!({ "name": function.name, side: list }), None);
                        }
                    }
                    ui.horizontal(|ui| {
                        let call = ui.small_button(tl("Call")).on_hover_text(tl("Place a node that runs it"));
                        ui.ctx().accesskit_node_builder(call.id, |node| node.set_label(crate::i18n::tf("Call {name}", &[("name", &function.name)])));
                        if call.clicked() {
                            place_function(editor, "callFunction", at, function);
                        }
                        let remove = ui.small_button(tl("Remove")).on_hover_text(tl("Its nodes stay and say they name no function"));
                        ui.ctx()
                            .accesskit_node_builder(remove.id, |node| node.set_label(crate::i18n::tf("Remove function {name}", &[("name", &function.name)])));
                        if remove.clicked() {
                            editor.dispatch("graph.function.remove", &json!({ "name": function.name }), None);
                        }
                    });
                });
            });
            ui.add_space(theme::SECTION_GAP);
        }
        if ui.button(tl("Add Function")).on_hover_text(tl("Declare a function and place its Function and Return nodes")).clicked() {
            let name = free_name("function", |n| graph.function(n).is_some());
            if editor.dispatch("graph.function.add", &json!({ "name": name }), None).is_some() {
                let function = Function { name, inputs: Vec::new(), outputs: Vec::new() };
                place_function(editor, "functionStart", at, &function);
                place_function(editor, "functionReturn", [at[0] + 240.0, at[1]], &function);
            }
        }
    });
}

/// A name typed in place: what is typed stays until the field is left, then it is returned if it
/// changed. `key` keeps the typing apart from other fields.
fn name_field(ui: &mut egui::Ui, panel: &mut ScriptPanel, key: &str, current: &str, label: &str) -> Option<String> {
    let text = panel.renaming.entry(key.to_owned()).or_insert_with(|| current.to_owned());
    let field = ui.add_sized([ui.available_width().max(40.0), ROW_HEIGHT - 4.0], egui::TextEdit::singleline(text));
    ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(label));
    if field.lost_focus() {
        let typed = text.trim().to_owned();
        panel.renaming.remove(key);
        return (typed != current && !typed.is_empty()).then_some(typed);
    }
    if !field.has_focus() {
        *text = current.to_owned();
    }
    None
}

/// Where Add Component lists the script: a path typed here (`graph.menu.set`). What is typed stays
/// while the field has focus, even when it does not read right yet (a trailing `/`).
fn script(editor: &mut Editor, ui: &mut egui::Ui, t: &Tokens, graph: &Graph, panel: &mut ScriptPanel) {
    theme::headed(t, "Script").show(ui, |ui| {
        ui.label(egui::RichText::new(tl("Menu path")).small().color(t.text_muted));
        let (text, problem) = panel.menu.get_or_insert_with(|| (graph.menu.clone(), None));
        let field = ui
            .add_sized([ui.available_width(), ROW_HEIGHT - 4.0], egui::TextEdit::singleline(text).hint_text(tl("Scripts/My Script")))
            .on_hover_text(tl("Folders, then the name: Light/Flicker. Empty: under Scripts."));
        ui.ctx().accesskit_node_builder(field.id, |n| n.set_label("Menu path in Add Component"));
        if field.changed() {
            let key = format!("menu path {}", editor.gesture);
            *problem = match editor.dispatch("graph.menu.set", &json!({ "path": text.as_str() }), Some(&key)) {
                Some(_) => None,
                None => Some(editor.status.clone()),
            };
        }
        if let Some(problem) = problem {
            ui.add(
                egui::Label::new(egui::RichText::new(crate::i18n::tf("Not set: {problem}", &[("problem", &problem.to_string())])).small().color(t.text)).wrap(),
            );
        }
        if !field.has_focus() && !field.changed() {
            panel.menu = None;
        }
    });
}
