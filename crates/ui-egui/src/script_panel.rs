//! The visual script's own panel, left of its graph: its variables (name, what each holds (a type,
//! one value or a list or map of them, changed from its menu), starting value,
//! buttons that place their nodes), its properties (the Property nodes the Inspector shows per
//! object), its functions (inputs and outputs, and buttons that place their nodes), its events (the
//! values each brings, and buttons that place On Event and Trigger Event) and where Add Component lists it. Everything changes through `graph.*` commands, so
//! it undoes like any other graph edit.

use crate::i18n::tl;
use std::collections::HashMap;

use bayleaf_engine::graph::{Graph, Shape, VariableType};
use serde_json::{Value, json};

use crate::Editor;
use crate::drag_wrap::WrapDrag;
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
        properties(editor, ui, t, graph, at, panel);
        ui.add_space(theme::SECTION_GAP);
        functions(editor, ui, t, graph, at, panel);
        ui.add_space(theme::SECTION_GAP);
        events(editor, ui, t, graph, at, panel);
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
                        let holds = Holds::of(variable.ty, variable.shape, variable.key);
                        if let Some(new) = type_menu(ui, t, holds, &variable.name) {
                            let mut params = new.params();
                            params["name"] = json!(variable.name);
                            editor.dispatch("graph.variable.set", &params, None);
                        }
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
                        let key = format!("variable {}", variable.name);
                        if let Some(start) =
                            shaped_editor(ui, panel, &key, Holds::of(variable.ty, variable.shape, variable.key), &variable.start, &variable.name)
                        {
                            let key = format!("variable start {} {}", variable.name, editor.gesture);
                            editor.dispatch("graph.variable.set", &json!({ "name": variable.name, "start": start }), Some(&key));
                        }
                    });
                    ui.horizontal(|ui| {
                        if variable.shape.is_single() {
                            let [get, set] = bayleaf_engine::script::visual::variable_kinds(variable.ty);
                            let read = ui.small_button(tl("Get")).on_hover_text(tl("Place a node that reads it"));
                            ui.ctx()
                                .accesskit_node_builder(read.id, |node| node.set_label(crate::i18n::tf("Get {name}", &[("name", &variable.name.to_string())])));
                            if read.clicked() {
                                place(editor, get, at, &variable.name);
                            }
                            let write = ui.small_button(tl("Set")).on_hover_text(tl("Place a node that sets it"));
                            ui.ctx().accesskit_node_builder(write.id, |node| {
                                node.set_label(crate::i18n::tf("Set {name}", &[("name", &variable.name.to_string())]))
                            });
                            if write.clicked() {
                                place(editor, set, at, &variable.name);
                            }
                        } else {
                            place_node_menu(editor, ui, at, &variable.name, Holds::of(variable.ty, variable.shape, variable.key));
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
            let field = ui.add(egui::DragValue::new(&mut v).speed(0.05).max_decimals(3)).wrap_drag();
            ui.ctx().accesskit_node_builder(field.id, |node| node.set_label(crate::i18n::tf("Start of {name}", &[("name", name)])));
            field.changed().then(|| json!(v))
        }
        VariableType::Vector => {
            let mut v: Vec<f64> = (0..3).map(|i| start[i].as_f64().unwrap_or(0.0)).collect();
            let mut changed = false;
            for (i, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
                let field = ui.add(egui::DragValue::new(&mut v[i]).speed(0.05).max_decimals(2)).wrap_drag();
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
            let field = ui.add(egui::DragValue::new(&mut v).speed(0.1)).wrap_drag();
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

/// The script's properties: values the Inspector sets per object, each with a name, a type and a
/// default edited here, and buttons that place a node reading it (**Get**) or setting it while the
/// game plays (**Set**). Add Property declares one of a type.
fn properties(editor: &mut Editor, ui: &mut egui::Ui, t: &Tokens, graph: &Graph, at: [f32; 2], panel: &mut ScriptPanel) {
    use bayleaf_engine::script::visual::property_kinds;
    theme::headed(t, "Properties").show(ui, |ui| {
        if graph.properties.is_empty() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(tl("Values set per object in the Inspector: numbers, text, objects and more.")).small().color(t.text_muted),
                )
                .wrap(),
            );
        }
        for property in &graph.properties {
            ui.push_id(("property", &property.name), |ui| {
                egui::Frame::new().fill(t.panel).corner_radius(theme::SECTION_RADIUS).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let holds = Holds::of(property.ty, property.shape, property.key);
                        if let Some(new) = type_menu(ui, t, holds, &property.name) {
                            let mut params = new.params();
                            params["name"] = json!(property.name);
                            editor.dispatch("graph.property.set", &params, None);
                        }
                        let key = format!("property {}", property.name);
                        let label = crate::i18n::tf("Name of property {name}", &[("name", &property.name)]);
                        if let Some(typed) = name_field(ui, panel, &key, &property.name, &label) {
                            editor.dispatch("graph.property.set", &json!({ "name": property.name, "rename": typed }), None);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(tl("Default")).color(t.text_muted));
                        let key = format!("property default {}", property.name);
                        if property.ty == VariableType::Object {
                            let note = if property.shape.is_single() {
                                "none; chosen per object in the Inspector"
                            } else {
                                "empty; filled per object in the Inspector"
                            };
                            ui.label(egui::RichText::new(tl(note)).color(t.text_muted));
                        } else if let Some(value) =
                            shaped_editor(ui, panel, &key, Holds::of(property.ty, property.shape, property.key), &property.default, &property.name)
                        {
                            let key = format!("property default {} {}", property.name, editor.gesture);
                            editor.dispatch("graph.property.set", &json!({ "name": property.name, "default": value }), Some(&key));
                        }
                    });
                    if let Some(options) = inspector_options(ui, t, graph, property) {
                        let key = format!("property inspector {} {}", property.name, editor.gesture);
                        editor.dispatch("graph.property.set", &json!({ "name": property.name, "inspector": options }), Some(&key));
                    }
                    ui.horizontal(|ui| {
                        if property.shape.is_single() {
                            let [get, set] = property_kinds(property.ty);
                            let read = ui.small_button(tl("Get")).on_hover_text(tl("Place a node that reads it"));
                            ui.ctx()
                                .accesskit_node_builder(read.id, |node| node.set_label(crate::i18n::tf("Get property {name}", &[("name", &property.name)])));
                            if read.clicked() {
                                place(editor, get, at, &property.name);
                            }
                            let write = ui.small_button(tl("Set")).on_hover_text(tl("Place a node that sets it while the game plays"));
                            ui.ctx()
                                .accesskit_node_builder(write.id, |node| node.set_label(crate::i18n::tf("Set property {name}", &[("name", &property.name)])));
                            if write.clicked() {
                                place(editor, set, at, &property.name);
                            }
                        } else {
                            place_node_menu(editor, ui, at, &property.name, Holds::of(property.ty, property.shape, property.key));
                        }
                        let remove = ui.small_button(tl("Remove")).on_hover_text(tl("Its nodes stay and read zero where no object sets it"));
                        ui.ctx()
                            .accesskit_node_builder(remove.id, |node| node.set_label(crate::i18n::tf("Remove property {name}", &[("name", &property.name)])));
                        if remove.clicked() {
                            editor.dispatch("graph.property.remove", &json!({ "name": property.name }), None);
                        }
                    });
                });
            });
            ui.add_space(theme::SECTION_GAP);
        }
        ui.menu_button(tl("Add Property"), |ui| {
            for ty in VariableType::ALL {
                if ui.button(tl(ty.label())).clicked() {
                    let stem = match ty {
                        VariableType::Number => "speed",
                        VariableType::WholeNumber => "count",
                        VariableType::TrueFalse => "enabled",
                        VariableType::Vector => "offset",
                        VariableType::Color => "tint",
                        VariableType::Text => "label",
                        VariableType::Object => "target",
                    };
                    let name = free_name(stem, |n| graph.property(n).is_some());
                    editor.dispatch("graph.property.add", &json!({ "name": name, "type": ty }), None);
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(tl("Declare a property of a type: a value the Inspector sets per object"));
    });
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
                                        if let Some(new) = type_menu(ui, t, Holds::of(param.ty, param.shape, param.key), &param.name) {
                                            let mut retyped = params.clone();
                                            (retyped[index].ty, retyped[index].shape, retyped[index].key) = (new.ty, new.shape, new.key);
                                            changed = Some(retyped);
                                        }
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
                                    more.push(Param::new(free_name(stem, |n| params.iter().any(|p| p.name == n)), ty));
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

/// Places an On Event or Trigger Event node of the event `name` bringing `params` (`name:type`).
fn place_event(editor: &mut Editor, kind: &str, at: [f32; 2], name: &str, params: &Value) {
    if let Some(added) = editor.dispatch("graph.node.add", &json!({ "kind": kind, "position": at }), None) {
        let node = added["node"].clone();
        editor.dispatch("graph.node.set", &json!({ "node": node, "name": "params", "value": params }), None);
        editor.dispatch("graph.node.set", &json!({ "node": node, "name": "event", "value": name }), None);
    }
}

/// For another script's event: a Get Component naming the script, then each node of `kinds` for the
/// event, those with a component input linked to it.
fn place_component_event(editor: &mut Editor, at: [f32; 2], module: &str, script: &str, event: &bayleaf_engine::script::DeclaredEvent, kinds: &[&str]) {
    let path = editor.session.script_dir().join(module);
    // The properties Get Component shows as pins, as the Graph panel keeps them.
    let fields: Vec<String> = editor
        .script_info
        .properties(&path)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.shape == bayleaf_engine::scene::components::PropertyShape::Single)
        .filter_map(|p| match p.ty.code() {
            "number" => Some(p.name),
            "text" | "object" => None,
            code => Some(format!("{}:{code}", p.name)),
        })
        .collect();
    let Some(get) = editor.dispatch("graph.node.add", &json!({ "kind": "getComponent", "position": [at[0] - 260.0, at[1]] }), None) else { return };
    let get = get["node"].clone();
    editor.dispatch("graph.node.set", &json!({ "node": get, "name": "fields", "value": fields }), None);
    editor.dispatch("graph.node.set", &json!({ "node": get, "name": "component", "value": script }), None);
    for (row, kind) in kinds.iter().enumerate() {
        let position = [at[0], at[1] + row as f32 * 180.0];
        let Some(added) = editor.dispatch("graph.node.add", &json!({ "kind": kind, "position": position }), None) else { continue };
        let node = added["node"].clone();
        editor.dispatch("graph.node.set", &json!({ "node": node, "name": "params", "value": event.params }), None);
        editor.dispatch("graph.node.set", &json!({ "node": node, "name": "script", "value": script }), None);
        editor.dispatch("graph.node.set", &json!({ "node": node, "name": "event", "value": event.name }), None);
        if *kind != "componentEventStart" {
            editor.dispatch("graph.link.add", &json!({ "from": { "node": get, "pin": "component" }, "to": { "node": node, "pin": "component" } }), None);
        }
    }
}

/// The events other scripts of the project declare, by script: what a Trigger Event here can name.
fn project_events(editor: &mut Editor, graph: &Graph) -> Vec<(String, bayleaf_engine::script::DeclaredEvent)> {
    let base = editor.session.script_dir();
    let own = editor.session.graph().and_then(|g| g.path.clone());
    let mut found = Vec::new();
    for module in editor.project.script_files() {
        let path = base.join(&module);
        if own.as_deref().is_some_and(|own| own.ends_with(&module) || own == path) {
            continue;
        }
        for event in editor.script_info.events(&path) {
            if graph.event(&event.name).is_none() {
                found.push((module.clone(), event));
            }
        }
    }
    found
}

/// The script's events: each one's name, the values it brings, and buttons that place an On Event
/// (what runs when it is triggered here) or a Trigger Event node, or remove it. Events other scripts
/// of the project declare can be triggered from here too.
fn events(editor: &mut Editor, ui: &mut egui::Ui, t: &Tokens, graph: &Graph, at: [f32; 2], panel: &mut ScriptPanel) {
    use bayleaf_engine::graph::Param;
    theme::headed(t, "Events").show(ui, |ui| {
        if graph.events.is_empty() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(tl("Named happenings with values that other scripts, the interface or this script trigger on an object."))
                        .small()
                        .color(t.text_muted),
                )
                .wrap(),
            );
        }
        for event in &graph.events {
            ui.push_id(("event", &event.name), |ui| {
                egui::Frame::new().fill(t.panel).corner_radius(theme::SECTION_RADIUS).inner_margin(egui::Margin::symmetric(6, 4)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let key = format!("event {}", event.name);
                    if let Some(typed) = name_field(ui, panel, &key, &event.name, &crate::i18n::tf("Name of event {name}", &[("name", &event.name)])) {
                        editor.dispatch("graph.event.set", &json!({ "name": event.name, "rename": typed }), None);
                    }
                    ui.label(egui::RichText::new(tl("Values")).small().color(t.text_muted));
                    let mut changed: Option<Vec<Param>> = None;
                    for (index, param) in event.params.iter().enumerate() {
                        ui.push_id(("value", index), |ui| {
                            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let remove = ui.small_button("×").on_hover_text(tl("Remove"));
                                ui.ctx().accesskit_node_builder(remove.id, |node| {
                                    node.set_label(crate::i18n::tf("Remove {name} from {event}", &[("name", &param.name), ("event", &event.name)]))
                                });
                                if remove.clicked() {
                                    let mut kept = event.params.clone();
                                    kept.remove(index);
                                    changed = Some(kept);
                                }
                                if let Some(new) = type_menu(ui, t, Holds::of(param.ty, param.shape, param.key), &param.name) {
                                    let mut retyped = event.params.clone();
                                    (retyped[index].ty, retyped[index].shape, retyped[index].key) = (new.ty, new.shape, new.key);
                                    changed = Some(retyped);
                                }
                                let key = format!("event {} value {index}", event.name);
                                let label = crate::i18n::tf("Name of {name} of {event}", &[("name", &param.name), ("event", &event.name)]);
                                if let Some(typed) = name_field(ui, panel, &key, &param.name, &label) {
                                    let mut renamed = event.params.clone();
                                    renamed[index].name = typed;
                                    changed = Some(renamed);
                                }
                            });
                        });
                    }
                    ui.menu_button(tl("Add Value"), |ui| {
                        for ty in VariableType::ALL {
                            if ui.button(tl(ty.label())).clicked() {
                                let mut more = event.params.clone();
                                more.push(Param::new(free_name("value", |n| event.params.iter().any(|p| p.name == n)), ty));
                                changed = Some(more);
                                ui.close();
                            }
                        }
                    });
                    if let Some(list) = changed {
                        editor.dispatch("graph.event.set", &json!({ "name": event.name, "params": list }), None);
                    }
                    ui.horizontal(|ui| {
                        let pins = bayleaf_engine::graph::param_pins(&event.params);
                        let on = ui.small_button(tl("On Event")).on_hover_text(tl("Place the node that runs when it is triggered on this object"));
                        ui.ctx().accesskit_node_builder(on.id, |node| node.set_label(crate::i18n::tf("On {name}", &[("name", &event.name)])));
                        if on.clicked() {
                            place_event(editor, "eventStart", at, &event.name, &pins);
                        }
                        let trigger = ui.small_button(tl("Trigger")).on_hover_text(tl("Place a node that triggers it"));
                        ui.ctx().accesskit_node_builder(trigger.id, |node| node.set_label(crate::i18n::tf("Trigger {name}", &[("name", &event.name)])));
                        if trigger.clicked() {
                            place_event(editor, "triggerEvent", at, &event.name, &pins);
                        }
                        let remove = ui.small_button(tl("Remove")).on_hover_text(tl("Its nodes stay, and say which event they name"));
                        ui.ctx().accesskit_node_builder(remove.id, |node| node.set_label(crate::i18n::tf("Remove event {name}", &[("name", &event.name)])));
                        if remove.clicked() {
                            editor.dispatch("graph.event.remove", &json!({ "name": event.name }), None);
                        }
                    });
                });
            });
            ui.add_space(theme::SECTION_GAP);
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button(tl("Add Event")).on_hover_text(tl("Declare an event and place its On Event node")).clicked() {
                let name = free_name("Event", |n| graph.event(n).is_some());
                if editor.dispatch("graph.event.add", &json!({ "name": name }), None).is_some() {
                    place_event(editor, "eventStart", at, &name, &json!([]));
                }
            }
            // Events other scripts declare, to trigger on their objects.
            let others = project_events(editor, graph);
            ui.add_enabled_ui(!others.is_empty(), |ui| {
                ui.menu_button(tl("Other Scripts' Events"), |ui| {
                    for (module, event) in &others {
                        let script = bayleaf_engine::scene::components::ScriptRef { module: module.clone(), properties: Default::default() }.display_name();
                        ui.menu_button(format!("{} · {script}", event.name), |ui| {
                            if ui.button(tl("Trigger")).on_hover_text(tl("Place a Get Component for the script and a Trigger Component Event")).clicked() {
                                place_component_event(editor, at, module, &script, event, &["triggerComponentEvent"]);
                                ui.close();
                            }
                            if ui
                                .button(tl("Subscribe"))
                                .on_hover_text(tl("Place a Get Component for the script, Subscribe to Event, and On Component Event"))
                                .clicked()
                            {
                                place_component_event(editor, at, module, &script, event, &["subscribeEvent", "componentEventStart"]);
                                ui.close();
                            }
                        });
                    }
                })
                .response
                .on_disabled_hover_text(tl("No other script of the project declares an event"));
            });
        });
    });
}

