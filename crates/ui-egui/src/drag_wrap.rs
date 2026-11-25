//! Number fields keep dragging at the window's edge.
//!
//! Dragging a number field changes its value while the pointer moves. When the pointer reaches an
//! edge of the window, it jumps to the opposite edge and the drag goes on, so a value changes as far
//! as it needs in one drag. Every dragged number field calls [`WrapDrag::wrap_drag`] on its response.
//!
//! The jump must not change the value: egui measures a drag from the pointer's place, so while a
//! drag has wrapped, the pointer egui sees is the real one plus the jumps so far, as if it went on
//! past the edge. When the drag ends, the pointer goes back to where the drag began. Where the
//! platform does not move the pointer (no jump arrives), the drag follows the pointer's raw motion
//! instead of its place, where the platform reports it. Keyboard editing is untouched: only a
//! drag by the primary pointer button wraps.

use egui::{Context, Event, PointerButton, Pos2, RawInput, Rect, Response, Ui, Vec2, ViewportCommand, ViewportId, ViewportIdMap};

/// How close to an edge, in points, the pointer wraps.
const EDGE: f32 = 3.0;

/// Pointer moves after a jump was asked for without it arriving, before the drag follows raw motion.
const MISSED_JUMPS: u32 = 6;

/// Makes a dragged number field wrap the pointer at the window's edge.
pub(crate) trait WrapDrag {
    /// Wraps the pointer while this response is dragged by the primary button.
    fn wrap_drag(self) -> Self;
}

impl WrapDrag for Response {
    fn wrap_drag(self) -> Self {
        if self.dragged_by(PointerButton::Primary) {
            let viewport = self.ctx.viewport_id();
            self.ctx.plugin_or_default::<DragWrap>().lock().wanted = Some(viewport);
        }
        self
    }
}

/// The pointer as the platform reports it in one viewport.
#[derive(Default, Clone, Copy)]
struct Track {
    pointer: Option<Pos2>,
}

/// A number field drag that may wrap.
struct Drag {
    viewport: ViewportId,
    /// Where the drag began, in the platform's coordinates.
    start: Option<Pos2>,
    /// Added to the platform's pointer so egui sees the pointer go on past the edge.
    offset: Vec2,
    /// A jump that was asked for and has not arrived yet.
    pending: Option<Pending>,
    wrapped: bool,
    /// The platform did not move the pointer: the drag follows raw motion from `motion_at`.
    motion_at: Option<Pos2>,
}

struct Pending {
    target: Pos2,
    missed: u32,
}

#[derive(Default)]
struct DragWrap {
    /// A number field was dragged in this pass of that viewport.
    wanted: Option<ViewportId>,
    drag: Option<Drag>,
    tracks: ViewportIdMap<Track>,
}

impl egui::Plugin for DragWrap {
    fn debug_name(&self) -> &'static str {
        "DragWrap"
    }

    fn input_hook(&mut self, ctx: &Context, input: &mut RawInput) {
        let viewport = input.viewport_id;
        let track = self.tracks.entry(viewport).or_default();
        let Some(drag) = self.drag.as_mut().filter(|drag| drag.viewport == viewport) else {
            for event in &input.events {
                record(track, event);
            }
            return;
        };
        if let Some(at) = drag.motion_at {
            follow_motion(ctx, input, track, at, drag);
            return;
        }
        for event in &mut input.events {
            match event {
                Event::PointerMoved(pos) => {
                    let real = *pos;
                    let before = track.pointer.unwrap_or(real);
                    if let Some(pending) = &mut drag.pending {
                        if real.distance(pending.target) < real.distance(before) {
                            // The jump: from here on egui sees the pointer carry on from where it was.
                            drag.offset += before - pending.target;
                            drag.pending = None;
                            drag.wrapped = true;
                        } else {
                            pending.missed += 1;
                            if pending.missed >= MISSED_JUMPS {
                                drag.pending = None;
                                drag.motion_at = Some(real + drag.offset);
                            }
                        }
                    }
                    track.pointer = Some(real);
                    *pos = real + drag.offset;
                }
                Event::PointerButton { pos, .. } => {
                    track.pointer = Some(*pos);
                    *pos += drag.offset;
                }
                Event::PointerGone => track.pointer = None,
                _ => {}
            }
        }
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx();
        let viewport = ctx.viewport_id();
        let wanted = self.wanted == Some(viewport);
        if wanted {
            self.wanted = None;
        }
        if self.drag.as_ref().is_some_and(|drag| drag.viewport != viewport) {
            return;
        }
        let track = self.tracks.get(&viewport).copied().unwrap_or_default();
        if !wanted {
            if let Some(drag) = self.drag.take()
                && drag.wrapped
                && drag.motion_at.is_none()
                && let Some(start) = drag.start
            {
                ctx.send_viewport_cmd(ViewportCommand::CursorPosition(start));
            }
            return;
        }
        // A drag starts unwrapped, so egui's own press position is the platform's.
        let drag = self.drag.get_or_insert_with(|| Drag {
            viewport,
            start: ctx.input(|i| i.pointer.press_origin()),
            offset: Vec2::ZERO,
            pending: None,
            wrapped: false,
            motion_at: None,
        });
        if drag.pending.is_some() || drag.motion_at.is_some() {
            return;
        }
        if let Some(target) = track.pointer.and_then(|pointer| wrap_target(ctx.viewport_rect(), pointer)) {
            ctx.send_viewport_cmd(ViewportCommand::CursorPosition(target));
            drag.pending = Some(Pending { target, missed: 0 });
        }
    }
}

fn record(track: &mut Track, event: &Event) {
    match event {
        Event::PointerMoved(pos) | Event::PointerButton { pos, .. } => track.pointer = Some(*pos),
        Event::PointerGone => track.pointer = None,
        _ => {}
    }
}

/// The platform did not move the pointer, so the pointer egui sees moves by the raw motion, where
/// the platform reports it; without raw motion, the pointer's place is all there is.
fn follow_motion(ctx: &Context, input: &mut RawInput, track: &mut Track, at: Pos2, drag: &mut Drag) {
    let scale = ctx.pixels_per_point().max(f32::EPSILON);
    let mut motion = Vec2::ZERO;
    let mut has_motion = false;
    for event in &input.events {
        if let Event::MouseMoved(delta) = event {
            motion += *delta / scale;
            has_motion = true;
        }
    }
    for event in &input.events {
        record(track, event);
    }
    if !has_motion {
        for event in &mut input.events {
            if let Event::PointerMoved(pos) | Event::PointerButton { pos, .. } = event {
                *pos += drag.offset;
            }
        }
        return;
    }
    let pointer = at + motion;
    drag.motion_at = Some(pointer);
    input.events.retain(|event| !matches!(event, Event::PointerMoved(_)));
    for event in &mut input.events {
        if let Event::PointerButton { pos, .. } = event {
            *pos = pointer;
        }
    }
    input.events.insert(0, Event::PointerMoved(pointer));
}

/// Where the pointer jumps to when it is at an edge of `area`: just inside the opposite edge.
fn wrap_target(area: Rect, pointer: Pos2) -> Option<Pos2> {
    let axis = |at: f32, min: f32, max: f32| {
        if max - min <= 4.0 * EDGE {
            at
        } else if at <= min + EDGE {
            max - 2.0 * EDGE
        } else if at >= max - EDGE {
            min + 2.0 * EDGE
        } else {
            at
        }
    };
    let target = Pos2::new(axis(pointer.x, area.min.x, area.max.x), axis(pointer.y, area.min.y, area.max.y));
    (target != pointer).then_some(target)
}
