//! Docked panels: drag a tab to move a panel, drag a divider to resize. The Window menu offers the
//! same without dragging: show or hide panels, pick a ready-made layout, or reset.

use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use serde::{Deserialize, Serialize};

use crate::{Editor, console, game_view, graph_view, hierarchy, inspector, project, project_settings, scene_view, theme};

/// The panels of the editor window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Panel {
    Hierarchy,
    Scene,
    Inspector,
    Console,
    Project,
    /// The graph editor, a tab beside the scene view.
    Graph,
    /// Settings for the whole project, a tab beside the scene view.
    ProjectSettings,
    /// The game as players see it, a tab beside the scene view.
    Game,
}

impl Panel {
    pub const ALL: [Panel; 8] =
        [Panel::Hierarchy, Panel::Scene, Panel::Game, Panel::Graph, Panel::Inspector, Panel::Project, Panel::Console, Panel::ProjectSettings];

    pub fn title(self) -> &'static str {
        match self {
            Panel::Hierarchy => "Hierarchy",
            Panel::Scene => "Scene",
            Panel::Inspector => "Inspector",
            Panel::Console => "Console",
            Panel::Project => "Project",
            Panel::Graph => "Graph",
            Panel::ProjectSettings => "Project Settings",
            Panel::Game => "Game",
        }
    }

    /// The scene view always stays open.
    fn closable(self) -> bool {
        self != Panel::Scene
    }
}

/// Ready-made arrangements, also reachable from the Window menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Hierarchy left, scene centre, inspector right.
    Standard,
    /// Hierarchy and inspector stacked on the right, so the scene gets the most room.
    WideScene,
    /// Inspector left, scene centre, hierarchy right.
    Mirrored,
}

impl Layout {
    pub const ALL: [Layout; 3] = [Layout::Standard, Layout::WideScene, Layout::Mirrored];

    pub fn label(self) -> &'static str {
        match self {
            Layout::Standard => "Standard Layout",
            Layout::WideScene => "Wide Scene Layout",
            Layout::Mirrored => "Mirrored Layout",
        }
    }

    pub fn build(self) -> DockState<Panel> {
        let mut state = DockState::new(vec![Panel::Scene, Panel::Game, Panel::Graph, Panel::ProjectSettings]);
        let surface = state.main_surface_mut();
        match self {
            Layout::Standard => {
                let [top, _] = surface.split_below(NodeIndex::root(), 0.76, vec![Panel::Project, Panel::Console]);
                let [scene, _] = surface.split_right(top, 0.76, vec![Panel::Inspector]);
                surface.split_left(scene, 0.24, vec![Panel::Hierarchy]);
            }
            Layout::WideScene => {
                let [top, _] = surface.split_below(NodeIndex::root(), 0.8, vec![Panel::Project, Panel::Console]);
                let [_, side] = surface.split_right(top, 0.74, vec![Panel::Hierarchy]);
                surface.split_below(side, 0.42, vec![Panel::Inspector]);
            }
            Layout::Mirrored => {
                let [top, _] = surface.split_below(NodeIndex::root(), 0.76, vec![Panel::Project, Panel::Console]);
                let [scene, _] = surface.split_left(top, 0.26, vec![Panel::Inspector]);
                surface.split_right(scene, 0.76, vec![Panel::Hierarchy]);
            }
        }
        state
    }
}

/// Saved form of the layout, versioned so a future change can ignore old data instead of failing.
#[derive(Serialize, Deserialize)]
struct SavedLayout {
    version: u32,
    dock: DockState<Panel>,
}

/// Version 2 added the Project and Console panels, version 3 the Graph panel; older saved layouts
/// fall back to the default.
const LAYOUT_VERSION: u32 = 3;

pub(crate) fn to_text(dock: &DockState<Panel>) -> Option<String> {
    ron::to_string(&SavedLayout { version: LAYOUT_VERSION, dock: dock.clone() }).ok()
}

