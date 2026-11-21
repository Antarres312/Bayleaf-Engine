//! Markdown files shown formatted in the Inspector: headings, emphasis, code, links, lists,
//! quotes, rules and tables, read by the Craft apps' shared parser (`craft_markdown`) into its
//! flow model and laid out here with the theme's colours.

use craft_flow::{FBlock, FTable, Flow, Fmt, Inline, Kind, Para};

use crate::theme::{self, Tokens};

/// Largest Markdown file read, in bytes; the rest is left out with a note.
pub(crate) const MAX_BYTES: u64 = 256 * 1024;
/// Deepest nesting drawn (tables in tables); deeper content is left out.
const MAX_DEPTH: usize = 4;

/// Whether `path` is a Markdown file by its name.
pub(crate) fn is_markdown(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown"))
}

/// Reads and parses a Markdown file, at most [`MAX_BYTES`] of it.
pub(crate) fn read(path: &std::path::Path) -> Option<(Flow, String)> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(MAX_BYTES).read_to_end(&mut bytes).ok()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Some((craft_markdown::parse(&text), text))
}

pub(crate) fn show(ui: &mut egui::Ui, flow: &Flow) {
    let t = theme::tokens(ui);
    ui.scope(|ui| {
        // Lines of text, not rows of controls: no row height, and lines close together.
        ui.spacing_mut().interact_size.y = 0.0;
        ui.spacing_mut().item_spacing.y = 3.0;
        blocks(ui, t, &flow.blocks, 0);
    });
}

fn blocks(ui: &mut egui::Ui, t: &Tokens, blocks: &[FBlock], depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    // Ordered-list numbers by nesting level.
    let mut numbers = [0u32; 9];
    let mut index = 0;
    while index < blocks.len() {
        match &blocks[index] {
            FBlock::Para(para) if para.kind == Kind::Code => {
                // Consecutive code lines form one block.
                let lines: Vec<&Para> = blocks[index..]
                    .iter()
                    .map_while(|b| match b {
                        FBlock::Para(p) if p.kind == Kind::Code => Some(p),
                        _ => None,
                    })
                    .collect();
                index += lines.len();
                code_block(ui, t, &lines);
                numbers = [0; 9];
                continue;
            }
            FBlock::Para(para) => {
                match para.list {
                    Some(list) => {
                        let level = usize::from(list.level.min(8));
                        // Numbers count ordered items only; a bulleted item at the level restarts them.
                        numbers[level] = match (list.ordered, numbers[level]) {
                            (false, _) => 0,
                            (true, 0) => list.start,
                            (true, n) => n + 1,
                        };
                        numbers[level + 1..].fill(0);
                        let marker = if list.ordered { format!("{}.", numbers[level]) } else { ["•", "–", "·"][level % 3].to_owned() };
                        list_item(ui, t, para, level, &marker);
                    }
                    None => {
                        numbers = [0; 9];
                        paragraph(ui, t, para);
                    }
                }
            }
            FBlock::Table(table) => {
                numbers = [0; 9];
                self::table(ui, t, table, depth);
            }
        }
        index += 1;
    }
}

fn paragraph(ui: &mut egui::Ui, t: &Tokens, para: &Para) {
    match para.kind {
        Kind::Rule => {
            ui.separator();
        }
        Kind::Heading(level) => {
            ui.add_space(if level <= 2 { 6.0 } else { 3.0 });
            let size = match level {
                1 => 21.0,
                2 => 18.0,
                3 => 16.0,
                _ => 14.5,
            };
            runs(ui, t, &para.inlines, Some(size));
            if level <= 2 {
                ui.separator();
            }
        }
        Kind::Title => runs(ui, t, &para.inlines, Some(21.0)),
        Kind::Quote => {
            // A bar down the left side, as quotes are usually drawn.
            let inner = egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 0, top: 2, bottom: 2 }).show(ui, |ui| runs(ui, t, &para.inlines, None));
            let rect = inner.response.rect;
            ui.painter().line_segment([rect.left_top(), rect.left_bottom()], egui::Stroke::new(3.0, t.text_muted));
        }
        Kind::Normal | Kind::Code => runs(ui, t, &para.inlines, None),
    }
    ui.add_space(4.0);
}

