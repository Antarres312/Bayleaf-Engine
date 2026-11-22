//! The graph editor: material graphs and visual scripts on a pannable, zoomable canvas.
//!
//! Every change goes through the `graph.*` commands. Mouse: drag a node's title to move it, drag
//! from a pin to another pin to link them (or into empty space to add a linked node), right-click
//! the canvas to add a node, drag the background to pan, the mouse wheel to zoom, Ctrl + click to select
//! several. Keyboard: Ctrl + Plus, Minus and 0 zoom in, out and to every node; Ctrl+C, Ctrl+V and
//! Ctrl+D copy, paste and duplicate the selected nodes; Tab moves
//! between nodes, pins and fields; arrows move the focused node; Delete removes it; Enter on a pin
//! starts a link and Enter on a second pin completes it (Escape cancels). Comment boxes sit behind
//! the nodes: their title moves them with the nodes inside (arrows), Ctrl + arrows resize, Enter
//! edits the text. A material graph shows a preview of the material on a sphere.

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
    pub(crate) scene_rect: Rect,
    /// The visual script's panel of variables, properties and menu path.
    script_panel: crate::script_panel::ScriptPanel,
    /// The selected nodes; Ctrl + click adds or takes one out.
    selected: Vec<u32>,
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
    /// A comment whose text is being typed, and the text so far.
    editing_comment: Option<(u32, String)>,
    /// Gives a comment's title keyboard focus next frame.
    focus_comment: Option<u32>,
    /// Shows the material preview in the canvas corner.
    show_preview: bool,
    /// The material preview's renderer, kept when another graph opens.
    preview: Option<Preview>,
    /// What a search node can look for, gathered each frame while a script graph is shown.
    search: SearchChoices,
}

/// The components, project scripts and scene tags a Find Object With, Count Objects With or For
/// Each Object With node can name.
#[derive(Default)]
struct SearchChoices {
    components: Vec<&'static str>,
    /// The project's scripts by name, with the properties each declares (empty when they cannot be read).
    scripts: Vec<(String, Vec<String>)>,
    tags: Vec<String>,
}

/// The node kinds that read or set a component's fields, named in their `component` setting.
const COMPONENT_KINDS: [&str; 3] = ["getComponent", "tryGetComponent", "setComponent"];

/// The node kinds that search by a component, a tag or a script, in their `kind` setting.
const SEARCH_KINDS: [&str; 3] = ["findWith", "countWith", "forEachWith"];

/// Keeps the property names of nodes that name a script in step with what the script declares now:
/// one edit per node that changed, so it undoes, and links to properties the script dropped go away.
fn refresh_script_fields(editor: &mut Editor, graph: &Graph, search: &SearchChoices) {
    for node in graph.nodes.iter().filter(|n| COMPONENT_KINDS.contains(&n.kind.as_str())) {
        let Some(kept) = bayleaf_engine::script::visual::script_fields(node) else { continue };
        let Some(name) = node.values.get("component").and_then(Value::as_str) else { continue };
        let Some((script, declared)) = search.scripts.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) else { continue };
        if *declared != kept && editor.dispatch("graph.node.set", &json!({ "node": node.id, "name": "fields", "value": declared }), None).is_some() {
            editor.status = format!("Node {}: the properties of {script} changed, so its pins did too.", node.id);
        }
    }
}

/// The name a search uses for a script file: the name the Inspector shows for it.
fn script_name(module: &str) -> String {
    bayleaf_engine::scene::components::ScriptRef { module: module.to_owned(), properties: Default::default() }.display_name()
}

impl SearchChoices {
    fn gather(editor: &mut Editor) -> Self {
        let dir = editor.session.script_dir();
        let mut scripts: Vec<(String, Vec<String>)> = Vec::new();
        for module in editor.project.script_files() {
            let name = script_name(&module);
            if scripts.iter().any(|(n, _)| *n == name) {
                continue;
            }
            // A script that cannot be read (not built yet, or broken) is left out, so nodes keep its
            // last known properties.
            if let Ok(properties) = editor.script_info.properties(&dir.join(&module)) {
                scripts.push((name, properties.into_iter().map(|(n, _)| n).collect()));
            }
        }
        let scene = editor.session.scene();
        let components = scene.registry().kinds().iter().filter(|k| k.listed).map(|k| k.label).collect();
        let mut tags: Vec<String> = scene
            .world_matrices()
            .iter()
            .filter_map(|(id, _)| scene.get::<bayleaf_engine::scene::components::Tags>(*id))
            .flat_map(|tags| tags.0.iter().cloned())
            .collect();
        tags.sort_unstable();
        tags.dedup();
        Self { components, scripts, tags }
    }
}

impl Default for GraphView {
    fn default() -> Self {
        Self {
            scene_rect: Rect::NOTHING,
            script_panel: crate::script_panel::ScriptPanel::default(),
            selected: Vec::new(),
            drag: None,
            picked: None,
            add_menu: None,
            focus_node: None,
            shown: None,
            focused: false,
            editing_comment: None,
            focus_comment: None,
            show_preview: true,
            preview: None,
            search: SearchChoices::default(),
        }
    }
}

/// A material shown on a sphere, drawn on the GPU into a texture.
struct Preview {
    renderer: bayleaf_render::Renderer,
    target: bayleaf_render::RenderTarget,
    texture: egui::TextureId,
    /// The graph last compiled, and the result (`None`: it does not compile).
    compiled: Option<(Graph, Option<std::sync::Arc<bayleaf_render::CompiledMaterial>>)>,
    started: std::time::Instant,
}

/// Size of the material preview, in points.
const PREVIEW_SIZE: f32 = 160.0;
/// Room around nodes in a comment made around them.
const COMMENT_PADDING: f32 = 30.0;
/// Size of a comment's corner handle.
const COMMENT_HANDLE: f32 = 16.0;

/// Height of a node of kind `def`.
fn node_height(def: &NodeDef) -> f32 {
    TITLE + (shown_settings(def) + def.outputs.len() + def.inputs.len()) as f32 * ROW + 6.0
}

