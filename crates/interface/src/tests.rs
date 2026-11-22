use super::*;

/// Every character half the font size wide; lines wrap at whole characters.
struct Monospace;

impl TextMeasure for Monospace {
    fn text(&self, text: &str, font_size: f32, wrap: Option<f32>) -> [f32; 2] {
        let per = font_size * 0.5;
        let chars = text.chars().count() as f32;
        let line = font_size * 1.25;
        match wrap {
            Some(width) if width >= per && chars * per > width => {
                let per_line = (width / per).floor().max(1.0);
                [per_line * per, (chars / per_line).ceil() * line]
            }
            _ => [chars * per, if text.is_empty() { 0.0 } else { line }],
        }
    }

    fn image(&self, _path: &str) -> Option<[f32; 2]> {
        Some([64.0, 32.0])
    }
}

fn element(kind: Kind, name: &str, style: &[(&str, &str)]) -> Element {
    let mut e = Element::new(kind);
    e.name = name.to_owned();
    e.style = style.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    e
}

fn placed<'a>(all: &'a [Placed<'a>], name: &str) -> &'a Placed<'a> {
    all.iter().find(|p| p.element.name == name).unwrap_or_else(|| panic!("{name} is not placed"))
}

fn inside(inner: Rect, outer: Rect) -> bool {
    inner.x >= outer.x - 0.01 && inner.y >= outer.y - 0.01 && inner.right() <= outer.right() + 0.01 && inner.bottom() <= outer.bottom() + 0.01
}

#[test]
fn layout_files_are_checked_and_say_what_is_wrong() {
    let starter = Layout::starter();
    assert_eq!(Layout::from_text(&starter.to_text()).unwrap(), starter, "a written layout reads back the same");

    let mut text_with_children = Layout::starter();
    text_with_children.root.children[0].children.push(Element::new(Kind::Text));
    assert!(text_with_children.validate().unwrap_err().contains("holds other elements"));

    let mut twice = Layout::starter();
    twice.root.children[1].name = "Title".to_owned();
    assert!(twice.validate().unwrap_err().contains("named"));

    for (property, value) in [("width", "12"), ("color", "red"), ("gap", "5%"), ("float", "left"), ("fontSize", "1px"), ("grow", "-1")] {
        let mut bad = Layout::starter();
        bad.root.style.insert(property.to_owned(), value.to_owned());
        assert!(bad.validate().is_err(), "{property}: {value} is refused");
    }

    let two_picks = r#"{"format": "bayleaf.style", "version": 1, "rules": [{"kind": "button", "class": "big", "set": {}}]}"#;
    assert!(StyleSheet::from_text(two_picks).unwrap_err().contains("exactly one"));
    let deep = (0..MAX_DEPTH + 1).fold(Element::new(Kind::Text), |inner, _| {
        let mut outer = Element::new(Kind::Panel);
        outer.children.push(inner);
        outer
    });
    let deep = Layout { root: deep, ..Layout::starter() };
    assert!(deep.validate().is_err(), "nesting has a limit");
}

#[test]
fn styles_cascade_kind_then_class_then_name_then_the_elements_own() {
    let sheet = StyleSheet::from_text(
        r##"{"format": "bayleaf.style", "version": 1, "rules": [
            {"name": "Go", "set": {"color": "#00ff00"}},
            {"class": "loud", "set": {"color": "#ff0000", "fontSize": "30px"}},
            {"kind": "button", "set": {"color": "#0000ff", "radius": "2px", "fontSize": "20px"}}
        ]}"##,
    )
    .unwrap();
    let mut button = element(Kind::Button, "Go", &[]);
    button.classes.push("loud".to_owned());
    let style = Style::of(&button, std::slice::from_ref(&sheet));
    assert_eq!(style.color, Color([0, 255, 0, 255]), "the name rule wins over class and kind, whatever the order in the file");
    assert_eq!(style.font_size, 30.0, "the class rule wins over the kind rule");
    assert_eq!(style.radius, 2.0, "the kind rule still sets what nothing later sets");
    button.style.insert("color".to_owned(), "#ffffff".to_owned());
    assert_eq!(Style::of(&button, &[sheet]).color, Color([255, 255, 255, 255]), "the element's own style comes last");
}

#[test]
fn a_row_shares_the_room_left_by_grow_and_keeps_fixed_sizes() {
    let mut root = element(Kind::Row, "Bar", &[("width", "400px"), ("height", "50px"), ("gap", "10px")]);
    root.children = vec![
        element(Kind::Panel, "Fixed", &[("width", "100px")]),
        element(Kind::Panel, "One", &[("grow", "1")]),
        element(Kind::Panel, "Three", &[("grow", "3")]),
    ];
    let all = layout(&root, &[], [800.0, 600.0], 1.0, &Monospace);
    let (bar, fixed, one, three) = (placed(&all, "Bar"), placed(&all, "Fixed"), placed(&all, "One"), placed(&all, "Three"));
    assert!((fixed.rect.w - 100.0).abs() < 0.01, "a fixed size is kept");
    assert!(three.rect.w > one.rect.w * 2.9 && three.rect.w < one.rect.w * 3.1, "grow 3 takes three times grow 1's share");
    assert!(one.rect.x >= fixed.rect.right() + 9.9 && three.rect.x >= one.rect.right() + 9.9, "the gap lies between them");
    assert!((three.rect.right() - bar.rect.right()).abs() < 0.01, "together they fill the row");
    assert!(one.rect.h == bar.rect.h, "they stretch across by default");
}

