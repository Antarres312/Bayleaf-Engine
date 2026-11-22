// Lit shapes and unlit sprites.
// Lighting: Lambert diffuse plus Cook-Torrance specular with the GGX distribution, Smith geometry
// term (Schlick-GGX, k = (r + 1)^2 / 8) and Schlick's Fresnel approximation, as described in
// Walter et al. 2007, "Microfacet Models for Refraction through Rough Surfaces", and
// Karis 2013, "Real Shading in Unreal Engine 4" course notes.

const PI: f32 = 3.14159265;

@group(1) @binding(0) var shadow_map: texture_depth_2d_array;
@group(1) @binding(1) var shadow_sampler: sampler_comparison;

// Images: a sprite's image in t0, or a material's Image nodes by slot. Unused slots hold a white pixel.
@group(2) @binding(0) var t0: texture_2d<f32>;
@group(2) @binding(1) var t1: texture_2d<f32>;
@group(2) @binding(2) var t2: texture_2d<f32>;
@group(2) @binding(3) var t3: texture_2d<f32>;
@group(2) @binding(4) var material_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(11) uv: vec2<f32>,
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
    // x: metallic, y: roughness, z: selected (outlined in its own pass), w: unlit
    @location(10) params: vec4<f32>,
    // The part of the images shown: offset xy, size zw (a flipbook's frame).
    @location(12) uv_rect: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) params: vec4<f32>,
    @location(4) uv: vec2<f32>,
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
    out.uv = v.uv * i.uv_rect.zw + i.uv_rect.xy;
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

// How much of shadow-casting light `slot` reaches a point: 1 lit, 0 in shadow. Nine taps of the
// shadow map (percentage-closer filtering) soften the edge.
fn shadow_factor(slot: u32, world_pos: vec3<f32>, n: vec3<f32>) -> f32 {
    let info = globals.shadow_lights[slot];
    let mode = globals.shadow_modes[slot].y;
    let first = i32(info.w);
    var face = 0;
    var bias = info.y;
    var offset_along_normal = info.z;
    if (mode > 1.5) {
        // Sun cascades: the slice of view depth the point lies in; past the last one, no shadow.
        // Each cascade's texel size, and so its bias, comes from its matrix.
        let depth = (globals.view_proj * vec4<f32>(world_pos, 1.0)).w;
        let splits = globals.shadow_splits[slot];
        let count = i32(globals.shadow_modes[slot].x);
        face = select(0, 1, depth >= splits.x) + select(0, 1, depth >= splits.y) + select(0, 1, depth >= splits.z) + select(0, 1, depth >= splits.w);
        if (face >= count) {
            return 1.0;
        }
        let m = globals.shadow_faces[first + face];
        let texel = 2.0 / max(length(m[0].xyz), 1e-6) / f32(textureDimensions(shadow_map).x);
        // How much depth one world unit along the light makes: the length of the matrix's depth row.
        bias = texel * length(vec3<f32>(m[0].z, m[1].z, m[2].z)) * 1.5;
        offset_along_normal = texel * 1.5;
    }
    // Pushing the point out along the normal keeps surfaces from shadowing themselves.
    let pushed = world_pos + n * offset_along_normal;
    // Around a point light, the face the point lies in: the main direction from the light.
    if (mode > 0.5 && mode < 1.5) {
        let d = pushed - globals.lights[u32(info.x)].position.xyz;
        let a = abs(d);
        if (a.x >= a.y && a.x >= a.z) {
            face = select(1, 0, d.x > 0.0);
        } else if (a.y >= a.z) {
            face = select(3, 2, d.y > 0.0);
        } else {
            face = select(5, 4, d.z > 0.0);
        }
    }
    let layer = first + face;
    let p = globals.shadow_faces[layer] * vec4<f32>(pushed, 1.0);
    // Behind a spot light: outside its shadow map.
    if (p.w <= 0.0) {
        return 1.0;
    }
    let ndc = p.xyz / p.w;
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z > 1.0) {
        return 1.0;
    }
    var lit = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let offset = vec2<f32>(f32(x), f32(y)) * globals.shadow_count.y;
            lit = lit + textureSampleCompareLevel(shadow_map, shadow_sampler, uv + offset, layer, ndc.z - bias);
        }
    }
    return lit / 9.0;
}