/// What a variable, property, input or event value holds: its type, shape and a map's keys' type.
#[derive(Clone, Copy, PartialEq)]
struct Holds {
    ty: VariableType,
    shape: Shape,
    key: VariableType,
}

impl Holds {
    fn of(ty: VariableType, shape: Shape, key: VariableType) -> Self {
        Self { ty, shape, key }
    }

    /// As words: `Number`, `List of Number`, or `Map from Text to Number`.
    fn words(self) -> String {
        match self.shape {
            Shape::Single => tl(self.ty.label()).to_owned(),
            Shape::List => crate::i18n::tf("List of {type}", &[("type", tl(self.ty.label()))]),
            Shape::Map => crate::i18n::tf("Map from {key} to {type}", &[("key", tl(self.key.label())), ("type", tl(self.ty.label()))]),
        }
    }

    /// The type codes list and map nodes keep in `item`: the values', then the keys'.
    fn item(self) -> Value {
        json!([self.ty.code(), self.key.code()])
    }

    /// As `graph.*.set` takes it.
    fn params(self) -> Value {
        json!({ "type": self.ty, "shape": self.shape, "key": self.key })
    }
}

/// Places a list or map node of `kind` naming `name`, which holds what `holds` says.
fn place_collection_node(editor: &mut Editor, kind: &str, at: [f32; 2], name: &str, holds: Holds) {
    if let Some(added) = editor.dispatch("graph.node.add", &json!({ "kind": kind, "position": at }), None) {
        let node = added["node"].clone();
        editor.dispatch("graph.node.set", &json!({ "node": node, "name": "item", "value": holds.item() }), None);
        editor.dispatch("graph.node.set", &json!({ "node": node, "name": "name", "value": name }), None);
    }
}

