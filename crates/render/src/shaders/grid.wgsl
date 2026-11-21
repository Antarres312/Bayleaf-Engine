// Editor ground grid on the y = 0 plane: thin lines every unit, stronger lines every 10 units,
// the X axis in red and the Z axis in blue. Lines use screen-space derivatives so they stay
// one pixel wide at any distance.

const MAX_LIGHTS: u32 = 4u;
const EXTENT: f32 = 500.0;

struct Light {
    direction: vec4<f32>,
    radiance: vec4<f32>,
};

struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    ambient: vec4<f32>,
    lights: array<Light, MAX_LIGHTS>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_xz: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let xz = corners[index] * EXTENT + globals.camera_pos.xz;
    var out: VertexOut;
    out.clip = globals.view_proj * vec4<f32>(xz.x, 0.0, xz.y, 1.0);
    out.world_xz = xz;
    return out;
}

fn line_strength(coord: vec2<f32>, spacing: f32) -> f32 {
    let c = coord / spacing;
    let width = fwidth(c);
    let distance = abs(fract(c - 0.5) - 0.5) / max(width, vec2<f32>(1e-6));
    return 1.0 - min(min(distance.x, distance.y), 1.0);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let minor = line_strength(in.world_xz, 1.0);
    let major = line_strength(in.world_xz, 10.0);
    let width = fwidth(in.world_xz);
    let on_x_axis = abs(in.world_xz.y) < width.y;
    let on_z_axis = abs(in.world_xz.x) < width.x;

    let distance = length(in.world_xz - globals.camera_pos.xz);
    // Fade out with distance, further when the camera is higher, so the horizon does not shimmer.
    let reach = abs(globals.camera_pos.y) * 40.0 + 60.0;
    let fade = 1.0 - smoothstep(0.25, 1.0, distance / reach);

    var color = vec3<f32>(0.55, 0.55, 0.58);
    var alpha = max(minor * 0.25, major * 0.45);
    if (on_x_axis) {
        color = vec3<f32>(0.85, 0.25, 0.25);
        alpha = 0.8;
    } else if (on_z_axis) {
        color = vec3<f32>(0.25, 0.4, 0.9);
        alpha = 0.8;
    }
    return vec4<f32>(color, alpha * fade);
}
