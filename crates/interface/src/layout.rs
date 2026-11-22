//! Flexible boxes, one line each: a box lays its children along its direction with a gap, gives
//! the room left over to those that grow (or takes it from those that shrink when there is too
//! little), and places them across by `alignItems` (or their own `alignSelf`). Sizes are fixed
//! (`px`), shares of the parent's inner size (`%`), or `auto`: what the content needs. Hidden
//! elements take no room. The same layout and screen size always give the same places.

use crate::style::{Align, Direction, Justify, Length, Style, StyleSheet};
use crate::{Element, Kind};

/// A box on screen, in points from the top left.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The part of this box inside `other` (empty when they do not meet).
    pub fn intersect(&self, other: &Rect) -> Rect {
        let (x, y) = (self.x.max(other.x), self.y.max(other.y));
        let (r, b) = (self.right().min(other.right()), self.bottom().min(other.bottom()));
        Rect { x, y, w: (r - x).max(0.0), h: (b - y).max(0.0) }
    }

    /// This box less `padding` (top, right, bottom, left).
    fn inset(&self, padding: [f32; 4]) -> Rect {
        Rect { x: self.x + padding[3], y: self.y + padding[0], w: (self.w - padding[1] - padding[3]).max(0.0), h: (self.h - padding[0] - padding[2]).max(0.0) }
    }
}

/// How big text and images are, from whoever draws them (the fonts live there).
pub trait TextMeasure {
    /// Width and height of `text` at `font_size` points, wrapped at `wrap` points when given.
    fn text(&self, text: &str, font_size: f32, wrap: Option<f32>) -> [f32; 2];
    /// An image's size in points, when it can be read.
    fn image(&self, path: &str) -> Option<[f32; 2]>;
}

/// An element placed on screen, with its look (lengths already scaled).
#[derive(Clone, Debug)]
pub struct Placed<'a> {
    pub element: &'a Element,
    pub style: Style,
    pub rect: Rect,
    /// What of it shows: its parents' boxes cut it (a list's entries past its bottom).
    pub clip: Rect,
    /// The index of its parent in the placed list.
    pub parent: Option<usize>,
}

struct Node<'a> {
    element: &'a Element,
    style: Style,
    children: Vec<Node<'a>>,
}

/// Places every visible element of `root` on a screen `screen` points wide and high, with every
/// length times `scale`. Parents come before their children, in the order they are drawn.
pub fn layout<'a>(root: &'a Element, sheets: &[StyleSheet], screen: [f32; 2], scale: f32, measure: &dyn TextMeasure) -> Vec<Placed<'a>> {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let Some(root) = build(root, sheets, scale) else { return Vec::new() };
    let screen_rect = Rect { x: 0.0, y: 0.0, w: screen[0].max(0.0), h: screen[1].max(0.0) };
    let [w, h] = outer_size(&root, Some(screen_rect.w), screen_rect.w, screen_rect.h, measure);
    let mut out = Vec::new();
    place(&root, Rect { x: 0.0, y: 0.0, w, h }, screen_rect, None, measure, &mut out);
    out
}

fn build<'a>(element: &'a Element, sheets: &[StyleSheet], scale: f32) -> Option<Node<'a>> {
    let mut style = Style::of(element, sheets);
    if !style.visible {
        return None;
    }
    scale_style(&mut style, scale);
    let children = element.children.iter().filter_map(|c| build(c, sheets, scale)).collect();
    Some(Node { element, style, children })
}

fn scale_style(style: &mut Style, scale: f32) {
    let length = |l: &mut Length| {
        if let Length::Px(v) = l {
            *v *= scale;
        }
    };
    for l in [&mut style.width, &mut style.height, &mut style.min_width, &mut style.min_height, &mut style.max_width, &mut style.max_height] {
        length(l);
    }
    style.gap *= scale;
    for p in &mut style.padding {
        *p *= scale;
    }
    style.border_width *= scale;
    style.radius *= scale;
    style.font_size *= scale;
}

