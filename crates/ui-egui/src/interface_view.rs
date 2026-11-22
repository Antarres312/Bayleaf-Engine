//! The Interface panel: edits a game interface layout (`.bui`) visually. On the left the element
//! tree, with buttons to add, remove and move elements; in the middle the layout as a game shows
//! it at a chosen screen size, where a click picks an element; on the right the picked element's
//! fields and its own style. Every change goes through the `ui.element.*` commands and is written
//! at once; Undo puts back the layout as it was before the last change.

use std::path::PathBuf;

use bayleaf_interface::{Element, Kind, Layout};
use serde_json::{Value, json};

use crate::Editor;
use crate::i18n::{tf, tl};
use crate::theme::{self, ROW_HEIGHT};

/// Most changes Undo goes back through.
const HISTORY: usize = 100;

/// The panel's own state: the layout open, the element picked, and the fields being edited.
#[derive(Default)]
pub(crate) struct InterfaceView {
    /// The layout open, as a full path.
    pub(crate) path: Option<PathBuf>,
    pub(crate) selected: Vec<usize>,
    screen: usize,
    /// The picked element's fields as being edited (without its children), and where it is.
    draft: Option<(Vec<usize>, Element)>,
    /// The layout before each change, newest last.
    history: Vec<Layout>,
}

impl InterfaceView {
    /// The panel with `path` open and its root picked.
    pub(crate) fn open(path: &std::path::Path) -> Self {
        Self { path: Some(path.to_path_buf()), ..Self::default() }
    }
}

/// A kind's name as the panel shows it.
fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Panel => "Panel",
        Kind::Row => "Row",
        Kind::Column => "Column",
        Kind::Text => "Text",
        Kind::Image => "Image",
        Kind::Button => "Button",
        Kind::Toggle => "Toggle",
        Kind::Slider => "Slider",
        Kind::TextField => "Text Field",
        Kind::List => "List",
        Kind::Progress => "Progress Bar",
        Kind::Part => "Part",
    }
}

/// How a row of the tree reads: the kind, then the name or the text.
fn row_label(element: &Element) -> String {
    let kind = tl(kind_label(element.kind));
    let named = match (element.kind, element.name.is_empty()) {
        (_, false) => element.name.as_str(),
        (Kind::Part, true) => element.layout.as_str(),
        (_, true) => element.text.lines().next().unwrap_or_default(),
    };
    if named.is_empty() { kind.to_owned() } else { format!("{kind}: {named}") }
}