#[test]
fn a_column_centres_its_children_and_hidden_ones_take_no_room() {
    let mut root = element(Kind::Column, "Screen", &[("width", "100%"), ("height", "100%"), ("alignItems", "center"), ("justify", "center")]);
    let mut title = element(Kind::Text, "Title", &[]);
    title.text = "Hello".to_owned();
    let gone = element(Kind::Panel, "Gone", &[("height", "200px"), ("visible", "false")]);
    let mut start = element(Kind::Button, "Start", &[]);
    start.text = "Start".to_owned();
    root.children = vec![title, gone, start];
    let all = layout(&root, &[], [800.0, 600.0], 1.0, &Monospace);
    assert!(all.iter().all(|p| p.element.name != "Gone"), "a hidden element is not placed");
    let (screen, title, start) = (placed(&all, "Screen"), placed(&all, "Title"), placed(&all, "Start"));
    assert_eq!((screen.rect.w, screen.rect.h), (800.0, 600.0), "100% of the screen");
    let middle = |r: Rect| r.x + r.w / 2.0;
    assert!((middle(title.rect) - 400.0).abs() < 0.5 && (middle(start.rect) - 400.0).abs() < 0.5, "centred across");
    let top_gap = title.rect.y;
    let bottom_gap = 600.0 - start.rect.bottom();
    assert!((top_gap - bottom_gap).abs() < 0.5, "centred along: {top_gap} and {bottom_gap}");
    assert!(start.rect.y >= title.rect.bottom(), "the button comes after the title, with no room kept for the hidden panel between");
    assert!(start.rect.w > 5.0 * 9.0, "a button is wider than its text, by its padding");
}

#[test]
fn too_little_room_shrinks_children_to_fit_and_shares_follow_the_parent() {
    let mut root = element(Kind::Row, "Narrow", &[("width", "300px"), ("height", "40px")]);
    root.children = vec![element(Kind::Panel, "A", &[("width", "200px")]), element(Kind::Panel, "B", &[("width", "200px")])];
    let all = layout(&root, &[], [800.0, 600.0], 1.0, &Monospace);
    let narrow = placed(&all, "Narrow").rect;
    assert!(all.iter().all(|p| inside(p.rect, narrow)), "everything fits inside the row");

    let mut root = element(Kind::Panel, "Half", &[("width", "50%"), ("height", "50%"), ("padding", "10px")]);
    root.children = vec![element(Kind::Panel, "Quarter", &[("width", "50%"), ("height", "20px")])];
    let all = layout(&root, &[], [800.0, 600.0], 1.0, &Monospace);
    let (half, quarter) = (placed(&all, "Half").rect, placed(&all, "Quarter").rect);
    assert!(half.w > 399.0 && half.w < 401.0);
    assert!(quarter.w < half.w / 2.0 && quarter.w > half.w / 2.0 - 11.0, "a share of the parent's inner width, inside its padding");
    assert!(inside(quarter, half));
}

#[test]
fn text_wraps_in_a_narrow_column_and_a_bigger_scale_makes_everything_bigger() {
    let mut text = element(Kind::Text, "Words", &[]);
    text.text = "a sentence long enough to wrap".to_owned();
    let mut wide = element(Kind::Column, "Wide", &[("width", "600px")]);
    wide.children = vec![text.clone()];
    let mut narrow = element(Kind::Column, "Narrow", &[("width", "100px")]);
    narrow.children = vec![text];
    let wide_text = layout(&wide, &[], [800.0, 600.0], 1.0, &Monospace);
    let narrow_text = layout(&narrow, &[], [800.0, 600.0], 1.0, &Monospace);
    let (w, n) = (placed(&wide_text, "Words").rect, placed(&narrow_text, "Words").rect);
    assert!(n.h > w.h * 2.0, "wrapped onto more lines: {} against {}", n.h, w.h);
    assert!(n.w <= 100.0 + 0.01, "inside the column");

    let small = layout(&wide, &[], [800.0, 600.0], 1.0, &Monospace);
    let big = layout(&wide, &[], [800.0, 600.0], 2.0, &Monospace);
    assert!(placed(&big, "Wide").rect.w > placed(&small, "Wide").rect.w * 1.9, "fixed sizes scale");
    assert!(placed(&big, "Words").style.font_size > placed(&small, "Words").style.font_size, "so do fonts");
}

#[test]
fn a_list_cuts_off_entries_past_its_bottom() {
    let mut list = element(Kind::List, "Scores", &[("height", "100px"), ("width", "200px")]);
    list.children = (0..10).map(|i| element(Kind::Panel, &format!("Entry {i}"), &[("height", "30px")])).collect();
    let all = layout(&list, &[], [800.0, 600.0], 1.0, &Monospace);
    let box_rect = placed(&all, "Scores").rect;
    let last = placed(&all, "Entry 9");
    assert!(last.rect.y > box_rect.bottom(), "the last entry lies below the list");
    assert!(last.clip.h == 0.0, "and none of it shows");
    assert!(placed(&all, "Entry 0").clip.h > 0.0);
}

#[test]
fn every_listed_property_starts_from_a_value_it_takes() {
    for (property, value) in PROPERTIES {
        assert!(Style::defaults(Kind::Panel).apply(property, value).is_ok(), "{property}: {value}");
    }
    // And the list has every property the styles read: an unknown name says so.
    assert!(Style::defaults(Kind::Panel).apply("float", "left").is_err());
}
