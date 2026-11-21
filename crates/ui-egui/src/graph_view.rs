//! The graph editor: material graphs and visual scripts on a pannable, zoomable canvas.
//!
//! Every change goes through the `graph.*` commands. Mouse: drag a node's title to move it, drag
//! from a pin to another pin to link them (or into empty space to add a linked node), right-click
//! the canvas to add a node, drag the background to pan, Ctrl + wheel to zoom. Keyboard: Tab moves
//! between nodes, pins and fields; arrows move the focused node; Delete removes it; Enter on a pin
//! starts a link and Enter on a second pin completes it (Escape cancels).

use bayleaf_engine::graph::{Catalog, Graph, Node, NodeDef, PinDef, PinDefault, PinRef, PinType, Problem, SettingKind};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2, pos2, vec2};
use serde_json::{Value, json};

use crate::Editor;
use crate::theme::{self, Tokens};

/// Node width, title height and row height, in canvas units.
const NODE_WIDTH: f32 = 220.0;
const TITLE: f32 = 28.0;
const ROW: f32 = 24.0;
const PIN_RADIUS: f32 = 5.5;
/// Arrow-key step for moving a node; Shift moves finely.
const NUDGE: f32 = 20.0;

/// Where a link drag or a keyboard link started.
#[derive(Clone, Debug, PartialEq)]
struct PinPick {
    pin: PinRef,
    output: bool,
    ty: PinType,
}

/// The add-node popup.
struct AddMenu {
    /// Canvas position of the new node.
    at: Pos2,
    /// Screen position of the popup.
    screen: Pos2,
    search: String,
    /// Link the new node to this pin.
    from: Option<PinPick>,
    focus_search: bool,
    /// True on the frame the menu opens.
    fresh: bool,
}

/// The graph editor's own state, kept between frames.
pub(crate) struct GraphView {
    /// The part of the canvas in view.
    scene_rect: Rect,
    selected: Option<u32>,
    /// A link being dragged with the mouse.
    drag: Option<PinPick>,
    /// A link started from the keyboard, waiting for its second pin.
    picked: Option<PinPick>,
    add_menu: Option<AddMenu>,
    /// Gives a node keyboard focus next frame (after adding it, or from the problem list).
    focus_node: Option<u32>,
    /// Which graph the view was set up for, so opening another one resets the view.
    shown: Option<(String, Option<std::path::PathBuf>)>,
    /// A node or pin had keyboard focus last frame; plain keys (Delete, arrows) then belong to the graph.
    pub focused: bool,
}

impl Default for GraphView {
    fn default() -> Self {
        Self { scene_rect: Rect::NOTHING, selected: None, drag: None, picked: None, add_menu: None, focus_node: None, shown: None, focused: false }
    }
}

/// Height of a node of kind `def`.
fn node_height(def: &NodeDef) -> f32 {
    TITLE + (def.settings.len() + def.outputs.len() + def.inputs.len()) as f32 * ROW + 6.0
}

fn node_rect(node: &Node, def: &NodeDef) -> Rect {
    Rect::from_min_size(pos2(node.position[0], node.position[1]), vec2(NODE_WIDTH, node_height(def)))
}

/// Centre of a pin on the canvas. Rows: settings, then outputs, then inputs, so a value field on an
/// input never shares a row with an output's label.
fn pin_pos(node: &Node, def: &NodeDef, name: &str, output: bool) -> Option<Pos2> {
    let rect = node_rect(node, def);
    let pins = if output { def.outputs } else { def.inputs };
    let index = pins.iter().position(|p| p.name == name)?;
    let row = def.settings.len() + if output { index } else { def.outputs.len() + index };
    let y = rect.top() + TITLE + 3.0 + (row as f32 + 0.5) * ROW;
    Some(pos2(if output { rect.right() } else { rect.left() }, y))
}

fn pin_color(t: &Tokens, ty: PinType) -> Color32 {
    match ty {
        PinType::Flow => t.pin_flow,
        PinType::Float => t.pin_number,
        PinType::Vec3 => t.pin_vector,
        PinType::Color => t.pin_color,
        PinType::Bool => t.pin_bool,
        PinType::Any => t.pin_any,
    }
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let Some(doc) = editor.session.graph().cloned() else {
        empty_state(editor, ui);
        return;
    };
    let Some(catalog) = doc.catalog() else {
        ui.label("This graph's kind is not known to this editor.");
        return;
    };
    let mut view = std::mem::take(&mut editor.graph_view);
    let key = (doc.graph.domain.clone(), doc.path.clone());
    if view.shown.as_ref() != Some(&key) {
        view = GraphView { shown: Some(key), ..GraphView::default() };
    }
    if view.selected.is_some_and(|id| doc.graph.node(id).is_none()) {
        view.selected = None;
    }
    let problems = bayleaf_engine::problems(&doc.graph);

    toolbar(editor, ui, &doc, catalog, &mut view);
    let t = theme::tokens(ui);
    let strip_height = if problems.is_empty() { theme::ROW_HEIGHT } else { (problems.len().min(4) as f32) * theme::ROW_HEIGHT + 8.0 };
    egui::Panel::bottom("graph problems").exact_size(strip_height).frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(8, 2))).show(
        ui,
        |ui| {
            problem_strip(ui, &doc.graph, catalog, &problems, &mut view);
        },
    );
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.backdrop)).show(ui, |ui| {
        canvas(editor, ui, &doc.graph, catalog, &problems, &mut view);
    });
    add_menu(editor, ui.ctx(), &doc.graph, catalog, &mut view);
    editor.graph_view = view;
}