/// The element's fields without its children, as the commands take them.
fn fields(element: &Element) -> serde_json::Map<String, Value> {
    let mut without = element.clone();
    without.children.clear();
    let mut map = match serde_json::to_value(&without) {
        Ok(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    map.remove("children");
    // Fields left at their default are not written; give them back, so clearing one clears it.
    for (key, empty) in [
        ("name", json!("")),
        ("classes", json!([])),
        ("text", json!("")),
        ("image", json!("")),
        ("layout", json!("")),
        ("onClick", json!("")),
        ("bind", json!("")),
        ("style", json!({})),
        ("texts", json!({})),
    ] {
        map.entry(key).or_insert(empty);
    }
    map.entry("value").or_insert(json!(0.0));
    map.entry("min").or_insert(json!(0.0));
    map.entry("max").or_insert(json!(1.0));
    map
}

/// What the panel asks for after drawing.
enum Change {
    Add(Kind),
    Remove,
    /// Up (-1) or down (+1) among its siblings.
    Shift(isize),
    /// Out of its parent, after it.
    Out,
    /// Into the sibling before it, last.
    In,
    /// Dragged in the tree: the element at the first path into the second, at an index counted
    /// without it.
    Place(Vec<usize>, Vec<usize>, usize),
    /// Into a layout file of its own, used here as a part.
    SaveAsPart,
    /// The picked part's layout, opened in the panel.
    OpenPart(String),
    Set(Vec<usize>, serde_json::Map<String, Value>),
    Undo,
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    let Some(path) = editor.interface_view.path.clone() else {
        crate::inspector::empty_state(ui, "No interface open", "Double-click an interface (.bui) in the Project panel to edit it here.");
        return;
    };
    let base = editor.session.script_dir();
    let relative = path.strip_prefix(&base).unwrap_or(&path).to_string_lossy().replace('\\', "/");
    let loaded = crate::game_interface::loaded(&mut editor.game_interfaces, &base, &relative);
    let mut change = None;
    let mut view = std::mem::take(&mut editor.interface_view);

    // The file, Undo, and the screen size the preview stands for.
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&relative).strong());
        if ui
            .add_enabled(!view.history.is_empty(), egui::Button::new(tl("Undo")))
            .on_hover_text(tl("Put the layout back as it was before the last change"))
            .clicked()
        {
            change = Some(Change::Undo);
        }
        let screens = crate::game_interface::PREVIEW_SCREENS;
        let chosen = view.screen.min(screens.len() - 1);
        let label = ui.label(tl("Screen"));
        craft_ui::ComboBox::from_id_salt("interface panel screen")
            .selected_text(tl(screens[chosen].0))
            .show_ui(ui, |ui| {
                for (i, (name, _)) in screens.iter().enumerate() {
                    if ui.selectable_label(i == chosen, tl(name)).clicked() {
                        view.screen = i;
                    }
                }
            })
            .response
            .labelled_by(label.id);
    });
    ui.separator();
    let loaded = match loaded {
        Ok(loaded) => loaded,
        Err(problem) => {
            ui.add(egui::Label::new(egui::RichText::new(tf("This interface does not read: {problem}", &[("problem", &problem)])).color(t.text)).wrap());
            editor.interface_view = view;
            return;
        }
    };
    let layout = &loaded.layout;
    if layout.element_at(&view.selected).is_none() {
        view.selected.clear();
    }
    // The draft follows the picked element, and the file when nothing is being typed.
    let picked = layout.element_at(&view.selected).cloned().map(|mut e| {
        e.children.clear();
        e
    });
    let typing = ui.ctx().memory(|m| m.focused().is_some()) && view.draft.as_ref().is_some_and(|(at, _)| *at == view.selected);
    if !typing || view.draft.is_none() {
        view.draft = picked.map(|e| (view.selected.clone(), e));
    }

    egui::Panel::left("interface tree").resizable(true).default_size(180.0).min_size(140.0).frame(egui::Frame::new()).show(ui, |ui| {
        tree_tools(ui, layout, &view, &mut change);
        ui.separator();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            theme::dense(ui);
            tree(ui, &layout.root, &mut Vec::new(), &mut view.selected, &mut change);
        });
    });
    egui::Panel::right("interface element")
        .resizable(true)
        .default_size(250.0)
        .min_size(200.0)
        .frame(egui::Frame::new().inner_margin(egui::Margin { left: 8, ..egui::Margin::ZERO }))
        .show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if let Some((at, draft)) = view.draft.as_mut() {
                    // A part's named elements that show text, with the part's own text.
                    let inside: Vec<(String, String)> = if draft.kind == Kind::Part {
                        bayleaf_interface::Layout::load(&editor.session.script_dir().join(&draft.layout))
                            .map(|part| {
                                part.root
                                    .walk()
                                    .into_iter()
                                    .filter(|e| !e.name.is_empty() && matches!(e.kind, Kind::Text | Kind::Button | Kind::Toggle | Kind::TextField))
                                    .map(|e| (e.name.clone(), e.text.clone()))
                                    .collect()
                            })
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };
                    if let Some(set) = element_fields(ui, t, draft, at.is_empty(), &inside) {
                        change = Some(Change::Set(at.clone(), set));
                    }
                } else {
                    crate::inspector::empty_state(ui, "Nothing picked", "Pick an element in the tree or the preview.");
                }
            });
        });
    egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(egui::Margin::same(8))).show(ui, |ui| {
        let screen = crate::game_interface::PREVIEW_SCREENS[view.screen.min(crate::game_interface::PREVIEW_SCREENS.len() - 1)].1;
        let room = ui.available_size();
        match crate::game_interface::preview(ui, room, screen, &base, &relative, &mut editor.game_interfaces) {
            Ok(shown) => {
                // A click picks the smallest element under the pointer.
                let response = ui.interact(shown.area, ui.scope_id().with("interface preview pick"), egui::Sense::click());
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, tl("Interface preview")));
                if response.clicked()
                    && let Some(pos) = response.interact_pointer_pos()
                    && let Some((_, at)) = shown.elements.iter().filter(|(r, _)| r.contains(pos)).min_by(|a, b| (a.0.area()).total_cmp(&b.0.area()))
                {
                    view.selected = at.clone();
                }
                if let Some((rect, _)) = shown.elements.iter().find(|(_, at)| *at == view.selected) {
                    ui.painter().rect_stroke(rect.expand(1.0), 2.0, egui::Stroke::new(2.0, t.focus), egui::StrokeKind::Outside);
                }
            }
            Err(problem) => {
                ui.label(problem);
            }
        }
    });

    if let Some(change) = change {
        apply(editor, &mut view, layout, &relative, change);
    }
    editor.interface_view = view;
}