/// A length against its parent's size along the same axis; `None` for `auto` (or a share of an
/// unknown size).
fn resolve(length: Length, parent: f32) -> Option<f32> {
    match length {
        Length::Auto => None,
        Length::Px(v) => Some(v),
        Length::Percent(p) => Some(parent * p / 100.0),
    }
}

/// `size` kept between the style's smallest and largest sizes on one axis.
fn clamp(size: f32, style: &Style, parent: f32, horizontal: bool) -> f32 {
    let (min, max) = if horizontal { (style.min_width, style.max_width) } else { (style.min_height, style.max_height) };
    let mut size = size.max(0.0);
    if let Some(max) = resolve(max, parent) {
        size = size.min(max);
    }
    if let Some(min) = resolve(min, parent) {
        size = size.max(min);
    }
    size
}

fn pad_w(style: &Style) -> f32 {
    style.padding[1] + style.padding[3] + style.border_width * 2.0
}

fn pad_h(style: &Style) -> f32 {
    style.padding[0] + style.padding[2] + style.border_width * 2.0
}

/// What a node needs, padding included, when `width` points are there for it (to wrap text in).
fn content_size(node: &Node<'_>, width: Option<f32>, measure: &dyn TextMeasure) -> [f32; 2] {
    let style = &node.style;
    let e = node.element;
    let inner_width = width.map(|w| (w - pad_w(style)).max(0.0));
    let line = style.font_size * 1.25;
    let inner = match e.kind {
        Kind::Text => measure.text(&e.text, style.font_size, inner_width),
        Kind::Button | Kind::TextField => {
            let [w, h] = measure.text(&e.text, style.font_size, inner_width);
            [w, h.max(line)]
        }
        Kind::Toggle => {
            let [w, h] = measure.text(&e.text, style.font_size, inner_width.map(|w| (w - line - style.gap).max(0.0)));
            [line + style.gap + w, h.max(line)]
        }
        Kind::Image => measure.image(&e.image).unwrap_or([0.0, 0.0]),
        // A part left in place (one that could not be read) takes no room.
        Kind::Slider | Kind::Progress | Kind::Part => [0.0, 0.0],
        Kind::Panel | Kind::Row | Kind::Column | Kind::List => {
            let row = style.direction == Direction::Row;
            let (mut main, mut cross) = (0.0f32, 0.0f32);
            for (i, child) in node.children.iter().enumerate() {
                let [w, h] = outer_size(child, if row { None } else { inner_width }, inner_width.unwrap_or(0.0), 0.0, measure);
                let (m, c) = if row { (w, h) } else { (h, w) };
                main += m + if i > 0 { style.gap } else { 0.0 };
                cross = cross.max(c);
            }
            if row { [main, cross] } else { [cross, main] }
        }
    };
    [inner[0] + pad_w(style), inner[1] + pad_h(style)]
}

/// A child's size before growing and shrinking: fixed, a share of the parent, or its content.
fn outer_size(node: &Node<'_>, width: Option<f32>, parent_w: f32, parent_h: f32, measure: &dyn TextMeasure) -> [f32; 2] {
    // A share of a size not known yet counts as `auto`.
    let known = |length: Length, parent: f32| match length {
        Length::Percent(_) if parent <= 0.0 => None,
        length => resolve(length, parent),
    };
    let fixed_w = known(node.style.width, parent_w);
    let fixed_h = known(node.style.height, parent_h);
    let content = content_size(node, fixed_w.or(width), measure);
    [clamp(fixed_w.unwrap_or(content[0]), &node.style, parent_w, true), clamp(fixed_h.unwrap_or(content[1]), &node.style, parent_h, false)]
}

