// Selection outline: the selected shapes are drawn into a mask (vs_mask, fs_mask), then a
// full-screen pass (vs_screen, fs_outline) paints a band around the mask's edge, over everything,
// so a selected shape stays outlined behind others. The surface inside keeps its own colours.

// Outline colour (linear) and width in pixels.
const OUTLINE: vec3<f32> = vec3<f32>(1.0, 0.45, 0.08);
const WIDTH: i32 = 3;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(13) joints: vec4<u32>,
    @location(14) weights: vec4<f32>,
};

struct InstanceIn {
    @location(2) model_0: vec4<f32>,
    @location(3) model_1: vec4<f32>,
    @location(4) model_2: vec4<f32>,
    @location(5) model_3: vec4<f32>,
    @location(15) skin: vec4<f32>,
};

@vertex
fn vs_mask(v: VertexIn, i: InstanceIn) -> @builtin(position) vec4<f32> {
    let model = mat4x4<f32>(i.model_0, i.model_1, i.model_2, i.model_3);
    return globals.view_proj * model * bl_skin(v.joints, v.weights, i.skin) * vec4<f32>(v.position, 1.0);
}

@fragment
fn fs_mask() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}

@group(1) @binding(0) var mask: texture_2d<f32>;

@vertex
fn vs_screen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle that covers the screen.
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_outline(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(mask));
    let here = vec2<i32>(at.xy);
    if (textureLoad(mask, here, 0).r > 0.5) {
        discard;
    }
    // The distance to the nearest selected pixel; the band fades over its last pixel.
    var nearest = f32(WIDTH + 1);
    for (var dy = -WIDTH; dy <= WIDTH; dy = dy + 1) {
        for (var dx = -WIDTH; dx <= WIDTH; dx = dx + 1) {
            let p = here + vec2<i32>(dx, dy);
            if (p.x < 0 || p.y < 0 || p.x >= size.x || p.y >= size.y) {
                continue;
            }
            if (textureLoad(mask, p, 0).r > 0.5) {
                nearest = min(nearest, length(vec2<f32>(f32(dx), f32(dy))));
            }
        }
    }
    let alpha = clamp(f32(WIDTH) + 0.5 - nearest, 0.0, 1.0);
    if (alpha <= 0.0) {
        discard;
    }
    return vec4<f32>(OUTLINE, alpha);
}