/// Add (a menu of kinds), Remove and the moves, for the picked element.
fn tree_tools(ui: &mut egui::Ui, layout: &Layout, view: &InterfaceView, change: &mut Option<Change>) {
    let at = &view.selected;
    let Some((index, parent_at)) = at.split_last() else {
        ui.horizontal_wrapped(|ui| add_menu(ui, change));
        return;
    };
    let siblings = layout.element_at(parent_at).map_or(0, |p| p.children.len());
    let before_holds = index.checked_sub(1).and_then(|i| layout.element_at(parent_at)?.children.get(i)).is_some_and(|e| e.kind.holds_children());
    ui.horizontal_wrapped(|ui| {
        add_menu(ui, change);
        if ui.button(tl("Remove")).on_hover_text(tl("Remove the picked element and what it holds")).clicked() {
            *change = Some(Change::Remove);
        }
        if ui.add_enabled(*index > 0, egui::Button::new(tl("Up"))).on_hover_text(tl("Move it before the element above")).clicked() {
            *change = Some(Change::Shift(-1));
        }
        if ui.add_enabled(index + 1 < siblings, egui::Button::new(tl("Down"))).on_hover_text(tl("Move it after the element below")).clicked() {
            *change = Some(Change::Shift(1));
        }
        if ui.add_enabled(!parent_at.is_empty(), egui::Button::new(tl("Out"))).on_hover_text(tl("Move it out of the element holding it")).clicked() {
            *change = Some(Change::Out);
        }
        if ui.add_enabled(before_holds, egui::Button::new(tl("In"))).on_hover_text(tl("Move it into the element above it, last")).clicked() {
            *change = Some(Change::In);
        }
        match layout.element_at(at) {
            Some(part) if part.kind == Kind::Part => {
                if ui.button(tl("Open Part")).on_hover_text(tl("Edit the part's own layout; every interface using it changes")).clicked() {
                    *change = Some(Change::OpenPart(part.layout.clone()));
                }
            }
            Some(_)
                if ui
                    .button(tl("Save as Part"))
                    .on_hover_text(tl("Keep it and what it holds as a layout of its own, used here as a part and ready for other interfaces"))
                    .clicked() =>
            {
                *change = Some(Change::SaveAsPart);
            }
            _ => {}
        }
    });
}

fn add_menu(ui: &mut egui::Ui, change: &mut Option<Change>) {
    ui.menu_button(tl("Add"), |ui| {
        for kind in Kind::ALL {
            if ui.button(tl(kind_label(kind))).clicked() {
                *change = Some(Change::Add(kind));
                ui.close();
            }
        }
    })
    .response
    .on_hover_text(tl("Add an element into the picked one, or after it"));
}

/// An element being dragged in the tree, by its path.
#[derive(Clone, Debug)]
struct Dragged(Vec<usize>);

/// Where a dragged element goes when let go over a row: before it, into it (last), or after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Drop {
    Before,
    Into,
    After,
}

