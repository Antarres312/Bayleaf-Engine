//! Inspector panel: the selected object's name and its components as cards, edited in place.
//! Each script on the object is its own card, named after its module, with the properties the
//! script declares. Its width comes from the dock; nothing inside sets it.

use crate::i18n::tl;
use bayleaf_engine::scene::components::{DeclaredProperty, PropertyValue, SceneComponent, ScriptRef, Scripts};
use bayleaf_engine::scene::inspector::{InspectorItem, PropertyLayout};
use serde_json::{Value, json};

use crate::theme::{self, ROW_HEIGHT};
use crate::{Editor, project, value_editor};

enum Pending {
    Set {
        component: &'static str,
        value: Value,
    },
    Add(&'static str),
    Remove(&'static str),
    AddScript(String),
    RemoveScript(usize),
    ScriptProperty {
        index: usize,
        name: String,
        value: Option<PropertyValue>,
    },
    /// A prefab command (`scene.prefab.apply` or `scene.prefab.revert`) for this object.
    Prefab(&'static str),
    /// Edit the prefab this object is a copy of, on its own.
    OpenPrefab(String),
    /// A script's button: send this object the message while the game plays.
    Message(String),
    /// Open a script to change it: a visual script in its graph tab, a Rust script's source in the
    /// code editor.
    EditScript(std::path::PathBuf),
}

/// One script card's data, gathered before drawing.
struct ScriptCard {
    script: ScriptRef,
    /// Declared properties with their defaults, or why they could not be read.
    declared: Result<Vec<DeclaredProperty>, String>,
    /// How the script lays its properties out (empty: one row each).
    layout: Vec<InspectorItem>,
}

pub(crate) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    if editor.selected.is_some() {
        editor.inspected_file = None;
    } else if editor.inspected_file.is_some() {
        crate::file_inspector::show(editor, ui);
        return;
    }
    let t = theme::tokens(ui);
    let Some(id) = editor.selected else {
        if editor.scene_selected {
            // An unsaved scene: its settings, and a word on its file.
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(tl("Untitled scene")).heading());
                ui.label(egui::RichText::new(tl("Save the scene to see what its file holds here.")).small().color(t.text_muted));
                ui.add_space(theme::SECTION_GAP);
                crate::scene_settings::show(editor, ui);
            });
        } else {
            empty_state(ui, "Nothing selected", "Select an object in the hierarchy or click it in the scene view.");
        }
        return;
    };
    let Ok(components) = editor.session.scene().components(id) else {
        empty_state(ui, "Object not found", "The selected object no longer exists.");
        return;
    };
    let selected_count = editor.selection().len();
    let kinds = editor.session.scene().registry().kinds().to_vec();
    let script_dir = editor.session.script_dir();
    let scripts: Vec<ScriptRef> = editor.session.scene().get::<Scripts>(id).map(|s| s.0.clone()).unwrap_or_default();
    // The scene's objects by name, for object properties.
    let objects: Vec<(bayleaf_core::StableId, String)> = {
        let scene = editor.session.scene();
        scene.world_matrices().into_iter().map(|(o, _)| (o, scene.name(o).unwrap_or_default().to_owned())).filter(|(_, n)| !n.is_empty()).collect()
    };
    let cards: Vec<ScriptCard> = scripts
        .into_iter()
        .map(|script| {
            let path = script_dir.join(&script.module);
            ScriptCard { declared: editor.script_info.properties(&path), layout: editor.script_info.layout(&path), script }
        })
        .collect();
    let playing = editor.session.is_playing();
    // Every script of the project, with the menu path it gives itself (read once per file change).
    let script_menus: Vec<(String, Option<String>)> =
        editor.project.script_files().into_iter().map(|module| (module.clone(), editor.script_info.menu(&script_dir.join(&module)))).collect();
    let file_fields = file_fields(editor, id, &script_dir);
    let mut open_material: Option<std::path::PathBuf> = None;
    let mut pending = None;
    let mut gesture_ended = false;

    let panel = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        // Name: a large field at the top instead of a card.
        if let Some((_, Value::String(name))) = components.iter().find(|(n, _)| *n == "name") {
            let mut edited = name.clone();
            let label = ui.label(egui::RichText::new(tl("Name")).small().color(t.text_muted));
            let response = ui
                .add_sized(
                    [ui.available_width(), ROW_HEIGHT + 4.0],
                    egui::TextEdit::singleline(&mut edited).font(egui::TextStyle::Heading).align(egui::Align2::LEFT_CENTER),
                )
                .labelled_by(label.id);
            gesture_ended |= response.lost_focus();
            if response.changed() {
                pending = Some(Pending::Set { component: "name", value: Value::String(edited) });
            }
            ui.label(egui::RichText::new(crate::i18n::tf("ID {id}", &[("id", &id.to_hex()[..8])])).small().color(t.text_muted)).on_hover_text(id.to_hex());
            if selected_count > 1 {
                // Say plainly what applies to all and what only to the object shown.
                ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tf("{selected_count} objects selected. Delete, Duplicate, Copy and the handles in the scene view act on all of them; the fields below change only this one.", &[("selected_count", &selected_count.to_string())])).small().color(t.text)).wrap());
            }
            ui.add_space(8.0);
        }

        // A placed prefab says where it came from, and offers Apply, Revert and Unlink.
        if let Some((_, link)) = components.iter().find(|(n, _)| *n == "prefab") {
            let source = link["source"].as_str().unwrap_or_default().to_owned();
            card(ui, t, "prefab card", "Prefab", Some(&source), CardMenu { removable: false, script: None }, |ui| {
                ui.add(egui::Label::new(egui::RichText::new(&source).color(t.text_muted)).truncate());
                // What this copy changed, which stays when the prefab file changes.
                let overrides: Vec<String> = link["overrides"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .filter_map(|o| o.as_str()?.split_once('/').map(|(_, c)| c.to_owned()))
                            .map(|c| kinds.iter().find(|k| k.name == c).map_or(c.clone(), |k| k.label.to_owned()))
                            .collect()
                    })
                    .unwrap_or_default();
                let summary = if overrides.is_empty() {
                    "Same as the prefab file".to_owned()
                } else {
                    let mut labels = overrides.clone();
                    labels.sort();
                    labels.dedup();
                    crate::i18n::tf("Own changes, kept when the file changes: {changes}", &[("changes", &(labels.join(", ")).to_string())])
                };
                ui.add(egui::Label::new(egui::RichText::new(summary).small().color(t.text_muted)).wrap());
                ui.horizontal(|ui| {
                    if ui.button(tl("Open")).on_hover_text(tl("Edit the prefab on its own; every copy follows when you go back")).clicked() {
                        pending = Some(Pending::OpenPrefab(source.clone()));
                    }
                    if ui.button(tl("Apply")).on_hover_text(tl("Write this object and its children to the prefab file, and update its other copies in the scene")).clicked() {
                        pending = Some(Pending::Prefab("scene.prefab.apply"));
                    }
                    if ui.button(tl("Revert")).on_hover_text(tl("Throw away this copy's changes and read the prefab file again")).clicked() {
                        pending = Some(Pending::Prefab("scene.prefab.revert"));
                    }
                    if ui.button(tl("Unlink")).on_hover_text(tl("Keep the objects but stop treating them as a copy of the prefab")).clicked() {
                        pending = Some(Pending::Remove("prefab"));
                    }
                });
            });
        }

        for (name, value) in components.iter().filter(|(n, _)| *n != "name" && *n != Scripts::NAME && *n != "prefab") {
            let Some(kind) = kinds.iter().find(|k| k.name == *name) else { continue };
            let menu = CardMenu { removable: kind.removable, script: None };
            card(ui, t, ("component card", *name), kind.label, None, menu, |ui| {
                let mut edited = value.clone();
                // File fields are picked from the project's files below, not typed.
                let fields: Vec<&FileField> = file_fields.iter().filter(|f| f.component == *name).collect();
                let current: Vec<String> = fields
                    .iter()
                    .map(|f| edited.as_object_mut().and_then(|o| o.remove(f.field)).and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default())
                    .collect();
                let outcome = value_editor::edit_fields(ui, &mut edited, kind.name, &|field| kind.options(field));
                gesture_ended |= outcome.ended;
                if outcome.changed {
                    if let Some(object) = edited.as_object_mut() {
                        for (field, path) in fields.iter().zip(&current).filter(|(_, p)| !p.is_empty()) {
                            object.insert(field.field.to_owned(), Value::String(path.clone()));
                        }
                    }
                    pending = Some(Pending::Set { component: kind.name, value: edited.clone() });
                }
                for (field, path) in fields.iter().zip(&current) {
                    if let Some(chosen) = file_row(ui, t, field, path, &script_dir, &mut open_material) {
                        let mut value = value.clone();
                        if let Some(object) = value.as_object_mut() {
                            // An empty path is "none" (and fields such as Player Prefab must be present).
                            object.insert(field.field.to_owned(), Value::String(chosen));
                        }
                        pending = Some(Pending::Set { component: kind.name, value });
                    }
                }
            })
            .removed
            .then(|| pending = Some(Pending::Remove(kind.name)));
        }

        for (index, card_data) in cards.iter().enumerate() {
            let title = card_data.script.display_name();
            let path = script_dir.join(&card_data.script.module);
            let menu = CardMenu { removable: true, script: Some(editable_script(&path)) };
            let chosen = card(ui, t, ("script card", index), &title, Some(&card_data.script.module), menu, |ui| match &card_data.declared {
                Err(message) => {
                    ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tf("Warning: {message}", &[("message", &message.to_string())])).color(t.warning)).wrap());
                }
                Ok(declared) => {
                    let card = Card { index, script: &card_data.script, declared, layout: &card_data.layout, objects: &objects, playing };
                    let outcome = script_properties(ui, &card, &mut pending);
                    gesture_ended |= outcome.ended;
                }
            });
            if chosen.removed {
                pending = Some(Pending::RemoveScript(index));
            }
            if chosen.edit {
                pending = Some(Pending::EditScript(path));
            }
        }

        let missing: Vec<_> = kinds.iter().filter(|k| k.listed && !components.iter().any(|(n, _)| n == &k.name)).collect();
        ui.add_space(4.0);
        let width = ui.available_width();
        // Built-in components and scripts, in folders by their menu paths (`Light/Point Light`).
        let mut entries: Vec<MenuEntry> = missing
            .iter()
            .map(|kind| MenuEntry {
                path: if kind.menu.is_empty() { vec![tl(kind.label).to_owned()] } else { kind.menu.split('/').map(|part| tl(part).to_owned()).collect() },
                hint: None,
                add: Pending::Add(kind.name),
            })
            .collect();
        for (module, menu) in &script_menus {
            let name = ScriptRef { module: module.clone(), properties: Default::default() }.display_name();
            let path = match menu {
                Some(menu) => menu.split('/').map(str::to_owned).collect(),
                None => vec!["Scripts".to_owned(), name],
            };
            entries.push(MenuEntry { path, hint: Some(module.clone()), add: Pending::AddScript(module.clone()) });
        }
        let mut chosen = None;
        egui::containers::menu::MenuButton::from_button(egui::Button::new(tl("Add Component")).min_size(egui::vec2(width, ROW_HEIGHT + 2.0))).ui(ui, |ui| {
            ui.set_min_width(width.min(260.0));
            let all: Vec<usize> = (0..entries.len()).collect();
            menu_tree(ui, &entries, &all, 0, &mut chosen);
            if script_menus.is_empty() {
                ui.separator();
                ui.label(egui::RichText::new(tl("No scripts in the project folder")).color(t.text_muted));
            }
        });
        if let Some(index) = chosen {
            pending = Some(entries.swap_remove(index).add);
        }
        ui.label(egui::RichText::new(tl("Tip: drag a script from the Project panel onto this panel.")).small().color(t.text_muted));
    });

    // A script dragged from the Project panel and dropped anywhere on the inspector is added.
    let area = panel.inner_rect;
    if let Some(asset) = egui::DragAndDrop::payload::<project::DraggedAsset>(ui.ctx())
        && asset.is_script()
        && ui.ctx().pointer_interact_pos().is_some_and(|p| area.contains(p))
    {
        ui.painter().rect_stroke(area, theme::RADIUS, egui::Stroke::new(2.0, t.focus), egui::StrokeKind::Inside);
        if ui.input(|i| i.pointer.any_released()) {
            egui::DragAndDrop::clear_payload(ui.ctx());
            pending = Some(Pending::AddScript(asset.relative.clone()));
        }
    }

    if let Some(path) = open_material {
        // A graph opens in the graph editor; an Input Map in the Inspector's editor.
        if path.extension().is_some_and(|e| e == bayleaf_engine::input_cmds::INPUT_EXTENSION) {
            editor.inspect_file(path);
        } else {
            editor.open_graph(&path);
        }
    }
    apply(editor, id, pending);
    if gesture_ended {
        editor.gesture += 1;
    }
}

