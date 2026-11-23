//! Handles drawn over the scene view for moving, turning and resizing the selected object.
//!
//! Move arrows follow the world axes, with a square between each two that moves in their plane;
//! turn rings circle the world axes, and resize handles follow
//! the object's own axes. Dragging a handle changes the object's transform through
//! `scene.component.set`, as one undo step per drag. The same values can be typed in the
//! Inspector, which is the keyboard path.

use bayleaf_core::math::{DMat4, DVec3, EulerRot, Mat4, Quat, Vec3};
use bayleaf_engine::scene::components::Transform;
use egui::{Pos2, Rect, pos2};

/// What dragging a handle does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Tool {
    #[default]
    Move,
    Rotate,
    Scale,
}

impl Tool {
    pub const ALL: [Tool; 3] = [Tool::Move, Tool::Rotate, Tool::Scale];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Move => "Move",
            Tool::Rotate => "Rotate",
            Tool::Scale => "Scale",
        }
    }

    /// Key that picks the tool while the scene view has focus.
    pub fn key(self) -> egui::Key {
        match self {
            Tool::Move => egui::Key::M,
            Tool::Rotate => egui::Key::R,
            Tool::Scale => egui::Key::S,
        }
    }
}

/// One handle: a tool and an axis (0 = X, 1 = Y, 2 = Z).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Handle {
    pub tool: Tool,
    pub axis: usize,
    /// A square that moves in the plane across `axis` (the YZ plane for X), with the Move tool.
    pub plane: bool,
}

impl Handle {
    pub fn new(tool: Tool, axis: usize) -> Self {
        Self { tool, axis, plane: false }
    }

    pub fn plane(axis: usize) -> Self {
        Self { tool: Tool::Move, axis, plane: true }
    }
}

/// Planes seen closer to edge-on than this (the cosine between the plane's normal and the view)
/// hide their square: moving in them would jump with the smallest move of the pointer.
const EDGE_ON: f32 = 0.2;
/// Where a plane square starts and ends along its two axes, in handle lengths.
const PLANE_FROM: f32 = 0.22;
const PLANE_TO: f32 = 0.45;

/// World to screen for one frame of the scene view.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Projector {
    /// Points are measured from `origin` (the frame's), as the renderer measures them.
    pub view_proj: Mat4,
    pub rect: Rect,
    pub eye: Vec3,
    pub origin: DVec3,
}

impl Projector {
    /// A world matrix measured from the projector's origin.
    pub fn relative(&self, world: DMat4) -> Mat4 {
        let mut m = world;
        m.w_axis -= self.origin.extend(0.0);
        m.as_mat4()
    }

    /// Where the ray through screen point `p` meets the plane through `origin` across `normal`;
    /// `None` when the ray runs along the plane or meets it behind the camera.
    pub fn on_plane(&self, p: Pos2, origin: Vec3, normal: Vec3) -> Option<Vec3> {
        let x = (p.x - self.rect.left()) / self.rect.width() * 2.0 - 1.0;
        let y = 1.0 - (p.y - self.rect.top()) / self.rect.height() * 2.0;
        let back = self.view_proj.inverse();
        let near = back.project_point3(Vec3::new(x, y, 0.0));
        let ray = back.project_point3(Vec3::new(x, y, 1.0)) - near;
        let facing = ray.dot(normal);
        if facing.abs() < 1e-6 {
            return None;
        }
        let t = (origin - near).dot(normal) / facing;
        (t >= 0.0 && t.is_finite()).then(|| near + ray * t)
    }

    /// Where a world point appears in the view; `None` behind the camera.
    pub fn project(&self, point: Vec3) -> Option<Pos2> {
        let clip = self.view_proj * point.extend(1.0);
        if clip.w <= 1e-4 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(pos2(self.rect.left() + (ndc.x * 0.5 + 0.5) * self.rect.width(), self.rect.top() + (0.5 - ndc.y * 0.5) * self.rect.height()))
    }
}