/// Where `from` lands when dropped `how` on the row at `on`: the element to go into and the index
/// there, counted without `from` (as `ui.element.move` takes it); `None` onto itself or inside it.
fn drop_target(from: &[usize], on: &[usize], how: Drop, holds: usize) -> Option<(Vec<usize>, usize)> {
    if on.starts_with(from) {
        return None;
    }
    let (into, index) = match (how, on.split_last()) {
        (Drop::Into, _) => (on.to_vec(), holds),
        (Drop::Before, Some((i, parent))) => (parent.to_vec(), *i),
        (Drop::After, Some((i, parent))) => (parent.to_vec(), i + 1),
        // Before or after the root: into it.
        (_, None) => (Vec::new(), holds),
    };
    // Taken out first, it no longer counts among its old siblings before the place.
    let index = match from.split_last() {
        Some((i, parent)) if parent == into.as_slice() && *i < index => index - 1,
        _ => index,
    };
    Some((into, index))
}

/// The tree's rows, indented by depth. Delete on a focused row removes it; a row dragged onto
/// another goes before it, into it (its middle, when it holds others) or after it.
fn tree(ui: &mut egui::Ui, element: &Element, at: &mut Vec<usize>, selected: &mut Vec<usize>, change: &mut Option<Change>) {
    let depth = at.len() as f32;
    ui.horizontal(|ui| {
        ui.add_space(depth * 14.0);
        let row = ui.add(
            egui::Button::selectable(*at == *selected, (row_label(element), egui::Atom::grow()))
                .truncate()
                .sense(if at.is_empty() { egui::Sense::click() } else { egui::Sense::click_and_drag() })
                .min_size(egui::vec2(ui.available_width(), ROW_HEIGHT)),
        );
        if row.clicked() {
            selected.clone_from(at);
            row.request_focus();
        }
        if row.has_focus() && !at.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
            selected.clone_from(at);
            *change = Some(Change::Remove);
        }
        if !at.is_empty() {
            row.dnd_set_drag_payload(Dragged(at.clone()));
        }
        if let (Some(dragged), Some(pointer)) = (row.dnd_hover_payload::<Dragged>(), ui.input(|i| i.pointer.interact_pos())) {
            let third = (pointer.y - row.rect.top()) / row.rect.height().max(1.0);
            let how = match third {
                t if element.kind.holds_children() && (0.3..0.7).contains(&t) => Drop::Into,
                t if t < 0.5 => Drop::Before,
                _ => Drop::After,
            };
            if let Some((into, index)) = drop_target(&dragged.0, at, how, element.children.len()) {
                let t = theme::tokens(ui);
                let stroke = egui::Stroke::new(2.0, t.focus);
                match how {
                    Drop::Into => {
                        ui.painter().rect_stroke(row.rect, theme::RADIUS, stroke, egui::StrokeKind::Inside);
                    }
                    Drop::Before => {
                        ui.painter().hline(row.rect.x_range(), row.rect.top(), stroke);
                    }
                    Drop::After => {
                        ui.painter().hline(row.rect.x_range(), row.rect.bottom(), stroke);
                    }
                }
                if ui.input(|i| i.pointer.any_released()) {
                    egui::DragAndDrop::clear_payload(ui.ctx());
                    *change = Some(Change::Place(dragged.0.clone(), into, index));
                }
            }
        }
    });
    for (i, child) in element.children.iter().enumerate() {
        at.push(i);
        tree(ui, child, at, selected, change);
        at.pop();
    }
}