/// One choice of Add Component: where it is listed (folders, then its name) and what it adds.
struct MenuEntry {
    path: Vec<String>,
    /// Shown on hover: a script's module.
    hint: Option<String>,
    add: Pending,
}

/// The entries `indices` at folder depth `depth`: their folders first, each a submenu, then the
/// entries that end here. Sets `chosen` to the clicked entry.
fn menu_tree(ui: &mut egui::Ui, entries: &[MenuEntry], indices: &[usize], depth: usize, chosen: &mut Option<usize>) {
    let mut folders: Vec<&str> = Vec::new();
    for &i in indices {
        if let Some(folder) = entries[i].path.get(depth).filter(|_| entries[i].path.len() > depth + 1)
            && !folders.contains(&folder.as_str())
        {
            folders.push(folder);
        }
    }
    folders.sort_unstable();
    for folder in &folders {
        let inside: Vec<usize> = indices.iter().copied().filter(|&i| entries[i].path.len() > depth + 1 && entries[i].path[depth] == *folder).collect();
        ui.menu_button(*folder, |ui| menu_tree(ui, entries, &inside, depth + 1, chosen));
    }
    let leaves: Vec<usize> = indices.iter().copied().filter(|&i| entries[i].path.len() == depth + 1).collect();
    if !folders.is_empty() && !leaves.is_empty() {
        ui.separator();
    }
    for i in leaves {
        let mut button = ui.button(&entries[i].path[depth]);
        if let Some(hint) = &entries[i].hint {
            button = button.on_hover_text(hint);
        }
        if button.clicked() {
            *chosen = Some(i);
            ui.close();
        }
    }
}