/// Shown when no graph is open.
fn empty_state(editor: &mut Editor, ui: &mut egui::Ui) {
    let t = theme::tokens(ui);
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.3).max(12.0));
        ui.label(egui::RichText::new("No graph open").heading().color(t.text));
        ui.add_space(theme::GAP);
        ui.label(egui::RichText::new("Make a material or a visual script, or double-click one in the Project panel.").color(t.text_muted));
        ui.add_space(theme::GAP * 2.0);
        ui.horizontal(|ui| {
            let width = 3.0 * 150.0 + 2.0 * ui.spacing().item_spacing.x;
            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
            if ui.add_sized([150.0, theme::ROW_HEIGHT], egui::Button::new("New Material")).clicked() {
                editor.dispatch("graph.new", &json!({ "domain": "material" }), None);
            }
            if ui.add_sized([150.0, theme::ROW_HEIGHT], egui::Button::new("New Visual Script")).clicked() {
                editor.dispatch("graph.new", &json!({ "domain": "script" }), None);
            }
            if ui.add_sized([150.0, theme::ROW_HEIGHT], egui::Button::new("Open Graph…")).clicked()
                && let Some(path) = rfd::FileDialog::new().add_filter("Graphs", &["bmaterial", "bscript"]).pick_file()
            {
                editor.open_graph(&path);
            }
        });
    });
}

fn toolbar(editor: &mut Editor, ui: &mut egui::Ui, doc: &bayleaf_engine::GraphDoc, catalog: &Catalog, view: &mut GraphView) {
    let t = theme::tokens(ui);
    egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let name = doc
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map_or_else(|| format!("Untitled {}", catalog.label.to_lowercase()), |n| n.to_string_lossy().into_owned());
            ui.label(egui::RichText::new(&name).strong().color(t.text));
            ui.label(egui::RichText::new(catalog.label).small().color(t.text_muted));
            if doc.dirty {
                egui::Frame::new().fill(t.surface_active).corner_radius(theme::RADIUS).inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| {
                    ui.label(egui::RichText::new("Modified").small().color(t.text));
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() && (!doc.dirty || crate::confirm_discard_graph()) {
                    editor.dispatch("graph.close", &Value::Null, None);
                }
                if ui.button("Save As…").clicked() {
                    save_as(editor, catalog);
                }
                let save = ui.add_enabled(doc.dirty || doc.path.is_none(), egui::Button::new("Save"));
                if save.clicked() {
                    if doc.path.is_some() {
                        editor.dispatch("graph.save", &Value::Null, None);
                    } else {
                        save_as(editor, catalog);
                    }
                }
                if ui.button("Add Node").clicked() {
                    let at = if view.scene_rect.is_finite() && view.scene_rect.is_positive() { view.scene_rect.center() } else { pos2(0.0, 0.0) };
                    view.add_menu =
                        Some(AddMenu { at, screen: ui.min_rect().left_bottom(), search: String::new(), from: None, focus_search: true, fresh: true });
                }
            });
        });
    });
}

fn save_as(editor: &mut Editor, catalog: &Catalog) {
    if let Some(path) = rfd::FileDialog::new().add_filter(catalog.label, &[catalog.extension]).set_file_name(format!("new.{}", catalog.extension)).save_file() {
        let path = if path.extension().is_none() { path.with_extension(catalog.extension) } else { path };
        editor.dispatch("graph.save", &json!({ "path": path }), None);
    }
}

fn problem_strip(ui: &mut egui::Ui, graph: &Graph, catalog: &Catalog, problems: &[Problem], view: &mut GraphView) {
    let t = theme::tokens(ui);
    if problems.is_empty() {
        ui.label(egui::RichText::new(format!("Ready: this {} compiles.", catalog.label.to_lowercase())).color(t.text_muted));
        return;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for problem in problems {
            ui.horizontal(|ui| {
                warning_badge(ui, t);
                let place = problem
                    .node
                    .and_then(|id| graph.node(id))
                    .map(|n| format!("{} {}: ", catalog.get(&n.kind).map_or(n.kind.as_str(), |d| d.label), n.id))
                    .unwrap_or_default();
                let text = format!("{place}{}", problem.message);
                if let Some(id) = problem.node {
                    if ui.link(egui::RichText::new(text).color(t.text)).on_hover_text("Show this node").clicked() {
                        view.selected = Some(id);
                        view.focus_node = Some(id);
                    }
                } else {
                    ui.label(egui::RichText::new(text).color(t.text));
                }
            });
        }
    });
}

