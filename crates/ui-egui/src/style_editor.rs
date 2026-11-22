//! An interface style file (`.bstyle`) edited in the Inspector: its rules in order, each picking
//! elements by one thing (a kind, a class or a name) and setting properties. A value is written
//! when its field is left; one that does not read is refused by `ui.style.set`, with the reason in
//! the status bar, and stays in its field to be fixed.

use bayleaf_interface::{Kind, PROPERTIES, Rule, StyleSheet};

use crate::i18n::{tf, tl};
use crate::theme::{self, ROW_HEIGHT};

/// What a rule picks elements by.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    Kind,
    Class,
    Name,
}

impl Pick {
    const ALL: [Pick; 3] = [Pick::Kind, Pick::Class, Pick::Name];

    fn of(rule: &Rule) -> Self {
        if rule.kind.is_some() {
            Pick::Kind
        } else if rule.class.is_some() {
            Pick::Class
        } else {
            Pick::Name
        }
    }

    fn label(self) -> &'static str {
        match self {
            Pick::Kind => "Kind",
            Pick::Class => "Class",
            Pick::Name => "Name",
        }
    }
}

/// A rule's heading: what it picks.
fn rule_title(rule: &Rule) -> String {
    match (&rule.kind, &rule.class, &rule.name) {
        (Some(kind), _, _) => tf("Every {kind}", &[("kind", kind.name())]),
        (_, Some(class), _) => tf("Class {class}", &[("class", class)]),
        (_, _, Some(name)) => tf("Named {name}", &[("name", name)]),
        _ => tl("Picks nothing").to_owned(),
    }
}

/// Shows the style's rules; true when an edit is finished and the file should be written.
pub(crate) fn show(ui: &mut egui::Ui, style: &mut StyleSheet) -> bool {
    let t = theme::tokens(ui);
    let mut done = false;
    let mut remove = None;
    let mut shift = None;
    let count = style.rules.len();
    for (i, rule) in style.rules.iter_mut().enumerate() {
        ui.push_id(("style rule", i), |ui| {
            theme::headed(t, &rule_title(rule)).show(ui, |ui| {
                // What it picks: a kind from the list, or a class or name typed.
                crate::value_editor::row(ui, t, "Picks By", |ui, label, width| {
                    let mut pick = Pick::of(rule);
                    craft_ui::ComboBox::from_id_salt("pick by")
                        .width(width)
                        .selected_text(tl(pick.label()))
                        .show_ui(ui, |ui| {
                            for p in Pick::ALL {
                                ui.selectable_value(&mut pick, p, tl(p.label()));
                            }
                        })
                        .response
                        .labelled_by(label);
                    if pick != Pick::of(rule) {
                        let word = rule.class.take().or(rule.name.take()).unwrap_or_default();
                        rule.kind = None;
                        match pick {
                            Pick::Kind => rule.kind = Some(Kind::Button),
                            Pick::Class => rule.class = Some(if word.is_empty() { "big".to_owned() } else { word }),
                            Pick::Name => rule.name = Some(if word.is_empty() { "Start".to_owned() } else { word }),
                        }
                        done = true;
                    }
                });
                match Pick::of(rule) {
                    Pick::Kind => crate::value_editor::row(ui, t, "Kind", |ui, label, width| {
                        let mut kind = rule.kind.unwrap_or(Kind::Button);
                        craft_ui::ComboBox::from_id_salt("rule kind")
                            .width(width)
                            .selected_text(kind.name())
                            .show_ui(ui, |ui| {
                                for k in Kind::ALL.into_iter().filter(|k| *k != Kind::Part) {
                                    ui.selectable_value(&mut kind, k, k.name());
                                }
                            })
                            .response
                            .labelled_by(label);
                        if Some(kind) != rule.kind {
                            rule.kind = Some(kind);
                            done = true;
                        }
                    }),
                    pick => {
                        let word = if pick == Pick::Class { rule.class.get_or_insert_with(String::new) } else { rule.name.get_or_insert_with(String::new) };
                        crate::value_editor::row(ui, t, pick.label(), |ui, label, width| {
                            done |= ui.add_sized([width, ROW_HEIGHT], egui::TextEdit::singleline(word)).labelled_by(label).lost_focus();
                        });
                    }
                }
                // Its properties.
                let mut drop_property = None;
                for (property, value) in &mut rule.set {
                    crate::value_editor::row(ui, t, property, |ui, label, width| {
                        done |= ui.add_sized([(width - 28.0).max(40.0), ROW_HEIGHT], egui::TextEdit::singleline(value)).labelled_by(label).lost_focus();
                        if ui.small_button("×").on_hover_text(tf("Remove {property}", &[("property", property)])).clicked() {
                            drop_property = Some(property.clone());
                        }
                    });
                }
                if let Some(property) = drop_property {
                    rule.set.remove(&property);
                    done = true;
                }
                ui.horizontal_wrapped(|ui| {
                    let unset: Vec<(&str, &str)> = PROPERTIES.iter().copied().filter(|(p, _)| !rule.set.contains_key(*p)).collect();
                    if !unset.is_empty() {
                        ui.menu_button(tl("Add Property"), |ui| {
                            for (property, start) in unset {
                                if ui.button(property).clicked() {
                                    rule.set.insert(property.to_owned(), start.to_owned());
                                    done = true;
                                    ui.close();
                                }
                            }
                        });
                    }
                    if ui.add_enabled(i > 0, egui::Button::new(tl("Up"))).on_hover_text(tl("Earlier rules come first; later ones win")).clicked() {
                        shift = Some((i, i - 1));
                    }
                    if ui.add_enabled(i + 1 < count, egui::Button::new(tl("Down"))).clicked() {
                        shift = Some((i, i + 1));
                    }
                    if ui.button(tl("Remove Rule")).clicked() {
                        remove = Some(i);
                    }
                });
            });
        });
        ui.add_space(theme::GAP);
    }
    if let Some((a, b)) = shift {
        style.rules.swap(a, b);
        done = true;
    }
    if let Some(i) = remove {
        style.rules.remove(i);
        done = true;
    }
    if ui.button(tl("Add Rule")).on_hover_text(tl("A rule for every button, to change from")).clicked() {
        style.rules.push(Rule { kind: Some(Kind::Button), class: None, name: None, set: Default::default() });
        done = true;
    }
    ui.label(
        egui::RichText::new(tl("Within each kind of rule (kind, then class, then name) later rules win; an element's own style comes last."))
            .small()
            .color(t.text_muted),
    );
    ui.add_space(theme::SECTION_GAP);
    done
}