/// The world size of the handles at `origin`: a fixed share of the distance to the camera, so
/// they keep their size on screen.
pub(crate) fn handle_length(projector: &Projector, origin: Vec3) -> f32 {
    projector.eye.distance(origin).max(0.01) * 0.16
}

/// The direction a handle works along or around, in world space.
pub(crate) fn axis_dir(handle: Handle, world: Mat4) -> Vec3 {
    match handle.tool {
        // Resizing follows the object's own axes.
        Tool::Scale => {
            let column = match handle.axis {
                0 => world.x_axis,
                1 => world.y_axis,
                _ => world.z_axis,
            };
            column.truncate().normalize_or(Vec3::AXES[handle.axis.min(2)])
        }
        _ => Vec3::AXES[handle.axis.min(2)],
    }
}

/// The screen polyline of a handle: a line for move and resize, a ring for turn.
pub(crate) fn handle_points(projector: &Projector, world: Mat4, handle: Handle) -> Vec<Pos2> {
    let origin = world.w_axis.truncate();
    let len = handle_length(projector, origin);
    let dir = axis_dir(handle, world);
    match handle.tool {
        Tool::Move | Tool::Scale => [origin + dir * len * 0.18, origin + dir * len].iter().filter_map(|p| projector.project(*p)).collect(),
        Tool::Rotate => {
            let u = dir.any_orthonormal_vector();
            let v = dir.cross(u);
            (0..=64)
                .filter_map(|i| {
                    let a = i as f32 / 64.0 * std::f32::consts::TAU;
                    projector.project(origin + (u * a.cos() + v * a.sin()) * len * 0.85)
                })
                .collect()
        }
    }
}

/// The screen corners of the square that moves in the plane across `axis`, in the corner between
/// the other two axes that faces the camera; `None` while the plane is seen edge-on or behind.
pub(crate) fn plane_points(projector: &Projector, world: Mat4, axis: usize) -> Option<Vec<Pos2>> {
    let origin = world.w_axis.truncate();
    let len = handle_length(projector, origin);
    let to_eye = (projector.eye - origin).normalize_or_zero();
    if Vec3::AXES[axis % 3].dot(to_eye).abs() < EDGE_ON {
        return None;
    }
    let toward = |a: Vec3| if a.dot(to_eye) < 0.0 { -a } else { a };
    let (u, v) = (toward(Vec3::AXES[(axis + 1) % 3]), toward(Vec3::AXES[(axis + 2) % 3]));
    let (a, b) = (PLANE_FROM * len, PLANE_TO * len);
    [u * a + v * a, u * b + v * a, u * b + v * b, u * a + v * b].iter().map(|c| projector.project(origin + *c)).collect()
}

/// Whether `p` is inside the convex polygon `corners`.
fn inside(corners: &[Pos2], p: Pos2) -> bool {
    let sides = corners.iter().zip(corners.iter().cycle().skip(1)).map(|(a, b)| (*b - *a).x * (p - *a).y - (*b - *a).y * (p - *a).x);
    let (mut left, mut right) = (false, false);
    for side in sides {
        left |= side > 0.0;
        right |= side < 0.0;
    }
    !(left && right)
}