/// A menu of the nodes that work on the list or map `name`: Place Node.
fn place_node_menu(editor: &mut Editor, ui: &mut egui::Ui, at: [f32; 2], name: &str, holds: Holds) {
    use bayleaf_engine::script::visual::{CATALOG, LIST_KINDS, MAP_KINDS};
    let kinds: &[&str] = if holds.shape == Shape::Map { &MAP_KINDS } else { &LIST_KINDS };
    let nodes = ui.menu_button(tl("Place Node"), |ui| {
        for def in CATALOG.nodes.iter().filter(|d| kinds.contains(&d.kind)) {
            if ui.button(tl(def.label)).on_hover_text(tl(def.description)).clicked() {
                place_collection_node(editor, def.kind, at, name, holds);
                ui.close();
            }
        }
    });
    let id = nodes.response.on_hover_text(tl("Place a node that works on it")).id;
    ui.ctx().accesskit_node_builder(id, |node| node.set_label(crate::i18n::tf("Place a node for {name}", &[("name", name)])));
}

/// What a variable or property holds, as a menu that changes it: its type, whether it is one value,
/// a list or a map, and a map's keys' type. What it holds then, when something new is picked.
fn type_menu(ui: &mut egui::Ui, t: &Tokens, holds: Holds, name: &str) -> Option<Holds> {
    let current = holds.words();
    let mut picked = None;
    let menu = ui.menu_button(egui::RichText::new(&current).small().color(t.text_muted), |ui| {
        ui.label(egui::RichText::new(tl("Type")).small().color(t.text_muted));
        for other in VariableType::ALL {
            if ui.selectable_label(other == holds.ty, tl(other.label())).clicked() {
                picked = Some(Holds { ty: other, ..holds });
                ui.close();
            }
        }
        ui.separator();
        ui.label(egui::RichText::new(tl("Holds")).small().color(t.text_muted));
        for other in Shape::ALL {
            if ui.selectable_label(other == holds.shape, tl(other.label())).clicked() {
                picked = Some(Holds { shape: other, ..holds });
                ui.close();
            }
        }
        // A map's keys can be of any type too.
        if holds.shape == Shape::Map {
            ui.separator();
            ui.label(egui::RichText::new(tl("Key")).small().color(t.text_muted));
            for other in VariableType::ALL {
                let label = crate::i18n::tf("Keys of {type}", &[("type", tl(other.label()))]);
                if ui.selectable_label(other == holds.key, label).clicked() {
                    picked = Some(Holds { key: other, ..holds });
                    ui.close();
                }
            }
        }
    });
    let id = menu.response.on_hover_text(tl("Change the type, make it a list or a map, or change a map's keys")).id;
    ui.ctx().accesskit_node_builder(id, |node| node.set_label(crate::i18n::tf("What {name} holds: {type}", &[("name", name), ("type", &current)])));
    picked.filter(|p| *p != holds)
}

