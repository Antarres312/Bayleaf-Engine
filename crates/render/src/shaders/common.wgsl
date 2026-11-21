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
    // x: index of the light that casts shadows, or -1; y: depth bias; z: one shadow-map texel in uv;
    // w: how far to push points out along their normal before the test, in world units
    shadow: vec4<f32>,
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
    // World to each face of the shadow map (one for a sun or spot light; +X, -X, +Y, -Y, +Z, -Z
    // around a point light).
    shadow_faces: array<mat4x4<f32>, 6>,
    // x: faces in use
    shadow_extra: vec4<f32>,
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