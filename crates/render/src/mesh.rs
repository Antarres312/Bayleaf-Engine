//! Built-in shapes, generated in code: unit size, centred on the origin.

use bayleaf_scene::components::Shape;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub(crate) struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Texture coordinates: 0..1 across a face, (0, 0) at the top left of an image.
    pub uv: [f32; 2],
}

pub(crate) struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

pub(crate) fn build(shape: Shape) -> MeshData {
    match shape {
        Shape::Cube => cube(),
        Shape::Sphere => sphere(32, 16),
        Shape::Plane => square([0.0, 1.0, 0.0], |u, v| [u, 0.0, -v]),
        Shape::Quad => square([0.0, 0.0, 1.0], |u, v| [u, v, 0.0]),
    }
}

/// One square face of side 1 with normal `normal`; `place` maps (u, v) in −0.5..0.5 to a position.
fn square(normal: [f32; 3], place: impl Fn(f32, f32) -> [f32; 3]) -> MeshData {
    let corners = [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)];
    let vertices = corners.iter().map(|&(u, v)| Vertex { position: place(u, v), normal, uv: [u + 0.5, 0.5 - v] }).collect();
    MeshData { vertices, indices: vec![0, 1, 2, 0, 2, 3] }
}

fn cube() -> MeshData {
    // (normal, tangent u, tangent v) per face; u × v = normal keeps the winding counter-clockwise.
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ];
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (n, u, v) in faces {
        let base = vertices.len() as u32;
        for (su, sv) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
            let position = [0, 1, 2].map(|i| n[i] * 0.5 + u[i] * su + v[i] * sv);
            vertices.push(Vertex { position, normal: n, uv: [su + 0.5, 0.5 - sv] });
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    MeshData { vertices, indices }
}

fn sphere(segments: u32, rings: u32) -> MeshData {
    let mut vertices = Vec::new();
    for ring in 0..=rings {
        let theta = std::f32::consts::PI * ring as f32 / rings as f32;
        // Exact poles: sin(π) in f32 is not 0, which would leave sliver triangles there.
        let (sin_t, cos_t) = match ring {
            0 => (0.0, 1.0),
            r if r == rings => (0.0, -1.0),
            _ => theta.sin_cos(),
        };
        for segment in 0..=segments {
            let phi = std::f32::consts::TAU * segment as f32 / segments as f32;
            let normal = [sin_t * phi.cos(), cos_t, -sin_t * phi.sin()];
            vertices.push(Vertex { position: normal.map(|c| c * 0.5), normal, uv: [segment as f32 / segments as f32, ring as f32 / rings as f32] });
        }
    }
    let mut indices = Vec::new();
    let stride = segments + 1;
    for ring in 0..rings {
        for segment in 0..segments {
            let a = ring * stride + segment;
            let b = a + stride;
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    MeshData { vertices, indices }
}

#[cfg(test)]
mod tests {
    use super::build;
    use bayleaf_core::math::Vec3;
    use bayleaf_scene::components::Shape;

    /// Every triangle winds counter-clockwise seen from outside, so lighting and culling agree.
    #[test]
    fn triangles_face_the_way_their_normals_point() {
        for shape in Shape::ALL {
            let mesh = build(shape);
            for tri in mesh.indices.chunks(3) {
                let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| mesh.vertices[i as usize]);
                let face = (Vec3::from(b.position) - Vec3::from(a.position)).cross(Vec3::from(c.position) - Vec3::from(a.position));
                if face.length() < 1e-9 {
                    continue; // degenerate triangles at the sphere poles
                }
                let normal = Vec3::from(a.normal) + Vec3::from(b.normal) + Vec3::from(c.normal);
                assert!(face.dot(normal) > 0.0, "{shape:?} has an inward-facing triangle");
            }
        }
    }

    #[test]
    fn texture_coordinates_cover_each_face_once() {
        for shape in Shape::ALL {
            let mesh = build(shape);
            let (min, max) = mesh
                .vertices
                .iter()
                .fold(([f32::MAX; 2], [f32::MIN; 2]), |(lo, hi), v| ([lo[0].min(v.uv[0]), lo[1].min(v.uv[1])], [hi[0].max(v.uv[0]), hi[1].max(v.uv[1])]));
            assert_eq!((min, max), ([0.0, 0.0], [1.0, 1.0]), "{shape:?}");
        }
    }

    #[test]
    fn shapes_fit_in_the_unit_box() {
        for shape in Shape::ALL {
            for v in build(shape).vertices {
                assert!(v.position.iter().all(|c| c.abs() <= 0.5 + 1e-6), "{shape:?} vertex outside the unit box");
            }
        }
    }
}