/// What a card's menu offers.
#[derive(Clone, Copy)]
struct CardMenu {
    removable: bool,
    /// A script card: whether there is something to edit (a visual script, or a Rust script with
    /// its source in the project).
    script: Option<bool>,
}

/// What was chosen on a card.
#[derive(Default)]
struct CardChoice {
    removed: bool,
    edit: bool,
}

/// The card's menu: Edit Script on a script's card, and Remove.
fn card_menu(ui: &mut egui::Ui, menu: CardMenu, chosen: &mut CardChoice) {
    if let Some(editable) = menu.script {
        let edit = ui.add_enabled(editable, egui::Button::new(tl("Edit Script")));
        let edit = edit.on_disabled_hover_text(tl("This script has no Rust source or graph in the project to edit."));
        if edit.clicked() {
            chosen.edit = true;
            ui.close();
        }
    }
    if menu.removable && ui.button(tl("Remove")).clicked() {
        chosen.removed = true;
        ui.close();
    }
}

/// Whether a script's module can be opened to edit: a visual script, or a Rust script's built
/// module with its crate's source beside it.
fn editable_script(module: &std::path::Path) -> bool {
    module.extension().is_some_and(|e| e == bayleaf_engine::script::VISUAL_EXTENSION) || project::rust_source(module).is_some()
}