/// The picked element's fields and its own style. Returns the fields to write once an edit is
/// finished (a field left, a drag let go, a choice made).
fn element_fields(
    ui: &mut egui::Ui,
    t: &theme::Tokens,
    draft: &mut Element,
    is_root: bool,
    inside: &[(String, String)],
) -> Option<serde_json::Map<String, Value>> {
    let mut done = false;
    let finished = |r: &egui::Response| r.lost_focus() || r.drag_stopped() || (r.changed() && !r.dragged() && !r.has_focus());
    theme::headed(t, "Element").show(ui, |ui| {
        crate::value_editor::row(ui, t, "Kind", |ui, label, width| {
            let mut kind = draft.kind;
            craft_ui::ComboBox::from_id_salt("interface element kind")
                .width(width)
                .selected_text(tl(kind_label(kind)))
                .show_ui(ui, |ui| {
                    for k in Kind::ALL {
                        ui.selectable_value(&mut kind, k, tl(kind_label(k)));
                    }
                })
                .response
                .labelled_by(label);
            if kind != draft.kind {
                draft.kind = kind;
                done = true;
            }
        });
        done |= text_row(ui, t, "Name", &mut draft.name, "How scripts find it");
        if matches!(draft.kind, Kind::Text | Kind::Button | Kind::Toggle | Kind::TextField) {
            done |= text_row(ui, t, "Text", &mut draft.text, "What it shows");
        }
        if draft.kind == Kind::Image {
            done |= text_row(ui, t, "Image", &mut draft.image, "A .png or .jpg in the project");
        }
        if draft.kind == Kind::Part {
            done |= text_row(ui, t, "Layout", &mut draft.layout, "A .bui in the project");
            // The texts inside the part, changed here only: empty keeps the part's own.
            for (name, own) in inside {
                let mut text = draft.texts.get(name).cloned().unwrap_or_default();
                crate::value_editor::row(ui, t, name, |ui, label, width| {
                    let r = ui.add_sized([width, ROW_HEIGHT], egui::TextEdit::singleline(&mut text).hint_text(own.as_str())).labelled_by(label);
                    if r.lost_focus() {
                        done = true;
                    }
                });
                if text.is_empty() {
                    draft.texts.remove(name);
                } else {
                    draft.texts.insert(name.clone(), text);
                }
            }
        }
        if matches!(draft.kind, Kind::Toggle | Kind::Slider | Kind::Progress) {
            for (name, value) in [("Value", &mut draft.value), ("Min", &mut draft.min), ("Max", &mut draft.max)] {
                if draft.kind != Kind::Slider && name != "Value" {
                    continue;
                }
                crate::value_editor::row(ui, t, name, |ui, label, _| {
                    let r = ui.add(egui::DragValue::new(value).speed(0.01)).labelled_by(label);
                    done |= finished(&r);
                });
            }
        }
        if !draft.kind.holds_children() {
            done |= text_row(ui, t, "Bind", &mut draft.bind, "game:score or player:health");
        }
        if draft.kind != Kind::TextField {
            done |= text_row(ui, t, "On Click", &mut draft.on_click, "A message for the scripts, such as openMenu");
        }
        let mut classes = draft.classes.join(" ");
        crate::value_editor::row(ui, t, "Classes", |ui, label, width| {
            let r =
                ui.add_sized([width, ROW_HEIGHT], egui::TextEdit::singleline(&mut classes).hint_text(tl("Names style rules pick it by"))).labelled_by(label);
            if r.changed() {
                draft.classes = classes.split_whitespace().map(str::to_owned).collect();
            }
            done |= r.lost_focus();
        });
        if is_root {
            ui.label(
                egui::RichText::new(tl("The root element fills what its size says; give it a width and height of 100% to fill the screen."))
                    .small()
                    .color(t.text_muted),
            );
        }
    });
    ui.add_space(theme::SECTION_GAP);
    theme::headed(t, "Style").show(ui, |ui| {
        let mut remove = None;
        for (property, value) in &mut draft.style {
            crate::value_editor::row(ui, t, property, |ui, label, width| {
                let r = ui.add_sized([(width - 28.0).max(40.0), ROW_HEIGHT], egui::TextEdit::singleline(value)).labelled_by(label);
                done |= r.lost_focus();
                if ui.small_button("×").on_hover_text(tf("Remove {property}", &[("property", property)])).clicked() {
                    remove = Some(property.clone());
                }
            });
        }
        if let Some(property) = remove {
            draft.style.remove(&property);
            done = true;
        }
        let unset: Vec<(&str, &str)> = bayleaf_interface::PROPERTIES.iter().copied().filter(|(p, _)| !draft.style.contains_key(*p)).collect();
        if !unset.is_empty() {
            ui.menu_button(tl("Add Property"), |ui| {
                for (property, start) in unset {
                    if ui.button(property).clicked() {
                        draft.style.insert(property.to_owned(), start.to_owned());
                        done = true;
                        ui.close();
                    }
                }
            });
        }
        ui.label(egui::RichText::new(tl("Lengths carry their unit: 12px, 50% or auto. Colours are #rrggbb or #rrggbbaa.")).small().color(t.text_muted));
    });
    done.then(|| fields(draft))
}

