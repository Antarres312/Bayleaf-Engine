// Lit shapes and unlit sprites.
// Lighting: Lambert diffuse plus Cook-Torrance specular with the GGX distribution, Smith geometry
// term (Schlick-GGX, k = (r + 1)^2 / 8) and Schlick's Fresnel approximation, as described in
// Walter et al. 2007, "Microfacet Models for Refraction through Rough Surfaces", and
// Karis 2013, "Real Shading in Unreal Engine 4" course notes.

const MAX_LIGHTS: u32 = 4u;
const PI: f32 = 3.14159265;

struct Light {
    direction: vec4<f32>,
    radiance: vec4<f32>,
};

struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    // rgb: ambient light, w: number of lights
    ambient: vec4<f32>,
    lights: array<Light, MAX_LIGHTS>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

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

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let selected = in.params.z > 0.5;
    if (in.params.w > 0.5) {
        var unlit = in.color.rgb;
        if (selected) {
            unlit = mix(unlit, vec3<f32>(1.0, 0.55, 0.1), 0.35);
        }
        return vec4<f32>(unlit, in.color.a);
    }

    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    let v = normalize(globals.camera_pos.xyz - in.world_pos);
    let base = in.color.rgb;
    let metallic = clamp(in.params.x, 0.0, 1.0);
    let roughness = clamp(in.params.y, 0.045, 1.0);
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
        let l = normalize(-light.direction.xyz);
        let h = normalize(v + l);
        let n_dot_l = max(dot(n, l), 0.0);
        let n_dot_h = max(dot(n, h), 0.0);
        let f = fresnel_schlick(max(dot(h, v), 0.0), f0);
        let g = geometry_schlick_ggx(n_dot_v, k) * geometry_schlick_ggx(n_dot_l, k);
        let specular = distribution_ggx(n_dot_h, roughness) * g * f / (4.0 * n_dot_v * n_dot_l + 1e-4);
        let diffuse = (vec3<f32>(1.0) - f) * (1.0 - metallic) * base / PI;
        color = color + (diffuse + specular) * light.radiance.rgb * n_dot_l;
    }

    if (selected) {
        color = mix(color, vec3<f32>(1.0, 0.55, 0.1), 0.35);
    }
    return vec4<f32>(color, in.color.a);
}