/// Opens a script to change it: a visual script in its graph tab, a Rust script's source in the
/// code editor the system uses for it.
fn edit_script(editor: &mut Editor, module: &std::path::Path) {
    if module.extension().is_some_and(|e| e == bayleaf_engine::script::VISUAL_EXTENSION) {
        editor.open_graph(module);
        return;
    }
    match project::rust_source(module) {
        Some(source) => {
            if let Err(error) = project::system_open(&source, false) {
                editor.status = crate::i18n::tf("Could not open {path}: {error}", &[("path", &source.display().to_string()), ("error", &error.to_string())]);
            }
        }
        None => editor.status = crate::i18n::tf("{path} has no Rust source in the project to edit.", &[("path", &module.display().to_string())]),
    }
}

/// A collapsible card with a title, an optional muted subtitle, and a menu (its title right-clicked,
/// or the … button) with Edit Script on a script's card and Remove. Returns what
/// was chosen.
fn card(
    ui: &mut egui::Ui,
    t: &theme::Tokens,
    salt: impl std::hash::Hash + std::fmt::Debug,
    title: &str,
    subtitle: Option<&str>,
    menu: CardMenu,
    body: impl FnOnce(&mut egui::Ui),
) -> CardChoice {
    let mut chosen = CardChoice::default();
    theme::section(t).show(ui, |ui| {
        ui.set_width(ui.available_width());
        // A compact title row, and rows packed a little closer than elsewhere.
        ui.spacing_mut().interact_size.y = theme::SECTION_TITLE;
        ui.spacing_mut().item_spacing.y = 4.0;
        let state_id = ui.make_persistent_id(salt);
        // A card with nothing to choose (Transform) has no menu.
        let has_menu = menu.removable || menu.script.is_some();
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), state_id, true)
            .show_header(ui, |ui| {
                // Right-clicking the title, or the free space beside it, opens the card's menu.
                let label =
                    ui.add(egui::Label::new(egui::RichText::new(tl(title)).strong()).sense(if has_menu { egui::Sense::click() } else { egui::Sense::hover() }));
                if has_menu {
                    label.context_menu(|ui| card_menu(ui, menu, &mut chosen));
                }
                if let Some(subtitle) = subtitle {
                    label.on_hover_text(subtitle);
                }
                if !has_menu {
                    return;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // The menu from the keyboard too: a button named after the card.
                    let more = egui::containers::menu::MenuButton::new("…").ui(ui, |ui| card_menu(ui, menu, &mut chosen)).0;
                    let named = crate::i18n::tf("More for {title}", &[("title", title)]);
                    more.ctx.accesskit_node_builder(more.id, |node| node.set_label(named.clone()));
                    more.on_hover_text(named);
                    // The free space between the title and the buttons opens the menu as well.
                    let rest = ui.available_rect_before_wrap();
                    ui.interact(rest, state_id.with("header space"), egui::Sense::click()).context_menu(|ui| card_menu(ui, menu, &mut chosen));
                });
            })
            .body(|ui| {
                ui.spacing_mut().interact_size.y = ROW_HEIGHT;
                body(ui);
                ui.add_space(2.0);
            });
    });
    ui.add_space(theme::SECTION_GAP);
    chosen
}