/// How many settings a node shows as rows.
fn shown_settings(def: &NodeDef) -> usize {
    def.settings.iter().filter(|s| s.shown()).count()
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
    let row = shown_settings(def) + if output { index } else { def.outputs.len() + index };
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
        PinType::Object => t.pin_object,
    }
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui, gpu: Option<&egui_wgpu::RenderState>) {
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
        let (preview, show_preview) = (view.preview.take(), view.show_preview);
        view = GraphView { shown: Some(key), preview, show_preview, ..GraphView::default() };
    }
    if view.editing_comment.as_ref().is_some_and(|(id, _)| doc.graph.comment(*id).is_none()) {
        view.editing_comment = None;
    }
    view.selected.retain(|id| doc.graph.node(*id).is_some());
    view.search = if doc.graph.nodes.iter().any(|n| SEARCH_KINDS.contains(&n.kind.as_str()) || COMPONENT_KINDS.contains(&n.kind.as_str())) {
        SearchChoices::gather(editor)
    } else {
        SearchChoices::default()
    };
    refresh_script_fields(editor, &doc.graph, &view.search);
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
    // A visual script's variables, properties and menu path, beside its graph.
    if catalog.domain == bayleaf_engine::script::visual::CATALOG.domain {
        egui::Panel::left("script panel")
            .resizable(true)
            .default_size(230.0)
            .min_size(160.0)
            .frame(egui::Frame::new().fill(t.panel).inner_margin(egui::Margin::symmetric(6, 2)))
            .show(ui, |ui| {
                let center = if view.scene_rect.is_finite() && view.scene_rect.is_positive() { view.scene_rect.center() } else { egui::pos2(0.0, 0.0) };
                crate::script_panel::show(editor, ui, &doc.graph, [center.x, center.y], &mut view.script_panel);
            });
    }
    let canvas_rect = egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(t.backdrop))
        .show(ui, |ui| {
            canvas(editor, ui, &doc.graph, catalog, &problems, &mut view);
        })
        .response
        .rect;
    if catalog.domain == bayleaf_render::material::CATALOG.domain && view.show_preview {
        material_preview(editor, ui, &doc.graph, gpu, &mut view, canvas_rect);
    }
    add_menu(editor, ui.ctx(), &doc.graph, catalog, &mut view);
    editor.graph_view = view;
}