/// A start or default of `holds`: one value's editor, or a list's items or a map's keys and values,
/// each with a button that takes it out, and one that adds another. A map's key is typed as text
/// (`3`, `true`, `1,2,3`) and kept only when it is a key of the map's keys' type. `key` keeps a map
/// key's typing apart from other fields. The new value when it changed.
fn shaped_editor(ui: &mut egui::Ui, panel: &mut ScriptPanel, key: &str, holds: Holds, value: &Value, name: &str) -> Option<Value> {
    use bayleaf_engine::graph::MAX_ITEMS;
    let ty = holds.ty;
    match holds.shape {
        Shape::Single => start_editor(ui, ty, value, name),
        Shape::List => {
            let mut items = value.as_array().cloned().unwrap_or_default();
            let mut changed = false;
            let mut gone = None;
            ui.vertical(|ui| {
                for (i, item) in items.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let label = crate::i18n::tf("{name} item {index}", &[("name", name), ("index", &i.to_string())]);
                        if let Some(new) = start_editor(ui, ty, item, &label) {
                            *item = new;
                            changed = true;
                        }
                        let remove = ui.small_button("×").on_hover_text(tl("Remove"));
                        ui.ctx().accesskit_node_builder(remove.id, |node| node.set_label(crate::i18n::tf("Remove {item}", &[("item", &label)])));
                        if remove.clicked() {
                            gone = Some(i);
                        }
                    });
                }
                let add = ui.add_enabled(items.len() < MAX_ITEMS, egui::Button::new(tl("Add Item")).small());
                ui.ctx().accesskit_node_builder(add.id, |node| node.set_label(crate::i18n::tf("Add an item to {name}", &[("name", name)])));
                if add.clicked() {
                    items.push(ty.zero());
                    changed = true;
                }
            });
            if let Some(i) = gone {
                items.remove(i);
                changed = true;
            }
            changed.then_some(Value::Array(items))
        }
        Shape::Map => {
            let mut entries = value.as_object().cloned().unwrap_or_default();
            let mut changed = false;
            let mut gone = None;
            let mut renamed = None;
            ui.vertical(|ui| {
                for (i, (k, item)) in entries.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let label = crate::i18n::tf("{name} under {key}", &[("name", name), ("key", k)]);
                        let remove = ui.small_button("×").on_hover_text(tl("Remove"));
                        ui.ctx().accesskit_node_builder(remove.id, |node| node.set_label(crate::i18n::tf("Remove {item}", &[("item", &label)])));
                        if remove.clicked() {
                            gone = Some(k.clone());
                        }
                        let key_label = crate::i18n::tf("Key {index} of {name}", &[("index", &i.to_string()), ("name", name)]);
                        ui.allocate_ui(egui::vec2(96.0, ROW_HEIGHT), |ui| {
                            if let Some(typed) = name_field(ui, panel, &format!("{key} key {i}"), k, &key_label) {
                                renamed = Some((k.clone(), typed));
                            }
                        });
                        if let Some(new) = start_editor(ui, ty, item, &label) {
                            *item = new;
                            changed = true;
                        }
                    });
                }
                let fresh = holds.key.fresh_key(|k| entries.contains_key(k));
                let add = ui.add_enabled(entries.len() < MAX_ITEMS && fresh.is_some(), egui::Button::new(tl("Add Key")).small());
                ui.ctx().accesskit_node_builder(add.id, |node| node.set_label(crate::i18n::tf("Add a key to {name}", &[("name", name)])));
                if add.clicked()
                    && let Some(fresh) = fresh
                {
                    entries.insert(fresh, ty.zero());
                    changed = true;
                }
            });
            if let Some(k) = gone {
                entries.remove(&k);
                changed = true;
            }
            // A key renamed to one the map has already, or to no key of its type, stays as it was.
            if let Some((old, new)) = renamed.filter(|(old, new)| old != new && holds.key.fits_key(new) && !entries.contains_key(new))
                && let Some(item) = entries.remove(&old)
            {
                entries.insert(new, item);
                changed = true;
            }
            changed.then_some(Value::Object(entries))
        }
    }
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