/// What one script card draws from.
struct Card<'a> {
    index: usize,
    script: &'a ScriptRef,
    declared: &'a [DeclaredProperty],
    layout: &'a [InspectorItem],
    objects: &'a [(bayleaf_core::StableId, String)],
    /// Buttons send their messages only while the game plays.
    playing: bool,
}

/// A property's value as the card shows it: the one set on the object when it fits, else the default.
fn current_value(card: &Card<'_>, name: &str) -> Option<PropertyValue> {
    let declared = card.declared.iter().find(|p| p.name == name);
    match declared {
        Some(p) => Some(card.script.properties.get(name).filter(|v| p.accepts(v)).unwrap_or(&p.default).clone()),
        None => card.script.properties.get(name).cloned(),
    }
}

/// One property's row, as its layout (if any) says.
fn property_row(ui: &mut egui::Ui, card: &Card<'_>, name: &str, layout: Option<&PropertyLayout>, pending: &mut Option<Pending>) -> value_editor::Outcome {
    let declared = card.declared.iter().find(|p| p.name == name);
    let set = card.script.properties.get(name);
    let (ty, mut value, changed) = match declared {
        Some(p) => {
            let set = set.filter(|v| p.accepts(v));
            (p.ty, set.unwrap_or(&p.default).clone(), set.is_some_and(|v| *v != p.default))
        }
        // A value the script no longer declares: shown by its shape.
        None => match set {
            Some(v) => (v.shape_type(), v.clone(), true),
            None => return value_editor::Outcome::default(),
        },
    };
    let row = value_editor::PropertyRow { name, ty, changed, objects: card.objects, layout };
    let salt = ("script property", card.index, name);
    // A list or map edits item by item; one value in one row.
    let (outcome, reset) = if declared.is_some_and(|p| p.shape != bayleaf_engine::scene::components::PropertyShape::Single) {
        value_editor::shaped_property(ui, &row, &mut value, salt)
    } else {
        value_editor::named_property(ui, &row, &mut value, salt)
    };
    if outcome.changed {
        *pending = Some(Pending::ScriptProperty { index: card.index, name: name.to_owned(), value: Some(value) });
    }
    if reset {
        *pending = Some(Pending::ScriptProperty { index: card.index, name: name.to_owned(), value: None });
    }
    outcome
}

