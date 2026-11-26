// Editor ground grid on the world's y = 0 plane: thin lines every unit, stronger lines every 10
// units, the X axis in red and the Z axis in blue. Lines use screen-space derivatives so they stay
// one pixel wide at any distance. Positions are measured from the eye (`globals.grid` says where the
// ground and the grid's pattern are), so the grid stays exact far from the world's centre.

const EXTENT: f32 = 500.0;


struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // The point measured from the eye, along the ground.
    @location(0) local_xz: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let xz = corners[index] * EXTENT + globals.camera_pos.xz;
    var out: VertexOut;
    out.clip = globals.view_proj * vec4<f32>(xz.x, globals.grid.y, xz.y, 1.0);
    out.local_xz = xz;
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
    // The grid's pattern repeats every 10 units, so the world position modulo 10 places it.
    let pattern = in.local_xz + globals.grid.xz;
    let minor = line_strength(pattern, 1.0);
    let major = line_strength(pattern, 10.0);
    let width = fwidth(in.local_xz);
    let on_x_axis = abs(in.local_xz.y - globals.grid_axes.x) < width.y;
    let on_z_axis = abs(in.local_xz.x - globals.grid_axes.y) < width.x;

    let distance = length(in.local_xz - globals.camera_pos.xz);
    // Fade out with distance, further when the eye is higher above the ground, so the horizon does
    // not shimmer.
    let reach = abs(globals.camera_pos.y - globals.grid.y) * 40.0 + 60.0;
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