/// A text field that keeps what is typed while it has the keyboard, so a value the command tidies
/// (choices without empty lines) does not undo a keystroke. Returns the text when it changed.
fn kept_text(ui: &mut egui::Ui, current: &str, label: egui::Id, multiline: bool, salt: &str) -> Option<String> {
    let id = ui.make_persistent_id(("kept text", salt));
    let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| current.to_owned());
    let edit = if multiline { egui::TextEdit::multiline(&mut text).desired_rows(2) } else { egui::TextEdit::singleline(&mut text) };
    let response = ui.add(edit.id(id).desired_width(ui.available_width())).labelled_by(label);
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    response.changed().then_some(text)
}

/// How a property shows in the Inspector: label, hint, group, read-only, range and slider (numbers),
/// choices (whole numbers and text) and a condition on another property. Returns new options.
fn inspector_options(
    ui: &mut egui::Ui,
    t: &Tokens,
    graph: &Graph,
    property: &bayleaf_engine::graph::Property,
) -> Option<bayleaf_engine::graph::PropertyOptions> {
    use crate::value_editor::row;
    let mut options = property.inspector.clone();
    let before = options.clone();
    let salt = |what: &str| format!("{} {what}", property.name);
    egui::CollapsingHeader::new(tl("How the Inspector shows it")).id_salt(("inspector options", &property.name)).default_open(false).show(ui, |ui| {
        row(ui, t, "Label", |ui, label, _| {
            if let Some(text) = kept_text(ui, &options.label, label, false, &salt("label")) {
                options.label = text;
            }
        });
        row(ui, t, "Hint", |ui, label, _| {
            if let Some(text) = kept_text(ui, &options.hint, label, false, &salt("hint")) {
                options.hint = text;
            }
        });
        row(ui, t, "Group", |ui, label, _| {
            if let Some(text) = kept_text(ui, &options.group, label, false, &salt("group")) {
                options.group = text.trim().to_owned();
            }
        });
        row(ui, t, "Read Only", |ui, label, _| {
            ui.checkbox(&mut options.read_only, "").labelled_by(label);
        });
        if matches!(property.ty, VariableType::Number | VariableType::WholeNumber) {
            row(ui, t, "Range", |ui, label, _| {
                let mut on = options.range.is_some();
                if ui.checkbox(&mut on, "").labelled_by(label).changed() {
                    options.range = on.then_some([0.0, 10.0]);
                    options.slider &= on;
                }
                if let Some(range) = options.range.as_mut() {
                    ui.add(egui::DragValue::new(&mut range[0]).speed(0.1).prefix(tl("from "))).wrap_drag().labelled_by(label);
                    ui.add(egui::DragValue::new(&mut range[1]).speed(0.1).prefix(tl("to "))).wrap_drag().labelled_by(label);
                }
            });
            if options.range.is_some() {
                row(ui, t, "Slider", |ui, label, _| {
                    ui.checkbox(&mut options.slider, "").labelled_by(label);
                });
            }
        }
        if matches!(property.ty, VariableType::WholeNumber | VariableType::Text) {
            row(ui, t, "Choices", |ui, label, _| {
                if let Some(text) = kept_text(ui, &options.choices.join("\n"), label, true, &salt("choices")) {
                    options.choices = text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
                }
            });
            ui.label(
                egui::RichText::new(tl("One per line: a whole number is chosen by its place (0 first), a text by its words.")).small().color(t.text_muted),
            );
        }
        show_when(ui, t, graph, property, &mut options);
    });
    (options != before).then_some(options)
}