/// Draws a script's layout items: properties with their options, groups, buttons and notes, each
/// only while its condition holds.
fn layout_items(ui: &mut egui::Ui, card: &Card<'_>, items: &[InspectorItem], pending: &mut Option<Pending>, outcome: &mut value_editor::Outcome) {
    let t = theme::tokens(ui);
    for (i, item) in items.iter().enumerate() {
        if let Some(condition) = item.show_if()
            && !current_value(card, &condition.property).is_some_and(|v| condition.holds(&v))
        {
            continue;
        }
        match item {
            InspectorItem::Property(layout) => {
                let row = property_row(ui, card, &layout.name, Some(layout), pending);
                outcome.ended |= row.ended;
            }
            InspectorItem::Group { title, closed, items, .. } => {
                let id = ui.make_persistent_id(("script group", card.index, i, title.as_str()));
                egui::CollapsingHeader::new(egui::RichText::new(title).strong()).id_salt(id).default_open(!closed).show(ui, |ui| {
                    layout_items(ui, card, items, pending, outcome);
                });
            }
            InspectorItem::Button { label, message, hint, .. } => {
                let button = ui.add_enabled(card.playing, egui::Button::new(label.as_str()).min_size(egui::vec2(0.0, ROW_HEIGHT)));
                let button = if hint.is_empty() { button } else { button.on_hover_text(hint.as_str()) };
                if button.on_disabled_hover_text(tl("Sends its message while the game plays")).clicked() {
                    *pending = Some(Pending::Message(message.clone()));
                }
            }
            InspectorItem::Note { text, warning, .. } => {
                // A warning says so in words, not only in colour.
                let (shown, color) =
                    if *warning { (crate::i18n::tf("Warning: {message}", &[("message", text)]), t.warning) } else { (text.clone(), t.text_muted) };
                ui.add(egui::Label::new(egui::RichText::new(shown).color(color)).wrap());
            }
        }
    }
}

/// The names of the properties a layout places, groups' included.
fn placed(items: &[InspectorItem], out: &mut Vec<String>) {
    for item in items {
        match item {
            InspectorItem::Property(layout) => out.push(layout.name.clone()),
            InspectorItem::Group { items, .. } => placed(items, out),
            _ => {}
        }
    }
}

/// One script's properties: as its layout places them, then the rest one row each, then any saved
/// values it no longer declares.
fn script_properties(ui: &mut egui::Ui, card: &Card<'_>, pending: &mut Option<Pending>) -> value_editor::Outcome {
    let t = theme::tokens(ui);
    let mut outcome = value_editor::Outcome::default();
    let (declared, script) = (card.declared, card.script);
    if declared.is_empty() && script.properties.is_empty() && card.layout.is_empty() {
        ui.label(egui::RichText::new(tl("This script declares no properties.")).color(t.text_muted));
        return outcome;
    }
    layout_items(ui, card, card.layout, pending, &mut outcome);
    let mut shown = Vec::new();
    placed(card.layout, &mut shown);
    let rest: Vec<String> = declared
        .iter()
        .map(|p| p.name.clone())
        .chain(script.properties.keys().filter(|n| !declared.iter().any(|p| p.name == **n)).cloned())
        .filter(|n| !shown.contains(n))
        .collect();
    for name in rest {
        let row = property_row(ui, card, &name, None, pending);
        outcome.ended |= row.ended;
    }
    outcome
}

fn apply(editor: &mut Editor, id: bayleaf_core::StableId, pending: Option<Pending>) {
    match pending {
        Some(Pending::Prefab(command)) => {
            editor.dispatch(command, &json!({ "id": id }), None);
        }
        Some(Pending::Set { component, value }) => {
            let key = format!("set:{id}:{component}:{}", editor.gesture);
            editor.dispatch("scene.component.set", &json!({ "id": id, "component": component, "value": value }), Some(&key));
        }
        Some(Pending::Add(component)) => {
            editor.dispatch("scene.component.add", &json!({ "id": id, "component": component }), None);
        }
        Some(Pending::Remove(component)) => {
            editor.dispatch("scene.component.remove", &json!({ "id": id, "component": component }), None);
        }
        Some(Pending::AddScript(module)) => {
            editor.dispatch("scene.script.add", &json!({ "id": id, "module": module }), None);
        }
        Some(Pending::RemoveScript(index)) => {
            editor.dispatch("scene.script.remove", &json!({ "id": id, "index": index }), None);
        }
        Some(Pending::OpenPrefab(source)) => {
            editor.dispatch("prefab.open", &json!({ "path": source }), None);
        }
        Some(Pending::Message(name)) => {
            editor.dispatch("play.message", &json!({ "id": id, "name": name }), None);
        }
        Some(Pending::EditScript(path)) => edit_script(editor, &path),
        Some(Pending::ScriptProperty { index, name, value }) => {
            let key = format!("script property:{id}:{index}:{name}:{}", editor.gesture);
            editor.dispatch("scene.script.setProperty", &json!({ "id": id, "index": index, "name": name, "value": value }), Some(&key));
        }
        None => {}
    }
}

/// Centred title and hint for a panel with nothing to show.
pub(crate) fn empty_state(ui: &mut egui::Ui, title: &str, hint: &str) {
    let t = theme::tokens(ui);
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.3).min(120.0));
        ui.label(egui::RichText::new(tl(title)).strong());
        ui.add(egui::Label::new(egui::RichText::new(tl(hint)).color(t.text_muted)).wrap());
    });
}

