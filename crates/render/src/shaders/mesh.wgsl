// Lit shapes and unlit sprites.
// Lighting: Lambert diffuse plus Cook-Torrance specular with the GGX distribution, Smith geometry
// term (Schlick-GGX, k = (r + 1)^2 / 8) and Schlick's Fresnel approximation, as described in
// Walter et al. 2007, "Microfacet Models for Refraction through Rough Surfaces", and
// Karis 2013, "Real Shading in Unreal Engine 4" course notes.

const PI: f32 = 3.14159265;
// Highlight colour of selected objects (linear).
const SELECTION: vec3<f32> = vec3<f32>(1.0, 0.45, 0.08);

@group(1) @binding(0) var shadow_map: texture_depth_2d;
@group(1) @binding(1) var shadow_sampler: sampler_comparison;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct InstanceIn {
    @location(2) model_0: vec4<f32>,
    @location(3) model_1: vec4<f32>,
    @location(4) model_2: vec4<f32>,
    @location(5) model_3: vec4<f32>,
    @location(6) normal_0: vec4<f32>,
    @location(7) normal_1: vec4<f32>,
    @location(8) normal_2: vec4<f32>,
    @location(9) color: vec4<f32>,
    // x: metallic, y: roughness, z: selected, w: unlit
    @location(10) params: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) params: vec4<f32>,
};

@vertex
fn vs_main(v: VertexIn, i: InstanceIn) -> VertexOut {
    let model = mat4x4<f32>(i.model_0, i.model_1, i.model_2, i.model_3);
    let normal_matrix = mat3x3<f32>(i.normal_0.xyz, i.normal_1.xyz, i.normal_2.xyz);
    let world = model * vec4<f32>(v.position, 1.0);
    var out: VertexOut;
    out.clip = globals.view_proj * world;
    out.world_pos = world.xyz;
    out.normal = normal_matrix * v.normal;
    out.color = i.color;
    out.params = i.params;
    return out;
}

fn distribution_ggx(n_dot_h: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

fn geometry_schlick_ggx(n_dot_x: f32, k: f32) -> f32 {
    return n_dot_x / (n_dot_x * (1.0 - k) + k);
}

// How much of the shadow-casting light reaches a point: 1 lit, 0 in shadow. Nine taps of the
// shadow map (percentage-closer filtering) soften the edge.
fn shadow_factor(world_pos: vec3<f32>, n: vec3<f32>) -> f32 {
    // Pushing the point out along the normal keeps surfaces from shadowing themselves.
    let p = globals.light_view_proj * vec4<f32>(world_pos + n * globals.shadow.w, 1.0);
    let ndc = p.xyz / p.w;
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z > 1.0) {
        return 1.0;
    }
    var lit = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let offset = vec2<f32>(f32(x), f32(y)) * globals.shadow.z;
            lit = lit + textureSampleCompareLevel(shadow_map, shadow_sampler, uv + offset, ndc.z - globals.shadow.y);
        }
    }
    return lit / 9.0;
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

// What a pixel is made of. A material graph replaces `surface`; the default reads the mesh component.
struct Surface {
    base: vec3<f32>,
    metallic: f32,
    roughness: f32,
    emissive: vec3<f32>,
    alpha: f32,
};

struct SurfaceIn {
    position: vec3<f32>,
    normal: vec3<f32>,
    // Towards the camera.
    view: vec3<f32>,
    color: vec4<f32>,
    params: vec4<f32>,
    time: f32,
};

// Pattern helpers for material graphs: the usual sine-based hash, and value noise made by blending
// the hashes of the eight surrounding lattice points with a smoothstep fade.
fn bl_hash(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(127.1, 311.7, 74.7))) * 43758.5453);
}

fn bl_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(mix(bl_hash(i), bl_hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x), mix(bl_hash(i + vec3<f32>(0.0, 1.0, 0.0)), bl_hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x), u.y);
    let b = mix(mix(bl_hash(i + vec3<f32>(0.0, 0.0, 1.0)), bl_hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x), mix(bl_hash(i + vec3<f32>(0.0, 1.0, 1.0)), bl_hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x), u.y);
    return mix(a, b, u.z);
}