/// A small "!" in a warning disc; the text beside it says what is wrong, so colour is not the only signal.
fn warning_badge(ui: &mut egui::Ui, t: &Tokens) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 7.0, t.warning);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "!", egui::FontId::proportional(11.0), t.on_warning);
}

/// What the canvas asks for after drawing.
#[derive(Default)]
struct Requests {
    commands: Vec<(&'static str, Value, Option<String>)>,
    /// Ends the current edit gesture, so the next edit gets its own undo step.
    end_gesture: bool,
}

impl Requests {
    fn run(&mut self, id: &'static str, params: Value) {
        self.commands.push((id, params, None));
    }

    fn merge(&mut self, id: &'static str, params: Value, key: String) {
        self.commands.push((id, params, Some(key)));
    }
}

fn canvas(editor: &mut Editor, ui: &mut egui::Ui, graph: &Graph, catalog: &Catalog, problems: &[Problem], view: &mut GraphView) {
    let t = theme::tokens(ui);
    let gesture = editor.gesture;
    let mut requests = Requests::default();
    if !view.scene_rect.is_finite() || !view.scene_rect.is_positive() {
        view.scene_rect = fit(graph, catalog, ui.available_rect_before_wrap().size());
    }
    if let Some(id) = view.focus_node
        && let Some(node) = graph.node(id)
        && let Some(def) = catalog.get(&node.kind)
    {
        // Bring a node that gets focus into view.
        let rect = node_rect(node, def);
        if !view.scene_rect.contains_rect(rect) {
            view.scene_rect = view.scene_rect.translate(rect.center() - view.scene_rect.center());
        }
    }
    view.focused = false;
    let mut hovered_pin: Option<PinPick> = None;
    let mut scene_rect = view.scene_rect;
    let response = egui::Scene::new().zoom_range(0.25..=2.0).show(ui, &mut scene_rect, |ui| {
        let to_scene = ui.ctx().layer_transform_from_global(ui.layer_id()).unwrap_or_default();
        let zoom = 1.0 / to_scene.scaling.max(1e-3);
        paint_dots(ui, t, zoom);
        // Links under the nodes.
        for link in &graph.links {
            let ends = graph.node(link.from.node).zip(graph.node(link.to.node)).and_then(|(from, to)| {
                let (fd, td) = (catalog.get(&from.kind)?, catalog.get(&to.kind)?);
                let ty = fd.output(&link.from.pin)?.ty;
                Some((pin_pos(from, fd, &link.from.pin, true)?, pin_pos(to, td, &link.to.pin, false)?, ty))
            });
            if let Some((a, b, ty)) = ends {
                paint_link(ui.painter(), a, b, pin_color(t, ty), if ty == PinType::Flow { 3.0 } else { 2.0 });
            }
        }
        let mut problem_nodes: Vec<u32> = problems.iter().filter_map(|p| p.node).collect();
        problem_nodes.dedup();
        for node in &graph.nodes {
            if let Some(def) = catalog.get(&node.kind) {
                node_ui(ui, t, graph, catalog, node, def, problem_nodes.contains(&node.id), gesture, zoom, view, &mut requests, &mut hovered_pin);
            }
        }
        // The link being dragged follows the pointer.
        if let Some(drag) = &view.drag
            && let Some(start) = graph.node(drag.pin.node).and_then(|n| pin_pos(n, catalog.get(&n.kind)?, &drag.pin.pin, drag.output))
            && let Some(pointer) = ui.ctx().pointer_hover_pos()
        {
            let end = to_scene * pointer;
            let (a, b) = if drag.output { (start, end) } else { (end, start) };
            paint_link(ui.painter(), a, b, pin_color(t, drag.ty), 2.0);
        }
        to_scene
    });
    view.scene_rect = scene_rect;
    let to_scene = response.inner;
    let canvas_response = response.response;
    canvas_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, "Graph canvas"));

    // Dropping a dragged link: onto a pin links it, into empty space offers a linked new node.
    if let Some(drag) = view.drag.clone()
        && ui.input(|i| i.pointer.any_released())
    {
        view.drag = None;
        match hovered_pin {
            Some(target) => link_pins(&drag, &target, &mut requests),
            None => {
                if let Some(pointer) = ui.ctx().pointer_interact_pos() {
                    view.add_menu =
                        Some(AddMenu { at: to_scene * pointer, screen: pointer, search: String::new(), from: Some(drag), focus_search: true, fresh: true });
                }
            }
        }
    }
    if canvas_response.clicked() {
        view.selected = None;
        view.picked = None;
    }
    if canvas_response.secondary_clicked()
        && let Some(pointer) = ui.ctx().pointer_interact_pos()
    {
        view.add_menu = Some(AddMenu { at: to_scene * pointer, screen: pointer, search: String::new(), from: None, focus_search: true, fresh: true });
    }
    if view.picked.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        view.picked = None;
        editor.status = "Linking cancelled".to_owned();
    }
    if requests.end_gesture {
        editor.gesture += 1;
    }
    for (id, params, key) in requests.commands {
        editor.dispatch(id, &params, key.as_deref());
    }
    if let Some(picked) = &view.picked {
        editor.status = format!("Linking from pin {}: press Enter on the pin to link to, or Escape to cancel", picked.pin.pin);
    }
}