/// Restores a saved layout. Unknown versions and broken data give `None`; the panel set is
/// completed so a hidden scene view can never get lost.
pub(crate) fn from_text(text: &str) -> Option<DockState<Panel>> {
    let saved: SavedLayout = ron::from_str(text).ok()?;
    if saved.version != LAYOUT_VERSION {
        return None;
    }
    let mut dock = saved.dock;
    if dock.find_tab(&Panel::Scene).is_none() {
        dock.push_to_first_leaf(Panel::Scene);
    }
    Some(dock)
}

pub(crate) fn is_open(dock: &DockState<Panel>, panel: Panel) -> bool {
    dock.find_tab(&panel).is_some()
}

/// Shows or hides a panel. Shown panels open next to the scene view.
pub(crate) fn set_open(dock: &mut DockState<Panel>, panel: Panel, open: bool) {
    match (dock.find_tab(&panel), open) {
        (Some(path), false) if panel.closable() => {
            dock.remove_tab(path);
        }
        (None, true) => match dock.find_tab(&Panel::Scene) {
            Some(scene) => {
                let surface = dock.main_surface_mut();
                let node = scene.node_path().node;
                match panel {
                    // The graph editor shares the scene view's area, as another tab.
                    Panel::Graph | Panel::ProjectSettings | Panel::Game => {
                        surface[node].append_tab(panel);
                    }
                    Panel::Hierarchy => {
                        surface.split_left(node, 0.24, vec![panel]);
                    }
                    Panel::Console | Panel::Project => {
                        surface.split_below(node, 0.76, vec![panel]);
                    }
                    _ => {
                        surface.split_right(node, 0.76, vec![panel]);
                    }
                }
            }
            None => dock.push_to_first_leaf(panel),
        },
        _ => {}
    }
}

struct Panels<'a> {
    editor: &'a mut Editor,
    gpu: Option<&'a egui_wgpu::RenderState>,
    /// A panel fills the dock area.
    maximized: bool,
    /// A panel whose tab asked to be maximised or restored.
    toggle: Option<Panel>,
}

impl TabViewer for Panels<'_> {
    type Tab = Panel;

    fn id(&mut self, tab: &mut Panel) -> egui::Id {
        egui::Id::unique(("bayleaf panel", *tab))
    }

    fn title(&mut self, tab: &mut Panel) -> egui::WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Panel) {
        match tab {
            Panel::Hierarchy => hierarchy::show(self.editor, ui),
            Panel::Scene => scene_view::show(self.editor, ui, self.gpu),
            Panel::Inspector => inspector::show(self.editor, ui),
            Panel::Console => console::show(self.editor, ui),
            Panel::Project => project::show(self.editor, ui),
            Panel::Graph => graph_view::show(self.editor, ui, self.gpu),
            Panel::ProjectSettings => project_settings::show(self.editor, ui),
            Panel::Game => game_view::show(self.editor, ui, self.gpu),
        }
    }

    /// A double-click on a tab maximises its panel, or restores the layout.
    fn on_tab_button(&mut self, tab: &mut Panel, response: &egui::Response) {
        if response.double_clicked() {
            self.toggle = Some(*tab);
        }
        response.ctx.accesskit_node_builder(response.id, |node| {
            node.set_description(if self.maximized { "Double-click to restore the layout" } else { "Double-click to maximise" });
        });
    }

    fn context_menu(&mut self, ui: &mut egui::Ui, tab: &mut Panel, _path: egui_dock::NodePath) {
        let label = if self.maximized { "Restore Layout" } else { "Maximise" };
        if ui.button(label).clicked() {
            self.toggle = Some(*tab);
            ui.close();
        }
    }

    fn is_closeable(&self, tab: &Panel) -> bool {
        // A maximised panel is restored, not closed.
        !self.maximized && tab.closable()
    }

    fn allowed_in_windows(&self, _tab: &mut Panel) -> bool {
        false
    }

    /// Panels scroll themselves, so content never widens a panel.
    fn scroll_bars(&self, _tab: &Panel) -> [bool; 2] {
        [false, false]
    }
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui, gpu: Option<&egui_wgpu::RenderState>) {
    let mut dock = std::mem::replace(&mut editor.dock, DockState::new(Vec::new()));
    let t = theme::tokens(ui);
    let mut style = egui_dock::Style::from_egui(ui.style());
    style.dock_area_padding = Some(egui::Margin::same(4));
    style.main_surface_border_stroke = egui::Stroke::NONE;
    style.separator.width = 4.0;
    style.separator.color_idle = t.backdrop;
    style.separator.color_hovered = t.control_hover;
    style.separator.color_dragged = t.accent;
    style.tab_bar.bg_fill = t.backdrop;
    style.tab_bar.hline_color = t.divider;
    style.tab.tab_body.bg_fill = t.panel;
    style.tab.tab_body.stroke = egui::Stroke::new(1.0, t.divider);
    style.tab.active.bg_fill = t.panel;
    style.tab.active.text_color = t.text;
    style.tab.focused.bg_fill = t.panel;
    style.tab.focused.text_color = t.text;
    style.tab.inactive.bg_fill = t.backdrop;
    style.tab.inactive.text_color = t.text_muted;
    style.tab.hovered.bg_fill = t.surface_hover;
    style.tab.hovered.text_color = t.text;
    style.overlay.selection_color = t.accent.gamma_multiply(0.35);

    // A maximised panel is shown alone; the layout itself stays as it was.
    let maximized = editor.maximized.filter(|panel| is_open(&dock, *panel));
    let mut alone = maximized.map(|panel| DockState::new(vec![panel]));
    let mut panels = Panels { editor, gpu, maximized: maximized.is_some(), toggle: None };
    egui::Frame::new().fill(t.backdrop).show(ui, |ui| {
        DockArea::new(alone.as_mut().unwrap_or(&mut dock))
            .id(egui::Id::unique(if maximized.is_some() { "bayleaf dock maximised" } else { "bayleaf dock" }))
            .style(style)
            .show_add_buttons(false)
            .show_leaf_collapse_buttons(false)
            .show_leaf_close_all_buttons(false)
            .show_inside(ui, &mut panels);
    });
    let toggle = panels.toggle;
    editor.maximized = match toggle {
        Some(panel) if maximized.is_none() => Some(panel),
        Some(_) => None,
        None => maximized,
    };
    editor.dock = dock;
    name_dividers(ui.ctx());
}