fn bl_checker(p: vec3<f32>) -> f32 {
    // A small offset keeps faces that lie exactly on a cell border from flickering.
    let c = floor(p + vec3<f32>(1e-3));
    return f32(i32(c.x + c.y + c.z) & 1);
}

// @surface

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let selected = in.params.z > 0.5;
    if (in.params.w > 0.5) {
        var unlit = in.color.rgb;
        if (selected) {
            unlit = mix(unlit, SELECTION, 0.25);
        }
        return vec4<f32>(unlit, in.color.a);
    }

    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    let v = normalize(globals.camera_pos.xyz - in.world_pos);
    let s = surface(SurfaceIn(in.world_pos, n, v, in.color, in.params, globals.camera_pos.w));
    let base = max(s.base, vec3<f32>(0.0));
    let metallic = clamp(s.metallic, 0.0, 1.0);
    let roughness = clamp(s.roughness, 0.045, 1.0);
    let alpha = clamp(s.alpha, 0.0, 1.0);
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    let n_dot_v = max(dot(n, v), 1e-4);
    let r = roughness + 1.0;
    let k = r * r / 8.0;

    // Hemisphere ambient, brighter from above. Metals reflect it tinted by their own colour.
    let ambient = mix(globals.ambient.rgb * 0.35, globals.ambient.rgb, n.y * 0.5 + 0.5);
    let ambient_f = fresnel_schlick(n_dot_v, f0);
    var color = ambient * ((vec3<f32>(1.0) - ambient_f) * (1.0 - metallic) * base + ambient_f);
    let count = min(u32(globals.ambient.w), MAX_LIGHTS);
    for (var index = 0u; index < count; index = index + 1u) {
        let light = globals.lights[index];
        var l = normalize(-light.direction.xyz);
        var falloff = 1.0;
        if (light.direction.w > 0.5) {
            // Point and spot lights: inverse-square falloff, windowed to reach 0 at the range
            // (the window from Karis 2013).
            let to_light = light.position.xyz - in.world_pos;
            let d = length(to_light);
            l = to_light / max(d, 1e-4);
            let ratio = d / max(light.position.w, 1e-4);
            let window = clamp(1.0 - ratio * ratio * ratio * ratio, 0.0, 1.0);
            falloff = window * window / (d * d + 1.0);
            if (light.direction.w > 1.5) {
                let along = dot(-l, normalize(light.direction.xyz));
                falloff = falloff * smoothstep(light.radiance.w, max(light.extra.x, light.radiance.w + 1e-4), along);
            }
        }
        if (f32(index) == globals.shadow.x) {
            falloff = falloff * shadow_factor(in.world_pos, n);
        }
        let h = normalize(v + l);
        let n_dot_l = max(dot(n, l), 0.0);
        let n_dot_h = max(dot(n, h), 0.0);
        let f = fresnel_schlick(max(dot(h, v), 0.0), f0);
        let g = geometry_schlick_ggx(n_dot_v, k) * geometry_schlick_ggx(n_dot_l, k);
        let specular = distribution_ggx(n_dot_h, roughness) * g * f / (4.0 * n_dot_v * n_dot_l + 1e-4);
        let diffuse = (vec3<f32>(1.0) - f) * (1.0 - metallic) * base / PI;
        color = color + (diffuse + specular) * light.radiance.rgb * n_dot_l * falloff;
    }

    color = color + max(s.emissive, vec3<f32>(0.0));
    if (selected) {
        // A light tint plus a glow toward grazing angles, so the surface keeps its own colour.
        let rim = pow(1.0 - n_dot_v, 4.0);
        color = mix(color, SELECTION, 0.06) + SELECTION * rim * 0.9;
    }
    return vec4<f32>(color, alpha);
}