/// A field that holds a file path, picked from the project's files instead of typed.
struct FileField {
    component: &'static str,
    field: &'static str,
    label: &'static str,
    /// What an empty choice means, e.g. "None (colour above)".
    none: &'static str,
    /// Said when the project has no files of the kind.
    no_files: &'static str,
    files: Vec<String>,
    /// Why the chosen file does not load, if it does not.
    problem: Option<String>,
    /// The file opens in the graph editor (materials).
    editable: bool,
}

/// One file field: a choice of the project's files, an Edit button for graphs, and a warning
/// when the chosen file does not load. Returns a new choice ("" for none).
fn file_row(
    ui: &mut egui::Ui,
    t: &theme::Tokens,
    field: &FileField,
    current: &str,
    base: &std::path::Path,
    open: &mut Option<std::path::PathBuf>,
) -> Option<String> {
    let files = &field.files;
    let mut chosen = None;
    crate::value_editor::row(ui, t, tl(field.label), |ui, label, width| {
        // Edit sits at the right edge and the list takes what is left of the value's own rect; a
        // long file name is cut short with an ellipsis (hover shows it all), never over the label.
        ui.allocate_ui_with_layout(egui::vec2(width, ROW_HEIGHT), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if field.editable && !current.is_empty() {
                let edit = ui.add(egui::Button::new(tl("Edit")).min_size(egui::vec2(0.0, ROW_HEIGHT)));
                ui.ctx().accesskit_node_builder(edit.id, |node| {
                    node.set_label(crate::i18n::tf("Edit {field}", &[("field", &tl(field.label).to_lowercase().to_string())]))
                });
                if edit.on_hover_text(tl("Open it in the graph editor")).clicked() {
                    *open = Some(base.join(current));
                }
            }
            let shown = if current.is_empty() { field.none.to_owned() } else { current.to_owned() };
            let list = craft_ui::ComboBox::from_id_salt((field.component, field.field))
                .width((ui.available_width() - 1.0).max(16.0))
                .truncate()
                .selected_text(egui::RichText::new(shown).color(t.text))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_empty(), tl(field.none)).clicked() {
                        chosen = Some(String::new());
                    }
                    for file in files {
                        if ui.selectable_label(current == file, file).clicked() {
                            chosen = Some(file.clone());
                        }
                    }
                    if files.is_empty() {
                        ui.label(egui::RichText::new(tl(field.no_files)).color(t.text_muted));
                    }
                })
                .response
                .labelled_by(label);
            // A file of the right kind can also be dragged here from the Project panel.
            if let Some(dropped) = crate::value_editor::file_drop(ui, &list, |path| files.iter().any(|f| f == path)) {
                chosen = Some(dropped);
            }
        });
    });
    if let Some(problem) = &field.problem {
        ui.add(egui::Label::new(egui::RichText::new(crate::i18n::tf("Warning: {problem}", &[("problem", &problem.to_string())])).color(t.warning)).wrap());
    }
    chosen.filter(|c| c != current)
}