/// egui_dock's dividers can be moved with the keyboard and by assistive technology, but carry no
/// name. Give each one a name, so screen readers announce what it is.
fn name_dividers(ctx: &egui::Context) {
    let ids: Vec<egui::Id> = ctx.viewport(|viewport| viewport.this_pass.widgets.layers().flat_map(|(_, rects)| rects.iter().map(|rect| rect.id)).collect());
    for id in ids {
        if !ctx.has_accesskit_node(id) {
            continue;
        }
        ctx.accesskit_node_builder(id, |node| {
            if node.role() == egui::accesskit::Role::Splitter && node.label().is_none() {
                node.set_label("Panel divider");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{Layout, Panel, from_text, is_open, set_open, to_text};

    #[test]
    fn every_layout_contains_every_panel_once() {
        for layout in Layout::ALL {
            let dock = layout.build();
            for panel in Panel::ALL {
                assert_eq!(dock.iter_all_tabs().filter(|(_, p)| **p == panel).count(), 1, "{layout:?} / {panel:?}");
            }
        }
    }

    #[test]
    fn hiding_and_showing_a_panel_round_trips_but_the_scene_stays() {
        let mut dock = Layout::Standard.build();
        set_open(&mut dock, Panel::Inspector, false);
        assert!(!is_open(&dock, Panel::Inspector));
        set_open(&mut dock, Panel::Inspector, true);
        assert!(is_open(&dock, Panel::Inspector));
        set_open(&mut dock, Panel::Scene, false);
        assert!(is_open(&dock, Panel::Scene));
    }

    #[test]
    fn saved_layouts_restore_and_broken_ones_are_ignored() {
        let dock = Layout::WideScene.build();
        let restored = from_text(&to_text(&dock).unwrap()).unwrap();
        assert_eq!(restored.iter_all_tabs().count(), dock.iter_all_tabs().count());
        assert!(from_text("{not json").is_none());
        assert!(from_text("(version: 99, dock: ())").is_none());
    }
}