// Fog in the horizon colour: the further away, the more it hides (exponential with distance).
fn fogged(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    if (globals.fog.w <= 0.0) {
        return color;
    }
    let hidden = 1.0 - exp(-globals.fog.w * distance(globals.camera_pos.xyz, world_pos));
    return mix(color, globals.fog.rgb, hidden);
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
    // World-space shading normal, facing the camera's side.
    normal: vec3<f32>,
};

struct SurfaceIn {
    position: vec3<f32>,
    normal: vec3<f32>,
    // Towards the camera.
    view: vec3<f32>,
    color: vec4<f32>,
    params: vec4<f32>,
    time: f32,
    uv: vec2<f32>,
    // The main light (the first: suns come first): towards it, its colour and strength, and how
    // much of it reaches the point past shadows (1 lit). Filled only when the surface uses lights.
    light_dir: vec3<f32>,
    light_color: vec3<f32>,
    light_shadow: f32,
    // The other lights' diffuse light on the point, summed.
    other_light: vec3<f32>,
    // Where the pixel is on the screen, 0 to 1 from the top left.
    screen: vec2<f32>,
};

// How one light reaches a point: the direction towards it, and its strength there from distance,
// cone and shadow (0 to 1, times the light's radiance).
struct LightReach {
    dir: vec3<f32>,
    falloff: f32,
};

fn light_reach(index: u32, world_pos: vec3<f32>, geometric: vec3<f32>) -> LightReach {
    let light = globals.lights[index];
    var l = normalize(-light.direction.xyz);
    var falloff = 1.0;
    if (light.direction.w > 0.5) {
        // Point and spot lights: inverse-square falloff, windowed to reach 0 at the range
        // (the window from Karis 2013).
        let to_light = light.position.xyz - world_pos;
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
    for (var slot = 0u; slot < u32(globals.shadow_count.x); slot = slot + 1u) {
        if (f32(index) == globals.shadow_lights[slot].x) {
            falloff = falloff * shadow_factor(slot, world_pos, geometric);
        }
    }
    return LightReach(l, falloff);
}

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

// sRGB-encoded numbers from a texture that the sampler decoded as colour (normal and
// metal/roughness images are stored as plain numbers).
fn bl_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(high, low, c <= vec3<f32>(0.0031308));
}

// Bends normal n by a tangent-space normal map value. The tangent frame comes from how position
// and texture coordinates change across the screen, so models need no stored tangents (the
// cotangent frame of Schüler 2013, "Normal Mapping Without Precomputed Tangents"). Texture v
// runs down the image, so the map's green (up) is the frame's minus v direction.
fn bl_normal_map(n: vec3<f32>, p: vec3<f32>, uv: vec2<f32>, mapped: vec3<f32>) -> vec3<f32> {
    let dp1 = dpdx(p);
    let dp2 = dpdy(p);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let size = max(dot(t, t), dot(b, b));
    if (size < 1e-20) {
        return n;
    }
    let scale = inverseSqrt(size);
    // Screen y runs down in wgpu, which turns this frame over: both directions change sign.
    return normalize(-t * scale * mapped.x + b * scale * mapped.y + n * mapped.z);
}

fn bl_checker(p: vec3<f32>) -> f32 {
    // A small offset keeps faces that lie exactly on a cell border from flickering.
    let c = floor(p + vec3<f32>(1e-3));
    return f32(i32(c.x + c.y + c.z) & 1);
}