/// The file fields of `id`'s components, with the project's files of each kind and whether the
/// chosen file loads.
fn file_fields(editor: &mut Editor, id: bayleaf_core::StableId, base: &std::path::Path) -> Vec<FileField> {
    use bayleaf_engine::scene::components::{AudioSource, GameRules, InputMap, Mesh, Sprite, StreamedArea};
    let scene = editor.session.scene();
    let mesh = scene.get::<Mesh>(id).cloned();
    let sprite = scene.get::<Sprite>(id).cloned();
    let input_map = scene.get::<InputMap>(id).cloned();
    let rules = scene.get::<GameRules>(id).is_some();
    let audio = scene.get::<AudioSource>(id).cloned();
    let camera = scene.get::<bayleaf_engine::scene::components::Camera>(id).cloned();
    let emitter = scene.get::<bayleaf_engine::scene::components::ParticleEmitter>(id).cloned();
    let interface = scene.get::<bayleaf_engine::scene::components::Interface>(id).cloned();
    let streamed = scene.get::<StreamedArea>(id).cloned();
    let mut fields = Vec::new();
    if let Some(interface) = interface {
        let missing = !interface.layout.is_empty() && !base.join(&interface.layout).is_file();
        fields.push(FileField {
            component: "interface",
            field: "layout",
            label: "Layout",
            none: "None (shows nothing)",
            no_files: "No .bui files in the project folder",
            files: editor.project.interface_files(),
            problem: missing.then(|| crate::i18n::tf("{file} is not there", &[("file", &interface.layout.to_string())])),
            editable: false,
        });
    }
    if let Some(emitter) = emitter {
        let missing = |path: &str| (!path.is_empty() && !base.join(path).is_file()).then(|| crate::i18n::tf("{file} is not there", &[("file", path)]));
        fields.push(FileField {
            component: "particleEmitter",
            field: "image",
            label: "Image",
            none: "None (a soft dot)",
            no_files: "No PNG or JPEG images in the project folder",
            files: editor.project.image_files(),
            problem: missing(&emitter.image),
            editable: false,
        });
        fields.push(FileField {
            component: "particleEmitter",
            field: "material",
            label: "Material",
            none: "None (colour and image)",
            no_files: "No .bmaterial files in the project folder",
            files: editor.project.material_files(),
            problem: missing(&emitter.material),
            editable: true,
        });
    }
    if let Some(camera) = camera {
        let missing = !camera.post_processing.is_empty() && !base.join(&camera.post_processing).is_file();
        fields.push(FileField {
            component: "camera",
            field: "postProcessing",
            label: "Post-processing",
            none: "The scene's",
            no_files: "No .bpost files in the project folder",
            files: editor.project.post_files(),
            problem: missing.then(|| crate::i18n::tf("{file} is not there", &[("file", &camera.post_processing.to_string())])),
            editable: false,
        });
    }
    if let Some(source) = audio {
        let missing = !source.clip.is_empty() && !base.join(&source.clip).is_file();
        fields.push(FileField {
            component: "audioSource",
            field: "clip",
            label: "Clip",
            none: "None (silent)",
            no_files: "No sound files (WAV, Ogg Vorbis, MP3…) in the project folder",
            files: editor.project.sound_files(),
            problem: missing.then(|| crate::i18n::tf("{file} is not there", &[("file", &source.clip.to_string())])),
            editable: false,
        });
    }
    if let Some(area) = streamed {
        let missing = !area.scene.is_empty() && !base.join(&area.scene).is_file();
        fields.push(FileField {
            component: "streamedArea",
            field: "scene",
            label: "Scene",
            none: "None (stays empty)",
            no_files: "No .bscene files in the project folder",
            files: editor.project.scene_files(),
            problem: missing.then(|| crate::i18n::tf("{file} is not there", &[("file", &area.scene.to_string())])),
            editable: false,
        });
    }
    if rules {
        fields.push(FileField {
            component: "gameRules",
            field: "playerPrefab",
            label: "Player Prefab",
            none: "None (the scene's own player)",
            no_files: "No .bprefab files in the project folder",
            files: editor.project.prefab_files(),
            problem: None,
            editable: false,
        });
    }
    if let Some(mesh) = mesh {
        let material = (!mesh.material.is_empty()).then(|| editor.scene_view.materials.get(&base.join(&mesh.material)).err()).flatten();
        fields.push(FileField {
            component: "mesh",
            field: "material",
            label: "Material",
            none: "None (colour above)",
            no_files: "No .bmaterial files in the project folder",
            files: editor.project.material_files(),
            problem: material,
            editable: true,
        });
        let model = (!mesh.model.is_empty()).then(|| editor.scene_view.assets.model(&base.join(&mesh.model)).err()).flatten();
        fields.push(FileField {
            component: "mesh",
            field: "model",
            label: "Model",
            none: "None (the shape above)",
            no_files: "No .glb or .gltf files in the project folder",
            files: editor.project.model_files(),
            problem: model,
            editable: false,
        });
    }
    if let Some(sprite) = sprite {
        let image = (!sprite.image.is_empty()).then(|| editor.scene_view.assets.image(&base.join(&sprite.image)).err()).flatten();
        fields.push(FileField {
            component: "sprite",
            field: "image",
            label: "Image",
            none: "None (plain colour)",
            no_files: "No .png files in the project folder",
            files: editor.project.image_files(),
            problem: image,
            editable: false,
        });
    }
    if let Some(component) = input_map {
        let missing = !component.asset.is_empty() && !base.join(&component.asset).is_file();
        fields.push(FileField {
            component: "inputMap",
            field: "asset",
            label: "Asset",
            none: "None (the project's Input Map)",
            no_files: "No .binput files in the project folder",
            files: editor.project.input_map_files(),
            problem: missing.then(|| crate::i18n::tf("{file} is not there", &[("file", &component.asset.to_string())])),
            editable: true,
        });
    }
    fields
}