/// A view that shows every node, or the area around the origin for an empty graph.
fn fit(graph: &Graph, catalog: &Catalog, size: Vec2) -> Rect {
    let mut bounds = Rect::NOTHING;
    for node in &graph.nodes {
        if let Some(def) = catalog.get(&node.kind) {
            bounds = bounds.union(node_rect(node, def));
        }
    }
    let size = size.max(vec2(200.0, 150.0));
    if !bounds.is_positive() {
        return Rect::from_center_size(pos2(200.0, 150.0), size);
    }
    Rect::from_center_size(bounds.center(), size.max(bounds.size() + vec2(80.0, 80.0)))
}

/// A dot grid, so panning is visible.
fn paint_dots(ui: &egui::Ui, t: &Tokens, zoom: f32) {
    let clip = ui.clip_rect();
    let step = if zoom < 0.5 { 80.0 } else { 40.0 };
    let color = t.divider;
    let radius = (1.2 / zoom).min(3.0);
    let mut x = (clip.left() / step).floor() * step;
    let painter = ui.painter();
    while x <= clip.right() {
        let mut y = (clip.top() / step).floor() * step;
        while y <= clip.bottom() {
            painter.circle_filled(pos2(x, y), radius, color);
            y += step;
        }
        x += step;
    }
}

fn paint_link(painter: &egui::Painter, a: Pos2, b: Pos2, color: Color32, width: f32) {
    let dx = ((b.x - a.x).abs() * 0.5).max(40.0);
    let points = [a, a + vec2(dx, 0.0), b - vec2(dx, 0.0), b];
    painter.add(egui::epaint::CubicBezierShape::from_points_stroke(points, false, Color32::TRANSPARENT, Stroke::new(width, color)));
}

fn link_pins(a: &PinPick, b: &PinPick, requests: &mut Requests) {
    if a.output == b.output || a.pin.node == b.pin.node {
        return;
    }
    let (from, to) = if a.output { (&a.pin, &b.pin) } else { (&b.pin, &a.pin) };
    requests.run("graph.link.add", json!({ "from": from, "to": to }));
}