/// A labelled text field; true once it is left (the edit is finished).
fn text_row(ui: &mut egui::Ui, t: &theme::Tokens, name: &str, value: &mut String, hint: &str) -> bool {
    crate::value_editor::row(ui, t, name, |ui, label, width| {
        ui.add_sized([width, ROW_HEIGHT], egui::TextEdit::singleline(value).hint_text(tl(hint))).labelled_by(label).lost_focus()
    })
}

/// Runs a change through the `ui.element.*` commands, keeping the layout before it for Undo.
fn apply(editor: &mut Editor, view: &mut InterfaceView, layout: &Layout, path: &str, change: Change) {
    let at = view.selected.clone();
    let before = layout.clone();
    let result = match change {
        Change::Undo => {
            let Some(previous) = view.history.pop() else { return };
            editor.dispatch("ui.layout.set", &json!({ "path": path, "layout": previous }), None);
            view.draft = None;
            return;
        }
        Change::Add(kind) => {
            // Into the picked element when it holds others, else after it.
            let holds = layout.element_at(&at).is_some_and(|e| e.kind.holds_children());
            let params = match at.split_last() {
                Some((index, parent)) if !holds => json!({ "path": path, "into": parent, "kind": kind, "index": index + 1 }),
                _ => json!({ "path": path, "into": at, "kind": kind }),
            };
            editor.dispatch("ui.element.add", &params, None)
        }
        Change::Remove => {
            editor.dispatch("ui.element.remove", &json!({ "path": path, "at": at }), None).map(|_| json!({ "at": at[..at.len().saturating_sub(1)] }))
        }
        Change::Shift(by) => {
            let Some((index, parent)) = at.split_last() else { return };
            let to = index.saturating_add_signed(by);
            editor.dispatch("ui.element.move", &json!({ "path": path, "at": at, "into": parent, "index": to }), None)
        }
        Change::Out => {
            let Some((_, parent)) = at.split_last() else { return };
            let Some((parent_index, grand)) = parent.split_last() else { return };
            editor.dispatch("ui.element.move", &json!({ "path": path, "at": at, "into": grand, "index": parent_index + 1 }), None)
        }
        Change::In => {
            let Some((index, parent)) = at.split_last() else { return };
            let mut into = parent.to_vec();
            into.push(index.saturating_sub(1));
            let end = layout.element_at(&into).map_or(0, |e| e.children.len());
            editor.dispatch("ui.element.move", &json!({ "path": path, "at": at, "into": into, "index": end }), None)
        }
        Change::Place(from, into, index) => editor.dispatch("ui.element.move", &json!({ "path": path, "at": from, "into": into, "index": index }), None),
        Change::Set(at, set) => editor.dispatch("ui.element.set", &json!({ "path": path, "at": at, "element": set }), None).map(|_| json!({ "at": at })),
        Change::SaveAsPart => {
            let Some(element) = layout.element_at(&at) else { return };
            let folder = std::path::Path::new(path).parent().map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let stem = if element.name.is_empty() { kind_label(element.kind).to_owned() } else { element.name.clone() };
            let base = editor.session.script_dir();
            let file = (1..1000)
                .map(|n| if n == 1 { format!("{stem}.bui") } else { format!("{stem} {n}.bui") })
                .map(|name| if folder.is_empty() { name } else { format!("{folder}/{name}") })
                .find(|candidate| !base.join(candidate).exists())
                .unwrap_or_else(|| format!("{stem}.bui"));
            editor.dispatch("ui.element.saveAsPart", &json!({ "path": path, "at": at, "to": file }), None).map(|_| json!({ "at": at }))
        }
        Change::OpenPart(part) => {
            let full = editor.session.script_dir().join(part);
            editor.open_interface(&full);
            *view = std::mem::take(&mut editor.interface_view);
            return;
        }
    };
    // A refused change leaves the layout as it was; the status bar says why.
    if let Some(done) = result {
        view.history.push(before);
        if view.history.len() > HISTORY {
            view.history.remove(0);
        }
        if let Some(at) = done.get("at").and_then(|a| serde_json::from_value::<Vec<usize>>(a.clone()).ok()) {
            view.selected = at;
        }
        view.draft = None;
    }
}