/// The open material on a sphere, in the canvas's top-right corner. It follows unsaved changes.
fn material_preview(editor: &mut Editor, ui: &egui::Ui, graph: &Graph, gpu: Option<&egui_wgpu::RenderState>, view: &mut GraphView, canvas: Rect) {
    let t = theme::tokens(ui);
    let size = vec2(PREVIEW_SIZE, PREVIEW_SIZE);
    if canvas.width() < size.x * 2.0 || canvas.height() < size.y + 40.0 {
        return;
    }
    let ppp = ui.ctx().pixels_per_point();
    let pixels = (PREVIEW_SIZE * ppp).round().max(1.0) as u32;
    let mut texture = None;
    let mut message = None;
    if let Some(gpu) = gpu {
        let preview = view.preview.get_or_insert_with(|| {
            let target = bayleaf_render::RenderTarget::new(&gpu.device, pixels, pixels);
            let texture = gpu.renderer.write().register_native_texture(&gpu.device, target.view(), bayleaf_render::wgpu::FilterMode::Linear);
            Preview { renderer: bayleaf_render::Renderer::new(&gpu.device), target, texture, compiled: None, started: std::time::Instant::now() }
        });
        if preview.target.size() != (pixels, pixels) {
            preview.target = bayleaf_render::RenderTarget::new(&gpu.device, pixels, pixels);
            gpu.renderer.write().update_egui_texture_from_wgpu_texture(
                &gpu.device,
                preview.target.view(),
                bayleaf_render::wgpu::FilterMode::Linear,
                preview.texture,
            );
        }
        if preview.compiled.as_ref().is_none_or(|(g, _)| g != graph) {
            preview.compiled = Some((graph.clone(), bayleaf_render::material::compile(graph).ok().map(std::sync::Arc::new)));
        }
        match preview.compiled.as_ref().and_then(|(_, m)| m.clone()) {
            Some(material) => {
                let base = editor.session.script_dir();
                let cache = &mut editor.scene_view.assets;
                let textures = material.textures.iter().map(|path| cache.image(&base.join(path)).ok()).collect();
                if material.animated {
                    ui.ctx().request_repaint();
                }
                let frame = bayleaf_render::material_preview(material, textures, 1.0, preview.started.elapsed().as_secs_f32());
                preview.renderer.render(&gpu.device, &gpu.queue, &preview.target, &frame);
                texture = Some(preview.texture);
            }
            None => message = Some("Does not compile yet"),
        }
    } else {
        message = Some("Needs the GPU renderer");
    }
    let at = pos2(canvas.right() - size.x - 10.0, canvas.top() + 10.0);
    egui::Area::new(egui::Id::unique("graph material preview")).order(egui::Order::Middle).fixed_pos(at).show(ui.ctx(), |ui| {
        egui::Frame::new().fill(t.panel).corner_radius(theme::CARD_RADIUS).inner_margin(4.0).show(ui, |ui| {
            ui.set_width(size.x);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Preview").small().color(t.text_muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let hide = ui.add(egui::Button::new(egui::RichText::new("Hide").small()).frame(false));
                    ui.ctx().accesskit_node_builder(hide.id, |node| node.set_label("Hide material preview"));
                    if hide.on_hover_text("Show it again from the toolbar").clicked() {
                        view.show_preview = false;
                    }
                });
            });
            match texture {
                Some(texture) => {
                    let image = ui.add(egui::Image::new(egui::load::SizedTexture::new(texture, size)).corner_radius(theme::RADIUS));
                    image.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, "Material preview: the material on a sphere"));
                }
                None => {
                    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
                    ui.painter().rect_filled(rect, theme::RADIUS, t.surface);
                    let text = message.unwrap_or_default();
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(12.0), t.text_muted);
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, format!("Material preview: {text}")));
                }
            }
        });
    });
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
                theme::tag(ui, "Modified", t.surface_active, t.text);
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
                if ui.button("Fit All").on_hover_text("Show every node (Ctrl+0)").clicked() {
                    // The canvas fits the view to the nodes when it has none.
                    view.scene_rect = Rect::NOTHING;
                }
                if catalog.domain == bayleaf_render::material::CATALOG.domain
                    && !view.show_preview
                    && ui.button("Show Preview").on_hover_text("Show the material on a sphere").clicked()
                {
                    view.show_preview = true;
                }
                if ui.button("Add Comment").on_hover_text("A box with a note, around the selected nodes").clicked() {
                    add_comment(editor, view, &doc.graph, catalog);
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

/// Adds a comment around the selected nodes, or in the middle of the view, and starts typing its text.
fn add_comment(editor: &mut Editor, view: &mut GraphView, graph: &Graph, catalog: &Catalog) {
    let mut bounds = Rect::NOTHING;
    for node in view.selected.iter().filter_map(|id| graph.node(*id)) {
        if let Some(def) = catalog.def_of(node) {
            bounds = bounds.union(node_rect(node, def));
        }
    }
    let rect = if bounds.is_positive() {
        let mut rect = bounds.expand(COMMENT_PADDING);
        rect.min.y -= TITLE;
        rect
    } else {
        let [w, h] = bayleaf_engine::graph::COMMENT_SIZE;
        let center = if view.scene_rect.is_positive() { view.scene_rect.center() } else { pos2(0.0, 0.0) };
        Rect::from_center_size(center, vec2(w, h))
    };
    let params = json!({ "position": [rect.min.x, rect.min.y], "size": [rect.width(), rect.height()], "text": "Comment" });
    if let Some(id) = editor.dispatch("graph.comment.add", &params, None).and_then(|r| r["comment"].as_u64()).and_then(|v| u32::try_from(v).ok()) {
        view.editing_comment = Some((id, "Comment".to_owned()));
        view.focus_comment = Some(id);
    }
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
                    .map(|n| format!("{} {}: ", catalog.def_of(n).map_or(n.kind.as_str(), |d| d.label), n.id))
                    .unwrap_or_default();
                let text = format!("{place}{}", problem.message);
                if let Some(id) = problem.node {
                    if ui.link(egui::RichText::new(text).color(t.text)).on_hover_text("Show this node").clicked() {
                        view.selected = vec![id];
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
    /// Copy (false) or duplicate (true) the selected nodes, from a node's menu.
    clipboard: Option<bool>,
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
        && let Some(def) = catalog.def_of(node)
    {
        // Bring a node that gets focus into view.
        let rect = node_rect(node, def);
        if !view.scene_rect.contains_rect(rect) {
            view.scene_rect = view.scene_rect.translate(rect.center() - view.scene_rect.center());
        }
    }
    zoom_keys(editor, ui, graph, catalog, view);
    view.focused = false;
    let mut hovered_pin: Option<PinPick> = None;
    let mut scene_rect = view.scene_rect;
    // The mouse wheel zooms around the pointer; dragging the background pans.
    let response = egui::Scene::new().zoom_range(ZOOM_RANGE).scroll_zooms(true).show(ui, &mut scene_rect, |ui| {
        let to_scene = ui.ctx().layer_transform_from_global(ui.layer_id()).unwrap_or_default();
        let zoom = 1.0 / to_scene.scaling.max(1e-3);
        paint_dots(ui, t, zoom);
        // Comments under everything, so the nodes in them stay on top.
        for comment in &graph.comments {
            comment_ui(ui, t, graph, catalog, comment, gesture, zoom, view, &mut requests);
        }
        // Links under the nodes.
        for link in &graph.links {
            let ends = graph.node(link.from.node).zip(graph.node(link.to.node)).and_then(|(from, to)| {
                let (fd, td) = (catalog.def_of(from)?, catalog.def_of(to)?);
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
            if let Some(def) = catalog.def_of(node) {
                node_ui(ui, t, graph, catalog, node, def, problem_nodes.contains(&node.id), gesture, zoom, view, &mut requests, &mut hovered_pin);
            }
        }
        // The link being dragged follows the pointer.
        if let Some(drag) = &view.drag
            && let Some(start) = graph.node(drag.pin.node).and_then(|n| pin_pos(n, catalog.def_of(n)?, &drag.pin.pin, drag.output))
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
        view.selected.clear();
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
    match requests.clipboard {
        Some(false) => copy_nodes(editor, view),
        Some(true) => duplicate_nodes(editor, view),
        None => {}
    }
    if let Some(picked) = &view.picked {
        editor.status = format!("Linking from pin {}: press Enter on the pin to link to, or Escape to cancel", picked.pin.pin);
    }
}

/// Closest and farthest canvas zoom.
const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.25..=2.0;
/// One keyboard zoom step.
const ZOOM_STEP: f32 = 1.25;

/// Ctrl + Plus and Ctrl + Minus zoom the canvas around its centre and Ctrl + 0 shows every node,
/// while a node has focus or the pointer is over the canvas. They are taken before egui's own
/// keys, which would scale the whole editor.
fn zoom_keys(editor: &mut Editor, ui: &egui::Ui, graph: &Graph, catalog: &Catalog, view: &mut GraphView) {
    let area = ui.available_rect_before_wrap();
    if !(view.focused || ui.rect_contains_pointer(area)) || !view.scene_rect.is_positive() {
        return;
    }
    use egui::gui_zoom::kb_shortcuts::{ZOOM_IN, ZOOM_IN_SECONDARY, ZOOM_OUT, ZOOM_RESET};
    let (zoom_in, zoom_out, reset) = ui
        .input_mut(|i| (i.consume_shortcut(&ZOOM_IN) | i.consume_shortcut(&ZOOM_IN_SECONDARY), i.consume_shortcut(&ZOOM_OUT), i.consume_shortcut(&ZOOM_RESET)));
    if reset {
        view.scene_rect = fit(graph, catalog, area.size());
    } else if zoom_in || zoom_out {
        let current = area.width() / view.scene_rect.width();
        let wanted = if zoom_in { current * ZOOM_STEP } else { current / ZOOM_STEP };
        let zoom = wanted.clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
        view.scene_rect = Rect::from_center_size(view.scene_rect.center(), area.size() / zoom);
    } else {
        return;
    }
    editor.status = format!("Graph zoom {:.0}%", area.width() / view.scene_rect.width() * 100.0);
}

/// A view that shows every node, or the area around the origin for an empty graph.
fn fit(graph: &Graph, catalog: &Catalog, size: Vec2) -> Rect {
    let mut bounds = Rect::NOTHING;
    for node in &graph.nodes {
        if let Some(def) = catalog.def_of(node) {
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

/// One comment box: its title moves it with the nodes inside, its corner resizes it. Keyboard: Tab
/// to the title, arrows move, Ctrl + arrows resize, Enter edits the text, Delete removes it.
#[allow(clippy::too_many_arguments)]
fn comment_ui(
    ui: &mut egui::Ui,
    t: &Tokens,
    graph: &Graph,
    catalog: &Catalog,
    comment: &bayleaf_engine::graph::Comment,
    gesture: u64,
    zoom: f32,
    view: &mut GraphView,
    requests: &mut Requests,
) {
    let rect = Rect::from_min_size(pos2(comment.position[0], comment.position[1]), vec2(comment.size[0], comment.size[1]));
    let painter = ui.painter().clone();
    painter.rect_filled(rect, theme::CARD_RADIUS, t.panel);
    let title_rect = Rect::from_min_size(rect.min, vec2(rect.width(), TITLE));
    painter.rect_filled(title_rect, egui::CornerRadius { nw: theme::CARD_RADIUS, ne: theme::CARD_RADIUS, sw: 0, se: 0 }, t.surface_hover);
    let id = ui.scope_id().with(("graph comment", comment.id));
    let first_line = comment.text.lines().next().unwrap_or_default().to_owned();
    let name = if first_line.is_empty() { "Comment".to_owned() } else { format!("Comment: {first_line}") };

    // Typing the text.
    if let Some((editing, mut text)) = view.editing_comment.clone()
        && editing == comment.id
    {
        let field = title_rect.shrink2(vec2(6.0, 3.0));
        let response = ui.put(field, egui::TextEdit::singleline(&mut text).id(id.with("text")).char_limit(bayleaf_engine::graph::MAX_COMMENT_TEXT));
        labelled(ui, &response, "Comment text");
        if view.focus_comment == Some(comment.id) {
            response.request_focus();
            view.focus_comment = None;
        }
        if response.lost_focus() {
            // Enter or a click elsewhere keeps the text; Escape keeps the old one.
            if !ui.input(|i| i.key_pressed(egui::Key::Escape)) && text != comment.text {
                requests.run("graph.comment.set", json!({ "comment": comment.id, "text": text }));
            }
            view.editing_comment = None;
            view.focus_comment = Some(comment.id);
        } else {
            view.editing_comment = Some((editing, text));
        }
        view.focused |= response.has_focus();
        return;
    }
    painter.with_clip_rect(title_rect.shrink(4.0)).text(
        pos2(title_rect.left() + 10.0, title_rect.center().y),
        egui::Align2::LEFT_CENTER,
        if first_line.is_empty() { "Comment" } else { &first_line },
        egui::FontId::proportional(14.0),
        t.text,
    );

    let title = ui.interact(title_rect, id, Sense::click_and_drag());
    title.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
    screen_bounds(ui, &title);
    if view.focus_comment == Some(comment.id) {
        title.request_focus();
        view.focus_comment = None;
    }
    // The nodes inside move with the box.
    let inside: Vec<&Node> = graph.nodes.iter().filter(|n| catalog.def_of(n).is_some_and(|d| rect.contains_rect(node_rect(n, d)))).collect();
    let move_by = |delta: Vec2, requests: &mut Requests, key: Option<String>| {
        let mut moves = vec![("graph.comment.set", json!({ "comment": comment.id, "position": [rect.min.x + delta.x, rect.min.y + delta.y] }))];
        for node in &inside {
            moves.push(("graph.node.move", json!({ "node": node.id, "position": [node.position[0] + delta.x, node.position[1] + delta.y] })));
        }
        for (command, params) in moves {
            match &key {
                Some(key) => requests.merge(command, params, key.clone()),
                None => requests.run(command, params),
            }
        }
    };
    if title.dragged() {
        let delta = title.drag_delta() * zoom;
        if delta != Vec2::ZERO {
            move_by(delta, requests, Some(format!("graph move {gesture}")));
        }
    }
    if title.drag_stopped() {
        requests.end_gesture = true;
    }
    let edit = title.double_clicked() || (title.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
    view.focused |= title.has_focus();
    ui.memory_mut(|m| m.set_focus_lock_filter(id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() }));
    if title.has_focus() {
        let (step, resize, delete) = ui.input(|i| {
            let mut d = Vec2::ZERO;
            for (key, v) in [
                (egui::Key::ArrowLeft, vec2(-1.0, 0.0)),
                (egui::Key::ArrowRight, vec2(1.0, 0.0)),
                (egui::Key::ArrowUp, vec2(0.0, -1.0)),
                (egui::Key::ArrowDown, vec2(0.0, 1.0)),
            ] {
                if i.key_pressed(key) {
                    d += v;
                }
            }
            let amount = if i.modifiers.shift { NUDGE / 4.0 } else { NUDGE };
            (d * amount, i.modifiers.command, i.key_pressed(egui::Key::Delete))
        });
        if step != Vec2::ZERO && resize {
            requests.run("graph.comment.set", json!({ "comment": comment.id, "size": [comment.size[0] + step.x, comment.size[1] + step.y] }));
        } else if step != Vec2::ZERO {
            move_by(step, requests, None);
        }
        if delete {
            requests.run("graph.comment.remove", json!({ "comment": comment.id }));
        }
        painter.rect_stroke(rect.expand(2.0), theme::CARD_RADIUS, Stroke::new(2.0, t.focus), egui::StrokeKind::Outside);
    }
    let mut start_editing = edit;
    title.context_menu(|ui| {
        if ui.add(egui::Button::new("Edit Text").shortcut_text("Enter")).clicked() {
            start_editing = true;
            ui.close();
        }
        if ui.add(egui::Button::new("Delete Comment").shortcut_text("Delete")).clicked() {
            requests.run("graph.comment.remove", json!({ "comment": comment.id }));
            ui.close();
        }
    });
    if start_editing {
        view.editing_comment = Some((comment.id, comment.text.clone()));
        view.focus_comment = Some(comment.id);
    }
    title.on_hover_text("Drag to move it with the nodes inside; double-click to edit the text");

    // The corner handle resizes the box.
    let handle_rect = Rect::from_min_max(rect.max - vec2(COMMENT_HANDLE, COMMENT_HANDLE), rect.max);
    let handle = ui.interact(handle_rect, id.with("size"), Sense::drag());
    handle.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Resize {name}")));
    screen_bounds(ui, &handle);
    let handle_color = if handle.hovered() || handle.dragged() { t.text } else { t.text_muted };
    for k in [4.0, 8.0, 12.0] {
        painter.line_segment([pos2(rect.max.x - k, rect.max.y - 3.0), pos2(rect.max.x - 3.0, rect.max.y - k)], Stroke::new(1.5, handle_color));
    }
    if handle.hovered() || handle.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
    }
    if handle.dragged() {
        let delta = handle.drag_delta() * zoom;
        if delta != Vec2::ZERO {
            requests.merge(
                "graph.comment.set",
                json!({ "comment": comment.id, "size": [comment.size[0] + delta.x, comment.size[1] + delta.y] }),
                format!("graph comment size {gesture}"),
            );
        }
    }
    if handle.drag_stopped() {
        requests.end_gesture = true;
    }
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
    let selected = view.selected.contains(&node.id);
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
    let toggle = ui.input(|i| i.modifiers.command);
    if title.clicked() && toggle {
        // Ctrl + click adds the node to the selection or takes it out.
        if selected {
            view.selected.retain(|id| *id != node.id);
        } else {
            view.selected.push(node.id);
        }
    } else if title.clicked() || ((title.drag_started() || title.gained_focus()) && !selected) {
        view.selected = vec![node.id];
    }
    if title.dragged() {
        let delta = title.drag_delta() * zoom;
        if delta != Vec2::ZERO {
            // Dragging a selected node moves every selected node, as one undo step.
            let moving = if view.selected.contains(&node.id) { view.selected.clone() } else { vec![node.id] };
            for moved in moving.iter().filter_map(|id| graph.node(*id)) {
                let position = [moved.position[0] + delta.x, moved.position[1] + delta.y];
                requests.merge("graph.node.move", json!({ "node": moved.id, "position": position }), format!("graph move {}", gesture));
            }
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
            let removing = if view.selected.contains(&node.id) { view.selected.clone() } else { vec![node.id] };
            for id in removing {
                requests.merge("graph.node.remove", json!({ "node": id }), format!("graph delete {gesture}"));
            }
            requests.end_gesture = true;
        }
        painter.rect_stroke(rect.expand(2.0), theme::CARD_RADIUS, Stroke::new(2.0, t.focus), egui::StrokeKind::Outside);
    }
    title.context_menu(|ui| {
        ui.label(egui::RichText::new(def.description).small().color(t.text_muted));
        ui.separator();
        if !view.selected.contains(&node.id) {
            view.selected = vec![node.id];
        }
        if ui.add(egui::Button::new("Copy").shortcut_text("Ctrl+C")).clicked() {
            requests.clipboard = Some(false);
            ui.close();
        }
        if ui.add(egui::Button::new("Duplicate").shortcut_text("Ctrl+D")).clicked() {
            requests.clipboard = Some(true);
            ui.close();
        }
        if ui.add(egui::Button::new("Delete Node").shortcut_text("Delete")).clicked() {
            requests.run("graph.node.remove", json!({ "node": node.id }));
            ui.close();
        }
    });
    title.on_hover_text(def.description);

    let mut y = rect.top() + TITLE + 3.0;
    for setting in def.settings.iter().filter(|s| s.shown()) {
        let row = Rect::from_min_size(pos2(rect.left() + 10.0, y), vec2(rect.width() - 20.0, ROW));
        setting_row(ui, &Lists { graph, search: &view.search }, node, setting, row, gesture, requests);
        y += ROW;
    }
    for pin in def.inputs {
        let center = pin_pos(node, def, pin.name, false).unwrap_or(rect.left_top());
        pin_ui(ui, t, graph, catalog, node, def, pin, false, center, view, requests, hovered_pin);
        let linked = graph.link_into(node.id, pin.name).is_some();
        let label_rect = Rect::from_min_size(pos2(rect.left() + 12.0, center.y - ROW / 2.0), vec2(rect.width() * 0.42, ROW));
        ui.interact(label_rect, ui.scope_id().with(("pin label", node.id, pin.name, false)), Sense::hover()).on_hover_text(pin_hint(pin));
        painter.text(pos2(label_rect.left(), center.y), egui::Align2::LEFT_CENTER, pin.label, egui::FontId::proportional(13.0), t.text);
        if !linked && pin.ty != PinType::Flow && !matches!(pin.default, PinDefault::Implicit(_)) {
            let field =
                Rect::from_min_max(pos2(rect.left() + rect.width() * 0.42, center.y - ROW / 2.0 + 2.0), pos2(rect.right() - 10.0, center.y + ROW / 2.0 - 2.0));
            input_editor(ui, node, def, pin, field, gesture, requests);
        } else if let PinDefault::Implicit(meaning) = pin.default
            && !linked
        {
            // What the pin means without a link: a material's surface, or a field Set Component leaves as it is.
            let shown = if meaning == "unchanged" { "unchanged" } else { "surface" };
            painter.text(pos2(rect.right() - 10.0, center.y), egui::Align2::RIGHT_CENTER, shown, egui::FontId::proportional(12.0), t.text_muted);
        }
    }
    for pin in def.outputs {
        let center = pin_pos(node, def, pin.name, true).unwrap_or(rect.right_top());
        pin_ui(ui, t, graph, catalog, node, def, pin, true, center, view, requests, hovered_pin);
        let label_rect = Rect::from_min_max(pos2(rect.left() + rect.width() * 0.5, center.y - ROW / 2.0), pos2(rect.right() - 12.0, center.y + ROW / 2.0));
        ui.interact(label_rect, ui.scope_id().with(("pin label", node.id, pin.name, true)), Sense::hover()).on_hover_text(pin_hint(pin));
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
    let response = response.on_hover_text(pin_hint(pin));
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
    } else if pin.ty == PinType::Object {
        // Object pins are diamonds.
        let d = r + 1.0;
        let points = vec![center + vec2(0.0, -d), center + vec2(d, 0.0), center + vec2(0.0, d), center + vec2(-d, 0.0)];
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
                let ty = graph.node(from.node).and_then(|n| catalog.def_of(n)).and_then(|d| d.output(&from.pin)).map_or(pin.ty, |p| p.ty);
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

/// What hovering a pin, its label or its value says: the pin's name and the type of data it carries.
fn pin_hint(pin: &PinDef) -> String {
    let meaning = match pin.ty {
        PinType::Flow => "what runs next",
        PinType::Float => "one number",
        PinType::Vec3 => "three numbers: X, Y and Z",
        PinType::Color => "red, green and blue",
        PinType::Bool => "true or false",
        PinType::Any => "a number or a vector, by what is linked",
        PinType::Object => "an object in the scene, or none",
    };
    format!("{}: {} ({meaning})", pin.label, pin.ty.label())
}

fn node_name(catalog: &Catalog, node: &Node) -> String {
    format!("{} {}", catalog.def_of(node).map_or(node.kind.as_str(), |d| d.label), node.id)
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
    let hint = pin_hint(pin);
    match pin.ty {
        PinType::Float | PinType::Any => {
            let default = match pin.default {
                PinDefault::Float(v) => v,
                _ => 0.0,
            };
            let current = value_or(node, pin.name, json!(default));
            if let Some(mut v) = bayleaf_engine::graph::as_f32(&current) {
                let response = ui.put(rect, egui::DragValue::new(&mut v).speed(0.01).max_decimals(3)).on_hover_text(&hint);
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
            vec3_editor(ui, rect, &mut v, &name, Some(&hint), requests, node.id, pin.name, key);
        }
        PinType::Color => {
            let default = match pin.default {
                PinDefault::Color(v) => v,
                _ => [0.0; 3],
            };
            let mut v = triple(&value_or(node, pin.name, Value::Null), default);
            let response = ui
                .put(Rect::from_min_size(rect.min, vec2(rect.width().min(48.0), rect.height())), |ui: &mut egui::Ui| ui.color_edit_button_rgb(&mut v))
                .on_hover_text(&hint);
            labelled(ui, &response, &name);
            commit(&response, requests, || ("graph.node.set", json!({ "node": node.id, "name": pin.name, "value": v })), key);
        }
        PinType::Bool => {
            let default = matches!(pin.default, PinDefault::Bool(true));
            let mut b = value_or(node, pin.name, json!(default)).as_bool().unwrap_or(default);
            let response = ui.put(Rect::from_min_size(rect.min, vec2(rect.height(), rect.height())), egui::Checkbox::without_text(&mut b)).on_hover_text(&hint);
            labelled(ui, &response, &name);
            if response.changed() {
                requests.run("graph.node.set", json!({ "node": node.id, "name": pin.name, "value": b }));
            }
        }
        // An unlinked object input means the node's own choice (named in its settings, or this object).
        PinType::Flow | PinType::Object => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn vec3_editor(ui: &mut egui::Ui, rect: Rect, v: &mut [f32; 3], name: &str, hint: Option<&str>, requests: &mut Requests, node: u32, field: &str, key: String) {
    let width = (rect.width() - 4.0) / 3.0;
    let mut changed = false;
    let mut ended = false;
    for (i, axis) in ["X", "Y", "Z"].into_iter().enumerate() {
        let cell = Rect::from_min_size(rect.min + vec2(i as f32 * (width + 2.0), 0.0), vec2(width, rect.height()));
        let response = ui.put(cell, egui::DragValue::new(&mut v[i]).speed(0.01).max_decimals(2));
        let response = match hint {
            Some(hint) => response.on_hover_text(hint),
            None => response,
        };
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

/// What a node's settings choose from: the graph (its variables) and what searches can name.
struct Lists<'a> {
    graph: &'a Graph,
    search: &'a SearchChoices,
}

fn setting_row(
    ui: &mut egui::Ui,
    lists: &Lists<'_>,
    node: &Node,
    setting: &bayleaf_engine::graph::SettingDef,
    row: Rect,
    gesture: u64,
    requests: &mut Requests,
) {
    let t = theme::tokens(ui);
    let label_width = row.width() * 0.36;
    ui.painter().text(pos2(row.left(), row.center().y), egui::Align2::LEFT_CENTER, setting.label, egui::FontId::proportional(13.0), t.text_muted);
    let field = Rect::from_min_max(pos2(row.left() + label_width, row.top() + 2.0), pos2(row.right(), row.bottom() - 2.0));
    let key = format!("graph set {} {} {gesture}", node.id, setting.name);
    let name = format!("{} of node {}", setting.label, node.id);
    // A variable node picks its variable from the script's own, of its type.
    if setting.name == "name"
        && let Some(ty) =
            bayleaf_engine::graph::VariableType::ALL.into_iter().find(|ty| bayleaf_engine::script::visual::variable_kinds(*ty).contains(&node.kind.as_str()))
    {
        variable_picker(ui, lists.graph, node, ty, setting, field, requests);
        return;
    }
    // Get, Try Get and Set Component pick a component whose fields scripts can use; the node's
    // pins follow the choice.
    if setting.name == "component" && COMPONENT_KINDS.contains(&node.kind.as_str()) {
        component_picker(ui, lists.search, node, setting, field, requests);
        return;
    }
    // A search picks a component, a script or a tag, or takes a script dragged from the Project panel.
    if setting.name == "kind" && SEARCH_KINDS.contains(&node.kind.as_str()) {
        kind_picker(ui, lists.search, node, setting, field, requests);
        return;
    }
    match setting.kind {
        // Kept by the node, not shown.
        SettingKind::Names => {}
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
            vec3_editor(ui, field, &mut v, &name, None, requests, node.id, setting.name, key);
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
        SettingKind::Choice(choices) => {
            let default = choices.first().copied().unwrap_or_default();
            let current = value_or(node, setting.name, json!(default)).as_str().unwrap_or(default).to_owned();
            let mut picked = None;
            let response = ui.put(field, |ui: &mut egui::Ui| {
                craft_ui::ComboBox::from_id_salt(("graph choice", node.id, setting.name))
                    .selected_text(current.as_str())
                    .width(field.width())
                    .show_ui(ui, |ui| {
                        for choice in choices {
                            if ui.selectable_label(current == *choice, *choice).clicked() && current != *choice {
                                picked = Some(*choice);
                            }
                        }
                    })
                    .response
            });
            labelled(ui, &response, &format!("{name}: {current}"));
            if let Some(choice) = picked {
                requests.merge("graph.node.set", json!({ "node": node.id, "name": setting.name, "value": choice }), key);
                requests.end_gesture = true;
            }
        }
    }
}

/// The component a Get, Try Get or Set Component node reads or sets: a component whose fields scripts
/// can use (Set Component leaves out those physics reads only when play starts), or a project script
/// that declares properties, chosen from the list or dragged from the Project panel. Choosing a
/// script also keeps its property names on the node, as one undo step.
fn component_picker(ui: &mut egui::Ui, search: &SearchChoices, node: &Node, setting: &bayleaf_engine::graph::SettingDef, field: Rect, requests: &mut Requests) {
    let current = bayleaf_engine::script::visual::component_of(node)
        .map_or_else(|| node.values.get(setting.name).and_then(Value::as_str).unwrap_or_default().to_owned(), |c| c.label.to_owned());
    let setting_it = node.kind == "setComponent";
    let mut chosen = None;
    let mut script_chosen: Option<String> = None;
    let response = ui.put(field, |ui: &mut egui::Ui| {
        craft_ui::ComboBox::from_id_salt(("component", node.id))
            .width(field.width())
            .truncate()
            .selected_text(current.clone())
            .show_ui(ui, |ui| {
                let t = theme::tokens(ui);
                ui.label(egui::RichText::new("Components").small().color(t.text_muted));
                for component in bayleaf_engine::script::visual::COMPONENTS.iter().filter(|c| !setting_it || c.settable()) {
                    if ui.selectable_label(component.label == current, component.label).clicked() && component.label != current {
                        chosen = Some(component.label);
                    }
                }
                let scripts: Vec<&(String, Vec<String>)> = search.scripts.iter().filter(|(_, p)| !p.is_empty()).collect();
                if !scripts.is_empty() {
                    ui.label(egui::RichText::new("Scripts").small().color(t.text_muted));
                }
                for (name, _) in scripts {
                    if ui.selectable_label(*name == current, name).clicked() && *name != current {
                        script_chosen = Some(name.clone());
                    }
                }
            })
            .response
    });
    labelled(ui, &response, &format!("{} of node {}: {current}", setting.label, node.id));
    // A script dragged from the Project panel, when it declares properties.
    let declared = |module: &str| search.scripts.iter().any(|(n, p)| *n == script_name(module) && !p.is_empty());
    if let Some(module) = crate::value_editor::file_drop(ui, &response, |path| (path.ends_with(".bscript") || path.ends_with(".wasm")) && declared(path)) {
        script_chosen = Some(script_name(&module));
    }
    if let Some(choice) = chosen {
        requests.run("graph.node.set", json!({ "node": node.id, "name": setting.name, "value": choice }));
    }
    if let Some(name) = script_chosen
        && let Some((_, properties)) = search.scripts.iter().find(|(n, _)| *n == name)
    {
        let key = format!("component {} {}", node.id, name);
        requests.merge("graph.node.set", json!({ "node": node.id, "name": setting.name, "value": name }), key.clone());
        requests.merge("graph.node.set", json!({ "node": node.id, "name": "fields", "value": properties }), key);
    }
}

/// What a search node looks for: a list of components, the project's scripts and the scene's tags.
/// A script dragged from the Project panel onto it is chosen by its name.
fn kind_picker(ui: &mut egui::Ui, search: &SearchChoices, node: &Node, setting: &bayleaf_engine::graph::SettingDef, field: Rect, requests: &mut Requests) {
    let default = match setting.kind {
        SettingKind::Text(default) => default,
        _ => "",
    };
    let current = node.values.get(setting.name).and_then(Value::as_str).unwrap_or(default).to_owned();
    let mut chosen = None;
    let response = ui.put(field, |ui: &mut egui::Ui| {
        let t = theme::tokens(ui);
        craft_ui::ComboBox::from_id_salt(("search kind", node.id))
            .width(field.width())
            .truncate()
            .selected_text(current.clone())
            .show_ui(ui, |ui| {
                let groups: [(&str, Vec<&str>); 3] = [
                    ("Components", search.components.clone()),
                    ("Scripts", search.scripts.iter().map(|(n, _)| n.as_str()).collect()),
                    ("Tags", search.tags.iter().map(String::as_str).collect()),
                ];
                for (heading, choices) in groups.iter().filter(|(_, c)| !c.is_empty()) {
                    ui.label(egui::RichText::new(*heading).small().color(t.text_muted));
                    for choice in choices {
                        if ui.selectable_label(*choice == current, *choice).clicked() && *choice != current {
                            chosen = Some((*choice).to_owned());
                        }
                    }
                }
            })
            .response
    });
    labelled(ui, &response, &format!("{} of node {}: {current}", setting.label, node.id));
    if let Some(module) = crate::value_editor::file_drop(ui, &response, |path| path.ends_with(".bscript") || path.ends_with(".wasm")) {
        chosen = Some(script_name(&module));
    }
    if let Some(choice) = chosen {
        requests.run("graph.node.set", json!({ "node": node.id, "name": setting.name, "value": choice }));
    }
}

/// The variable a variable node reads or sets: one of the script's variables of its type (and any
/// name other nodes of that type use without a declaration), or a new one.
fn variable_picker(
    ui: &mut egui::Ui,
    graph: &Graph,
    node: &Node,
    ty: bayleaf_engine::graph::VariableType,
    setting: &bayleaf_engine::graph::SettingDef,
    field: Rect,
    requests: &mut Requests,
) {
    let default = match setting.kind {
        bayleaf_engine::graph::SettingKind::Name(default) => default,
        _ => "",
    };
    let kinds = bayleaf_engine::script::visual::variable_kinds(ty);
    let current = node.values.get("name").and_then(Value::as_str).unwrap_or(default).to_owned();
    let mut names: Vec<String> = graph.variables.iter().filter(|v| v.ty == ty).map(|v| v.name.clone()).collect();
    for other in graph.nodes.iter().filter(|n| kinds.contains(&n.kind.as_str())) {
        if let Some(used) = other.values.get("name").and_then(Value::as_str).or(Some(default))
            && !names.iter().any(|n| n == used)
        {
            names.push(used.to_owned());
        }
    }
    let shown = if current.is_empty() { "Choose a variable".to_owned() } else { current.clone() };
    let response = ui.put(field, |ui: &mut egui::Ui| {
        craft_ui::ComboBox::from_id_salt(("variable", node.id))
            .width(field.width())
            .truncate()
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for choice in &names {
                    if ui.selectable_label(*choice == current, choice).clicked() && *choice != current {
                        requests.run("graph.node.set", json!({ "node": node.id, "name": "name", "value": choice }));
                    }
                }
                if !names.is_empty() {
                    ui.separator();
                }
                if ui.button("New Variable…").on_hover_text(format!("Declare a new {} variable for this node", ty.label())).clicked() {
                    let stem = kinds[0];
                    let fresh =
                        (1..).map(|n| format!("{stem}{n}")).find(|n| graph.variable(n).is_none() && !names.contains(n)).unwrap_or_else(|| stem.to_owned());
                    requests.run("graph.variable.add", json!({ "name": fresh, "type": ty }));
                    requests.run("graph.node.set", json!({ "node": node.id, "name": "name", "value": fresh }));
                }
            })
            .response
    });
    labelled(ui, &response, &format!("{} of node {}: {current}", setting.label, node.id));
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

/// Height of the add-node popup's list.
const MENU_HEIGHT: f32 = 320.0;

/// One node of the add-node popup; returns whether it was chosen.
fn node_button(ui: &mut egui::Ui, def: &NodeDef) -> bool {
    ui.add(egui::Button::new(def.label).min_size(vec2(ui.available_width(), theme::LIST_ROW))).on_hover_text(def.description).clicked()
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
            // A fixed size, so the popup does not jump or shrink while the search narrows the list.
            egui::ScrollArea::vertical().max_height(MENU_HEIGHT).min_scrolled_height(MENU_HEIGHT).auto_shrink([false, false]).show(ui, |ui| {
                let mut categories: Vec<&'static str> = Vec::new();
                for def in &matches {
                    if !categories.contains(&def.category) {
                        categories.push(def.category);
                    }
                }
                // While searching, or when only a few nodes fit a dragged pin, every match shows;
                // otherwise each category opens on a click.
                let flat = !needle.is_empty() || (menu.from.is_some() && matches.len() <= 12);
                for category in categories {
                    let in_category = matches.iter().filter(|d| d.category == category);
                    if flat {
                        ui.label(egui::RichText::new(category).small().color(t.text_muted));
                        for def in in_category {
                            if node_button(ui, def) {
                                chosen = Some(def);
                            }
                        }
                    } else {
                        egui::CollapsingHeader::new(category).id_salt(("add node category", category)).default_open(false).show(ui, |ui| {
                            for def in in_category {
                                if node_button(ui, def) {
                                    chosen = Some(def);
                                }
                            }
                        });
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
            view.selected = vec![id];
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

/// Copies the selected nodes and the links between them to the clipboard.
fn copy_nodes(editor: &mut Editor, view: &GraphView) {
    if view.selected.is_empty() {
        editor.status = "Copy: select a node first.".to_owned();
        return;
    }
    if let Some(text) = editor.dispatch("graph.node.copy", &json!({ "nodes": view.selected }), None).and_then(|r| r["text"].as_str().map(str::to_owned)) {
        let count = view.selected.len();
        editor.clipboard = Some(text.clone());
        editor.copy_out = Some(text);
        editor.status = if count == 1 { "Copy: the node is on the clipboard".to_owned() } else { format!("Copy: {count} nodes are on the clipboard") };
    }
}

/// Adds copies of copied nodes and selects them. Returns false when `text` holds no nodes.
fn paste_nodes(editor: &mut Editor, view: &mut GraphView, text: &str) -> bool {
    let is_nodes =
        serde_json::from_str::<Value>(text).ok().is_some_and(|v| v.get("format").and_then(Value::as_str) == Some(bayleaf_engine::graph::FRAGMENT_FORMAT));
    if !is_nodes {
        return false;
    }
    if let Some(result) = editor.dispatch("graph.node.paste", &json!({ "text": text }), None) {
        select_new(editor, view, &result);
    }
    true
}

fn duplicate_nodes(editor: &mut Editor, view: &mut GraphView) {
    if view.selected.is_empty() {
        editor.status = "Duplicate: select a node first.".to_owned();
        return;
    }
    if let Some(result) = editor.dispatch("graph.node.duplicate", &json!({ "nodes": view.selected }), None) {
        select_new(editor, view, &result);
    }
}

/// Selects the nodes a paste or duplicate added, focuses the first, and says what was left out.
fn select_new(editor: &mut Editor, view: &mut GraphView, result: &Value) {
    view.selected = result["nodes"].as_array().into_iter().flatten().filter_map(|v| v.as_u64().and_then(|n| u32::try_from(n).ok())).collect();
    view.focus_node = view.selected.first().copied();
    let skipped: Vec<&str> = result["skipped"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !skipped.is_empty() {
        editor.status = format!("{} left out: a graph has only one", skipped.join(", "));
    }
}

/// The graph editor's share of Ctrl+C, Ctrl+V and Ctrl+D while one of its nodes has focus.
/// `paste` is the clipboard's text; `cut` copies, then removes the nodes as one undo step.
pub(crate) fn clipboard_keys(editor: &mut Editor, copy: bool, cut: bool, paste: Option<String>, duplicate: bool) {
    if editor.session.graph().is_none() {
        return;
    }
    let mut view = std::mem::take(&mut editor.graph_view);
    if copy || cut {
        copy_nodes(editor, &view);
    }
    if cut {
        let key = format!("graph cut {}", editor.gesture);
        for id in std::mem::take(&mut view.selected) {
            editor.dispatch("graph.node.remove", &json!({ "node": id }), Some(&key));
        }
        editor.gesture += 1;
    }
    if let Some(text) = paste
        && !paste_nodes(editor, &mut view, &text)
    {
        editor.status = "Paste: the clipboard holds no nodes.".to_owned();
    }
    if duplicate {
        duplicate_nodes(editor, &mut view);
    }
    editor.graph_view = view;
}