#[allow(clippy::too_many_arguments)]
fn node_ui(
    ui: &mut egui::Ui,
    t: &Tokens,
    graph: &Graph,
    catalog: &Catalog,
    node: &Node,
    def: &NodeDef,
    has_problem: bool,
    gesture: u64,
    zoom: f32,
    view: &mut GraphView,
    requests: &mut Requests,
    hovered_pin: &mut Option<PinPick>,
) {
    let rect = node_rect(node, def);
    let selected = view.selected == Some(node.id);
    let painter = ui.painter().clone();
    painter.rect_filled(rect, theme::CARD_RADIUS, t.surface);
    let title_rect = Rect::from_min_size(rect.min, vec2(rect.width(), TITLE));
    let title_fill = if selected { t.accent } else { t.surface_active };
    let title_text = if selected { t.on_accent } else { t.text };
    painter.rect_filled(title_rect, egui::CornerRadius { nw: theme::CARD_RADIUS, ne: theme::CARD_RADIUS, sw: 0, se: 0 }, title_fill);
    let mut title_x = rect.left() + 10.0;
    if has_problem {
        painter.circle_filled(pos2(title_x + 6.0, title_rect.center().y), 7.0, t.warning);
        painter.text(pos2(title_x + 6.0, title_rect.center().y), egui::Align2::CENTER_CENTER, "!", egui::FontId::proportional(11.0), t.on_warning);
        title_x += 18.0;
    }
    painter.text(pos2(title_x, title_rect.center().y), egui::Align2::LEFT_CENTER, def.label, egui::FontId::proportional(14.0), title_text);
    painter.text(
        pos2(rect.right() - 10.0, title_rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{}", node.id),
        egui::FontId::proportional(11.0),
        title_text,
    );

    // The title is the node's handle: click selects, drag moves, keys move and delete.
    let id = ui.scope_id().with(("graph node", node.id));
    let title = ui.interact(title_rect, id, Sense::click_and_drag());
    let problem_note = if has_problem { ", has a problem" } else { "" };
    title.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, format!("{} node {}{problem_note}", def.label, node.id)));
    screen_bounds(ui, &title);
    if view.focus_node == Some(node.id) {
        title.request_focus();
        view.focus_node = None;
    }
    if title.clicked() || title.drag_started() || title.gained_focus() {
        view.selected = Some(node.id);
    }
    if title.dragged() {
        let delta = title.drag_delta() * zoom;
        if delta != Vec2::ZERO {
            let position = [node.position[0] + delta.x, node.position[1] + delta.y];
            requests.merge("graph.node.move", json!({ "node": node.id, "position": position }), format!("graph move {}", gesture));
        }
    }
    if title.drag_stopped() {
        requests.end_gesture = true;
    }
    view.focused |= title.has_focus();
    // Arrows move the focused node instead of moving focus to the next widget.
    ui.memory_mut(|m| m.set_focus_lock_filter(id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
    if title.has_focus() {
        let (step, delete) = ui.input(|i| {
            let step = if i.modifiers.shift { NUDGE / 4.0 } else { NUDGE };
            let mut d = Vec2::ZERO;
            if i.key_pressed(egui::Key::ArrowLeft) {
                d.x -= step;
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                d.x += step;
            }
            if i.key_pressed(egui::Key::ArrowUp) {
                d.y -= step;
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                d.y += step;
            }
            (d, i.key_pressed(egui::Key::Delete))
        });
        if step != Vec2::ZERO {
            requests.run("graph.node.move", json!({ "node": node.id, "position": [node.position[0] + step.x, node.position[1] + step.y] }));
        }
        if delete {
            requests.run("graph.node.remove", json!({ "node": node.id }));
        }
        painter.rect_stroke(rect.expand(2.0), theme::CARD_RADIUS, Stroke::new(2.0, t.focus), egui::StrokeKind::Outside);
    }
    title.context_menu(|ui| {
        ui.label(egui::RichText::new(def.description).small().color(t.text_muted));
        ui.separator();
        if ui.button("Delete Node").clicked() {
            requests.run("graph.node.remove", json!({ "node": node.id }));
            ui.close();
        }
    });
    title.on_hover_text(def.description);

    let mut y = rect.top() + TITLE + 3.0;
    for setting in def.settings {
        let row = Rect::from_min_size(pos2(rect.left() + 10.0, y), vec2(rect.width() - 20.0, ROW));
        setting_row(ui, t, node, setting, row, gesture, requests);
        y += ROW;
    }
    for pin in def.inputs {
        let center = pin_pos(node, def, pin.name, false).unwrap_or(rect.left_top());
        pin_ui(ui, t, graph, catalog, node, def, pin, false, center, view, requests, hovered_pin);
        let linked = graph.link_into(node.id, pin.name).is_some();
        let label_rect = Rect::from_min_size(pos2(rect.left() + 12.0, center.y - ROW / 2.0), vec2(rect.width() * 0.42, ROW));
        painter.text(pos2(label_rect.left(), center.y), egui::Align2::LEFT_CENTER, pin.label, egui::FontId::proportional(13.0), t.text);
        if !linked && pin.ty != PinType::Flow && !matches!(pin.default, PinDefault::Implicit(_)) {
            let field =
                Rect::from_min_max(pos2(rect.left() + rect.width() * 0.42, center.y - ROW / 2.0 + 2.0), pos2(rect.right() - 10.0, center.y + ROW / 2.0 - 2.0));
            input_editor(ui, node, def, pin, field, gesture, requests);
        } else if matches!(pin.default, PinDefault::Implicit(_)) && !linked {
            painter.text(pos2(rect.right() - 10.0, center.y), egui::Align2::RIGHT_CENTER, "surface", egui::FontId::proportional(12.0), t.text_muted);
        }
    }
    for pin in def.outputs {
        let center = pin_pos(node, def, pin.name, true).unwrap_or(rect.right_top());
        pin_ui(ui, t, graph, catalog, node, def, pin, true, center, view, requests, hovered_pin);
        painter.text(pos2(rect.right() - 12.0, center.y), egui::Align2::RIGHT_CENTER, pin.label, egui::FontId::proportional(13.0), t.text);
    }
}

#[allow(clippy::too_many_arguments)]
fn pin_ui(
    ui: &mut egui::Ui,
    t: &Tokens,
    graph: &Graph,
    catalog: &Catalog,
    node: &Node,
    def: &NodeDef,
    pin: &PinDef,
    output: bool,
    center: Pos2,
    view: &mut GraphView,
    requests: &mut Requests,
    hovered_pin: &mut Option<PinPick>,
) {
    let pick = PinPick { pin: PinRef::new(node.id, pin.name), output, ty: pin.ty };
    let hit = Rect::from_center_size(center, vec2(PIN_RADIUS * 4.0, PIN_RADIUS * 4.0));
    let response = ui.interact(hit, ui.scope_id().with(("graph pin", node.id, pin.name, output)), Sense::click_and_drag());
    let links: Vec<String> = if output {
        graph.links_from(node.id, pin.name).filter_map(|l| graph.node(l.to.node).map(|n| node_name(catalog, n))).collect()
    } else {
        graph
            .links
            .iter()
            .filter(|l| l.to.node == node.id && l.to.pin == pin.name)
            .filter_map(|l| graph.node(l.from.node).map(|n| node_name(catalog, n)))
            .collect()
    };
    let side = if output { "Output" } else { "Input" };
    let linked = if links.is_empty() { "not linked".to_owned() } else { format!("linked {} {}", if output { "to" } else { "from" }, links.join(", ")) };
    let picked_here = view.picked.as_ref() == Some(&pick);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::Button,
            true,
            picked_here,
            format!("{side} {} ({}) of {} {}, {linked}", pin.label, pin.ty.label(), def.label, node.id),
        )
    });
    screen_bounds(ui, &response);

    let color = pin_color(t, pin.ty);
    let hot = response.hovered() || response.has_focus() || picked_here;
    let r = if hot { PIN_RADIUS + 1.5 } else { PIN_RADIUS };
    let painter = ui.painter();
    let filled = !links.is_empty();
    if pin.ty == PinType::Flow {
        // Flow pins are arrows, so they differ from data pins by shape as well as colour.
        let points = vec![center + vec2(-r, -r), center + vec2(r, 0.0), center + vec2(-r, r)];
        painter.add(egui::Shape::convex_polygon(points, if filled { color } else { t.surface }, Stroke::new(2.0, color)));
    } else {
        painter.circle(center, r, if filled { color } else { t.surface }, Stroke::new(2.0, color));
    }
    view.focused |= response.has_focus();
    if response.has_focus() {
        painter.circle_stroke(center, r + 4.0, Stroke::new(2.0, t.focus));
    }
    if response.contains_pointer() {
        *hovered_pin = Some(pick.clone());
    }

    if response.drag_started() {
        // Dragging from a linked input picks up its link from the other end.
        match (output, graph.link_into(node.id, pin.name)) {
            (false, Some(link)) if pin.ty != PinType::Flow => {
                let from = link.from.clone();
                let ty = graph.node(from.node).and_then(|n| catalog.get(&n.kind)).and_then(|d| d.output(&from.pin)).map_or(pin.ty, |p| p.ty);
                requests.run("graph.link.remove", json!({ "node": node.id, "pin": pin.name }));
                view.drag = Some(PinPick { pin: from, output: true, ty });
            }
            _ => view.drag = Some(pick.clone()),
        }
    }
    if response.clicked() {
        // Click or Enter: start a link here, or finish the one started on another pin.
        match view.picked.take() {
            Some(first) if first != pick => link_pins(&first, &pick, requests),
            Some(_) => {}
            None => view.picked = Some(pick.clone()),
        }
    }
    response.context_menu(|ui| {
        if ui.add_enabled(!links.is_empty(), egui::Button::new("Remove Links")).clicked() {
            requests.run("graph.link.remove", json!({ "node": node.id, "pin": pin.name }));
            ui.close();
        }
        if ui.button("Start Link Here").clicked() {
            view.picked = Some(pick.clone());
            ui.close();
        }
    });
}