fn place<'a>(node: &Node<'a>, rect: Rect, clip: Rect, parent: Option<usize>, measure: &dyn TextMeasure, out: &mut Vec<Placed<'a>>) {
    let index = out.len();
    let clip = rect.intersect(&clip);
    out.push(Placed { element: node.element, style: node.style.clone(), rect, clip, parent });
    if node.children.is_empty() {
        return;
    }
    let style = &node.style;
    let inner = rect.inset([
        style.padding[0] + style.border_width,
        style.padding[1] + style.border_width,
        style.padding[2] + style.border_width,
        style.padding[3] + style.border_width,
    ]);
    let row = style.direction == Direction::Row;
    let (inner_main, inner_cross) = if row { (inner.w, inner.h) } else { (inner.h, inner.w) };
    // Each child's size along the box and across it, before the room left over is shared.
    let mut sizes: Vec<[f32; 2]> = node
        .children
        .iter()
        .map(|child| {
            let stretch = child.style.align_self.unwrap_or(style.align_items) == Align::Stretch;
            // Across a column, a child that stretches wraps its text at the column's width.
            let width = if row {
                None
            } else if stretch || child.style.width == Length::Auto {
                Some(inner.w)
            } else {
                None
            };
            let [w, h] = outer_size(child, width, inner.w, inner.h, measure);
            if row { [w, h] } else { [h, w] }
        })
        .collect();
    let gaps = style.gap * node.children.len().saturating_sub(1) as f32;
    let used: f32 = sizes.iter().map(|s| s[0]).sum::<f32>() + gaps;
    let free = inner_main - used;
    let total_grow: f32 = node.children.iter().map(|c| c.style.grow).sum();
    if free > 0.0 && total_grow > 0.0 {
        for (size, child) in sizes.iter_mut().zip(&node.children) {
            size[0] += free * child.style.grow / total_grow;
        }
    } else if free < 0.0 && node.element.kind != Kind::List {
        // A list keeps its entries whole and cuts off what does not fit.
        let weight: f32 = sizes.iter().zip(&node.children).map(|(s, c)| s[0] * c.style.shrink).sum();
        if weight > 0.0 {
            for (size, child) in sizes.iter_mut().zip(&node.children) {
                size[0] = (size[0] + free * size[0] * child.style.shrink / weight).max(0.0);
            }
        }
    }
    // Sizes stay inside the smallest and largest each child allows.
    for (size, child) in sizes.iter_mut().zip(&node.children) {
        size[0] = clamp(size[0], &child.style, inner_main, row);
    }
    let left_over = (inner_main - sizes.iter().map(|s| s[0]).sum::<f32>() - gaps).max(0.0);
    let (mut cursor, between) = match style.justify {
        Justify::Start => (0.0, 0.0),
        Justify::Center => (left_over / 2.0, 0.0),
        Justify::End => (left_over, 0.0),
        Justify::SpaceBetween if node.children.len() > 1 => (0.0, left_over / (node.children.len() - 1) as f32),
        Justify::SpaceBetween => (0.0, 0.0),
    };
    // A list's entries past its box are cut off there.
    let child_clip = if node.element.kind == Kind::List { inner.intersect(&clip) } else { clip };
    for (child, size) in node.children.iter().zip(sizes) {
        let align = child.style.align_self.unwrap_or(style.align_items);
        let fixed_cross = if row { child.style.height } else { child.style.width };
        let cross = if align == Align::Stretch && fixed_cross == Length::Auto { clamp(inner_cross, &child.style, inner_cross, !row) } else { size[1] };
        let offset = match align {
            Align::Start | Align::Stretch => 0.0,
            Align::Center => (inner_cross - cross) / 2.0,
            Align::End => inner_cross - cross,
        };
        let child_rect = if row {
            Rect { x: inner.x + cursor, y: inner.y + offset, w: size[0], h: cross }
        } else {
            Rect { x: inner.x + offset, y: inner.y + cursor, w: cross, h: size[0] }
        };
        place(child, child_rect, child_clip, Some(index), measure, out);
        cursor += size[0] + style.gap + between;
    }
}
