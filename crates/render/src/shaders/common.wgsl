// Shared by every scene shader: the per-frame values. The renderer puts this file in front of each
// shader, so the layout is written once.

const MAX_LIGHTS: u32 = 8u;

struct Light {
    // xyz: direction the light travels (directional and spot lights); w: 0 directional, 1 point, 2 spot
    direction: vec4<f32>,
    // xyz: position (point and spot lights); w: range
    position: vec4<f32>,
    // rgb: colour times intensity; w: cosine of the spot cone's edge
    radiance: vec4<f32>,
    // x: cosine where the spot cone's fade starts
    extra: vec4<f32>,
};

struct Globals {
    view_proj: mat4x4<f32>,
    // xyz: camera position, w: seconds (material Time node)
    camera_pos: vec4<f32>,
    // rgb: ambient light, w: number of lights
    ambient: vec4<f32>,
    // World to the shadow map of the light that casts shadows.
    light_view_proj: mat4x4<f32>,
    // x: lights casting shadows, y: one shadow-map texel in uv
    shadow_count: vec4<f32>,
    // rgb: ambient light from below
    ambient_ground: vec4<f32>,
    // Clip space back to the world, for the sky's view directions.
    inv_view_proj: mat4x4<f32>,
    // Sky colours (rgb) straight up, at the horizon and below it.
    sky_zenith: vec4<f32>,
    sky_horizon: vec4<f32>,
    sky_ground: vec4<f32>,
    // rgb: fog colour, w: fog density per world unit (0: no fog)
    fog: vec4<f32>,
    // Per light casting shadows: x: its index in `lights`, y: depth bias, z: how far to push points
    // out along their normal, w: its first layer.
    shadow_lights: array<vec4<f32>, 4>,
    // Per light casting shadows: x: its layers, y: 0 one face, 1 six around a point light (+X, -X,
    // +Y, -Y, +Z, -Z), 2 sun cascades by view depth.
    shadow_modes: array<vec4<f32>, 4>,
    // Per light casting shadows: the view depths where its later cascades start, then the last's end.
    shadow_splits: array<vec4<f32>, 4>,
    // World to each layer of the shadow map.
    shadow_faces: array<mat4x4<f32>, 24>,
    // The ground grid from the eye: x, z: the world's position modulo 10 under the eye, y: the ground's
    // height (world y = 0).
    grid: vec4<f32>,
    // x: where world z = 0 is along z, y: where world x = 0 is along x, both from the eye.
    grid_axes: vec4<f32>,
    lights: array<Light, MAX_LIGHTS>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

// The sky's colour in direction `dir` (unit length): from the horizon up to the top, or down to the
// ground. Drawn behind the scene and seen in reflections.
fn bl_sky(dir: vec3<f32>) -> vec3<f32> {
    if (dir.y >= 0.0) {
        return mix(globals.sky_horizon.rgb, globals.sky_zenith.rgb, pow(dir.y, 0.6));
    }
    return mix(globals.sky_horizon.rgb, globals.sky_ground.rgb, pow(-dir.y, 0.4));
}