fn node_name(catalog: &Catalog, node: &Node) -> String {
    format!("{} {}", catalog.get(&node.kind).map_or(node.kind.as_str(), |d| d.label), node.id)
}

/// The stored value of an input or setting, or its default.
fn value_or(node: &Node, name: &str, default: Value) -> Value {
    node.values.get(name).cloned().unwrap_or(default)
}

fn triple(value: &Value, default: [f32; 3]) -> [f32; 3] {
    bayleaf_engine::graph::as_triple(value).unwrap_or(default)
}

fn input_editor(ui: &mut egui::Ui, node: &Node, def: &NodeDef, pin: &PinDef, rect: Rect, gesture: u64, requests: &mut Requests) {
    let key = format!("graph set {} {} {gesture}", node.id, pin.name);
    let name = format!("{} of {} {}", pin.label, def.label, node.id);
    match pin.ty {
        PinType::Float | PinType::Any => {
            let default = match pin.default {
                PinDefault::Float(v) => v,
                _ => 0.0,
            };
            let current = value_or(node, pin.name, json!(default));
            if let Some(mut v) = bayleaf_engine::graph::as_f32(&current) {
                let response = ui.put(rect, egui::DragValue::new(&mut v).speed(0.01).max_decimals(3));
                labelled(ui, &response, &name);
                commit(&response, requests, || ("graph.node.set", json!({ "node": node.id, "name": pin.name, "value": v })), key);
            } else {
                ui.put(rect, egui::Label::new("vector"));
            }
        }
        PinType::Vec3 => {
            let default = match pin.default {
                PinDefault::Vec3(v) => v,
                PinDefault::Float(v) => [v; 3],
                _ => [0.0; 3],
            };
            let mut v = triple(&value_or(node, pin.name, Value::Null), default);
            vec3_editor(ui, rect, &mut v, &name, requests, node.id, pin.name, key);
        }
        PinType::Color => {
            let default = match pin.default {
                PinDefault::Color(v) => v,
                _ => [0.0; 3],
            };
            let mut v = triple(&value_or(node, pin.name, Value::Null), default);
            let response =
                ui.put(Rect::from_min_size(rect.min, vec2(rect.width().min(48.0), rect.height())), |ui: &mut egui::Ui| ui.color_edit_button_rgb(&mut v));
            labelled(ui, &response, &name);
            commit(&response, requests, || ("graph.node.set", json!({ "node": node.id, "name": pin.name, "value": v })), key);
        }
        PinType::Bool => {
            let default = matches!(pin.default, PinDefault::Bool(true));
            let mut b = value_or(node, pin.name, json!(default)).as_bool().unwrap_or(default);
            let response = ui.put(Rect::from_min_size(rect.min, vec2(rect.height(), rect.height())), egui::Checkbox::without_text(&mut b));
            labelled(ui, &response, &name);
            if response.changed() {
                requests.run("graph.node.set", json!({ "node": node.id, "name": pin.name, "value": b }));
            }
        }
        PinType::Flow => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn vec3_editor(ui: &mut egui::Ui, rect: Rect, v: &mut [f32; 3], name: &str, requests: &mut Requests, node: u32, field: &str, key: String) {
    let width = (rect.width() - 4.0) / 3.0;
    let mut changed = false;
    let mut ended = false;
    for (i, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
        let cell = Rect::from_min_size(rect.min + vec2(i as f32 * (width + 2.0), 0.0), vec2(width, rect.height()));
        let response = ui.put(cell, egui::DragValue::new(&mut v[i]).speed(0.01).max_decimals(2));
        labelled(ui, &response, &format!("{name} {axis}"));
        changed |= response.changed();
        ended |= response.drag_stopped() || response.lost_focus();
    }
    if changed {
        requests.merge("graph.node.set", json!({ "node": node, "name": field, "value": v }), key);
    }
    if ended {
        requests.end_gesture = true;
    }
}

fn setting_row(ui: &mut egui::Ui, t: &Tokens, node: &Node, setting: &bayleaf_engine::graph::SettingDef, row: Rect, gesture: u64, requests: &mut Requests) {
    let label_width = row.width() * 0.36;
    ui.painter().text(pos2(row.left(), row.center().y), egui::Align2::LEFT_CENTER, setting.label, egui::FontId::proportional(13.0), t.text_muted);
    let field = Rect::from_min_max(pos2(row.left() + label_width, row.top() + 2.0), pos2(row.right(), row.bottom() - 2.0));
    let key = format!("graph set {} {} {gesture}", node.id, setting.name);
    let name = format!("{} of node {}", setting.label, node.id);
    match setting.kind {
        SettingKind::Float(default) => {
            let mut v = bayleaf_engine::graph::as_f32(&value_or(node, setting.name, json!(default))).unwrap_or(default);
            let response = ui.put(field, egui::DragValue::new(&mut v).speed(0.01).max_decimals(3));
            labelled(ui, &response, &name);
            commit(&response, requests, || ("graph.node.set", json!({ "node": node.id, "name": setting.name, "value": v })), key);
        }
        SettingKind::Color(default) => {
            let mut v = triple(&value_or(node, setting.name, Value::Null), default);
            let response = ui.put(Rect::from_min_size(field.min, vec2(48.0, field.height())), |ui: &mut egui::Ui| ui.color_edit_button_rgb(&mut v));
            labelled(ui, &response, &name);
            commit(&response, requests, || ("graph.node.set", json!({ "node": node.id, "name": setting.name, "value": v })), key);
        }
        SettingKind::Vec3(default) => {
            let mut v = triple(&value_or(node, setting.name, Value::Null), default);
            vec3_editor(ui, field, &mut v, &name, requests, node.id, setting.name, key);
        }
        SettingKind::Text(default) | SettingKind::Name(default) => {
            let mut text = value_or(node, setting.name, json!(default)).as_str().unwrap_or(default).to_owned();
            let response = ui.put(field, egui::TextEdit::singleline(&mut text));
            labelled(ui, &response, &name);
            if response.changed() {
                requests.merge("graph.node.set", json!({ "node": node.id, "name": setting.name, "value": text }), key);
            }
            if response.lost_focus() {
                requests.end_gesture = true;
            }
        }
    }
}

fn labelled(ui: &egui::Ui, response: &egui::Response, name: &str) {
    ui.ctx().accesskit_node_builder(response.id, |node| node.set_label(name.to_owned()));
    screen_bounds(ui, response);
}

/// egui reports a widget's bounds to assistive technology in canvas units, before the pan and
/// zoom of the canvas. Report where the widget really is on screen.
fn screen_bounds(ui: &egui::Ui, response: &egui::Response) {
    if let Some(to_global) = ui.ctx().layer_transform_to_global(ui.layer_id()) {
        let rect = to_global * response.rect;
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_bounds(egui::accesskit::Rect { x0: rect.min.x.into(), y0: rect.min.y.into(), x1: rect.max.x.into(), y1: rect.max.y.into() });
        });
    }
}