// @surface

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    let v = normalize(globals.camera_pos.xyz - in.world_pos);
    let count = min(u32(globals.ambient.w), MAX_LIGHTS);
    // The lights a stylised material reads, worked out only when it reads them.
    var light_dir = vec3<f32>(0.0, 1.0, 0.0);
    var light_color = vec3<f32>(0.0);
    var light_shadow = 1.0;
    var other_light = vec3<f32>(0.0);
    if (USES_LIGHTS) {
        if (count > 0u) {
            let main = light_reach(0u, in.world_pos, n);
            light_dir = main.dir;
            light_color = globals.lights[0].radiance.rgb;
            light_shadow = main.falloff;
        }
        for (var index = 1u; index < count; index = index + 1u) {
            let reach = light_reach(index, in.world_pos, n);
            other_light = other_light + globals.lights[index].radiance.rgb * max(dot(n, reach.dir), 0.0) * reach.falloff;
        }
    }
    let clip = globals.view_proj * vec4<f32>(in.world_pos, 1.0);
    let screen = vec2<f32>(clip.x / clip.w * 0.5 + 0.5, 0.5 - clip.y / clip.w * 0.5);
    // The surface is worked out before any branch that returns, so its images can pick their
    // smaller copies from how fast the texture coordinates change across the screen.
    let s = surface(SurfaceIn(in.world_pos, n, v, in.color, in.params, globals.camera_pos.w, in.uv, light_dir, light_color, light_shadow, other_light, screen));
    if (CUSTOM_LIGHTING) {
        // The material's own lighting: its colour as it is, fogged.
        return vec4<f32>(fogged(max(s.emissive, vec3<f32>(0.0)), in.world_pos), clamp(s.alpha, 0.0, 1.0));
    }
    if (in.params.w > 0.5) {
        // Sprites: their colour times their image, without light.
        return vec4<f32>(fogged(s.base, in.world_pos), s.alpha);
    }

    // Shadows test the geometric normal; light uses the shading normal.
    let geometric = n;
    n = normalize(s.normal);
    let base = max(s.base, vec3<f32>(0.0));
    let metallic = clamp(s.metallic, 0.0, 1.0);
    let roughness = clamp(s.roughness, 0.045, 1.0);
    let alpha = clamp(s.alpha, 0.0, 1.0);
    let f0 = mix(vec3<f32>(0.04), base, metallic);
    let n_dot_v = max(dot(n, v), 1e-4);
    let r = roughness + 1.0;
    let k = r * r / 8.0;

    // Hemisphere ambient, brighter from above, for the diffuse part.
    let ambient = mix(globals.ambient_ground.rgb, globals.ambient.rgb, n.y * 0.5 + 0.5);
    // Reflections: what lies in the mirror direction. A smooth surface sees the sky as drawn; a
    // rough one sees it blurred, which the hemisphere light stands in for. Rough surfaces also
    // brighten less at grazing angles (Fresnel with the roughness taken into account).
    let mirror = reflect(-v, n);
    let blurred = mix(globals.ambient_ground.rgb, globals.ambient.rgb, mirror.y * 0.5 + 0.5);
    var sharp = blurred;
    if (globals.sky_horizon.w > 0.5) {
        sharp = bl_sky(mirror);
    }
    let env = mix(sharp, blurred, roughness);
    let grazing = max(vec3<f32>(1.0 - roughness), f0) - f0;
    let ambient_f = f0 + grazing * pow(clamp(1.0 - n_dot_v, 0.0, 1.0), 5.0);
    var color = ambient * (vec3<f32>(1.0) - ambient_f) * (1.0 - metallic) * base + env * ambient_f;
    for (var index = 0u; index < count; index = index + 1u) {
        let light = globals.lights[index];
        let reach = light_reach(index, in.world_pos, geometric);
        let l = reach.dir;
        let falloff = reach.falloff;
        let h = normalize(v + l);
        let n_dot_l = max(dot(n, l), 0.0);
        let n_dot_h = max(dot(n, h), 0.0);
        let f = fresnel_schlick(max(dot(h, v), 0.0), f0);
        let g = geometry_schlick_ggx(n_dot_v, k) * geometry_schlick_ggx(n_dot_l, k);
        let specular = distribution_ggx(n_dot_h, roughness) * g * f / (4.0 * n_dot_v * n_dot_l + 1e-4);
        let diffuse = (vec3<f32>(1.0) - f) * (1.0 - metallic) * base / PI;
        color = color + (diffuse + specular) * light.radiance.rgb * n_dot_l * falloff;
    }

    color = fogged(color + max(s.emissive, vec3<f32>(0.0)), in.world_pos);
    return vec4<f32>(color, alpha);
}

// Outlines (a material's outline option): the back faces, grown along their normals by the
// outline's width, in its colour; the shape drawn over them leaves a rim around it.
@vertex
fn vs_outline(v: VertexIn, i: InstanceIn) -> VertexOut {
    let model = mat4x4<f32>(i.model_0, i.model_1, i.model_2, i.model_3);
    let normal_matrix = mat3x3<f32>(i.normal_0.xyz, i.normal_1.xyz, i.normal_2.xyz);
    var world = model * vec4<f32>(v.position, 1.0);
    let normal = normalize(normal_matrix * v.normal);
    world = vec4<f32>(world.xyz + normal * OUTLINE.w, 1.0);
    var out: VertexOut;
    out.clip = globals.view_proj * world;
    out.world_pos = world.xyz;
    out.normal = normal;
    out.color = i.color;
    out.params = i.params;
    out.uv = v.uv * i.uv_rect.zw + i.uv_rect.xy;
    return out;
}

@fragment
fn fs_outline(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(fogged(OUTLINE.rgb, in.world_pos), 1.0);
}