/// Shown only while another property (a number, whole number, true/false or text) has a value.
fn show_when(ui: &mut egui::Ui, t: &Tokens, graph: &Graph, property: &bayleaf_engine::graph::Property, options: &mut bayleaf_engine::graph::PropertyOptions) {
    let others: Vec<&bayleaf_engine::graph::Property> = graph
        .properties
        .iter()
        .filter(|p| p.name != property.name && matches!(p.ty, VariableType::Number | VariableType::WholeNumber | VariableType::TrueFalse | VariableType::Text))
        .collect();
    if others.is_empty() {
        return;
    }
    crate::value_editor::row(ui, t, "Show When", |ui, label, width| {
        let current = options.show_if.as_ref().map(|(name, _)| name.clone());
        let shown = current.clone().unwrap_or_else(|| tl("Always").to_owned());
        craft_ui::ComboBox::from_id_salt(("show when", &property.name))
            .width((width * 0.5).max(60.0))
            .truncate()
            .selected_text(shown)
            .show_ui(ui, |ui| {
                if ui.selectable_label(current.is_none(), tl("Always")).clicked() {
                    options.show_if = None;
                }
                for other in &others {
                    if ui.selectable_label(current.as_deref() == Some(other.name.as_str()), other.name.as_str()).clicked() {
                        options.show_if = Some((other.name.clone(), other.ty.zero()));
                    }
                }
            })
            .response
            .labelled_by(label);
        let Some((name, value)) = options.show_if.as_mut() else { return };
        let Some(other) = others.iter().find(|p| p.name == *name) else { return };
        ui.label(egui::RichText::new(tl("is")).color(t.text_muted));
        match other.ty {
            VariableType::TrueFalse => {
                let mut on = value.as_bool().unwrap_or(false);
                if ui.checkbox(&mut on, "").labelled_by(label).changed() {
                    *value = Value::from(on);
                }
            }
            VariableType::Text => {
                let mut text = value.as_str().unwrap_or_default().to_owned();
                if ui.add(egui::TextEdit::singleline(&mut text).desired_width(ui.available_width())).labelled_by(label).changed() {
                    *value = Value::from(text);
                }
            }
            _ => {
                let mut number = value.as_f64().unwrap_or(0.0);
                let whole = other.ty == VariableType::WholeNumber;
                if ui
                    .add(egui::DragValue::new(&mut number).speed(if whole { 0.1 } else { 0.05 }).max_decimals(if whole { 0 } else { 3 }))
                    .wrap_drag()
                    .labelled_by(label)
                    .changed()
                {
                    *value = if whole { Value::from(number.round() as i64) } else { Value::from(number) };
                }
            }
        }
    });
}