/// Sends a value edit; drags and typing merge into one undo step until the gesture ends.
fn commit(response: &egui::Response, requests: &mut Requests, edit: impl FnOnce() -> (&'static str, Value), key: String) {
    if response.changed() {
        let (id, params) = edit();
        requests.merge(id, params, key);
    }
    if response.drag_stopped() || response.lost_focus() {
        requests.end_gesture = true;
    }
}

/// The searchable add-node popup.
fn add_menu(editor: &mut Editor, ctx: &egui::Context, graph: &Graph, catalog: &Catalog, view: &mut GraphView) {
    let Some(mut menu) = view.add_menu.take() else { return };
    let mut close = false;
    let mut chosen: Option<&'static NodeDef> = None;
    let area = egui::Area::new(egui::Id::unique("graph add node")).order(egui::Order::Foreground).fixed_pos(menu.screen).show(ctx, |ui| {
        let t = theme::tokens(ui);
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_width(240.0);
            let search = ui.add(egui::TextEdit::singleline(&mut menu.search).hint_text("Search nodes"));
            ui.ctx().accesskit_node_builder(search.id, |node| node.set_label("Search nodes"));
            if menu.focus_search {
                search.request_focus();
                menu.focus_search = false;
            }
            let needle = menu.search.to_lowercase();
            let fits = |def: &NodeDef| match &menu.from {
                None => true,
                Some(from) if from.output => def.inputs.iter().any(|p| from.ty.connects_to(p.ty)),
                Some(from) => def.outputs.iter().any(|p| p.ty.connects_to(from.ty)),
            };
            let unique_taken = |def: &NodeDef| def.unique && graph.nodes.iter().any(|n| n.kind == def.kind);
            let matches: Vec<&'static NodeDef> = catalog
                .nodes
                .iter()
                .filter(|d| fits(d) && !unique_taken(d))
                .filter(|d| needle.is_empty() || d.label.to_lowercase().contains(&needle) || d.category.to_lowercase().contains(&needle))
                .collect();
            // Enter takes the first match. A one-line field gives up focus on Enter in the same frame.
            if (search.has_focus() || search.lost_focus())
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Some(first) = matches.first()
            {
                chosen = Some(first);
            }
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                let mut category = "";
                for def in &matches {
                    if def.category != category {
                        category = def.category;
                        ui.label(egui::RichText::new(category).small().color(t.text_muted));
                    }
                    if ui.add(egui::Button::new(def.label).min_size(vec2(ui.available_width(), theme::LIST_ROW))).on_hover_text(def.description).clicked() {
                        chosen = Some(def);
                    }
                }
                if matches.is_empty() {
                    ui.label(egui::RichText::new("No matching nodes").color(t.text_muted));
                }
            });
        });
    });
    // The click that opened the menu is not a click outside it.
    let opened_now = std::mem::replace(&mut menu.fresh, false);
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) || (area.response.clicked_elsewhere() && !opened_now) {
        close = true;
    }
    if let Some(def) = chosen {
        if let Some(created) = editor.dispatch("graph.node.add", &json!({ "kind": def.kind, "position": [menu.at.x, menu.at.y] }), None)
            && let Some(id) = created.get("node").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok())
        {
            view.selected = Some(id);
            view.focus_node = Some(id);
            if let Some(from) = &menu.from {
                let target =
                    if from.output { def.inputs.iter().find(|p| from.ty.connects_to(p.ty)) } else { def.outputs.iter().find(|p| p.ty.connects_to(from.ty)) };
                if let Some(pin) = target {
                    let other = PinPick { pin: PinRef::new(id, pin.name), output: !from.output, ty: pin.ty };
                    let mut requests = Requests::default();
                    link_pins(from, &other, &mut requests);
                    for (id, params, _) in requests.commands {
                        editor.dispatch(id, &params, None);
                    }
                }
            }
        }
        close = true;
    }
    if !close {
        view.add_menu = Some(menu);
    }
}