/// The handle of `tool` nearest to `pointer`, if one is within `reach` points.
pub(crate) fn hit(projector: &Projector, world: Mat4, tool: Tool, pointer: Pos2, reach: f32) -> Option<Handle> {
    // A plane square under the pointer comes first: it lies between the arrows, never on one.
    if tool == Tool::Move
        && let Some(axis) = (0..3).find(|axis| plane_points(projector, world, *axis).is_some_and(|q| inside(&q, pointer)))
    {
        return Some(Handle::plane(axis));
    }
    (0..3)
        .map(|axis| Handle::new(tool, axis))
        .filter_map(|handle| {
            let points = handle_points(projector, world, handle);
            let distance = points.windows(2).map(|s| distance_to_segment(pointer, s[0], s[1])).fold(f32::INFINITY, f32::min);
            (distance <= reach).then_some((handle, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(handle, _)| handle)
}

fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() > 0.0 { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

/// One object's transform when a drag started.
#[derive(Clone, Debug)]
pub(crate) struct Start {
    pub object: bayleaf_core::StableId,
    pub local: Transform,
    /// The object's and its parent's world matrices, in double precision.
    pub world: DMat4,
    pub parent: DMat4,
}

/// A drag in progress. The handle belongs to the first object, the primary selection; the
/// others follow it.
#[derive(Clone, Debug)]
pub(crate) struct Drag {
    pub handle: Handle,
    pub objects: Vec<Start>,
    pub start: Pos2,
}

/// What a drag does, the same for every dragged object.
#[derive(Clone, Copy, Debug)]
enum Change {
    Move(Vec3),
    Turn(Vec3, f32),
    Resize(usize, f32),
}

/// The new local transform of every dragged object after dragging from `drag.start` to `pointer`.
pub(crate) fn drag_to(projector: &Projector, drag: &Drag, pointer: Pos2) -> Vec<(bayleaf_core::StableId, Transform)> {
    let Some(primary) = drag.objects.first() else { return Vec::new() };
    let change = change(projector, drag.handle, primary, drag.start, pointer);
    drag.objects.iter().map(|start| (start.object, apply(change, start))).collect()
}

fn change(projector: &Projector, handle: Handle, primary: &Start, from: Pos2, pointer: Pos2) -> Change {
    let world = projector.relative(primary.world);
    let origin = world.w_axis.truncate();
    let len = handle_length(projector, origin);
    let dir = axis_dir(handle, world);
    // How far along the handle's direction on screen the pointer moved, in handle lengths.
    let along = || -> f32 {
        let (Some(a), Some(b)) = (projector.project(origin), projector.project(origin + dir * len)) else { return 0.0 };
        let axis = b - a;
        if axis.length_sq() < 1.0 {
            return 0.0;
        }
        (pointer - from).dot(axis) / axis.length_sq()
    };
    if handle.plane {
        // Where the pointer's ray meets the plane through the object, then and now.
        let normal = Vec3::AXES[handle.axis % 3];
        let meet = |p: Pos2| projector.on_plane(p, origin, normal);
        return Change::Move(meet(from).zip(meet(pointer)).map_or(Vec3::ZERO, |(a, b)| (b - a) - normal * (b - a).dot(normal)));
    }
    match handle.tool {
        Tool::Move => Change::Move(dir * along() * len),
        Tool::Scale => Change::Resize(handle.axis, (1.0 + along()).max(0.01)),
        Tool::Rotate => {
            let Some(center) = projector.project(origin) else { return Change::Turn(dir, 0.0) };
            let angle = |p: Pos2| (p.y - center.y).atan2(p.x - center.x);
            let screen = (angle(pointer) - angle(from) + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            // On screen, y points down. Seen from the side the axis points to, a turn that looks
            // anticlockwise is a positive turn about the axis.
            let facing = dir.dot(projector.eye - origin).signum();
            Change::Turn(dir, -facing * screen)
        }
    }
}

/// `change` applied to one object, about its own origin.
fn apply(change: Change, start: &Start) -> Transform {
    let mut local = start.local.clone();
    match change {
        Change::Move(offset) => {
            let world_target = start.world.w_axis.truncate() + offset.as_dvec3();
            local.position = start.parent.inverse().transform_point3(world_target).to_array();
        }
        Change::Resize(axis, factor) => {
            local.scale[axis] = start.local.scale[axis] * factor;
        }
        Change::Turn(axis, angle) => {
            let (_, world_rotation, _) = start.world.to_scale_rotation_translation();
            let (_, parent_rotation, _) = start.parent.to_scale_rotation_translation();
            let turned = Quat::from_axis_angle(axis, angle) * world_rotation.as_quat();
            let (x, y, z) = (parent_rotation.as_quat().inverse() * turned).normalize().to_euler(EulerRot::XYZ);
            local.rotation = [x.to_degrees(), y.to_degrees(), z.to_degrees()];
        }
    }
    local
}

#[cfg(test)]
mod tests {
    use super::{Drag, Handle, Projector, Start, Tool, drag_to, hit, plane_points};
    use bayleaf_core::math::{Mat4, Vec3};
    use bayleaf_engine::scene::components::Transform;
    use egui::{Rect, pos2, vec2};

    /// Looking down −Z from +Z at the origin, so +X is right and +Y is up on screen.
    fn front_view() -> Projector {
        let eye = Vec3::new(0.0, 0.0, 10.0);
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(50f32.to_radians(), 1.0, 0.1, 100.0);
        Projector { view_proj: proj * view, rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0)), eye, origin: bayleaf_core::math::DVec3::ZERO }
    }

    fn start(n: u128, local: Transform, parent: Mat4) -> Start {
        let parent = parent.as_dmat4();
        Start { object: bayleaf_core::StableId::from_u128(n), world: parent * local.matrix(), local, parent }
    }

    fn drag(tool: Tool, axis: usize, local: Transform, parent: Mat4) -> Drag {
        Drag { handle: Handle::new(tool, axis), objects: vec![start(1, local, parent)], start: pos2(200.0, 200.0) }
    }

    fn first(p: &Projector, d: &Drag, to: egui::Pos2) -> Transform {
        drag_to(p, d, to).remove(0).1
    }

    #[test]
    fn the_move_arrow_under_the_pointer_is_hit_and_moves_along_its_axis() {
        let p = front_view();
        let world = Mat4::IDENTITY;
        // The X arrow runs right from the centre.
        assert_eq!(hit(&p, world, Tool::Move, pos2(240.0, 200.0), 6.0).map(|h| h.axis), Some(0));
        assert_eq!(hit(&p, world, Tool::Move, pos2(200.0, 160.0), 6.0).map(|h| h.axis), Some(1));
        assert!(hit(&p, world, Tool::Move, pos2(330.0, 330.0), 6.0).is_none());

        let moved = first(&p, &drag(Tool::Move, 0, Transform::default(), Mat4::IDENTITY), pos2(260.0, 230.0));
        assert!(moved.position[0] > 0.1, "dragging right moves +X");
        assert!(moved.position[1].abs() < 1e-5 && moved.position[2].abs() < 1e-5, "and only along X: {:?}", moved.position);
    }

    #[test]
    fn a_plane_square_moves_in_its_plane_and_keeps_the_point_under_the_pointer() {
        let p = front_view();
        let world = Mat4::IDENTITY;
        // Looking along Z, only the XY square shows; the other two planes are edge-on.
        assert!(plane_points(&p, world, 0).is_none() && plane_points(&p, world, 1).is_none());
        let corners = plane_points(&p, world, 2).expect("the XY square faces the camera");
        let center = corners.iter().fold(egui::Vec2::ZERO, |s, c| s + c.to_vec2()) / 4.0;
        let center = center.to_pos2();
        assert_eq!(hit(&p, world, Tool::Move, center, 6.0), Some(Handle::plane(2)), "the square takes the pointer");
        assert!(hit(&p, world, Tool::Rotate, center, 6.0).is_none_or(|h| !h.plane), "only the Move tool has squares");

        let mut d = drag(Tool::Move, 2, Transform::default(), Mat4::IDENTITY);
        d.handle = Handle::plane(2);
        d.start = center;
        let to = center + vec2(40.0, -30.0);
        let moved = first(&p, &d, to);
        assert!(moved.position[0] > 0.1 && moved.position[1] > 0.1, "right and up move +X and +Y: {:?}", moved.position);
        assert!(moved.position[2].abs() < 1e-5, "and never across the plane");
        // The point of the plane that was under the pointer is under it still.
        let grabbed = p.on_plane(center, Vec3::ZERO, Vec3::Z).unwrap();
        let now = p.project(grabbed + bayleaf_core::math::DVec3::from_array(moved.position).as_vec3()).unwrap();
        assert!(now.distance(to) < 0.5, "{now:?} vs {to:?}");
    }

    #[test]
    fn the_ground_square_moves_on_the_ground_seen_from_above_at_an_angle() {
        let eye = Vec3::new(6.0, 8.0, 6.0);
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(50f32.to_radians(), 1.0, 0.1, 100.0);
        let p =
            Projector { view_proj: proj * view, rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 400.0)), eye, origin: bayleaf_core::math::DVec3::ZERO };
        let corners = plane_points(&p, Mat4::IDENTITY, 1).expect("the XZ square shows from above");
        let center = (corners.iter().fold(egui::Vec2::ZERO, |s, c| s + c.to_vec2()) / 4.0).to_pos2();
        assert_eq!(hit(&p, Mat4::IDENTITY, Tool::Move, center, 6.0), Some(Handle::plane(1)));
        let mut d = drag(Tool::Move, 1, Transform::default(), Mat4::IDENTITY);
        d.handle = Handle::plane(1);
        d.start = center;
        let moved = first(&p, &d, center + vec2(30.0, 20.0));
        assert!(moved.position[1].abs() < 1e-5, "the height stays: {:?}", moved.position);
        assert!(moved.position[0].abs() + moved.position[2].abs() > 0.05, "it moves on the ground");
    }

    #[test]
    fn moving_under_a_scaled_parent_keeps_the_world_motion() {
        let p = front_view();
        let parent = Mat4::from_scale(Vec3::splat(2.0));
        let moved = first(&p, &drag(Tool::Move, 0, Transform::default(), parent), pos2(260.0, 200.0));
        let alone = first(&p, &drag(Tool::Move, 0, Transform::default(), Mat4::IDENTITY), pos2(260.0, 200.0));
        assert!((moved.position[0] * 2.0 - alone.position[0]).abs() < 1e-4, "local units are half the world units");
    }

    #[test]
    fn turning_anticlockwise_seen_from_the_axis_is_a_positive_turn() {
        let p = front_view();
        // The Z axis points at the camera. Start right of the centre, end above it: a quarter turn
        // anticlockwise on screen.
        let mut d = drag(Tool::Rotate, 2, Transform::default(), Mat4::IDENTITY);
        d.start = pos2(260.0, 200.0);
        let turned = first(&p, &d, pos2(200.0, 140.0));
        assert!((turned.rotation[2] - 90.0).abs() < 1.0, "{:?}", turned.rotation);
    }

    #[test]
    fn resizing_grows_one_axis_and_never_flips_it() {
        let p = front_view();
        let grown = first(&p, &drag(Tool::Scale, 0, Transform::default(), Mat4::IDENTITY), pos2(300.0, 200.0));
        assert!(grown.scale[0] > 1.5 && grown.scale[1] == 1.0);
        let shrunk = first(&p, &drag(Tool::Scale, 0, Transform::default(), Mat4::IDENTITY), pos2(-400.0, 200.0));
        assert!(shrunk.scale[0] > 0.0, "a long drag inwards stops at a small positive size");
    }

    #[test]
    fn every_dragged_object_moves_by_the_same_amount_from_its_own_place() {
        let p = front_view();
        let other = Transform { position: [3.0, -1.0, 0.0], ..Transform::default() };
        let mut d = drag(Tool::Move, 0, Transform::default(), Mat4::IDENTITY);
        d.objects.push(start(2, other, Mat4::IDENTITY));
        let moved = drag_to(&p, &d, pos2(260.0, 200.0));
        let dx = moved[0].1.position[0];
        assert!(dx > 0.1);
        assert!((moved[1].1.position[0] - (3.0 + dx)).abs() < 1e-4 && moved[1].1.position[1] == -1.0);
    }
}