fn list_item(ui: &mut egui::Ui, t: &Tokens, para: &Para, level: usize, marker: &str) {
    ui.horizontal_top(|ui| {
        ui.add_space(4.0 + level as f32 * 16.0);
        ui.add_sized([18.0, 0.0], egui::Label::new(egui::RichText::new(marker).color(t.text_muted)));
        ui.vertical(|ui| runs(ui, t, &para.inlines, None));
    });
}

fn code_block(ui: &mut egui::Ui, t: &Tokens, lines: &[&Para]) {
    let text: Vec<String> = lines.iter().map(|p| plain(&p.inlines)).collect();
    theme::section(t).fill(t.field).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::horizontal().id_salt(("markdown code", text.first().cloned().unwrap_or_default())).show(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(text.join("\n")).monospace().color(t.text)).extend().selectable(true));
        });
    });
    ui.add_space(4.0);
}

fn table(ui: &mut egui::Ui, t: &Tokens, table: &FTable, depth: usize) {
    egui::ScrollArea::horizontal().id_salt(("markdown table", table.rows.len(), depth)).show(ui, |ui| {
        egui::Grid::new(("markdown grid", table.rows.len(), depth)).striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
            for row in &table.rows {
                for cell in row.iter().filter(|c| !c.covered) {
                    ui.vertical(|ui| {
                        if cell.header {
                            let text: Vec<String> =
                                cell.blocks.iter().filter_map(|b| if let FBlock::Para(p) = b { Some(plain(&p.inlines)) } else { None }).collect();
                            ui.add(egui::Label::new(egui::RichText::new(text.join(" ")).strong()).extend());
                        } else {
                            blocks(ui, t, &cell.blocks, depth + 1);
                        }
                    });
                }
                ui.end_row();
            }
        });
    });
    ui.add_space(4.0);
}

/// One paragraph's text runs, wrapping across lines. Web links open in the browser.
fn runs(ui: &mut egui::Ui, t: &Tokens, inlines: &[Inline], size: Option<f32>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for inline in inlines {
            match inline {
                Inline::Text(text, fmt) => text_run(ui, t, text, fmt, size),
                Inline::Image(image) if !image.alt.is_empty() => {
                    ui.label(egui::RichText::new(format!("[{}]", image.alt)).italics().color(t.text_muted));
                }
                Inline::Figure(alt) => {
                    ui.label(egui::RichText::new(format!("[{alt}]")).italics().color(t.text_muted));
                }
                Inline::Equation { linear, .. } => {
                    ui.label(egui::RichText::new(linear).monospace());
                }
                Inline::Ruby { base, fmt, .. } | Inline::Control { text: base, fmt, .. } => text_run(ui, t, base, fmt, size),
                _ => {}
            }
        }
    });
}

fn text_run(ui: &mut egui::Ui, t: &Tokens, text: &str, fmt: &Fmt, size: Option<f32>) {
    let mut rich = egui::RichText::new(text);
    if fmt.code {
        rich = rich.code();
    }
    if let Some(size) = size {
        rich = rich.size(size).strong();
    }
    if fmt.bold {
        rich = rich.strong();
    }
    if fmt.italic {
        rich = rich.italics();
    }
    if fmt.strike {
        rich = rich.strikethrough();
    }
    if fmt.underline {
        rich = rich.underline();
    }
    match fmt.link.as_deref() {
        Some(url) if url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:") => {
            ui.add(egui::Hyperlink::from_label_and_url(rich.color(t.accent).underline(), url)).on_hover_text(url);
        }
        Some(target) => {
            ui.label(rich.underline()).on_hover_text(target);
        }
        None => {
            ui.label(rich);
        }
    }
}

/// The text of some runs, without formatting.
fn plain(inlines: &[Inline]) -> String {
    inlines
        .iter()
        .filter_map(|i| match i {
            Inline::Text(text, _) => Some(text.as_str()),
            Inline::Ruby { base, .. } => Some(base.as_str()),
            Inline::Control { text, .. } => Some(text.as_str()),
            Inline::Equation { linear, .. } => Some(linear.as_str()),
            _ => None,
        })
        .collect()
}
